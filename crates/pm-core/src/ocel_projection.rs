//! Explicit, auditable projection of an object-centric view to one case notion.

use crate::{EventTypeId, Micros, ObjectId, ObjectTypeId, OcelEventId, OcelLog, OcelView};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const EVIDENCE_LIMIT: usize = 5;

/// One timestamp layer in a projected case. Events inside a layer are
/// simultaneous; their identifier order is canonical, not temporal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelProjectionLayer {
    pub at: Micros,
    pub events: Vec<OcelProjectedEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OcelProjectedEvent {
    pub source_event: OcelEventId,
    pub event_type: EventTypeId,
}

/// One selected object becomes one case. Source identifiers are never
/// renumbered, so every projected occurrence remains traceable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelProjectedCase {
    pub case_object: ObjectId,
    pub layers: Vec<OcelProjectionLayer>,
}

/// Convergence occurs when flattening replicates one source event into several
/// cases of the selected object type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelConvergenceEvidence {
    pub source_event: OcelEventId,
    pub projected_case_count: u64,
    pub additional_occurrences: u64,
    pub evidence_cases: Vec<ObjectId>,
}

/// Divergence risk is an apparent repeated activity inside one projected case
/// where the occurrences involve distinct objects of another type. This is a
/// candidate distortion, not proof that the repetitions are artificial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelDivergenceEvidence {
    pub projected_case: ObjectId,
    pub event_type: EventTypeId,
    pub related_object_type: ObjectTypeId,
    pub projected_occurrences: u64,
    pub distinct_related_objects: u64,
    pub evidence_events: Vec<OcelEventId>,
    pub evidence_objects: Vec<ObjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelProjection {
    pub case_object_type: ObjectTypeId,
    pub source_events: u64,
    pub projected_cases: u64,
    pub included_unique_events: u64,
    pub excluded_events: u64,
    pub projected_event_occurrences: u64,
    pub convergence_events: u64,
    pub convergence_additional_occurrences: u64,
    pub divergence_candidates: u64,
    pub cases: Vec<OcelProjectedCase>,
    pub convergence_evidence: Vec<OcelConvergenceEvidence>,
    pub divergence_evidence: Vec<OcelDivergenceEvidence>,
}

/// Project an already-filtered OCEL view using one explicit object type as the
/// case notion. This deliberately materializes projected cases: callers must
/// opt in and can inspect the distortion counters before applying traditional
/// case-centric algorithms.
pub fn project_ocel(
    log: &OcelLog,
    view: &OcelView,
    case_object_type: ObjectTypeId,
) -> OcelProjection {
    let mut event_cases = BTreeMap::<OcelEventId, Vec<ObjectId>>::new();
    let mut event_other_objects =
        BTreeMap::<OcelEventId, BTreeMap<ObjectTypeId, BTreeSet<ObjectId>>>::new();
    for relation_index in &view.event_object_relations {
        let relation = log.event_object_relations[*relation_index];
        let object_type = log.objects[relation.object.0 as usize].object_type;
        if object_type == case_object_type {
            event_cases
                .entry(relation.event)
                .or_default()
                .push(relation.object);
        } else {
            event_other_objects
                .entry(relation.event)
                .or_default()
                .entry(object_type)
                .or_default()
                .insert(relation.object);
        }
    }
    let mut fold = OcelProjectionFold::new(case_object_type, None);
    fold.source_events(count(view.events.len()));
    for (event, mut cases) in event_cases {
        cases.sort_unstable();
        cases.dedup();
        fold.event(event, &cases);
    }
    crate::for_each_timeline(log, view, |object, object_type, timeline| {
        if object_type == case_object_type {
            fold.case(object, timeline, |event| {
                event_other_objects.get(&event).cloned().unwrap_or_default()
            });
        }
    });
    fold.finish()
}

/// A projection built in pieces: each event with its cases, in event order,
/// then each case's timeline, in case order. Every case is counted; only the
/// first `keep` cases' layers are kept when a limit is given.
pub struct OcelProjectionFold {
    projection: OcelProjection,
    keep: Option<usize>,
}

impl OcelProjectionFold {
    pub fn new(case_object_type: ObjectTypeId, keep: Option<usize>) -> Self {
        Self {
            projection: OcelProjection {
                case_object_type,
                source_events: 0,
                projected_cases: 0,
                included_unique_events: 0,
                excluded_events: 0,
                projected_event_occurrences: 0,
                convergence_events: 0,
                convergence_additional_occurrences: 0,
                divergence_candidates: 0,
                cases: Vec::new(),
                convergence_evidence: Vec::new(),
                divergence_evidence: Vec::new(),
            },
            keep,
        }
    }

    /// The events of the view.
    pub fn source_events(&mut self, events: u64) {
        self.projection.source_events = events;
    }

    /// An event with its distinct cases, in case order.
    pub fn event(&mut self, event: OcelEventId, cases: &[ObjectId]) {
        let projection = &mut self.projection;
        let case_count = count(cases.len());
        projection.included_unique_events = projection.included_unique_events.saturating_add(1);
        projection.projected_event_occurrences = projection
            .projected_event_occurrences
            .saturating_add(case_count);
        if case_count > 1 {
            let additional = case_count.saturating_sub(1);
            projection.convergence_events = projection.convergence_events.saturating_add(1);
            projection.convergence_additional_occurrences = projection
                .convergence_additional_occurrences
                .saturating_add(additional);
            if projection.convergence_evidence.len() < EVIDENCE_LIMIT {
                projection
                    .convergence_evidence
                    .push(OcelConvergenceEvidence {
                        source_event: event,
                        projected_case_count: case_count,
                        additional_occurrences: additional,
                        evidence_cases: cases.iter().copied().take(EVIDENCE_LIMIT).collect(),
                    });
            }
        }
    }

    /// A case's timeline; `others(event)` gives the event's objects of the
    /// other types, by type.
    pub fn case(
        &mut self,
        case_object: ObjectId,
        timeline: &[crate::TimelineStep],
        mut others: impl FnMut(OcelEventId) -> BTreeMap<ObjectTypeId, BTreeSet<ObjectId>>,
    ) {
        let projection = &mut self.projection;
        projection.projected_cases = projection.projected_cases.saturating_add(1);
        let mut by_activity = BTreeMap::<EventTypeId, Vec<OcelEventId>>::new();
        for (_, event, event_type) in timeline {
            by_activity.entry(*event_type).or_default().push(*event);
        }
        for (event_type, mut activity_events) in by_activity {
            if activity_events.len() < 2 {
                continue;
            }
            activity_events.sort_unstable();
            let mut objects_by_type = BTreeMap::<ObjectTypeId, BTreeSet<ObjectId>>::new();
            for event in &activity_events {
                for (object_type, objects) in others(*event) {
                    objects_by_type
                        .entry(object_type)
                        .or_default()
                        .extend(objects);
                }
            }
            for (related_object_type, related_objects) in objects_by_type {
                if related_objects.len() < 2 {
                    continue;
                }
                projection.divergence_candidates =
                    projection.divergence_candidates.saturating_add(1);
                if projection.divergence_evidence.len() < EVIDENCE_LIMIT {
                    projection.divergence_evidence.push(OcelDivergenceEvidence {
                        projected_case: case_object,
                        event_type,
                        related_object_type,
                        projected_occurrences: count(activity_events.len()),
                        distinct_related_objects: count(related_objects.len()),
                        evidence_events: activity_events
                            .iter()
                            .copied()
                            .take(EVIDENCE_LIMIT)
                            .collect(),
                        evidence_objects: related_objects
                            .iter()
                            .copied()
                            .take(EVIDENCE_LIMIT)
                            .collect(),
                    });
                }
            }
        }
        if self.keep.is_some_and(|keep| projection.cases.len() >= keep) {
            return;
        }
        let layers = timeline
            .chunk_by(|left, right| left.0 == right.0)
            .map(|layer| OcelProjectionLayer {
                at: layer[0].0,
                events: layer
                    .iter()
                    .map(|(_, event, event_type)| OcelProjectedEvent {
                        source_event: *event,
                        event_type: *event_type,
                    })
                    .collect(),
            })
            .collect();
        projection.cases.push(OcelProjectedCase {
            case_object,
            layers,
        });
    }

    pub fn finish(mut self) -> OcelProjection {
        self.projection.excluded_events = self
            .projection
            .source_events
            .saturating_sub(self.projection.included_unique_events);
        self.projection
    }
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OcelBuilder, OcelFilter};

    fn log() -> OcelLog {
        let mut builder = OcelBuilder::new();
        for event_type in ["create order", "pick item", "invoice"] {
            builder.declare_event_type(event_type, vec![]).unwrap();
        }
        for object_type in ["order", "item"] {
            builder.declare_object_type(object_type, vec![]).unwrap();
        }
        for (event, event_type, at) in [
            ("e1", "create order", 10),
            ("e2", "pick item", 20),
            ("e3", "pick item", 20),
            ("e4", "invoice", 30),
            ("e5", "pick item", 40),
        ] {
            builder.add_event(event, event_type, at).unwrap();
        }
        for object in ["o1", "o2"] {
            builder.add_object(object, "order").unwrap();
        }
        for object in ["i1", "i2", "i3"] {
            builder.add_object(object, "item").unwrap();
        }
        builder.relate_event_object("e1", "o1", "order").unwrap();
        builder.relate_event_object("e2", "o1", "order").unwrap();
        builder.relate_event_object("e2", "i1", "item").unwrap();
        builder.relate_event_object("e3", "o1", "order").unwrap();
        builder.relate_event_object("e3", "i2", "item").unwrap();
        builder.relate_event_object("e4", "o1", "order").unwrap();
        builder.relate_event_object("e4", "o2", "order").unwrap();
        builder.relate_event_object("e5", "i3", "item").unwrap();
        builder.build()
    }

    #[test]
    fn projection_is_explicit_traceable_and_reports_both_distortions() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let projection = project_ocel(&log, &view, ObjectTypeId(0));

        assert_eq!(projection.source_events, 5);
        assert_eq!(projection.projected_cases, 2);
        assert_eq!(projection.included_unique_events, 4);
        assert_eq!(projection.excluded_events, 1);
        assert_eq!(projection.projected_event_occurrences, 5);
        assert_eq!(projection.convergence_events, 1);
        assert_eq!(projection.convergence_additional_occurrences, 1);
        assert_eq!(projection.divergence_candidates, 1);
        assert_eq!(
            projection.convergence_evidence[0].source_event,
            OcelEventId(3)
        );
        assert_eq!(
            projection.divergence_evidence[0].projected_case,
            ObjectId(0)
        );
        assert_eq!(projection.divergence_evidence[0].event_type, EventTypeId(1));
        assert_eq!(
            projection.divergence_evidence[0].related_object_type,
            ObjectTypeId(1)
        );

        let first = &projection.cases[0];
        assert_eq!(first.case_object, ObjectId(0));
        assert_eq!(first.layers.len(), 3);
        assert_eq!(first.layers[1].events.len(), 2);
        assert_eq!(first.layers[1].events[0].source_event, OcelEventId(1));
        assert_eq!(first.layers[1].events[1].source_event, OcelEventId(2));
    }

    #[test]
    fn repeated_activity_on_one_other_object_is_not_called_divergence() {
        let mut builder = OcelBuilder::new();
        builder.declare_event_type("revise", vec![]).unwrap();
        builder.declare_object_type("order", vec![]).unwrap();
        builder.declare_object_type("item", vec![]).unwrap();
        builder.add_event("e1", "revise", 1).unwrap();
        builder.add_event("e2", "revise", 2).unwrap();
        builder.add_object("o1", "order").unwrap();
        builder.add_object("i1", "item").unwrap();
        for event in ["e1", "e2"] {
            builder.relate_event_object(event, "o1", "order").unwrap();
            builder.relate_event_object(event, "i1", "item").unwrap();
        }
        let log = builder.build();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();

        let projection = project_ocel(&log, &view, ObjectTypeId(0));
        assert_eq!(projection.divergence_candidates, 0);
        assert!(projection.divergence_evidence.is_empty());
    }

    #[test]
    fn filtered_relationships_define_the_projection_without_renumbering() {
        let log = log();
        let view = OcelView::apply(
            &log,
            &OcelFilter {
                event_object_qualifiers: vec!["order".into()],
                ..Default::default()
            },
        )
        .unwrap();

        let projection = project_ocel(&log, &view, ObjectTypeId(0));
        assert_eq!(projection.projected_cases, 2);
        assert_eq!(projection.included_unique_events, 4);
        assert_eq!(projection.divergence_candidates, 0);
        assert_eq!(projection.cases[0].case_object, ObjectId(0));
        assert_eq!(projection.cases[1].case_object, ObjectId(1));
    }
}
