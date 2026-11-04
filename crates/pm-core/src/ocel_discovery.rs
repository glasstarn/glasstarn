//! Object-centric Petri-net discovery by typed lifecycle decomposition.
//!
//! Each retained object type is projected explicitly for mining, a sound
//! ordinary net is discovered with Inductive Miner, and equally labelled
//! visible transitions are merged. Places remain typed and variable arcs are
//! inferred from observed event/object multiplicity.

use crate::{
    ActivityId, EventTypeId, InductiveMinerConfig, ObjectId, ObjectTypeId, OcelLog, OcelView,
    PlaceId, TransitionId, discover_inductive_from_variants,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OcelDiscoveryConfig {
    pub noise_threshold_milli: u16,
    /// An arc is variable when the share of participating events containing
    /// exactly one object of the type is below this threshold.
    pub variable_arc_threshold_milli: u16,
}

impl Default for OcelDiscoveryConfig {
    fn default() -> Self {
        Self {
            noise_threshold_milli: 200,
            variable_arc_threshold_milli: 1_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OcelPlaceId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OcelTransitionId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelPlace {
    pub id: OcelPlaceId,
    pub object_type: ObjectTypeId,
    pub initial: bool,
    pub final_place: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelArcEndpoint {
    pub place: OcelPlaceId,
    pub variable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelTransition {
    pub id: OcelTransitionId,
    pub event_type: Option<EventTypeId>,
    pub inputs: Vec<OcelArcEndpoint>,
    pub outputs: Vec<OcelArcEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelObjectTypeDiscovery {
    pub object_type: ObjectTypeId,
    pub objects: u64,
    pub objects_with_events: u64,
    pub lifecycle_event_occurrences: u64,
    pub filtered_edges: usize,
    pub used_flower_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelPetriNet {
    pub places: Vec<OcelPlace>,
    pub transitions: Vec<OcelTransition>,
    pub object_types: Vec<OcelObjectTypeDiscovery>,
    pub variable_arcs: u64,
    pub fixed_arcs: u64,
    pub noise_threshold_milli: u16,
    pub variable_arc_threshold_milli: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OcelDiscoveryError {
    #[error("noise and variable-arc thresholds must be between 0 and 1000 thousandths")]
    InvalidThreshold,
    #[error(
        "object {object:?} has several related events at timestamp {at}; object-centric discovery will not invent an order within a simultaneous layer"
    )]
    SimultaneousEvents { object: ObjectId, at: i64 },
    #[error(transparent)]
    Discovery(#[from] crate::DiscoveryError),
}

pub fn discover_ocel_petri_net(
    log: &OcelLog,
    view: &OcelView,
    config: OcelDiscoveryConfig,
) -> Result<OcelPetriNet, OcelDiscoveryError> {
    let mut fold = OcelDiscoveryFold::default();
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
        fold.event(log.events[run[0].0.0 as usize].event_type, &types);
    }
    crate::for_each_timeline(log, view, |object, object_type, timeline| {
        fold.object(object, object_type, timeline);
    });
    fold.finish(config)
}

/// What discovery needs from the log, built one piece at a time: each
/// event's item types in event order, and each item's timeline in item
/// order. The miner reads only each type's variants with their counts.
#[derive(Default)]
pub struct OcelDiscoveryFold {
    types: BTreeMap<ObjectTypeId, TypeLifecycles>,
    multiplicity: BTreeMap<(EventTypeId, ObjectTypeId), (u64, u64)>,
    simultaneous: Option<(ObjectId, i64)>,
}

#[derive(Default)]
struct TypeLifecycles {
    objects: u64,
    objects_with_events: u64,
    occurrences: u64,
    variants: BTreeMap<Vec<ActivityId>, u64>,
}

impl OcelDiscoveryFold {
    /// An event's distinct items' types, one per item.
    pub fn event(&mut self, event_type: EventTypeId, object_types: &[ObjectTypeId]) {
        let mut by_type = BTreeMap::<ObjectTypeId, u64>::new();
        for object_type in object_types {
            *by_type.entry(*object_type).or_default() += 1;
        }
        for (object_type, objects) in by_type {
            let aggregate = self
                .multiplicity
                .entry((event_type, object_type))
                .or_default();
            aggregate.0 = aggregate.0.saturating_add(1);
            if objects == 1 {
                aggregate.1 = aggregate.1.saturating_add(1);
            }
        }
    }

    /// An item's timeline, in item order.
    pub fn object(
        &mut self,
        object: ObjectId,
        object_type: ObjectTypeId,
        timeline: &[crate::TimelineStep],
    ) {
        let lifecycles = self.types.entry(object_type).or_default();
        lifecycles.objects = lifecycles.objects.saturating_add(1);
        if timeline.is_empty() {
            return;
        }
        if self.simultaneous.is_none()
            && let Some(pair) = timeline.windows(2).find(|pair| pair[0].0 == pair[1].0)
        {
            self.simultaneous = Some((object, pair[0].0));
        }
        lifecycles.objects_with_events = lifecycles.objects_with_events.saturating_add(1);
        lifecycles.occurrences = lifecycles.occurrences.saturating_add(count(timeline.len()));
        *lifecycles
            .variants
            .entry(
                timeline
                    .iter()
                    .map(|(_, _, event_type)| ActivityId(event_type.0))
                    .collect(),
            )
            .or_default() += 1;
    }

    pub fn finish(self, config: OcelDiscoveryConfig) -> Result<OcelPetriNet, OcelDiscoveryError> {
        if config.noise_threshold_milli > 1_000 || config.variable_arc_threshold_milli > 1_000 {
            return Err(OcelDiscoveryError::InvalidThreshold);
        }
        if let Some((object, at)) = self.simultaneous {
            return Err(OcelDiscoveryError::SimultaneousEvents { object, at });
        }
        let multiplicity = self
            .multiplicity
            .into_iter()
            .map(|(key, (events, exactly_one))| {
                let score = u128::from(exactly_one)
                    .saturating_mul(1_000)
                    .checked_div(u128::from(events))
                    .unwrap_or(0);
                (key, u16::try_from(score).unwrap_or(1_000))
            })
            .collect::<BTreeMap<_, _>>();

        let mut places = Vec::new();
        let mut transitions = Vec::<OcelTransition>::new();
        let mut visible_transitions = BTreeMap::<EventTypeId, OcelTransitionId>::new();
        let mut object_types = Vec::new();

        for (object_type, lifecycles) in self.types {
            let model = discover_inductive_from_variants(
                lifecycles.variants,
                InductiveMinerConfig {
                    noise_threshold_milli: config.noise_threshold_milli,
                },
            )?;
            let place_offset = places.len();
            for local in 0..model.net.place_count {
                places.push(OcelPlace {
                    id: OcelPlaceId(u32::try_from(places.len()).unwrap_or(u32::MAX)),
                    object_type,
                    initial: model.net.initial_marking[local] > 0,
                    final_place: model.net.final_marking[local] > 0,
                });
            }

            let mut transition_mapping = BTreeMap::<TransitionId, OcelTransitionId>::new();
            for local in &model.net.transitions {
                let event_type = local.activity.map(|activity| EventTypeId(activity.0));
                let global = if let Some(event_type) = event_type {
                    *visible_transitions.entry(event_type).or_insert_with(|| {
                        let id =
                            OcelTransitionId(u32::try_from(transitions.len()).unwrap_or(u32::MAX));
                        transitions.push(OcelTransition {
                            id,
                            event_type: Some(event_type),
                            inputs: Vec::new(),
                            outputs: Vec::new(),
                        });
                        id
                    })
                } else {
                    let id = OcelTransitionId(u32::try_from(transitions.len()).unwrap_or(u32::MAX));
                    transitions.push(OcelTransition {
                        id,
                        event_type: None,
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                    });
                    id
                };
                transition_mapping.insert(local.id, global);
            }

            for local in &model.net.transitions {
                let global = transition_mapping[&local.id];
                let variable = local
                    .activity
                    .map(|activity| {
                        multiplicity
                            .get(&(EventTypeId(activity.0), object_type))
                            .is_some_and(|score| *score < config.variable_arc_threshold_milli)
                    })
                    .unwrap_or(false);
                let transition = &mut transitions[global.0 as usize];
                transition
                    .inputs
                    .extend(local.inputs.iter().map(|place| OcelArcEndpoint {
                        place: global_place(place_offset, *place),
                        variable,
                    }));
                transition
                    .outputs
                    .extend(local.outputs.iter().map(|place| OcelArcEndpoint {
                        place: global_place(place_offset, *place),
                        variable,
                    }));
            }
            object_types.push(OcelObjectTypeDiscovery {
                object_type,
                objects: lifecycles.objects,
                objects_with_events: lifecycles.objects_with_events,
                lifecycle_event_occurrences: lifecycles.occurrences,
                filtered_edges: model.filtered_edges,
                used_flower_fallback: model.used_flower_fallback,
            });
        }

        for transition in &mut transitions {
            transition.inputs.sort_by_key(|arc| arc.place);
            transition.outputs.sort_by_key(|arc| arc.place);
        }
        let variable_arcs = transitions
            .iter()
            .flat_map(|transition| transition.inputs.iter().chain(&transition.outputs))
            .filter(|arc| arc.variable)
            .count();
        let total_arcs = transitions
            .iter()
            .map(|transition| transition.inputs.len() + transition.outputs.len())
            .sum::<usize>();
        Ok(OcelPetriNet {
            places,
            transitions,
            object_types,
            variable_arcs: count(variable_arcs),
            fixed_arcs: count(total_arcs.saturating_sub(variable_arcs)),
            noise_threshold_milli: config.noise_threshold_milli,
            variable_arc_threshold_milli: config.variable_arc_threshold_milli,
        })
    }
}

fn global_place(offset: usize, place: PlaceId) -> OcelPlaceId {
    OcelPlaceId(u32::try_from(offset.saturating_add(place.0 as usize)).unwrap_or(u32::MAX))
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OcelBuilder, OcelFilter};
    use std::collections::BTreeSet;

    fn log() -> OcelLog {
        let mut builder = OcelBuilder::new();
        for activity in ["place", "pack", "deliver"] {
            builder.declare_event_type(activity, vec![]).unwrap();
        }
        for object_type in ["order", "item", "package"] {
            builder.declare_object_type(object_type, vec![]).unwrap();
        }
        for (event, activity, at) in [("e1", "place", 1), ("e2", "pack", 2), ("e3", "deliver", 3)] {
            builder.add_event(event, activity, at).unwrap();
        }
        builder.add_object("o1", "order").unwrap();
        builder.add_object("i1", "item").unwrap();
        builder.add_object("i2", "item").unwrap();
        builder.add_object("p1", "package").unwrap();
        builder.relate_event_object("e1", "o1", "order").unwrap();
        for item in ["i1", "i2"] {
            builder.relate_event_object("e1", item, "item").unwrap();
            builder.relate_event_object("e2", item, "item").unwrap();
        }
        builder.relate_event_object("e2", "p1", "package").unwrap();
        builder.relate_event_object("e3", "p1", "package").unwrap();
        builder.build()
    }

    #[test]
    fn discovers_typed_places_and_merges_shared_visible_transitions() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let model = discover_ocel_petri_net(&log, &view, OcelDiscoveryConfig::default()).unwrap();

        assert_eq!(model.object_types.len(), 3);
        assert_eq!(
            model
                .transitions
                .iter()
                .filter(|transition| transition.event_type == Some(EventTypeId(0)))
                .count(),
            1
        );
        let pack = model
            .transitions
            .iter()
            .find(|transition| transition.event_type == Some(EventTypeId(1)))
            .unwrap();
        let pack_types = pack
            .inputs
            .iter()
            .chain(&pack.outputs)
            .map(|arc| model.places[arc.place.0 as usize].object_type)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            pack_types,
            BTreeSet::from([ObjectTypeId(1), ObjectTypeId(2)])
        );
        assert!(pack.inputs.iter().any(|arc| {
            model.places[arc.place.0 as usize].object_type == ObjectTypeId(1) && arc.variable
        }));
        assert!(pack.inputs.iter().any(|arc| {
            model.places[arc.place.0 as usize].object_type == ObjectTypeId(2) && !arc.variable
        }));
        assert!(model.variable_arcs > 0);
        assert!(model.fixed_arcs > 0);
    }

    #[test]
    fn filters_choose_the_discovered_object_types() {
        let log = log();
        let view = OcelView::apply(
            &log,
            &OcelFilter {
                object_types: vec!["package".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let model = discover_ocel_petri_net(&log, &view, OcelDiscoveryConfig::default()).unwrap();
        assert_eq!(model.object_types.len(), 1);
        assert!(
            model
                .places
                .iter()
                .all(|place| place.object_type == ObjectTypeId(2))
        );
        assert!(
            model
                .transitions
                .iter()
                .filter_map(|transition| transition.event_type)
                .all(|event_type| event_type != EventTypeId(0))
        );
    }

    #[test]
    fn refuses_to_invent_order_for_simultaneous_lifecycle_events() {
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
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        assert!(matches!(
            discover_ocel_petri_net(&log, &view, OcelDiscoveryConfig::default()),
            Err(OcelDiscoveryError::SimultaneousEvents { .. })
        ));
    }
}
