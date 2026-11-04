//! Object-type-specific lifecycle variants and source lifecycle inspection.

use crate::{EventTypeId, ObjectId, ObjectTypeId, OcelEventId, OcelLog, OcelView, QualifierId};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const EVIDENCE_LIMIT: usize = 5;

/// One timestamp layer in an object lifecycle. Several activities in a layer
/// are simultaneous; their sorted order is canonical, not temporal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct OcelVariantLayer {
    pub activities: Vec<EventTypeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelVariant {
    pub object_type: ObjectTypeId,
    /// Rank starts at one within each object type.
    pub rank: usize,
    pub layers: Vec<OcelVariantLayer>,
    pub objects: u64,
    pub share_millionths: u64,
    pub duration_eligible_objects: u64,
    pub mean_duration_micros: Option<i64>,
    pub evidence_objects: Vec<ObjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelLifecycleEvent {
    pub event: OcelEventId,
    pub event_type: EventTypeId,
    pub at: i64,
    pub time_layer: usize,
    pub qualifiers: Vec<QualifierId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelLifecycle {
    pub object: ObjectId,
    pub object_type: ObjectTypeId,
    pub events: Vec<OcelLifecycleEvent>,
    pub duration_micros: Option<i64>,
}

#[derive(Default)]
struct VariantAggregate {
    objects: u64,
    eligible: u64,
    total_duration: i128,
    evidence: Vec<ObjectId>,
}

pub fn discover_ocel_variants(log: &OcelLog, view: &OcelView) -> Vec<OcelVariant> {
    let mut fold = OcelVariantFold::default();
    crate::for_each_timeline(log, view, |object, object_type, timeline| {
        fold.object(object, object_type, timeline);
    });
    fold.finish()
}

/// The variants, built one object timeline at a time in object order, from
/// memory or from disk alike.
#[derive(Default)]
pub struct OcelVariantFold {
    totals: BTreeMap<ObjectTypeId, u64>,
    variants: BTreeMap<(ObjectTypeId, Vec<OcelVariantLayer>), VariantAggregate>,
}

impl OcelVariantFold {
    pub fn object(
        &mut self,
        object: ObjectId,
        object_type: ObjectTypeId,
        timeline: &[crate::TimelineStep],
    ) {
        *self.totals.entry(object_type).or_default() += 1;
        let layers = timeline
            .chunk_by(|left, right| left.0 == right.0)
            .map(|layer| {
                let mut activities = layer
                    .iter()
                    .map(|(_, _, event_type)| *event_type)
                    .collect::<Vec<_>>();
                activities.sort_unstable();
                OcelVariantLayer { activities }
            })
            .collect::<Vec<_>>();
        let aggregate = self.variants.entry((object_type, layers)).or_default();
        aggregate.objects = aggregate.objects.saturating_add(1);
        if let (Some(first), Some(last)) = (timeline.first(), timeline.last()) {
            aggregate.eligible = aggregate.eligible.saturating_add(1);
            aggregate.total_duration += i128::from(last.0.saturating_sub(first.0));
        }
        if aggregate.evidence.len() < EVIDENCE_LIMIT {
            aggregate.evidence.push(object);
        }
    }

    pub fn finish(self) -> Vec<OcelVariant> {
        let mut grouped = BTreeMap::<ObjectTypeId, Vec<_>>::new();
        for ((object_type, layers), aggregate) in self.variants {
            grouped
                .entry(object_type)
                .or_default()
                .push((layers, aggregate));
        }
        let mut output = Vec::new();
        for (object_type, mut group) in grouped {
            group.sort_by(|(left_layers, left), (right_layers, right)| {
                right
                    .objects
                    .cmp(&left.objects)
                    .then_with(|| left_layers.cmp(right_layers))
            });
            let total = self.totals[&object_type];
            for (index, (layers, aggregate)) in group.into_iter().enumerate() {
                output.push(OcelVariant {
                    object_type,
                    rank: index + 1,
                    layers,
                    objects: aggregate.objects,
                    share_millionths: ratio_millionths(aggregate.objects, total),
                    duration_eligible_objects: aggregate.eligible,
                    mean_duration_micros: (aggregate.eligible > 0).then(|| {
                        let mean = aggregate.total_duration / i128::from(aggregate.eligible);
                        i64::try_from(mean).unwrap_or(i64::MAX)
                    }),
                    evidence_objects: aggregate.evidence,
                });
            }
        }
        output
    }
}

pub fn inspect_ocel_lifecycle(
    log: &OcelLog,
    view: &OcelView,
    object: ObjectId,
) -> Option<OcelLifecycle> {
    if view.objects.binary_search(&object).is_err() {
        return None;
    }
    let mut by_event = BTreeMap::<OcelEventId, BTreeSet<QualifierId>>::new();
    for index in &view.event_object_relations {
        let relation = log.event_object_relations[*index];
        if relation.object == object {
            by_event
                .entry(relation.event)
                .or_default()
                .insert(relation.qualifier);
        }
    }
    let mut ordered = by_event.into_iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(event, _)| (log.events[event.0 as usize].at, *event));
    let mut previous_time = None;
    let mut layer = 0usize;
    let events = ordered
        .into_iter()
        .map(|(event_id, qualifiers)| {
            let event = log.events[event_id.0 as usize];
            if previous_time.is_some_and(|previous| previous != event.at) {
                layer += 1;
            }
            previous_time = Some(event.at);
            OcelLifecycleEvent {
                event: event_id,
                event_type: event.event_type,
                at: event.at,
                time_layer: layer,
                qualifiers: qualifiers.into_iter().collect(),
            }
        })
        .collect::<Vec<_>>();
    let duration_micros = events
        .first()
        .zip(events.last())
        .map(|(first, last)| last.at.saturating_sub(first.at));
    Some(OcelLifecycle {
        object,
        object_type: log.objects[object.0 as usize].object_type,
        events,
        duration_micros,
    })
}

fn ratio_millionths(part: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    let numerator = u128::from(part)
        .saturating_mul(1_000_000)
        .saturating_add(u128::from(total / 2));
    u64::try_from(numerator / u128::from(total)).unwrap_or(1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OcelBuilder, OcelFilter};

    fn log() -> OcelLog {
        let mut builder = OcelBuilder::new();
        for event_type in ["create", "check", "finish"] {
            builder.declare_event_type(event_type, vec![]).unwrap();
        }
        builder.declare_object_type("order", vec![]).unwrap();
        for (id, event_type, at) in [
            ("e1", "create", 10),
            ("e2", "check", 20),
            ("e3", "finish", 30),
            ("e4", "create", 10),
            ("e5", "finish", 20),
            ("e6", "check", 20),
        ] {
            builder.add_event(id, event_type, at).unwrap();
        }
        for object in ["o1", "o2", "o3", "o4"] {
            builder.add_object(object, "order").unwrap();
        }
        for (event, object) in [
            ("e1", "o1"),
            ("e2", "o1"),
            ("e3", "o1"),
            ("e1", "o2"),
            ("e2", "o2"),
            ("e3", "o2"),
            ("e4", "o3"),
            ("e5", "o3"),
            ("e6", "o3"),
        ] {
            builder.relate_event_object(event, object, "order").unwrap();
        }
        builder.build()
    }

    #[test]
    fn variants_are_ranked_within_object_type_and_reconcile() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let variants = discover_ocel_variants(&log, &view);
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].objects, 2);
        assert_eq!(variants[0].share_millionths, 500_000);
        assert_eq!(
            variants.iter().map(|variant| variant.objects).sum::<u64>(),
            4
        );
        assert_eq!(
            variants
                .iter()
                .map(|variant| variant.share_millionths)
                .sum::<u64>(),
            1_000_000
        );
    }

    #[test]
    fn simultaneous_events_form_one_unordered_layer() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let variant = discover_ocel_variants(&log, &view)
            .into_iter()
            .find(|variant| variant.evidence_objects == [ObjectId(2)])
            .unwrap();
        assert_eq!(variant.layers.len(), 2);
        assert_eq!(
            variant.layers[1].activities,
            [EventTypeId(1), EventTypeId(2)]
        );
    }

    #[test]
    fn objects_without_events_are_an_explicit_empty_variant() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let variant = discover_ocel_variants(&log, &view)
            .into_iter()
            .find(|variant| variant.layers.is_empty())
            .unwrap();
        assert_eq!(variant.objects, 1);
        assert_eq!(variant.duration_eligible_objects, 0);
        assert_eq!(variant.mean_duration_micros, None);
    }

    #[test]
    fn lifecycle_preserves_original_events_roles_and_time_layers() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let lifecycle = inspect_ocel_lifecycle(&log, &view, ObjectId(2)).unwrap();
        assert_eq!(lifecycle.duration_micros, Some(10));
        assert_eq!(lifecycle.events.len(), 3);
        assert_eq!(lifecycle.events[0].time_layer, 0);
        assert_eq!(lifecycle.events[1].time_layer, 1);
        assert_eq!(lifecycle.events[2].time_layer, 1);
        assert_eq!(lifecycle.events[0].qualifiers.len(), 1);
    }

    #[test]
    fn lifecycle_respects_the_active_object_view() {
        let log = log();
        let view = OcelView::apply(
            &log,
            &OcelFilter {
                event_object_qualifiers: vec!["order".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(inspect_ocel_lifecycle(&log, &view, ObjectId(0)).is_some());
        assert!(inspect_ocel_lifecycle(&log, &view, ObjectId(99)).is_none());
    }

    #[test]
    fn variants_and_lifecycles_are_deterministic() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        assert_eq!(
            discover_ocel_variants(&log, &view),
            discover_ocel_variants(&log, &view)
        );
        assert_eq!(
            inspect_ocel_lifecycle(&log, &view, ObjectId(0)),
            inspect_ocel_lifecycle(&log, &view, ObjectId(0))
        );
    }
}
