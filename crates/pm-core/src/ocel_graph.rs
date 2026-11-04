//! Evidence-backed object-centric process and interaction graphs.

use crate::{
    EventTypeId, Micros, ObjectId, ObjectTypeId, OcelEventId, OcelLog, OcelView, QualifierId,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const EVIDENCE_LIMIT: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelActivityNode {
    pub event_type: EventTypeId,
    pub events: u64,
    pub evidence_events: Vec<OcelEventId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OcelDirectlyFollowsEvidence {
    pub object: ObjectId,
    pub source_event: OcelEventId,
    pub target_event: OcelEventId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelDirectlyFollowsEdge {
    pub object_type: ObjectTypeId,
    pub source: EventTypeId,
    pub target: EventTypeId,
    pub occurrences: u64,
    pub objects: u64,
    /// Start-to-start elapsed time between adjacent lifecycle layers. OCEL
    /// events do not carry end timestamps, so this is never labelled waiting.
    pub total_elapsed_micros: i64,
    pub mean_elapsed_micros: i64,
    pub min_elapsed_micros: i64,
    pub max_elapsed_micros: i64,
    pub evidence: Vec<OcelDirectlyFollowsEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelParticipationEdge {
    pub event_type: EventTypeId,
    pub object_type: ObjectTypeId,
    pub qualifier: QualifierId,
    pub occurrences: u64,
    pub evidence_events: Vec<OcelEventId>,
}

/// Undirected object-type interaction caused by participation in one event.
/// `object_pairs` retains multiplicity without materializing a quadratic list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelInteractionEdge {
    pub left: ObjectTypeId,
    pub right: ObjectTypeId,
    pub events: u64,
    pub object_pairs: u64,
    pub evidence_events: Vec<OcelEventId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelRelationshipEdge {
    pub source: ObjectTypeId,
    pub target: ObjectTypeId,
    pub qualifier: QualifierId,
    pub occurrences: u64,
    pub evidence_relations: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelGraphs {
    pub activities: Vec<OcelActivityNode>,
    pub directly_follows: Vec<OcelDirectlyFollowsEdge>,
    pub participation: Vec<OcelParticipationEdge>,
    pub interactions: Vec<OcelInteractionEdge>,
    pub relationships: Vec<OcelRelationshipEdge>,
}

impl OcelGraphs {
    pub fn discover(log: &OcelLog, view: &OcelView) -> Self {
        let mut fold = OcelGraphFold::default();
        for event in &view.events {
            fold.event(*event, log.events[event.0 as usize].event_type);
        }
        for_each_timeline(log, view, |object, object_type, timeline| {
            fold.lifecycle(object, object_type, timeline);
        });
        for index in &view.event_object_relations {
            let relation = log.event_object_relations[*index];
            fold.participation(
                relation.event,
                log.events[relation.event.0 as usize].event_type,
                log.objects[relation.object.0 as usize].object_type,
                relation.qualifier,
            );
        }
        let mut event_objects = view
            .event_object_relations
            .iter()
            .map(|index| {
                let relation = log.event_object_relations[*index];
                (relation.event, relation.object)
            })
            .collect::<Vec<_>>();
        event_objects.sort_unstable();
        event_objects.dedup();
        let mut types = Vec::new();
        for run in event_objects.chunk_by(|left, right| left.0 == right.0) {
            types.clear();
            types.extend(
                run.iter()
                    .map(|(_, object)| log.objects[object.0 as usize].object_type),
            );
            fold.event_objects(run[0].0, &types);
        }
        for index in &view.object_object_relations {
            let relation = log.object_object_relations[*index];
            fold.relationship(
                *index,
                log.objects[relation.source.0 as usize].object_type,
                log.objects[relation.target.0 as usize].object_type,
                relation.qualifier,
            );
        }
        fold.finish()
    }
}

/// One step of an object's timeline: when, which event, and its type. A
/// timeline is sorted by time, then event, with each event once.
pub type TimelineStep = (Micros, OcelEventId, EventTypeId);

/// Calls `f` with every object of the view, in object order, and its
/// timeline; an object without events gets an empty timeline.
pub fn for_each_timeline(
    log: &OcelLog,
    view: &OcelView,
    mut f: impl FnMut(ObjectId, ObjectTypeId, &[TimelineStep]),
) {
    let mut records = view
        .event_object_relations
        .iter()
        .map(|index| {
            let relation = log.event_object_relations[*index];
            (
                relation.object,
                log.events[relation.event.0 as usize].at,
                relation.event,
            )
        })
        .collect::<Vec<_>>();
    records.sort_unstable();
    // Several qualified rows may connect the same event and object. They are
    // distinct participation evidence but one lifecycle occurrence.
    records.dedup_by(|left, right| left.0 == right.0 && left.2 == right.2);
    let mut timeline = Vec::new();
    let mut start = 0;
    for object in &view.objects {
        while start < records.len() && records[start].0 < *object {
            start += 1;
        }
        timeline.clear();
        while start < records.len() && records[start].0 == *object {
            let (_, at, event) = records[start];
            timeline.push((at, event, log.events[event.0 as usize].event_type));
            start += 1;
        }
        f(
            *object,
            log.objects[object.0 as usize].object_type,
            &timeline,
        );
    }
}

/// The graphs, built from the log one piece at a time: events in order,
/// object timelines in object order, participation rows in file order, each
/// event's objects in event order, and relationships in file order. The same
/// pieces read from memory or from disk give the same graphs.
#[derive(Default)]
pub struct OcelGraphFold {
    activities: BTreeMap<EventTypeId, EvidenceAggregate>,
    directly_follows: BTreeMap<(ObjectTypeId, EventTypeId, EventTypeId), DfgAggregate>,
    participation: BTreeMap<(EventTypeId, ObjectTypeId, QualifierId), EvidenceAggregate>,
    interactions: BTreeMap<(ObjectTypeId, ObjectTypeId), InteractionAggregate>,
    relationships: BTreeMap<(ObjectTypeId, ObjectTypeId, QualifierId), RelationshipAggregate>,
}

#[derive(Default)]
struct DfgAggregate {
    occurrences: u64,
    objects: u64,
    total_elapsed_micros: i128,
    min_elapsed_micros: Option<i64>,
    max_elapsed_micros: Option<i64>,
    evidence: Vec<OcelDirectlyFollowsEvidence>,
}

#[derive(Default)]
struct EvidenceAggregate {
    occurrences: u64,
    evidence: Vec<OcelEventId>,
}

#[derive(Default)]
struct InteractionAggregate {
    events: u64,
    object_pairs: u64,
    evidence: Vec<OcelEventId>,
}

#[derive(Default)]
struct RelationshipAggregate {
    occurrences: u64,
    evidence: Vec<usize>,
}

/// A timeline's steps of one instant, grouped by type, events in order.
fn group_layer(layer: &[TimelineStep]) -> BTreeMap<EventTypeId, Vec<OcelEventId>> {
    let mut grouped: BTreeMap<EventTypeId, Vec<OcelEventId>> = BTreeMap::new();
    for (_, event, event_type) in layer {
        grouped.entry(*event_type).or_default().push(*event);
    }
    grouped
}

impl OcelGraphFold {
    /// An event of the view, in event order.
    pub fn event(&mut self, event: OcelEventId, event_type: EventTypeId) {
        let aggregate = self.activities.entry(event_type).or_default();
        aggregate.occurrences = aggregate.occurrences.saturating_add(1);
        push_unique(&mut aggregate.evidence, event);
    }

    /// An object's timeline, in object order.
    pub fn lifecycle(
        &mut self,
        object_id: ObjectId,
        object_type: ObjectTypeId,
        timeline: &[TimelineStep],
    ) {
        let mut seen_edges = BTreeSet::new();
        let mut previous: Option<&[TimelineStep]> = None;
        for layer in timeline.chunk_by(|left, right| left.0 == right.0) {
            let Some(source_layer) = previous else {
                previous = Some(layer);
                continue;
            };
            let elapsed = layer[0].0.saturating_sub(source_layer[0].0);
            let sources = group_layer(source_layer);
            let targets = group_layer(layer);
            for (source_type, source_events) in &sources {
                for (target_type, target_events) in &targets {
                    let key = (object_type, *source_type, *target_type);
                    let aggregate = self.directly_follows.entry(key).or_default();
                    let occurrences =
                        count(source_events.len()).saturating_mul(count(target_events.len()));
                    aggregate.occurrences = aggregate.occurrences.saturating_add(occurrences);
                    aggregate.total_elapsed_micros = aggregate.total_elapsed_micros.saturating_add(
                        i128::from(elapsed).saturating_mul(i128::from(occurrences)),
                    );
                    aggregate.min_elapsed_micros = Some(
                        aggregate
                            .min_elapsed_micros
                            .map_or(elapsed, |current| current.min(elapsed)),
                    );
                    aggregate.max_elapsed_micros = Some(
                        aggregate
                            .max_elapsed_micros
                            .map_or(elapsed, |current| current.max(elapsed)),
                    );
                    if seen_edges.insert((*source_type, *target_type)) {
                        aggregate.objects = aggregate.objects.saturating_add(1);
                    }
                    for source_event in source_events {
                        for target_event in target_events {
                            if aggregate.evidence.len() == EVIDENCE_LIMIT {
                                break;
                            }
                            aggregate.evidence.push(OcelDirectlyFollowsEvidence {
                                object: object_id,
                                source_event: *source_event,
                                target_event: *target_event,
                            });
                        }
                        if aggregate.evidence.len() == EVIDENCE_LIMIT {
                            break;
                        }
                    }
                }
            }
            previous = Some(layer);
        }
    }

    /// One event-to-object row of the view, in file order.
    pub fn participation(
        &mut self,
        event: OcelEventId,
        event_type: EventTypeId,
        object_type: ObjectTypeId,
        qualifier: QualifierId,
    ) {
        let aggregate = self
            .participation
            .entry((event_type, object_type, qualifier))
            .or_default();
        aggregate.occurrences = aggregate.occurrences.saturating_add(1);
        push_unique(&mut aggregate.evidence, event);
    }

    /// An event's distinct objects' types, one per object, in event order.
    pub fn event_objects(&mut self, event: OcelEventId, object_types: &[ObjectTypeId]) {
        let mut by_type = BTreeMap::<ObjectTypeId, u64>::new();
        for object_type in object_types {
            *by_type.entry(*object_type).or_default() += 1;
        }
        let groups = by_type.into_iter().collect::<Vec<_>>();
        for left_index in 0..groups.len() {
            for right_index in left_index..groups.len() {
                let (left_type, left_objects) = &groups[left_index];
                let (right_type, right_objects) = &groups[right_index];
                let pairs = if left_index == right_index {
                    choose_two(*left_objects)
                } else {
                    left_objects.saturating_mul(*right_objects)
                };
                if pairs == 0 {
                    continue;
                }
                let aggregate = self
                    .interactions
                    .entry((*left_type, *right_type))
                    .or_default();
                aggregate.events = aggregate.events.saturating_add(1);
                aggregate.object_pairs = aggregate.object_pairs.saturating_add(pairs);
                push_unique(&mut aggregate.evidence, event);
            }
        }
    }

    /// One object-to-object row of the view, by its index in file order.
    pub fn relationship(
        &mut self,
        index: usize,
        source: ObjectTypeId,
        target: ObjectTypeId,
        qualifier: QualifierId,
    ) {
        let aggregate = self
            .relationships
            .entry((source, target, qualifier))
            .or_default();
        aggregate.occurrences = aggregate.occurrences.saturating_add(1);
        if aggregate.evidence.len() < EVIDENCE_LIMIT {
            aggregate.evidence.push(index);
        }
    }

    pub fn finish(self) -> OcelGraphs {
        OcelGraphs {
            activities: self
                .activities
                .into_iter()
                .map(|(event_type, aggregate)| OcelActivityNode {
                    event_type,
                    events: aggregate.occurrences,
                    evidence_events: aggregate.evidence,
                })
                .collect(),
            directly_follows: self
                .directly_follows
                .into_iter()
                .map(
                    |((object_type, source, target), aggregate)| OcelDirectlyFollowsEdge {
                        object_type,
                        source,
                        target,
                        occurrences: aggregate.occurrences,
                        objects: aggregate.objects,
                        total_elapsed_micros: clamp_i128(aggregate.total_elapsed_micros),
                        mean_elapsed_micros: mean_elapsed(
                            aggregate.total_elapsed_micros,
                            aggregate.occurrences,
                        ),
                        min_elapsed_micros: aggregate.min_elapsed_micros.unwrap_or(0),
                        max_elapsed_micros: aggregate.max_elapsed_micros.unwrap_or(0),
                        evidence: aggregate.evidence,
                    },
                )
                .collect(),
            participation: self
                .participation
                .into_iter()
                .map(
                    |((event_type, object_type, qualifier), aggregate)| OcelParticipationEdge {
                        event_type,
                        object_type,
                        qualifier,
                        occurrences: aggregate.occurrences,
                        evidence_events: aggregate.evidence,
                    },
                )
                .collect(),
            interactions: self
                .interactions
                .into_iter()
                .map(|((left, right), aggregate)| OcelInteractionEdge {
                    left,
                    right,
                    events: aggregate.events,
                    object_pairs: aggregate.object_pairs,
                    evidence_events: aggregate.evidence,
                })
                .collect(),
            relationships: self
                .relationships
                .into_iter()
                .map(
                    |((source, target, qualifier), aggregate)| OcelRelationshipEdge {
                        source,
                        target,
                        qualifier,
                        occurrences: aggregate.occurrences,
                        evidence_relations: aggregate.evidence,
                    },
                )
                .collect(),
        }
    }
}

fn clamp_i128(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value.is_negative() {
        i64::MIN
    } else {
        i64::MAX
    })
}

fn push_unique<T: Copy + PartialEq>(evidence: &mut Vec<T>, value: T) {
    if evidence.len() < EVIDENCE_LIMIT && !evidence.contains(&value) {
        evidence.push(value);
    }
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn choose_two(value: u64) -> u64 {
    value.saturating_mul(value.saturating_sub(1)) / 2
}

fn mean_elapsed(total: i128, occurrences: u64) -> i64 {
    if occurrences == 0 {
        return 0;
    }
    i64::try_from(total / i128::from(occurrences)).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OcelBuilder, OcelFilter};

    fn log() -> OcelLog {
        let mut builder = OcelBuilder::new();
        builder.declare_event_type("place", vec![]).unwrap();
        builder.declare_event_type("pack", vec![]).unwrap();
        builder.declare_event_type("deliver", vec![]).unwrap();
        builder.declare_object_type("order", vec![]).unwrap();
        builder.declare_object_type("item", vec![]).unwrap();
        builder.declare_object_type("package", vec![]).unwrap();
        builder.add_event("e1", "place", 1).unwrap();
        builder.add_event("e2", "pack", 2).unwrap();
        builder.add_event("e3", "deliver", 3).unwrap();
        builder.add_object("o1", "order").unwrap();
        builder.add_object("i1", "item").unwrap();
        builder.add_object("i2", "item").unwrap();
        builder.add_object("p1", "package").unwrap();
        for object in ["o1", "i1", "i2"] {
            builder.relate_event_object("e1", object, "input").unwrap();
        }
        for object in ["i1", "i2", "p1"] {
            builder.relate_event_object("e2", object, "input").unwrap();
        }
        builder.relate_event_object("e3", "p1", "input").unwrap();
        builder.relate_objects("i1", "o1", "part of").unwrap();
        builder.relate_objects("i2", "o1", "part of").unwrap();
        builder.relate_objects("i1", "p1", "packed in").unwrap();
        builder.build()
    }

    fn graphs(log: &OcelLog) -> OcelGraphs {
        let view = OcelView::apply(log, &OcelFilter::default()).unwrap();
        OcelGraphs::discover(log, &view)
    }

    #[test]
    fn activity_events_are_counted_once_despite_multiple_objects() {
        let graphs = graphs(&log());
        assert_eq!(graphs.activities.len(), 3);
        assert!(graphs.activities.iter().all(|node| node.events == 1));
    }

    #[test]
    fn directly_follows_is_scoped_by_object_type() {
        let graphs = graphs(&log());
        assert!(graphs.directly_follows.iter().any(|edge| {
            edge.object_type == ObjectTypeId(1)
                && edge.source == EventTypeId(0)
                && edge.target == EventTypeId(1)
                && edge.occurrences == 2
                && edge.objects == 2
                && edge.mean_elapsed_micros == 1
                && edge.min_elapsed_micros == 1
                && edge.max_elapsed_micros == 1
        }));
        assert!(graphs.directly_follows.iter().any(|edge| {
            edge.object_type == ObjectTypeId(2)
                && edge.source == EventTypeId(1)
                && edge.target == EventTypeId(2)
        }));
    }

    #[test]
    fn interactions_count_events_and_object_pairs_without_materializing_pairs() {
        let graphs = graphs(&log());
        let order_items = graphs
            .interactions
            .iter()
            .find(|edge| edge.left == ObjectTypeId(0) && edge.right == ObjectTypeId(1))
            .unwrap();
        assert_eq!(order_items.events, 1);
        assert_eq!(order_items.object_pairs, 2);
        let item_items = graphs
            .interactions
            .iter()
            .find(|edge| edge.left == ObjectTypeId(1) && edge.right == ObjectTypeId(1))
            .unwrap();
        assert_eq!(item_items.events, 2);
        assert_eq!(item_items.object_pairs, 2);
    }

    #[test]
    fn qualified_participation_and_explicit_relationships_stay_distinct() {
        let graphs = graphs(&log());
        assert_eq!(graphs.participation.len(), 5);
        assert_eq!(graphs.relationships.len(), 2);
        assert_eq!(graphs.relationships[0].occurrences, 2);
        assert_eq!(graphs.relationships[0].evidence_relations, vec![0, 1]);
    }

    #[test]
    fn equal_timestamp_events_do_not_get_an_invented_order() {
        let mut builder = OcelBuilder::new();
        builder.declare_event_type("a", vec![]).unwrap();
        builder.declare_event_type("b", vec![]).unwrap();
        builder.declare_object_type("order", vec![]).unwrap();
        builder.add_event("e1", "a", 1).unwrap();
        builder.add_event("e2", "b", 1).unwrap();
        builder.add_object("o1", "order").unwrap();
        builder.relate_event_object("e1", "o1", "order").unwrap();
        builder.relate_event_object("e2", "o1", "order").unwrap();
        let log = builder.build();
        assert!(graphs(&log).directly_follows.is_empty());
    }

    #[test]
    fn several_qualifiers_do_not_duplicate_one_object_lifecycle() {
        let mut builder = OcelBuilder::new();
        builder.declare_event_type("a", vec![]).unwrap();
        builder.declare_event_type("b", vec![]).unwrap();
        builder.declare_object_type("order", vec![]).unwrap();
        builder.add_event("e1", "a", 1).unwrap();
        builder.add_event("e2", "b", 2).unwrap();
        builder.add_object("o1", "order").unwrap();
        builder.relate_event_object("e1", "o1", "input").unwrap();
        builder.relate_event_object("e1", "o1", "owner").unwrap();
        builder.relate_event_object("e2", "o1", "output").unwrap();
        builder.relate_event_object("e2", "o1", "owner").unwrap();

        let graphs = graphs(&builder.build());
        assert_eq!(graphs.directly_follows.len(), 1);
        assert_eq!(graphs.directly_follows[0].occurrences, 1);
        assert_eq!(graphs.directly_follows[0].objects, 1);
        assert!(graphs.interactions.is_empty());
        assert_eq!(graphs.participation.len(), 4);
    }

    #[test]
    fn lifecycle_elapsed_time_is_weighted_and_never_called_waiting() {
        let mut builder = OcelBuilder::new();
        builder.declare_event_type("a", vec![]).unwrap();
        builder.declare_event_type("b", vec![]).unwrap();
        builder.declare_object_type("order", vec![]).unwrap();
        for (event, activity, at) in [
            ("a1", "a", 0),
            ("b1", "b", 10),
            ("a2", "a", 20),
            ("b2", "b", 50),
        ] {
            builder.add_event(event, activity, at).unwrap();
        }
        for object in ["o1", "o2"] {
            builder.add_object(object, "order").unwrap();
        }
        for (event, object) in [("a1", "o1"), ("b1", "o1"), ("a2", "o2"), ("b2", "o2")] {
            builder.relate_event_object(event, object, "order").unwrap();
        }
        let edge = &graphs(&builder.build()).directly_follows[0];
        assert_eq!(edge.occurrences, 2);
        assert_eq!(edge.total_elapsed_micros, 40);
        assert_eq!(edge.mean_elapsed_micros, 20);
        assert_eq!(edge.min_elapsed_micros, 10);
        assert_eq!(edge.max_elapsed_micros, 30);
    }

    #[test]
    fn graph_respects_the_filtered_view_and_is_deterministic() {
        let log = log();
        let view = OcelView::apply(
            &log,
            &OcelFilter {
                object_types: vec!["package".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let first = OcelGraphs::discover(&log, &view);
        assert_eq!(first, OcelGraphs::discover(&log, &view));
        assert_eq!(first.directly_follows.len(), 1);
        assert!(first.interactions.is_empty());
    }
}
