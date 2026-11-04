//! The journey across linked item types: from a won deal to an order, an
//! invoice and a payment, say.
//!
//! Each stage is the step that typically starts one item type's history. The
//! chain starts at the followed type and continues through the types whose
//! histories typically start later, as long as consecutive stages are linked
//! by a shared item. Every figure comes from the engine's time-between
//! measurement through common items; nothing is estimated.

use crate::ocel::OcelQueryError;
use pm_core::{
    EventTypeId, ObjectTypeId, OcelEventId, OcelFilter, OcelLog, OcelView, measure_ocel_progress,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// At most this many stages are shown.
const MAX_STAGES: usize = 6;
const MICROS_PER_SECOND: i64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JourneyStage {
    pub object_type: String,
    /// The step that most often starts this type's history.
    pub activity: String,
    /// Items of this type with at least one recorded step.
    pub items: u64,
}

/// The hand-off from one stage's step to the next, measured through the
/// shared item type that links them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JourneyHandoff {
    /// Explicit type path from the root to this stage. Used to correlate
    /// filtered roots without expanding through unrelated shared masters.
    pub scope_path: Vec<String>,
    pub from_activity: String,
    pub to_activity: String,
    pub via_object_type: String,
    /// Shared items that record the earlier step.
    pub started: u64,
    /// Of those, the items that then record the later step.
    pub reached: u64,
    pub observations: u64,
    pub median_seconds: i64,
    pub p90_seconds: i64,
    /// What the items that did not reach the next stage are worth, when their
    /// type carries an amount.
    pub not_reached_value: Option<JourneyValue>,
}

/// A total of one amount field over a set of items.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JourneyValue {
    pub field: String,
    pub total: f64,
    /// Items whose latest value reads as a number; the total covers these.
    pub items_with_value: u64,
    pub items: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Journey {
    pub start_object_type: String,
    pub stages: Vec<JourneyStage>,
    /// `handoffs[i]` leads into `stages[i + 1]`, from the stage named by its
    /// `from_activity` (usually the one before it).
    pub handoffs: Vec<JourneyHandoff>,
    /// Items that stopped short of the next stage, as ranked findings.
    pub findings: Vec<crate::findings::KeyFinding>,
    pub semantics: String,
}

pub fn ocel_journey(
    path: impl AsRef<Path>,
    filter: &OcelFilter,
    start_object_type: &str,
) -> Result<Journey, OcelQueryError> {
    let path = path.as_ref();
    if crate::follow::from_disk(path) {
        return ocel_journey_disk(path, filter, start_object_type);
    }
    let log = pm_ingest::read_ocel(path)?;
    ocel_journey_log(&log, filter, start_object_type)
}

/// Remeasure an already established journey in a narrower cohort. A stage
/// remains visible when all its destinations are absent from that cohort.
pub fn measure_journey_plan(
    path: &Path,
    filter: &OcelFilter,
    plan: &Journey,
) -> Result<Journey, OcelQueryError> {
    if crate::follow::from_disk(path) {
        let log = crate::ocel::open_disk(path)?;
        let view = log.view(filter)?;
        let types = log
            .object_types
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), ObjectTypeId(i as u32)))
            .collect();
        let activities = log
            .event_types
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), EventTypeId(i as u32)))
            .collect();
        let mut counts = BTreeMap::new();
        log.for_each_timeline(&view, |_, kind, timeline| {
            if !timeline.is_empty() {
                *counts.entry(kind).or_insert(0) += 1;
            }
            Ok(())
        })
        .map_err(crate::ocel::disk_failed)?;
        let mut data = OnDisk {
            log: &log,
            view,
            unreached: BTreeMap::new(),
        };
        remeasure_plan(&mut data, plan, &types, &activities, &counts)
    } else {
        let log = pm_ingest::read_ocel(path)?;
        let view = OcelView::apply(&log, filter)?;
        let types = log
            .object_types
            .iter()
            .enumerate()
            .map(|(i, kind)| (kind.name.clone(), ObjectTypeId(i as u32)))
            .collect();
        let activities = log
            .event_types
            .iter()
            .enumerate()
            .map(|(i, kind)| (kind.name.clone(), EventTypeId(i as u32)))
            .collect();
        let objects = view
            .event_object_relations
            .iter()
            .map(|i| log.event_object_relations[*i].object)
            .collect::<std::collections::BTreeSet<_>>();
        let mut counts = BTreeMap::new();
        for object in objects {
            *counts
                .entry(log.objects[object.0 as usize].object_type)
                .or_insert(0) += 1;
        }
        let mut data = Loaded {
            log: &log,
            view: &view,
        };
        remeasure_plan(&mut data, plan, &types, &activities, &counts)
    }
}
fn remeasure_plan(
    data: &mut impl JourneyData,
    plan: &Journey,
    types: &BTreeMap<String, ObjectTypeId>,
    activities: &BTreeMap<String, EventTypeId>,
    counts: &BTreeMap<ObjectTypeId, u64>,
) -> Result<Journey, OcelQueryError> {
    let mut journey = plan.clone();
    let root = types
        .get(&plan.start_object_type)
        .ok_or_else(|| OcelQueryError::UnknownObjectType(plan.start_object_type.clone()))?;
    if counts.get(root).copied().unwrap_or(0) == 0 {
        journey.stages.clear();
        journey.handoffs.clear();
        journey.findings.clear();
        return Ok(journey);
    }
    for stage in &mut journey.stages {
        stage.items = types
            .get(&stage.object_type)
            .and_then(|kind| counts.get(kind))
            .copied()
            .unwrap_or(0);
    }
    let pairs = plan
        .handoffs
        .iter()
        .map(|handoff| {
            Ok((
                *activities.get(&handoff.from_activity).ok_or_else(|| {
                    OcelQueryError::UnknownActivity(handoff.from_activity.clone())
                })?,
                *activities
                    .get(&handoff.to_activity)
                    .ok_or_else(|| OcelQueryError::UnknownActivity(handoff.to_activity.clone()))?,
            ))
        })
        .collect::<Result<Vec<_>, OcelQueryError>>()?;
    let measured = data.measure(&pairs)?;
    for ((handoff, links), (from, to)) in journey.handoffs.iter_mut().zip(measured).zip(pairs) {
        if let Some(link) = links
            .iter()
            .find(|link| types.get(&handoff.via_object_type) == Some(&link.object_type))
        {
            handoff.started = link.source_objects;
            handoff.reached = link.objects;
            handoff.observations = link.observations;
            handoff.median_seconds = link.median_elapsed_micros / MICROS_PER_SECOND;
            handoff.p90_seconds = link.p90_elapsed_micros / MICROS_PER_SECOND;
            handoff.not_reached_value = data.value(from, to, link)?;
        } else {
            handoff.started = 0;
            handoff.reached = 0;
            handoff.observations = 0;
            handoff.median_seconds = 0;
            handoff.p90_seconds = 0;
            handoff.not_reached_value = None;
        }
    }
    journey.findings = crate::findings::journey_findings(&journey);
    journey.semantics.push_str(" The stage definitions come from the original linked scope; all counts and times are remeasured in the current selection, including stages reached by zero items.");
    Ok(journey)
}

pub fn ocel_journey_log(
    log: &OcelLog,
    filter: &OcelFilter,
    start_object_type: &str,
) -> Result<Journey, OcelQueryError> {
    let start = log
        .object_types
        .iter()
        .position(|candidate| candidate.name == start_object_type)
        .map(|index| ObjectTypeId(index as u32))
        .ok_or_else(|| OcelQueryError::UnknownObjectType(start_object_type.to_owned()))?;
    let view = OcelView::apply(log, filter)?;
    // Each item's first step, earliest time first and event order on ties.
    let mut first = BTreeMap::<pm_core::ObjectId, (i64, OcelEventId)>::new();
    let mut participation = BTreeMap::<(ObjectTypeId, EventTypeId), u64>::new();
    for index in &view.event_object_relations {
        let relation = log.event_object_relations[*index];
        let at = log.events[relation.event.0 as usize].at;
        first
            .entry(relation.object)
            .and_modify(|current| {
                if (at, relation.event) < *current {
                    *current = (at, relation.event);
                }
            })
            .or_insert((at, relation.event));
        *participation
            .entry((
                log.objects[relation.object.0 as usize].object_type,
                log.events[relation.event.0 as usize].event_type,
            ))
            .or_default() += 1;
    }
    let mut stats = Stats {
        participation,
        ..Default::default()
    };
    for (object, (_, event)) in &first {
        stats.first_step(
            log.objects[object.0 as usize].object_type,
            log.events[event.0 as usize].event_type,
        );
    }
    let mut data = Loaded { log, view: &view };
    journey(&mut data, start, start_object_type, &stats)
}

/// [`ocel_journey_log`] read from disk.
pub fn ocel_journey_disk(
    path: &Path,
    filter: &OcelFilter,
    start_object_type: &str,
) -> Result<Journey, OcelQueryError> {
    let log = crate::ocel::open_disk(path)?;
    let start = log
        .object_types
        .iter()
        .position(|candidate| candidate == start_object_type)
        .map(|index| ObjectTypeId(index as u32))
        .ok_or_else(|| OcelQueryError::UnknownObjectType(start_object_type.to_owned()))?;
    let view = log.view(filter)?;
    let mut stats = Stats::default();
    log.for_each_participation(&view, |_, event_type, object_type, _| {
        *stats
            .participation
            .entry((object_type, event_type))
            .or_default() += 1;
    })
    .map_err(crate::ocel::disk_failed)?;
    log.for_each_timeline(&view, |_, object_type, timeline| {
        if let Some((_, _, event_type)) = timeline.first() {
            stats.first_step(object_type, *event_type);
        }
        Ok(())
    })
    .map_err(crate::ocel::disk_failed)?;
    let mut data = OnDisk {
        log: &log,
        view,
        unreached: BTreeMap::new(),
    };
    journey(&mut data, start, start_object_type, &stats)
}

/// What the journey counts before measuring hand-offs.
#[derive(Default)]
struct Stats {
    /// How often each type takes part in each step, to recognise items that
    /// recur in many cases (products, people) rather than flow through one.
    participation: BTreeMap<(ObjectTypeId, EventTypeId), u64>,
    /// Per type: how often each step comes first.
    starts: BTreeMap<ObjectTypeId, BTreeMap<EventTypeId, u64>>,
    /// Items with at least one step, per type.
    items: BTreeMap<ObjectTypeId, u64>,
}

impl Stats {
    fn first_step(&mut self, object_type: ObjectTypeId, event_type: EventTypeId) {
        *self.items.entry(object_type).or_default() += 1;
        *self
            .starts
            .entry(object_type)
            .or_default()
            .entry(event_type)
            .or_default() += 1;
    }
}

/// Where the journey reads names, hand-off times and amounts.
trait JourneyData {
    fn type_name(&self, object_type: ObjectTypeId) -> String;
    fn activity_name(&self, event_type: EventTypeId) -> String;
    /// Time between each pair, in pair order.
    fn measure(
        &mut self,
        pairs: &[(EventTypeId, EventTypeId)],
    ) -> Result<Vec<Vec<pm_core::OcelTimeBetween>>, OcelQueryError>;
    /// What the items that never reached `to` from `from` are worth.
    fn value(
        &self,
        from: EventTypeId,
        to: EventTypeId,
        link: &pm_core::OcelTimeBetween,
    ) -> Result<Option<JourneyValue>, OcelQueryError>;
}

struct Loaded<'a> {
    log: &'a OcelLog,
    view: &'a OcelView,
}

impl JourneyData for Loaded<'_> {
    fn type_name(&self, object_type: ObjectTypeId) -> String {
        self.log
            .object_type(object_type)
            .map_or_else(String::new, |declared| declared.name.clone())
    }
    fn activity_name(&self, event_type: EventTypeId) -> String {
        self.log
            .event_type(event_type)
            .map_or_else(String::new, |declared| declared.name.clone())
    }
    fn measure(
        &mut self,
        pairs: &[(EventTypeId, EventTypeId)],
    ) -> Result<Vec<Vec<pm_core::OcelTimeBetween>>, OcelQueryError> {
        Ok(pairs
            .iter()
            .map(|(from, to)| measure_ocel_progress(self.log, self.view, *from, *to))
            .collect())
    }
    fn value(
        &self,
        _: EventTypeId,
        _: EventTypeId,
        link: &pm_core::OcelTimeBetween,
    ) -> Result<Option<JourneyValue>, OcelQueryError> {
        Ok(value_of(self.log, link.object_type, &link.unreached))
    }
}

struct OnDisk<'a> {
    log: &'a pm_ingest::DiskLog,
    view: pm_ingest::DiskView,
    /// Each measured pair's items that never reached the target, by type.
    unreached: BTreeMap<(EventTypeId, EventTypeId), pm_ingest::DiskList<'a>>,
}

impl JourneyData for OnDisk<'_> {
    fn type_name(&self, object_type: ObjectTypeId) -> String {
        self.log
            .object_types
            .get(object_type.0 as usize)
            .cloned()
            .unwrap_or_default()
    }
    fn activity_name(&self, event_type: EventTypeId) -> String {
        self.log
            .event_types
            .get(event_type.0 as usize)
            .cloned()
            .unwrap_or_default()
    }
    fn measure(
        &mut self,
        pairs: &[(EventTypeId, EventTypeId)],
    ) -> Result<Vec<Vec<pm_core::OcelTimeBetween>>, OcelQueryError> {
        let lists = pairs
            .iter()
            .map(|_| self.log.new_list())
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::ocel::disk_failed)?;
        let measured = crate::ocel::disk_time_between(
            self.log,
            &self.view,
            pairs,
            crate::ocel::ELAPSED_IN_MEMORY,
            Some(&lists),
            true,
        )?;
        for (pair, list) in pairs.iter().zip(lists) {
            self.unreached.insert(*pair, list);
        }
        Ok(measured)
    }
    fn value(
        &self,
        from: EventTypeId,
        to: EventTypeId,
        link: &pm_core::OcelTimeBetween,
    ) -> Result<Option<JourneyValue>, OcelQueryError> {
        let items = link.source_objects.saturating_sub(link.objects);
        if items == 0 {
            return Ok(None);
        }
        let Some(table) = self.log.object_tables().get(link.object_type.0 as usize) else {
            return Ok(None);
        };
        let Some(field) = money_column(
            table
                .attributes
                .iter()
                .map(|(name, value_type)| (name.as_str(), *value_type)),
        ) else {
            return Ok(None);
        };
        let Some(list) = self.unreached.get(&(from, to)) else {
            return Ok(None);
        };
        let numbers = self
            .log
            .latest_numbers(
                link.object_type,
                &field,
                list,
                i64::from(link.object_type.0),
            )
            .map_err(crate::ocel::disk_failed)?;
        Ok(total(field, numbers, items))
    }
}

fn journey(
    data: &mut dyn JourneyData,
    start: ObjectTypeId,
    start_object_type: &str,
    stats: &Stats,
) -> Result<Journey, OcelQueryError> {
    let Stats {
        participation,
        starts,
        items,
    } = stats;
    // The step that most often starts each type's history.
    let stage_of = |object_type: ObjectTypeId| {
        starts
            .get(&object_type)?
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(activity, _)| *activity)
    };
    if items.get(&start).copied().unwrap_or(0) == 0 {
        return Ok(Journey {
            start_object_type: start_object_type.to_owned(),
            stages: Vec::new(),
            handoffs: Vec::new(),
            findings: Vec::new(),
            semantics: "No items of the starting type match this scope.".into(),
        });
    }
    let start_activity = stage_of(start).ok_or_else(|| {
        OcelQueryError::UnknownObjectType(format!("{start_object_type} (no recorded steps)"))
    })?;

    // Grow the chain from the start. A type joins when a stage already in the
    // chain reaches its starting step through a shared item for at least a
    // fifth of the items that recorded that stage; rarer links are exceptions
    // (a dispute, a return), not the main journey. The type that arrives
    // soonest after the start joins first, so the stages read in time order.
    let mut pending = starts
        .keys()
        .filter(|object_type| **object_type != start)
        .filter_map(|object_type| stage_of(*object_type).map(|activity| (*object_type, activity)))
        .filter(|(_, activity)| *activity != start_activity)
        // An item that takes part in its own starting step about once flows
        // through the process; one that recurs in it (a product picked for
        // many orders) is reference data, not a stage of the journey.
        .filter(|(object_type, activity)| {
            let count = items.get(object_type).copied().unwrap_or(0);
            participation
                .get(&(*object_type, *activity))
                .is_some_and(|links| *links <= count.saturating_mul(2))
        })
        .collect::<Vec<_>>();
    let mut chain = vec![(start, start_activity, 0i64)];
    let mut scope_paths = vec![vec![start_object_type.to_owned()]];
    let mut links = Vec::new();
    let mut measured = BTreeMap::<(EventTypeId, EventTypeId), Vec<pm_core::OcelTimeBetween>>::new();
    while chain.len() < MAX_STAGES {
        let wanted = pending
            .iter()
            .flat_map(|(_, activity)| {
                chain
                    .iter()
                    .map(move |(_, from_activity, _)| (*from_activity, *activity))
            })
            .filter(|pair| !measured.contains_key(pair))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        for (pair, result) in wanted.iter().zip(data.measure(&wanted)?) {
            measured.insert(*pair, result);
        }
        let mut best: Option<(i64, usize, usize, pm_core::OcelTimeBetween)> = None;
        for (index, (_, activity)) in pending.iter().enumerate() {
            for (from, (_, from_activity, arrival)) in chain.iter().enumerate() {
                let Some(link) = measured[&(*from_activity, *activity)]
                    .iter()
                    // A shared customer/product can recur in the same start
                    // activity across different transactions. Only bind via
                    // objects for which this activity occurs exactly once,
                    // as their first event; otherwise the hand-off is not an
                    // unambiguous document-instance measurement.
                    .filter(|link| {
                        let firsts = starts
                            .get(&link.object_type)
                            .and_then(|steps| steps.get(from_activity))
                            .copied()
                            .unwrap_or(0);
                        firsts > 0
                            && participation
                                .get(&(link.object_type, *from_activity))
                                .copied()
                                .unwrap_or(0)
                                == firsts
                    })
                    .filter(|link| link.objects.saturating_mul(5) >= link.source_objects)
                    .max_by_key(|link| (link.objects, std::cmp::Reverse(link.object_type)))
                else {
                    continue;
                };
                let at = arrival.saturating_add(link.median_elapsed_micros);
                if best.as_ref().is_none_or(|(current, ..)| at < *current) {
                    best = Some((at, index, from, link.clone()));
                }
            }
        }
        let Some((at, index, from, link)) = best else {
            break;
        };
        let (object_type, activity) = pending.remove(index);
        let mut path = scope_paths[from].clone();
        for kind in [
            data.type_name(link.object_type),
            data.type_name(object_type),
        ] {
            if let Some(index) = path.iter().position(|existing| existing == &kind) {
                path.truncate(index + 1);
            } else {
                path.push(kind);
            }
        }
        scope_paths.push(path);
        links.push((chain[from].1, activity, link));
        chain.push((object_type, activity, at));
    }
    // Present stages in arrival order, each with the hand-off that reaches it.
    let mut order = (1..chain.len()).collect::<Vec<_>>();
    order.sort_by_key(|index| (chain[*index].2, *index));
    let mut stages = vec![JourneyStage {
        object_type: data.type_name(start),
        activity: data.activity_name(start_activity),
        items: items.get(&start).copied().unwrap_or(0),
    }];
    let mut handoffs = Vec::new();
    for index in order {
        let (object_type, activity, _) = chain[index];
        let (from, to, link) = &links[index - 1];
        stages.push(JourneyStage {
            object_type: data.type_name(object_type),
            activity: data.activity_name(activity),
            items: items.get(&object_type).copied().unwrap_or(0),
        });
        handoffs.push(JourneyHandoff {
            scope_path: scope_paths[index].clone(),
            from_activity: data.activity_name(*from),
            to_activity: data.activity_name(*to),
            via_object_type: data.type_name(link.object_type),
            started: link.source_objects,
            reached: link.objects,
            observations: link.observations,
            median_seconds: link.median_elapsed_micros / MICROS_PER_SECOND,
            p90_seconds: link.p90_elapsed_micros / MICROS_PER_SECOND,
            not_reached_value: data.value(*from, *to, link)?,
        });
    }
    let mut journey = Journey {
        start_object_type: start_object_type.to_owned(),
        stages,
        handoffs,
        findings: Vec::new(),
        semantics: "Each stage is the step that most often starts one item type's history. A hand-off is measured on the shared items that link the two steps, from the latest earlier step to the later one; it is elapsed time, which includes waiting and work. Items that record the earlier step but never the later one have not reached the next stage in this data.".to_owned(),
    };
    journey.findings = crate::findings::journey_findings(&journey);
    Ok(journey)
}

/// The amount field among declared attributes: the first whose name says it
/// is money; a declared number beats a text field that may hold one.
/// Quantities and weights are not summed as value.
fn money_column<'a>(
    declared: impl Iterator<Item = (&'a str, pm_core::AttributeType)>,
) -> Option<String> {
    let money = [
        "amount", "value", "price", "total", "gross", "revenue", "cost",
    ];
    let candidates = declared
        .filter(|(_, value_type)| {
            matches!(
                value_type,
                pm_core::AttributeType::Integer
                    | pm_core::AttributeType::Float
                    | pm_core::AttributeType::String
            )
        })
        .filter(|(name, _)| {
            let name = name.to_lowercase();
            money.iter().any(|word| name.contains(word))
        })
        .collect::<Vec<_>>();
    candidates
        .iter()
        .find(|(_, value_type)| *value_type != pm_core::AttributeType::String)
        .or_else(|| candidates.first())
        .map(|(name, _)| (*name).to_owned())
}

/// The amount field of an item type.
fn money_field(log: &OcelLog, object_type: ObjectTypeId) -> Option<pm_core::AttributeNameId> {
    let declared = log.object_type(object_type)?;
    let name = money_column(declared.attributes.iter().filter_map(|definition| {
        Some((
            log.attribute_names.label(definition.name.0)?,
            definition.value_type,
        ))
    }))?;
    declared
        .attributes
        .iter()
        .find(|definition| log.attribute_names.label(definition.name.0) == Some(name.as_str()))
        .map(|definition| definition.name)
}

/// The total of the numbers; absent when every amount is zero, which says
/// nothing about money.
fn total(field: String, numbers: Vec<f64>, items: u64) -> Option<JourneyValue> {
    if numbers.iter().all(|number| *number == 0.0) {
        return None;
    }
    Some(JourneyValue {
        field,
        total: numbers.iter().sum(),
        items_with_value: numbers.len() as u64,
        items,
    })
}

/// Sums the latest value of the type's amount field over `objects`. Absent
/// when the type has no amount field or every amount is zero.
fn value_of(
    log: &OcelLog,
    object_type: ObjectTypeId,
    objects: &[pm_core::ObjectId],
) -> Option<JourneyValue> {
    if objects.is_empty() {
        return None;
    }
    let field = money_field(log, object_type)?;
    let wanted = objects
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let mut latest = BTreeMap::<pm_core::ObjectId, (i64, usize, pm_core::AttributeValue)>::new();
    for (index, attribute) in log.object_attributes.iter().enumerate() {
        if attribute.name != field || !wanted.contains(&attribute.object) {
            continue;
        }
        let entry =
            latest
                .entry(attribute.object)
                .or_insert((attribute.at, index, attribute.value));
        if (attribute.at, index) >= (entry.0, entry.1) {
            *entry = (attribute.at, index, attribute.value);
        }
    }
    let numbers = latest
        .values()
        .filter_map(|(_, _, value)| match value {
            pm_core::AttributeValue::Integer(number) => Some(*number as f64),
            pm_core::AttributeValue::Float(number) => Some(*number).filter(|n| n.is_finite()),
            pm_core::AttributeValue::String(id) => log
                .attribute_values
                .label(id.0)
                .and_then(pm_core::parse_number),
            _ => None,
        })
        .collect::<Vec<_>>();
    total(
        log.attribute_names.label(field.0)?.to_owned(),
        numbers,
        objects.len() as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::OcelBuilder;

    /// Deal d1 is won on day 0, ordered on day 2, invoiced on day 5; deal d2
    /// is won but never ordered.
    fn log() -> OcelLog {
        let day = 86_400 * MICROS_PER_SECOND;
        let mut builder = OcelBuilder::new();
        for activity in ["Deal won", "Order created", "Invoice issued"] {
            builder.declare_event_type(activity, vec![]).unwrap();
        }
        for object_type in ["deal", "order", "invoice"] {
            builder.declare_object_type(object_type, vec![]).unwrap();
        }
        for (id, object_type) in [
            ("d1", "deal"),
            ("d2", "deal"),
            ("o1", "order"),
            ("i1", "invoice"),
        ] {
            builder.add_object(id, object_type).unwrap();
        }
        for (event, activity, at, objects) in [
            ("e1", "Deal won", 0, &["d1"][..]),
            ("e2", "Deal won", day, &["d2"][..]),
            ("e3", "Order created", 2 * day, &["o1", "d1"][..]),
            ("e4", "Invoice issued", 5 * day, &["i1", "o1"][..]),
        ] {
            builder.add_event(event, activity, at).unwrap();
            for object in objects {
                builder
                    .relate_event_object(event, object, "linked")
                    .unwrap();
            }
        }
        builder.build()
    }

    #[test]
    fn the_chain_follows_linked_stages_with_engine_hand_off_times() {
        let journey = ocel_journey_log(&log(), &OcelFilter::default(), "deal").unwrap();
        let stages = journey
            .stages
            .iter()
            .map(|stage| {
                (
                    stage.object_type.as_str(),
                    stage.activity.as_str(),
                    stage.items,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            stages,
            [
                ("deal", "Deal won", 2),
                ("order", "Order created", 1),
                ("invoice", "Invoice issued", 1)
            ]
        );
        let first = &journey.handoffs[0];
        assert_eq!(
            (first.via_object_type.as_str(), first.started, first.reached),
            ("deal", 2, 1)
        );
        assert_eq!(first.median_seconds, 2 * 86_400);
        assert_eq!(journey.handoffs[1].via_object_type, "order");
        assert_eq!(journey.handoffs[1].median_seconds, 3 * 86_400);
    }

    #[test]
    fn items_that_recur_in_many_steps_are_not_stages() {
        let day = 86_400 * MICROS_PER_SECOND;
        let mut builder = OcelBuilder::new();
        builder.declare_event_type("Order created", vec![]).unwrap();
        builder.declare_event_type("Pick list", vec![]).unwrap();
        builder.declare_object_type("order", vec![]).unwrap();
        builder.declare_object_type("product", vec![]).unwrap();
        builder.add_object("p", "product").unwrap();
        for n in 0..3 {
            let order = format!("o{n}");
            builder.add_object(&order, "order").unwrap();
            builder
                .add_event(&format!("c{n}"), "Order created", n * day)
                .unwrap();
            builder
                .relate_event_object(&format!("c{n}"), &order, "o")
                .unwrap();
            builder
                .add_event(&format!("p{n}"), "Pick list", n * day + 1)
                .unwrap();
            builder
                .relate_event_object(&format!("p{n}"), &order, "o")
                .unwrap();
            builder
                .relate_event_object(&format!("p{n}"), "p", "picked")
                .unwrap();
        }
        let journey = ocel_journey_log(&builder.build(), &OcelFilter::default(), "order").unwrap();
        assert_eq!(
            journey.stages.len(),
            1,
            "the one product recurs in every pick list"
        );
    }

    #[test]
    fn items_that_stop_short_carry_their_amount() {
        let day = 86_400 * MICROS_PER_SECOND;
        let mut builder = OcelBuilder::new();
        builder
            .declare_event_type("Invoice issued", vec![])
            .unwrap();
        builder
            .declare_event_type("Payment received", vec![])
            .unwrap();
        let amount = builder.attribute_name("invoice_amount");
        builder
            .declare_object_type(
                "invoice",
                vec![pm_core::AttributeDefinition {
                    name: amount,
                    value_type: pm_core::AttributeType::Float,
                }],
            )
            .unwrap();
        builder.declare_object_type("payment", vec![]).unwrap();
        for (n, value) in [(0, 100.0), (1, 250.5), (2, 40.0)] {
            let invoice = format!("i{n}");
            builder.add_object(&invoice, "invoice").unwrap();
            builder
                .add_object_attribute(&invoice, amount, 0, pm_core::AttributeValue::Float(1.0))
                .unwrap();
            // The amount was corrected later; the latest value counts.
            builder
                .add_object_attribute(&invoice, amount, day, pm_core::AttributeValue::Float(value))
                .unwrap();
            builder
                .add_event(&format!("x{n}"), "Invoice issued", n * day)
                .unwrap();
            builder
                .relate_event_object(&format!("x{n}"), &invoice, "i")
                .unwrap();
        }
        // Only invoice i0 is paid.
        builder.add_object("p0", "payment").unwrap();
        builder
            .add_event("pay", "Payment received", 5 * day)
            .unwrap();
        builder.relate_event_object("pay", "i0", "i").unwrap();
        builder.relate_event_object("pay", "p0", "p").unwrap();
        let journey =
            ocel_journey_log(&builder.build(), &OcelFilter::default(), "invoice").unwrap();
        let open = journey.handoffs[0].not_reached_value.as_ref().unwrap();
        assert_eq!(open.field, "invoice_amount");
        assert_eq!(
            (open.total, open.items_with_value, open.items),
            (290.5, 2, 2)
        );
    }

    #[test]
    fn types_without_an_amount_field_carry_no_value() {
        let journey = ocel_journey_log(&log(), &OcelFilter::default(), "deal").unwrap();
        assert_eq!(journey.handoffs[0].not_reached_value, None);
    }

    #[test]
    fn a_later_start_shows_only_the_rest_of_the_chain() {
        let journey = ocel_journey_log(&log(), &OcelFilter::default(), "order").unwrap();
        assert_eq!(journey.stages.len(), 2);
        assert_eq!(journey.stages[1].object_type, "invoice");
    }
}
