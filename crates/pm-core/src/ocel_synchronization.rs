//! Observable synchronization timing at native multi-object events.

use crate::{EventTypeId, ObjectId, ObjectTypeId, OcelEventId, OcelLog, OcelView};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const EVIDENCE_LIMIT: usize = 5;
const LAYER_EVIDENCE_LIMIT: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelSynchronizationReadinessEvidence {
    pub object: ObjectId,
    pub object_type: ObjectTypeId,
    pub prior_at: i64,
    pub prior_events: Vec<OcelEventId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelSynchronizationEvidence {
    pub event: OcelEventId,
    pub participating_objects: u64,
    pub historically_observed_objects: u64,
    pub first_seen_objects: u64,
    pub arrival_spread_micros: i64,
    pub elapsed_after_last_prior_micros: i64,
    pub elapsed_from_first_prior_micros: i64,
    pub readiness: Vec<OcelSynchronizationReadinessEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelSynchronization {
    pub activity: EventTypeId,
    pub object_types: Vec<ObjectTypeId>,
    pub multi_object_events: u64,
    pub eligible_events: u64,
    pub insufficient_history_events: u64,
    pub historically_observed_object_participations: u64,
    pub first_seen_object_participations: u64,
    pub total_arrival_spread_micros: i64,
    pub mean_arrival_spread_micros: Option<i64>,
    pub min_arrival_spread_micros: Option<i64>,
    pub max_arrival_spread_micros: Option<i64>,
    pub total_elapsed_after_last_prior_micros: i64,
    pub mean_elapsed_after_last_prior_micros: Option<i64>,
    pub min_elapsed_after_last_prior_micros: Option<i64>,
    pub max_elapsed_after_last_prior_micros: Option<i64>,
    pub total_elapsed_from_first_prior_micros: i64,
    pub mean_elapsed_from_first_prior_micros: Option<i64>,
    pub min_elapsed_from_first_prior_micros: Option<i64>,
    pub max_elapsed_from_first_prior_micros: Option<i64>,
    pub evidence: Vec<OcelSynchronizationEvidence>,
}

#[derive(Default)]
struct Aggregate {
    object_types: BTreeSet<ObjectTypeId>,
    multi_object_events: u64,
    eligible_events: u64,
    insufficient_history_events: u64,
    historically_observed_object_participations: u64,
    first_seen_object_participations: u64,
    arrival_spread: Metric,
    elapsed_after_last_prior: Metric,
    elapsed_from_first_prior: Metric,
    evidence: Vec<OcelSynchronizationEvidence>,
}

#[derive(Default)]
struct Metric {
    total: i128,
    minimum: Option<i64>,
    maximum: Option<i64>,
}

impl Metric {
    fn observe(&mut self, value: i64) {
        self.total = self.total.saturating_add(i128::from(value));
        self.minimum = Some(self.minimum.map_or(value, |current| current.min(value)));
        self.maximum = Some(self.maximum.map_or(value, |current| current.max(value)));
    }

    fn mean(&self, observations: u64) -> Option<i64> {
        (observations > 0)
            .then(|| i64::try_from(self.total / i128::from(observations)).unwrap_or(i64::MAX))
    }
}

/// Decomposes timing at events linked to at least two distinct objects.
///
/// For every participating object, the most recent strictly earlier lifecycle
/// layer is its observable prior point. Objects with no earlier event are
/// disclosed as first-seen and excluded from readiness timing. An event is
/// eligible when at least two participating objects have such prior points.
/// The measures are elapsed-time observations; they do not establish waiting,
/// availability, causation, or activity execution duration.
pub fn analyze_ocel_synchronization(log: &OcelLog, view: &OcelView) -> Vec<OcelSynchronization> {
    let mut participants = BTreeMap::<OcelEventId, BTreeSet<ObjectId>>::new();
    let mut timelines = BTreeMap::<ObjectId, BTreeMap<i64, BTreeSet<OcelEventId>>>::new();
    for index in &view.event_object_relations {
        let relation = log.event_object_relations[*index];
        participants
            .entry(relation.event)
            .or_default()
            .insert(relation.object);
        timelines
            .entry(relation.object)
            .or_default()
            .entry(log.events[relation.event.0 as usize].at)
            .or_default()
            .insert(relation.event);
    }
    let mut fold = OcelSynchronizationFold::default();
    let mut joined = Vec::new();
    for (event_id, objects) in participants {
        let event = log.events[event_id.0 as usize];
        joined.clear();
        joined.extend(objects.iter().map(|object| {
            OcelParticipant {
                object: *object,
                object_type: log.objects[object.0 as usize].object_type,
                prior_at: timelines
                    .get(object)
                    .and_then(|timeline| timeline.range(..event.at).next_back())
                    .map(|(at, _)| *at),
            }
        }));
        fold.event(
            event_id,
            event.event_type,
            event.at,
            &joined,
            |object, at| {
                timelines[&object][&at]
                    .iter()
                    .copied()
                    .take(LAYER_EVIDENCE_LIMIT)
                    .collect()
            },
        );
    }
    fold.finish()
}

/// An object taking part in an event, with the time of its most recent
/// strictly earlier lifecycle layer, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcelParticipant {
    pub object: ObjectId,
    pub object_type: ObjectTypeId,
    pub prior_at: Option<i64>,
}

/// Synchronization, built one event at a time in event order, from memory
/// or from disk alike.
#[derive(Default)]
pub struct OcelSynchronizationFold {
    by_activity: BTreeMap<EventTypeId, Aggregate>,
}

impl OcelSynchronizationFold {
    /// An event with its distinct participating objects in object order.
    /// `prior_events(object, at)` lists, in event order, the object's events
    /// at its prior layer; it is asked only for evidence.
    pub fn event(
        &mut self,
        event_id: OcelEventId,
        event_type: EventTypeId,
        at: i64,
        objects: &[OcelParticipant],
        mut prior_events: impl FnMut(ObjectId, i64) -> Vec<OcelEventId>,
    ) {
        if objects.len() < 2 {
            return;
        }
        let aggregate = self.by_activity.entry(event_type).or_default();
        aggregate.multi_object_events = aggregate.multi_object_events.saturating_add(1);
        aggregate
            .object_types
            .extend(objects.iter().map(|object| object.object_type));

        let readiness = objects
            .iter()
            .filter_map(|object| Some((object.object, object.object_type, object.prior_at?)))
            .collect::<Vec<_>>();
        let observed = u64::try_from(readiness.len()).unwrap_or(u64::MAX);
        let participating = u64::try_from(objects.len()).unwrap_or(u64::MAX);
        let first_seen = participating.saturating_sub(observed);
        aggregate.historically_observed_object_participations = aggregate
            .historically_observed_object_participations
            .saturating_add(observed);
        aggregate.first_seen_object_participations = aggregate
            .first_seen_object_participations
            .saturating_add(first_seen);

        if readiness.len() < 2 {
            aggregate.insufficient_history_events =
                aggregate.insufficient_history_events.saturating_add(1);
            return;
        }

        let first_prior = readiness.iter().map(|(_, _, at)| *at).min().unwrap_or(at);
        let last_prior = readiness.iter().map(|(_, _, at)| *at).max().unwrap_or(at);
        let arrival_spread = last_prior.saturating_sub(first_prior);
        let after_last = at.saturating_sub(last_prior);
        let from_first = at.saturating_sub(first_prior);
        aggregate.eligible_events = aggregate.eligible_events.saturating_add(1);
        aggregate.arrival_spread.observe(arrival_spread);
        aggregate.elapsed_after_last_prior.observe(after_last);
        aggregate.elapsed_from_first_prior.observe(from_first);

        if aggregate.evidence.len() < EVIDENCE_LIMIT {
            aggregate.evidence.push(OcelSynchronizationEvidence {
                event: event_id,
                participating_objects: participating,
                historically_observed_objects: observed,
                first_seen_objects: first_seen,
                arrival_spread_micros: arrival_spread,
                elapsed_after_last_prior_micros: after_last,
                elapsed_from_first_prior_micros: from_first,
                readiness: readiness
                    .into_iter()
                    .take(EVIDENCE_LIMIT)
                    .map(
                        |(object, object_type, prior_at)| OcelSynchronizationReadinessEvidence {
                            object,
                            object_type,
                            prior_at,
                            prior_events: prior_events(object, prior_at),
                        },
                    )
                    .collect(),
            });
        }
    }

    pub fn finish(self) -> Vec<OcelSynchronization> {
        self.by_activity
            .into_iter()
            .map(|(activity, aggregate)| OcelSynchronization {
                activity,
                object_types: aggregate.object_types.into_iter().collect(),
                multi_object_events: aggregate.multi_object_events,
                eligible_events: aggregate.eligible_events,
                insufficient_history_events: aggregate.insufficient_history_events,
                historically_observed_object_participations: aggregate
                    .historically_observed_object_participations,
                first_seen_object_participations: aggregate.first_seen_object_participations,
                total_arrival_spread_micros: clamp_i128(aggregate.arrival_spread.total),
                mean_arrival_spread_micros: aggregate
                    .arrival_spread
                    .mean(aggregate.eligible_events),
                min_arrival_spread_micros: aggregate.arrival_spread.minimum,
                max_arrival_spread_micros: aggregate.arrival_spread.maximum,
                total_elapsed_after_last_prior_micros: clamp_i128(
                    aggregate.elapsed_after_last_prior.total,
                ),
                mean_elapsed_after_last_prior_micros: aggregate
                    .elapsed_after_last_prior
                    .mean(aggregate.eligible_events),
                min_elapsed_after_last_prior_micros: aggregate.elapsed_after_last_prior.minimum,
                max_elapsed_after_last_prior_micros: aggregate.elapsed_after_last_prior.maximum,
                total_elapsed_from_first_prior_micros: clamp_i128(
                    aggregate.elapsed_from_first_prior.total,
                ),
                mean_elapsed_from_first_prior_micros: aggregate
                    .elapsed_from_first_prior
                    .mean(aggregate.eligible_events),
                min_elapsed_from_first_prior_micros: aggregate.elapsed_from_first_prior.minimum,
                max_elapsed_from_first_prior_micros: aggregate.elapsed_from_first_prior.maximum,
                evidence: aggregate.evidence,
            })
            .collect()
    }
}

fn clamp_i128(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value.is_negative() {
        i64::MIN
    } else {
        i64::MAX
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OcelBuilder, OcelFilter};

    fn log() -> OcelLog {
        let mut builder = OcelBuilder::new();
        for activity in ["ready a", "ready b", "also ready", "join", "cold join"] {
            builder.declare_event_type(activity, vec![]).unwrap();
        }
        for object_type in ["order", "item", "package"] {
            builder.declare_object_type(object_type, vec![]).unwrap();
        }
        for (event, activity, at) in [
            ("a", "ready a", 10),
            ("b", "ready b", 20),
            ("same", "also ready", 30),
            ("join", "join", 30),
            ("cold", "cold join", 40),
        ] {
            builder.add_event(event, activity, at).unwrap();
        }
        for (object, object_type) in [
            ("o1", "order"),
            ("i1", "item"),
            ("p1", "package"),
            ("p2", "package"),
        ] {
            builder.add_object(object, object_type).unwrap();
        }
        builder.relate_event_object("a", "o1", "input").unwrap();
        builder.relate_event_object("b", "i1", "input").unwrap();
        builder
            .relate_event_object("same", "i1", "simultaneous")
            .unwrap();
        for object in ["o1", "i1", "p1"] {
            builder
                .relate_event_object("join", object, "participant")
                .unwrap();
        }
        for object in ["p1", "p2"] {
            builder
                .relate_event_object("cold", object, "participant")
                .unwrap();
        }
        builder.build()
    }

    #[test]
    fn decomposes_prior_arrival_spread_without_treating_first_seen_objects_as_ready() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let result = analyze_ocel_synchronization(&log, &view);
        let join = result
            .iter()
            .find(|item| item.activity == EventTypeId(3))
            .unwrap();
        assert_eq!(join.multi_object_events, 1);
        assert_eq!(join.eligible_events, 1);
        assert_eq!(join.first_seen_object_participations, 1);
        assert_eq!(join.total_arrival_spread_micros, 10);
        assert_eq!(join.mean_arrival_spread_micros, Some(10));
        assert_eq!(join.total_elapsed_after_last_prior_micros, 10);
        assert_eq!(join.mean_elapsed_after_last_prior_micros, Some(10));
        assert_eq!(join.total_elapsed_from_first_prior_micros, 20);
        assert_eq!(join.mean_elapsed_from_first_prior_micros, Some(20));
        assert_eq!(join.evidence[0].readiness.len(), 2);
        assert_eq!(join.evidence[0].readiness[1].prior_events, [OcelEventId(1)]);
    }

    #[test]
    fn simultaneous_events_are_not_used_as_prior_history() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let result = analyze_ocel_synchronization(&log, &view);
        let join = result
            .iter()
            .find(|item| item.activity == EventTypeId(3))
            .unwrap();
        assert_eq!(join.evidence[0].readiness[1].prior_at, 20);
        assert_eq!(join.evidence[0].readiness[1].prior_events, [OcelEventId(1)]);
    }

    #[test]
    fn exposes_multi_object_events_that_lack_enough_history() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let result = analyze_ocel_synchronization(&log, &view);
        let cold = result
            .iter()
            .find(|item| item.activity == EventTypeId(4))
            .unwrap();
        assert_eq!(cold.multi_object_events, 1);
        assert_eq!(cold.eligible_events, 0);
        assert_eq!(cold.insufficient_history_events, 1);
        assert_eq!(cold.mean_arrival_spread_micros, None);
        assert!(cold.evidence.is_empty());
    }

    #[test]
    fn filtered_relationships_change_eligibility_without_changing_source_ids() {
        let log = log();
        let view = OcelView::apply(
            &log,
            &OcelFilter {
                object_types: vec!["order".into(), "package".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let result = analyze_ocel_synchronization(&log, &view);
        let join = result
            .iter()
            .find(|item| item.activity == EventTypeId(3))
            .unwrap();
        assert_eq!(join.eligible_events, 0);
        assert_eq!(join.insufficient_history_events, 1);
    }

    #[test]
    fn output_is_deterministic() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        assert_eq!(
            analyze_ocel_synchronization(&log, &view),
            analyze_ocel_synchronization(&log, &view)
        );
    }
}
