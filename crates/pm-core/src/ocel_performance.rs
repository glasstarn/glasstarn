//! Explicit time-between analysis over native object lifecycles.

use crate::{EventTypeId, ObjectId, ObjectTypeId, OcelEventId, OcelLog, OcelView};
use serde::Serialize;
use std::collections::BTreeMap;

const EVIDENCE_LIMIT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OcelTimeBetweenEvidence {
    pub object: ObjectId,
    pub source_event: OcelEventId,
    pub target_event: OcelEventId,
    pub elapsed_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelTimeBetween {
    pub object_type: ObjectTypeId,
    pub source: EventTypeId,
    pub target: EventTypeId,
    pub observations: u64,
    pub objects: u64,
    pub mean_elapsed_micros: i64,
    /// Nearest-rank median and 90th percentile of the observations.
    pub median_elapsed_micros: i64,
    pub p90_elapsed_micros: i64,
    pub min_elapsed_micros: i64,
    pub max_elapsed_micros: i64,
    /// Objects of this type that record the source activity at all; those
    /// not in `objects` never reached the target after it.
    pub source_objects: u64,
    pub evidence: Vec<OcelTimeBetweenEvidence>,
    /// The source objects that never reached the target, in identifier order.
    #[serde(skip)]
    pub unreached: Vec<ObjectId>,
}

#[derive(Default)]
struct Aggregate {
    observations: u64,
    source_objects: u64,
    objects: u64,
    total_elapsed: i128,
    minimum: Option<i64>,
    maximum: Option<i64>,
    evidence: Vec<OcelTimeBetweenEvidence>,
}

/// Measures from the most recent strictly earlier source layer to each target
/// event on the same object. If that source layer contains several matching
/// events, each source-target pair is an observation. Simultaneous source and
/// target events are never ordered or measured against one another.
pub fn measure_ocel_time_between(
    log: &OcelLog,
    view: &OcelView,
    source: EventTypeId,
    target: EventTypeId,
) -> Vec<OcelTimeBetween> {
    measure_with_progress(log, view, source, target, false)
}

/// Includes source-only groups for progress/drop-out counts. Zero observations
/// mean timing is unavailable, not a measured zero-duration hand-off.
pub fn measure_ocel_progress(
    log: &OcelLog,
    view: &OcelView,
    source: EventTypeId,
    target: EventTypeId,
) -> Vec<OcelTimeBetween> {
    measure_with_progress(log, view, source, target, true)
}
fn measure_with_progress(
    log: &OcelLog,
    view: &OcelView,
    source: EventTypeId,
    target: EventTypeId,
    include_unreached: bool,
) -> Vec<OcelTimeBetween> {
    let mut fold = OcelTimeBetweenFold::new(source, target);
    let mut elapsed = BTreeMap::<ObjectTypeId, Vec<i64>>::new();
    let mut unreached = BTreeMap::<ObjectTypeId, Vec<ObjectId>>::new();
    crate::for_each_timeline(log, view, |object, object_type, timeline| {
        fold.object(
            object,
            object_type,
            timeline,
            |kind, value| elapsed.entry(kind).or_default().push(value),
            |kind, object| unreached.entry(kind).or_default().push(object),
        );
    });
    let rank_value = |kind, numerator, denominator| {
        rank(
            elapsed.get_mut(&kind).map_or(&mut [][..], |v| &mut v[..]),
            numerator,
            denominator,
        )
    };
    let mut measured = if include_unreached {
        fold.finish_with_unreached(rank_value)
    } else {
        fold.finish(rank_value)
    };
    for measurement in &mut measured {
        measurement.unreached = unreached
            .remove(&measurement.object_type)
            .unwrap_or_default();
    }
    measured
}

/// Time between two activities, built one object timeline at a time in
/// object order. Each observation's elapsed time and each object that never
/// reached the target are handed to the caller, who keeps them in memory or
/// on disk and answers the percentiles.
pub struct OcelTimeBetweenFold {
    source: EventTypeId,
    target: EventTypeId,
    by_type: BTreeMap<ObjectTypeId, Aggregate>,
}

impl OcelTimeBetweenFold {
    pub fn new(source: EventTypeId, target: EventTypeId) -> Self {
        Self {
            source,
            target,
            by_type: BTreeMap::new(),
        }
    }

    /// An object's timeline; steps of other activities may be left out.
    pub fn object(
        &mut self,
        object: ObjectId,
        object_type: ObjectTypeId,
        timeline: &[crate::TimelineStep],
        mut elapsed_out: impl FnMut(ObjectTypeId, i64),
        mut unreached_out: impl FnMut(ObjectTypeId, ObjectId),
    ) {
        let mut latest_sources: Vec<OcelEventId> = Vec::new();
        let mut latest_source_at = 0;
        let (mut observed, mut sourced) = (false, false);
        for layer in timeline.chunk_by(|left, right| left.0 == right.0) {
            let at = layer[0].0;
            if !latest_sources.is_empty() {
                let elapsed = at.saturating_sub(latest_source_at);
                for (_, target_event, kind) in layer {
                    if *kind != self.target {
                        continue;
                    }
                    let aggregate = self.by_type.entry(object_type).or_default();
                    for source_event in &latest_sources {
                        aggregate.observations = aggregate.observations.saturating_add(1);
                        elapsed_out(object_type, elapsed);
                        observed = true;
                        aggregate.total_elapsed =
                            aggregate.total_elapsed.saturating_add(i128::from(elapsed));
                        aggregate.minimum = Some(
                            aggregate
                                .minimum
                                .map_or(elapsed, |current| current.min(elapsed)),
                        );
                        aggregate.maximum = Some(
                            aggregate
                                .maximum
                                .map_or(elapsed, |current| current.max(elapsed)),
                        );
                        if aggregate.evidence.len() < EVIDENCE_LIMIT {
                            aggregate.evidence.push(OcelTimeBetweenEvidence {
                                object,
                                source_event: *source_event,
                                target_event: *target_event,
                                elapsed_micros: elapsed,
                            });
                        }
                    }
                }
            }
            let sources = layer
                .iter()
                .filter(|(_, _, kind)| *kind == self.source)
                .map(|(_, event, _)| *event)
                .collect::<Vec<_>>();
            if !sources.is_empty() {
                self.by_type.entry(object_type).or_default();
                sourced = true;
                latest_sources = sources;
                latest_source_at = at;
            }
        }
        if observed || sourced {
            let aggregate = self.by_type.entry(object_type).or_default();
            if observed {
                aggregate.objects = aggregate.objects.saturating_add(1);
            }
            if sourced {
                aggregate.source_objects = aggregate.source_objects.saturating_add(1);
                if !observed {
                    unreached_out(object_type, object);
                }
            }
        }
    }

    /// The measurements; `rank(object type, numerator, denominator)` gives
    /// the nearest-rank percentile of that type's elapsed times.
    pub fn finish(
        self,
        rank: impl FnMut(ObjectTypeId, usize, usize) -> i64,
    ) -> Vec<OcelTimeBetween> {
        self.finish_inner(rank, false)
    }

    pub fn finish_with_unreached(
        self,
        rank: impl FnMut(ObjectTypeId, usize, usize) -> i64,
    ) -> Vec<OcelTimeBetween> {
        self.finish_inner(rank, true)
    }

    fn finish_inner(
        self,
        mut rank: impl FnMut(ObjectTypeId, usize, usize) -> i64,
        include_unreached: bool,
    ) -> Vec<OcelTimeBetween> {
        let (source, target) = (self.source, self.target);
        self.by_type
            .into_iter()
            // Objects that only record the source contribute no measurement.
            .filter(|(_, aggregate)| {
                aggregate.observations > 0 || include_unreached && aggregate.source_objects > 0
            })
            .map(|(object_type, aggregate)| OcelTimeBetween {
                object_type,
                source,
                target,
                observations: aggregate.observations,
                objects: aggregate.objects,
                mean_elapsed_micros: mean(aggregate.total_elapsed, aggregate.observations),
                median_elapsed_micros: if aggregate.observations > 0 {
                    rank(object_type, 1, 2)
                } else {
                    0
                },
                p90_elapsed_micros: if aggregate.observations > 0 {
                    rank(object_type, 9, 10)
                } else {
                    0
                },
                source_objects: aggregate.source_objects,
                min_elapsed_micros: aggregate.minimum.unwrap_or(0),
                max_elapsed_micros: aggregate.maximum.unwrap_or(0),
                unreached: Vec::new(),
                evidence: aggregate.evidence,
            })
            .collect()
    }
}

/// The position, from zero, of the nearest-rank percentile
/// `numerator / denominator` among `count` sorted values.
pub fn nearest_rank(count: usize, numerator: usize, denominator: usize) -> usize {
    (count * numerator).div_ceil(denominator).max(1) - 1
}

/// Nearest-rank percentile `numerator / denominator` of the values.
fn rank(values: &mut [i64], numerator: usize, denominator: usize) -> i64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[nearest_rank(values.len(), numerator, denominator)]
}

fn mean(total: i128, observations: u64) -> i64 {
    if observations == 0 {
        return 0;
    }
    i64::try_from(total / i128::from(observations)).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OcelBuilder, OcelFilter, OcelView};

    fn log() -> OcelLog {
        let mut builder = OcelBuilder::new();
        for activity in ["start", "middle", "finish"] {
            builder.declare_event_type(activity, vec![]).unwrap();
        }
        for object_type in ["order", "item"] {
            builder.declare_object_type(object_type, vec![]).unwrap();
        }
        for (event, activity, at) in [
            ("s1", "start", 0),
            ("s2", "start", 5),
            ("f1", "finish", 10),
            ("s3", "start", 0),
            ("f2", "finish", 30),
            ("s4", "start", 40),
            ("f3", "finish", 40),
        ] {
            builder.add_event(event, activity, at).unwrap();
        }
        builder.add_object("o1", "order").unwrap();
        builder.add_object("i1", "item").unwrap();
        for event in ["s1", "s2", "f1", "s4", "f3"] {
            builder.relate_event_object(event, "o1", "subject").unwrap();
        }
        for event in ["s3", "f2"] {
            builder.relate_event_object(event, "i1", "subject").unwrap();
        }
        builder.build()
    }

    #[test]
    fn measures_through_common_objects_and_uses_the_latest_prior_source() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let result = measure_ocel_time_between(&log, &view, EventTypeId(0), EventTypeId(2));
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].observations, 2);
        assert_eq!(result[0].mean_elapsed_micros, 20);
        assert_eq!(result[0].min_elapsed_micros, 5);
        assert_eq!(result[0].max_elapsed_micros, 35);
        assert_eq!(result[0].evidence[0].source_event, OcelEventId(1));
        assert_eq!(result[1].mean_elapsed_micros, 30);
        // Order o1 has 5 and 35: nearest-rank median 5, P90 35.
        assert_eq!(
            (
                result[0].median_elapsed_micros,
                result[0].p90_elapsed_micros
            ),
            (5, 35)
        );
        assert_eq!(result[0].source_objects, 1);
    }

    #[test]
    fn objects_that_start_but_never_reach_the_target_are_counted() {
        let mut builder = OcelBuilder::new();
        builder.declare_event_type("start", vec![]).unwrap();
        builder.declare_event_type("finish", vec![]).unwrap();
        builder.declare_object_type("order", vec![]).unwrap();
        for (id, finishes) in [("a", true), ("b", false), ("c", true)] {
            builder.add_object(id, "order").unwrap();
            builder.add_event(&format!("{id}-s"), "start", 0).unwrap();
            builder
                .relate_event_object(&format!("{id}-s"), id, "o")
                .unwrap();
            if finishes {
                builder.add_event(&format!("{id}-f"), "finish", 10).unwrap();
                builder
                    .relate_event_object(&format!("{id}-f"), id, "o")
                    .unwrap();
            }
        }
        let log = builder.build();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let result = measure_ocel_time_between(&log, &view, EventTypeId(0), EventTypeId(1));
        assert_eq!((result[0].objects, result[0].source_objects), (2, 3));
        assert_eq!(result[0].unreached.len(), 1);
    }

    #[test]
    fn simultaneous_source_and_target_are_not_ordered() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let result = measure_ocel_time_between(&log, &view, EventTypeId(0), EventTypeId(2));
        assert_eq!(result.iter().map(|item| item.observations).sum::<u64>(), 3);
        assert!(result.iter().all(|item| item.min_elapsed_micros > 0));
        let same_time_target = result[0]
            .evidence
            .iter()
            .find(|evidence| evidence.target_event == OcelEventId(6))
            .unwrap();
        assert_eq!(same_time_target.source_event, OcelEventId(1));
        assert_eq!(same_time_target.elapsed_micros, 35);
    }

    #[test]
    fn filter_scope_and_determinism_are_preserved() {
        let log = log();
        let view = OcelView::apply(
            &log,
            &OcelFilter {
                object_types: vec!["item".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let first = measure_ocel_time_between(&log, &view, EventTypeId(0), EventTypeId(2));
        assert_eq!(
            first,
            measure_ocel_time_between(&log, &view, EventTypeId(0), EventTypeId(2))
        );
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].object_type, ObjectTypeId(1));
    }
}
