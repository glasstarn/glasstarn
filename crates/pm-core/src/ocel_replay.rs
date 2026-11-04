//! Deterministic token replay for typed object-centric Petri nets.
//!
//! Control-flow is replayed independently for every retained object, while
//! event bindings are checked against the fixed/variable arcs of the shared
//! transition. This preserves object identity and catches both lifecycle and
//! cross-object multiplicity deviations without flattening to one case notion.

use crate::{
    ActivityId, AlignmentMove, CaseId, CaseTrace, Event, EventLog, EventTypeId, ObjectId,
    ObjectTypeId, OcelEventId, OcelPetriNet, OcelPlaceId, OcelTransitionId, OcelView, PetriNet,
    PetriTransition, PlaceId, ReplayDeviation, TransitionId, align_log, replay_tokens,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const FITNESS_SCALE: u64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OcelReplayDeviation {
    ActivityNotInModel {
        event: OcelEventId,
        event_type: EventTypeId,
        object: ObjectId,
    },
    MissingTokens {
        event: OcelEventId,
        event_type: EventTypeId,
        object: ObjectId,
        count: u64,
    },
    FinalTokensMissing {
        object: ObjectId,
        count: u64,
    },
    TokensRemaining {
        object: ObjectId,
        count: u64,
    },
    MissingObjectParticipation {
        event: OcelEventId,
        transition: OcelTransitionId,
        object_type: ObjectTypeId,
        variable: bool,
    },
    FixedArcMultiplicity {
        event: OcelEventId,
        transition: OcelTransitionId,
        object_type: ObjectTypeId,
        observed_objects: u64,
    },
    UnexpectedObjectParticipation {
        event: OcelEventId,
        transition: OcelTransitionId,
        object_type: ObjectTypeId,
        observed_objects: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelObjectReplay {
    pub object: ObjectId,
    pub object_type: ObjectTypeId,
    pub events: u64,
    pub fitness_millionths: u64,
    pub consumed_tokens: u64,
    pub produced_tokens: u64,
    pub missing_tokens: u64,
    pub remaining_tokens: u64,
    pub unmapped_events: u64,
    pub silent_firings: u64,
    pub lifecycle_fit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelTokenReplay {
    pub objects: Vec<OcelObjectReplay>,
    pub deviations: Vec<OcelReplayDeviation>,
    pub fitness_millionths: u64,
    pub lifecycle_fit_objects: u64,
    pub replayed_objects: u64,
    pub fit_events: u64,
    pub replayed_events: u64,
    pub out_of_scope_events: u64,
    pub consumed_tokens: u64,
    pub produced_tokens: u64,
    pub missing_tokens: u64,
    pub remaining_tokens: u64,
    pub unmapped_event_occurrences: u64,
    pub binding_violations: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OcelAlignmentMove {
    Synchronous {
        event: OcelEventId,
        event_type: EventTypeId,
        transition: OcelTransitionId,
    },
    InvisibleModel {
        transition: OcelTransitionId,
    },
    LogOnly {
        event: OcelEventId,
        event_type: EventTypeId,
    },
    VisibleModel {
        transition: OcelTransitionId,
        event_type: EventTypeId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelObjectAlignment {
    pub object: ObjectId,
    pub object_type: ObjectTypeId,
    pub events: u64,
    pub cost: u64,
    pub denominator: u64,
    pub fitness_millionths: u64,
    pub synchronous_moves: u64,
    pub invisible_moves: u64,
    pub log_moves: u64,
    pub visible_model_moves: u64,
    pub fit: bool,
    pub moves: Vec<OcelAlignmentMove>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelAlignmentConformance {
    pub objects: Vec<OcelObjectAlignment>,
    pub binding_deviations: Vec<OcelReplayDeviation>,
    pub total_cost: u64,
    pub denominator: u64,
    pub fitness_millionths: u64,
    pub lifecycle_fit_objects: u64,
    pub aligned_objects: u64,
    pub fit_events: u64,
    pub aligned_events: u64,
    pub out_of_scope_events: u64,
    pub binding_violations: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum OcelReplayError {
    #[error("place identifiers must be contiguous and in storage order")]
    InvalidPlaceId,
    #[error("the object-centric model must contain at least one typed place and transition")]
    EmptyModel,
    #[error("transition identifiers must be contiguous and in storage order")]
    InvalidTransitionId,
    #[error("the model refers to an object type outside the log schema")]
    UnknownObjectType,
    #[error("the model refers to an event type outside the log schema")]
    UnknownEventType,
    #[error("a transition refers to a place outside the model")]
    UnknownPlace,
    #[error("a transition contains a duplicate input or output place")]
    DuplicateArc,
    #[error("every transition must have matching input and output object types")]
    UnbalancedTransitionTypes,
    #[error("input and output arcs disagree about fixed or variable multiplicity")]
    InconsistentArcMultiplicity,
    #[error("each object type needs exactly one initial and one final place")]
    InvalidObjectTypeMarking,
    #[error("a silent transition may route only one object type")]
    CrossTypeSilentTransition,
    #[error("an activity may have only one shared visible transition")]
    DuplicateVisibleTransition,
    #[error(
        "object {object:?} has several related events at timestamp {at}; replay will not invent an order within a simultaneous layer"
    )]
    SimultaneousEvents { object: ObjectId, at: i64 },
    #[error(transparent)]
    Alignment(#[from] crate::AlignmentError),
    #[error(transparent)]
    Replay(#[from] crate::ReplayError),
}

/// Optimally aligns every active object lifecycle and separately validates
/// the shared-event object bindings of the same explicit typed model.
pub fn align_ocel(
    log: &crate::OcelLog,
    view: &OcelView,
    model: &OcelPetriNet,
) -> Result<OcelAlignmentConformance, OcelReplayError> {
    let names = log
        .event_types
        .iter()
        .map(|t| t.name.clone())
        .collect::<Vec<_>>();
    let mut fold = OcelConformanceFold::new(
        &names,
        log.object_types.len(),
        model,
        &view_types(log, view),
    )?;
    let mut binding_deviations = Vec::new();
    for_each_view_event(log, view, |event, event_type, object_types| {
        fold.event(event, event_type, object_types, |deviation| {
            binding_deviations.push(deviation)
        });
    });
    let mut objects = Vec::new();
    crate::for_each_timeline(log, view, |object, object_type, timeline| {
        if let Some(aligned) = fold.align_object(object, object_type, timeline) {
            objects.push(aligned);
        }
    });
    fold.check()?;
    let mut deviating = binding_deviations
        .iter()
        .filter_map(deviation_event)
        .collect::<BTreeSet<_>>();
    for object in &objects {
        deviating.extend(object.moves.iter().filter_map(|movement| match movement {
            OcelAlignmentMove::LogOnly { event, .. } => Some(*event),
            _ => None,
        }));
    }
    Ok(fold.alignment_totals(objects, binding_deviations, count(deviating.len())))
}

/// Replays the active OCEL view against an explicit typed model.
pub fn replay_ocel_tokens(
    log: &crate::OcelLog,
    view: &OcelView,
    model: &OcelPetriNet,
) -> Result<OcelTokenReplay, OcelReplayError> {
    let names = log
        .event_types
        .iter()
        .map(|t| t.name.clone())
        .collect::<Vec<_>>();
    let mut fold = OcelConformanceFold::new(
        &names,
        log.object_types.len(),
        model,
        &view_types(log, view),
    )?;
    let mut deviations = Vec::new();
    for_each_view_event(log, view, |event, event_type, object_types| {
        fold.event(event, event_type, object_types, |deviation| {
            deviations.push(deviation)
        });
    });
    let mut objects = Vec::new();
    crate::for_each_timeline(log, view, |object, object_type, timeline| {
        if let Some((replayed, found)) = fold.replay_object(object, object_type, timeline) {
            objects.push(replayed);
            deviations.extend(found);
        }
    });
    fold.check()?;
    let deviating = count(
        deviations
            .iter()
            .filter_map(deviation_event)
            .collect::<BTreeSet<_>>()
            .len(),
    );
    Ok(fold.replay_totals(objects, deviations, deviating))
}

/// The item types of a view's items.
fn view_types(log: &crate::OcelLog, view: &OcelView) -> BTreeSet<ObjectTypeId> {
    view.objects
        .iter()
        .map(|object| log.objects[object.0 as usize].object_type)
        .collect()
}

/// Every event of the view in event order, with the types of its distinct
/// items in item order.
fn for_each_view_event(
    log: &crate::OcelLog,
    view: &OcelView,
    mut f: impl FnMut(OcelEventId, EventTypeId, &[ObjectTypeId]),
) {
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
        f(run[0].0, log.events[run[0].0.0 as usize].event_type, &types);
    }
}

/// Conformance against a typed model, one piece at a time: each event of
/// the view with its items' types, in event order, then each item's
/// timeline, in item order. Results are handed back as they are found;
/// totals are kept here.
pub struct OcelConformanceFold<'m> {
    model: &'m OcelPetriNet,
    active: BTreeSet<ObjectTypeId>,
    transitions: BTreeMap<EventTypeId, &'m crate::OcelTransition>,
    activities: crate::Dictionary,
    nets: BTreeMap<ObjectTypeId, (PetriNet, Vec<OcelTransitionId>)>,
    simultaneous: Option<(ObjectId, i64)>,
    failed: Option<OcelReplayError>,
    view_events: u64,
    modeled_events: u64,
    binding_violations: u64,
    objects: u64,
    fit_objects: u64,
    consumed: u64,
    produced: u64,
    missing: u64,
    remaining: u64,
    unmapped: u64,
    cost: u64,
    denominator: u64,
}

impl<'m> OcelConformanceFold<'m> {
    /// Checks the model against the log's activities and item types.
    pub fn new(
        activities: &[String],
        object_types: usize,
        model: &'m OcelPetriNet,
        view_types: &BTreeSet<ObjectTypeId>,
    ) -> Result<Self, OcelReplayError> {
        validate_model(activities.len(), object_types, model)?;
        let active = model
            .places
            .iter()
            .map(|place| place.object_type)
            .filter(|kind| view_types.contains(kind))
            .collect::<BTreeSet<_>>();
        let mut names = crate::Dictionary::new();
        for name in activities {
            names.intern(name);
        }
        Ok(Self {
            model,
            active,
            transitions: model
                .transitions
                .iter()
                .filter_map(|transition| transition.event_type.map(|kind| (kind, transition)))
                .collect(),
            activities: names,
            nets: BTreeMap::new(),
            simultaneous: None,
            failed: None,
            view_events: 0,
            modeled_events: 0,
            binding_violations: 0,
            objects: 0,
            fit_objects: 0,
            consumed: 0,
            produced: 0,
            missing: 0,
            remaining: 0,
            unmapped: 0,
            cost: 0,
            denominator: 0,
        })
    }

    /// An event of the view with its distinct items' types; binding
    /// deviations go to `deviation` in order.
    pub fn event(
        &mut self,
        event: OcelEventId,
        event_type: EventTypeId,
        object_types: &[ObjectTypeId],
        mut deviation: impl FnMut(OcelReplayDeviation),
    ) {
        self.view_events += 1;
        let observed = object_types
            .iter()
            .filter(|kind| self.active.contains(kind))
            .fold(BTreeMap::<ObjectTypeId, u64>::new(), |mut map, kind| {
                *map.entry(*kind).or_default() += 1;
                map
            });
        let transition = self.transitions.get(&event_type).copied();
        let modeled = !observed.is_empty()
            || transition.is_some_and(|transition| {
                transition.inputs.iter().any(|arc| {
                    place(self.model, arc.place)
                        .is_ok_and(|place| self.active.contains(&place.object_type))
                })
            });
        if modeled {
            self.modeled_events += 1;
        }
        let Some(transition) = transition else {
            return;
        };
        let mut expected = BTreeMap::<ObjectTypeId, bool>::new();
        for arc in &transition.inputs {
            if let Ok(place) = place(self.model, arc.place) {
                if !self.active.contains(&place.object_type) {
                    continue;
                }
                expected
                    .entry(place.object_type)
                    .and_modify(|variable| *variable |= arc.variable)
                    .or_insert(arc.variable);
            }
        }
        let mut violations = 0;
        let mut found = |found: OcelReplayDeviation| {
            violations += 1;
            deviation(found);
        };
        for (object_type, variable) in &expected {
            let count = observed.get(object_type).copied().unwrap_or(0);
            if count == 0 {
                found(OcelReplayDeviation::MissingObjectParticipation {
                    event,
                    transition: transition.id,
                    object_type: *object_type,
                    variable: *variable,
                });
            } else if !variable && count != 1 {
                found(OcelReplayDeviation::FixedArcMultiplicity {
                    event,
                    transition: transition.id,
                    object_type: *object_type,
                    observed_objects: count,
                });
            }
        }
        for (object_type, count) in observed {
            if !expected.contains_key(&object_type) {
                found(OcelReplayDeviation::UnexpectedObjectParticipation {
                    event,
                    transition: transition.id,
                    object_type,
                    observed_objects: count,
                });
            }
        }
        self.binding_violations += violations;
    }

    /// The item's events as a one-case log, when it can be replayed.
    fn object_log(
        &mut self,
        object: ObjectId,
        object_type: ObjectTypeId,
        timeline: &[crate::TimelineStep],
    ) -> Option<(EventLog, Vec<OcelEventId>)> {
        if !self.active.contains(&object_type) {
            return None;
        }
        if self.simultaneous.is_none()
            && let Some(pair) = timeline.windows(2).find(|pair| pair[0].0 == pair[1].0)
        {
            self.simultaneous = Some((object, pair[0].0));
        }
        if self.simultaneous.is_some() || self.failed.is_some() {
            return None;
        }
        if !self.nets.contains_key(&object_type) {
            match type_net_with_transitions(self.model, object_type) {
                Ok(net) => {
                    self.nets.insert(object_type, net);
                }
                Err(error) => {
                    self.failed = Some(error);
                    return None;
                }
            }
        }
        let mut event_log = EventLog {
            activities: self.activities.clone(),
            ..EventLog::default()
        };
        let case = CaseId(event_log.cases.intern("object"));
        event_log.traces.push(CaseTrace {
            case,
            events: timeline
                .iter()
                .map(|(at, event, event_type)| {
                    Event::new(ActivityId(event_type.0), *at, event.0.saturating_add(1))
                })
                .collect(),
            incomplete: false,
            attributes: Vec::new(),
        });
        Some((
            event_log,
            timeline.iter().map(|(_, event, _)| *event).collect(),
        ))
    }

    /// Token replay of one item's timeline, with its deviations in order.
    pub fn replay_object(
        &mut self,
        object: ObjectId,
        object_type: ObjectTypeId,
        timeline: &[crate::TimelineStep],
    ) -> Option<(OcelObjectReplay, Vec<OcelReplayDeviation>)> {
        let (event_log, source_events) = self.object_log(object, object_type, timeline)?;
        let net = &self.nets[&object_type].0;
        let replay = match replay_tokens(&event_log, net) {
            Ok(replay) => replay,
            Err(error) => {
                self.failed = Some(error.into());
                return None;
            }
        };
        let result = &replay.cases[0];
        let deviations = result
            .deviations
            .iter()
            .filter_map(|deviation| label_deviation(object, deviation, &source_events))
            .collect();
        self.objects += 1;
        self.fit_objects += u64::from(result.fit);
        self.consumed += result.consumed_tokens;
        self.produced += result.produced_tokens;
        self.missing += result.missing_tokens;
        self.remaining += result.remaining_tokens;
        self.unmapped += result.unmapped_events;
        Some((
            OcelObjectReplay {
                object,
                object_type,
                events: count(source_events.len()),
                fitness_millionths: result.fitness_millionths,
                consumed_tokens: result.consumed_tokens,
                produced_tokens: result.produced_tokens,
                missing_tokens: result.missing_tokens,
                remaining_tokens: result.remaining_tokens,
                unmapped_events: result.unmapped_events,
                silent_firings: result.silent_firings,
                lifecycle_fit: result.fit,
            },
            deviations,
        ))
    }

    /// The optimal alignment of one item's timeline.
    pub fn align_object(
        &mut self,
        object: ObjectId,
        object_type: ObjectTypeId,
        timeline: &[crate::TimelineStep],
    ) -> Option<OcelObjectAlignment> {
        let (event_log, source_events) = self.object_log(object, object_type, timeline)?;
        let (net, transition_mapping) = &self.nets[&object_type];
        let alignment = match align_log(&event_log, net) {
            Ok(alignment) => alignment,
            Err(error) => {
                self.failed = Some(error.into());
                return None;
            }
        };
        let aligned = &alignment.cases[0];
        let mut log_move_events = BTreeSet::new();
        let moves = match aligned
            .moves
            .iter()
            .map(|movement| {
                label_alignment_move(
                    movement,
                    &source_events,
                    transition_mapping,
                    &mut log_move_events,
                )
            })
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(moves) => moves,
            Err(error) => {
                self.failed = Some(error);
                return None;
            }
        };
        let kind = |wanted: fn(&AlignmentMove) -> bool| {
            count(
                aligned
                    .moves
                    .iter()
                    .filter(|movement| wanted(movement))
                    .count(),
            )
        };
        self.objects += 1;
        self.fit_objects += u64::from(aligned.fit);
        self.cost += aligned.cost;
        self.denominator += aligned.denominator;
        Some(OcelObjectAlignment {
            object,
            object_type,
            events: count(source_events.len()),
            cost: aligned.cost,
            denominator: aligned.denominator,
            fitness_millionths: aligned.fitness_millionths,
            synchronous_moves: kind(|m| matches!(m, AlignmentMove::Synchronous { .. })),
            invisible_moves: kind(|m| matches!(m, AlignmentMove::InvisibleModel { .. })),
            log_moves: kind(|m| matches!(m, AlignmentMove::LogOnly { .. })),
            visible_model_moves: kind(|m| matches!(m, AlignmentMove::VisibleModel { .. })),
            fit: aligned.fit,
            moves,
        })
    }

    /// Fails as the whole replay fails: simultaneous events on an item
    /// first, then the first item that could not be replayed.
    pub fn check(&mut self) -> Result<(), OcelReplayError> {
        if let Some((object, at)) = self.simultaneous {
            return Err(OcelReplayError::SimultaneousEvents { object, at });
        }
        match self.failed.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// The token replay totals; `deviating_events` is how many distinct
    /// events have a deviation.
    pub fn replay_totals(
        &self,
        objects: Vec<OcelObjectReplay>,
        deviations: Vec<OcelReplayDeviation>,
        deviating_events: u64,
    ) -> OcelTokenReplay {
        OcelTokenReplay {
            objects,
            deviations,
            fitness_millionths: token_fitness(
                self.consumed,
                self.produced,
                self.missing.saturating_add(self.binding_violations),
                self.remaining,
                self.unmapped,
            ),
            lifecycle_fit_objects: self.fit_objects,
            replayed_objects: self.objects,
            fit_events: self.modeled_events.saturating_sub(deviating_events),
            replayed_events: self.modeled_events,
            out_of_scope_events: self.view_events.saturating_sub(self.modeled_events),
            consumed_tokens: self.consumed,
            produced_tokens: self.produced,
            missing_tokens: self.missing,
            remaining_tokens: self.remaining,
            unmapped_event_occurrences: self.unmapped,
            binding_violations: self.binding_violations,
        }
    }

    /// The alignment totals; `deviating_events` is how many distinct events
    /// are log-only moves or have a binding deviation.
    pub fn alignment_totals(
        &self,
        objects: Vec<OcelObjectAlignment>,
        binding_deviations: Vec<OcelReplayDeviation>,
        deviating_events: u64,
    ) -> OcelAlignmentConformance {
        OcelAlignmentConformance {
            total_cost: self.cost,
            denominator: self.denominator,
            fitness_millionths: alignment_fitness(self.cost, self.denominator),
            lifecycle_fit_objects: self.fit_objects,
            aligned_objects: self.objects,
            fit_events: self.modeled_events.saturating_sub(deviating_events),
            aligned_events: self.modeled_events,
            out_of_scope_events: self.view_events.saturating_sub(self.modeled_events),
            binding_violations: self.binding_violations,
            objects,
            binding_deviations,
        }
    }
}

fn validate_model(
    event_types: usize,
    object_types: usize,
    model: &OcelPetriNet,
) -> Result<(), OcelReplayError> {
    if model.places.is_empty() || model.transitions.is_empty() {
        return Err(OcelReplayError::EmptyModel);
    }
    for (index, place) in model.places.iter().enumerate() {
        if place.id.0 as usize != index {
            return Err(OcelReplayError::InvalidPlaceId);
        }
        if place.object_type.0 as usize >= object_types {
            return Err(OcelReplayError::UnknownObjectType);
        }
    }
    let mut visible = BTreeSet::new();
    for (index, transition) in model.transitions.iter().enumerate() {
        if transition.id.0 as usize != index {
            return Err(OcelReplayError::InvalidTransitionId);
        }
        if transition
            .event_type
            .is_some_and(|kind| kind.0 as usize >= event_types)
        {
            return Err(OcelReplayError::UnknownEventType);
        }
        if let Some(kind) = transition.event_type
            && !visible.insert(kind)
        {
            return Err(OcelReplayError::DuplicateVisibleTransition);
        }
        let input_places = transition
            .inputs
            .iter()
            .map(|arc| place(model, arc.place))
            .collect::<Result<Vec<_>, _>>()?;
        let output_places = transition
            .outputs
            .iter()
            .map(|arc| place(model, arc.place))
            .collect::<Result<Vec<_>, _>>()?;
        if input_places.len()
            != transition
                .inputs
                .iter()
                .map(|arc| arc.place)
                .collect::<BTreeSet<_>>()
                .len()
            || output_places.len()
                != transition
                    .outputs
                    .iter()
                    .map(|arc| arc.place)
                    .collect::<BTreeSet<_>>()
                    .len()
        {
            return Err(OcelReplayError::DuplicateArc);
        }
        let input_types = input_places
            .iter()
            .map(|place| place.object_type)
            .collect::<BTreeSet<_>>();
        let output_types = output_places
            .iter()
            .map(|place| place.object_type)
            .collect::<BTreeSet<_>>();
        if input_types.is_empty() || input_types != output_types {
            return Err(OcelReplayError::UnbalancedTransitionTypes);
        }
        for object_type in &input_types {
            let input_modes = transition
                .inputs
                .iter()
                .filter(|arc| {
                    place(model, arc.place).is_ok_and(|place| place.object_type == *object_type)
                })
                .map(|arc| arc.variable)
                .collect::<BTreeSet<_>>();
            let output_modes = transition
                .outputs
                .iter()
                .filter(|arc| {
                    place(model, arc.place).is_ok_and(|place| place.object_type == *object_type)
                })
                .map(|arc| arc.variable)
                .collect::<BTreeSet<_>>();
            if input_modes.len() != 1 || input_modes != output_modes {
                return Err(OcelReplayError::InconsistentArcMultiplicity);
            }
        }
        if transition.event_type.is_none() && input_types.len() != 1 {
            return Err(OcelReplayError::CrossTypeSilentTransition);
        }
    }
    for object_type in model
        .places
        .iter()
        .map(|place| place.object_type)
        .collect::<BTreeSet<_>>()
    {
        if model
            .places
            .iter()
            .filter(|place| place.object_type == object_type && place.initial)
            .count()
            != 1
            || model
                .places
                .iter()
                .filter(|place| place.object_type == object_type && place.final_place)
                .count()
                != 1
        {
            return Err(OcelReplayError::InvalidObjectTypeMarking);
        }
    }
    Ok(())
}

fn place(model: &OcelPetriNet, id: OcelPlaceId) -> Result<&crate::OcelPlace, OcelReplayError> {
    model
        .places
        .get(id.0 as usize)
        .filter(|place| place.id == id)
        .ok_or(OcelReplayError::UnknownPlace)
}

fn type_net_with_transitions(
    model: &OcelPetriNet,
    object_type: ObjectTypeId,
) -> Result<(PetriNet, Vec<OcelTransitionId>), OcelReplayError> {
    let selected_places = model
        .places
        .iter()
        .filter(|place| place.object_type == object_type)
        .collect::<Vec<_>>();
    let place_mapping = selected_places
        .iter()
        .enumerate()
        .map(|(index, place)| (place.id, PlaceId(u32::try_from(index).unwrap_or(u32::MAX))))
        .collect::<BTreeMap<_, _>>();
    let mut transitions = Vec::new();
    let mut transition_mapping = Vec::new();
    for transition in &model.transitions {
        let inputs = transition
            .inputs
            .iter()
            .filter_map(|arc| place_mapping.get(&arc.place).copied())
            .collect::<Vec<_>>();
        let outputs = transition
            .outputs
            .iter()
            .filter_map(|arc| place_mapping.get(&arc.place).copied())
            .collect::<Vec<_>>();
        if inputs.is_empty() && outputs.is_empty() {
            continue;
        }
        if inputs.is_empty() || outputs.is_empty() {
            return Err(OcelReplayError::UnbalancedTransitionTypes);
        }
        transitions.push(PetriTransition {
            id: TransitionId(u32::try_from(transitions.len()).unwrap_or(u32::MAX)),
            activity: transition.event_type.map(|kind| ActivityId(kind.0)),
            inputs,
            outputs,
        });
        transition_mapping.push(transition.id);
    }
    Ok((
        PetriNet {
            place_count: selected_places.len(),
            transitions,
            initial_marking: selected_places
                .iter()
                .map(|place| u32::from(place.initial))
                .collect(),
            final_marking: selected_places
                .iter()
                .map(|place| u32::from(place.final_place))
                .collect(),
        },
        transition_mapping,
    ))
}

fn label_alignment_move(
    movement: &AlignmentMove,
    source_events: &[OcelEventId],
    transitions: &[OcelTransitionId],
    log_move_events: &mut BTreeSet<OcelEventId>,
) -> Result<OcelAlignmentMove, OcelReplayError> {
    let transition = |id: TransitionId| {
        transitions
            .get(id.0 as usize)
            .copied()
            .ok_or(OcelReplayError::InvalidTransitionId)
    };
    match movement {
        AlignmentMove::Synchronous {
            event_index,
            activity,
            transition: id,
            ..
        } => Ok(OcelAlignmentMove::Synchronous {
            event: source_events
                .get(*event_index)
                .copied()
                .ok_or(OcelReplayError::InvalidTransitionId)?,
            event_type: EventTypeId(activity.0),
            transition: transition(*id)?,
        }),
        AlignmentMove::InvisibleModel { transition: id } => Ok(OcelAlignmentMove::InvisibleModel {
            transition: transition(*id)?,
        }),
        AlignmentMove::LogOnly {
            event_index,
            activity,
            ..
        } => {
            let event = source_events
                .get(*event_index)
                .copied()
                .ok_or(OcelReplayError::InvalidTransitionId)?;
            log_move_events.insert(event);
            Ok(OcelAlignmentMove::LogOnly {
                event,
                event_type: EventTypeId(activity.0),
            })
        }
        AlignmentMove::VisibleModel {
            transition: id,
            activity,
        } => Ok(OcelAlignmentMove::VisibleModel {
            transition: transition(*id)?,
            event_type: EventTypeId(activity.0),
        }),
    }
}

fn alignment_fitness(cost: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        return u64::from(cost == 0) * FITNESS_SCALE;
    }
    FITNESS_SCALE.saturating_sub(
        cost.saturating_mul(FITNESS_SCALE)
            .checked_div(denominator)
            .unwrap_or(FITNESS_SCALE),
    )
}

fn label_deviation(
    object: ObjectId,
    deviation: &ReplayDeviation,
    source_events: &[OcelEventId],
) -> Option<OcelReplayDeviation> {
    match deviation {
        ReplayDeviation::ActivityNotInModel {
            event_index,
            activity,
            ..
        } => source_events
            .get(*event_index)
            .map(|event| OcelReplayDeviation::ActivityNotInModel {
                event: *event,
                event_type: EventTypeId(activity.0),
                object,
            }),
        ReplayDeviation::MissingTokens {
            event_index,
            activity,
            count,
            ..
        } => source_events
            .get(*event_index)
            .map(|event| OcelReplayDeviation::MissingTokens {
                event: *event,
                event_type: EventTypeId(activity.0),
                object,
                count: *count,
            }),
        ReplayDeviation::FinalTokensMissing { count } => {
            Some(OcelReplayDeviation::FinalTokensMissing {
                object,
                count: *count,
            })
        }
        ReplayDeviation::TokensRemaining { count } => Some(OcelReplayDeviation::TokensRemaining {
            object,
            count: *count,
        }),
    }
}

fn deviation_event(deviation: &OcelReplayDeviation) -> Option<OcelEventId> {
    match deviation {
        OcelReplayDeviation::ActivityNotInModel { event, .. }
        | OcelReplayDeviation::MissingTokens { event, .. }
        | OcelReplayDeviation::MissingObjectParticipation { event, .. }
        | OcelReplayDeviation::FixedArcMultiplicity { event, .. }
        | OcelReplayDeviation::UnexpectedObjectParticipation { event, .. } => Some(*event),
        OcelReplayDeviation::FinalTokensMissing { .. }
        | OcelReplayDeviation::TokensRemaining { .. } => None,
    }
}

fn token_fitness(consumed: u64, produced: u64, missing: u64, remaining: u64, unmapped: u64) -> u64 {
    let missing = missing.saturating_add(unmapped);
    let consumed = consumed.saturating_add(unmapped);
    let missing_penalty = missing
        .saturating_mul(FITNESS_SCALE)
        .checked_div(consumed)
        .unwrap_or(0);
    let remaining_penalty = remaining
        .saturating_mul(FITNESS_SCALE)
        .checked_div(produced)
        .unwrap_or(0);
    (FITNESS_SCALE.saturating_sub(missing_penalty)
        + FITNESS_SCALE.saturating_sub(remaining_penalty))
        / 2
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OcelBuilder, OcelDiscoveryConfig, OcelFilter, discover_ocel_petri_net};

    fn log() -> crate::OcelLog {
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

    fn baseline(log: &crate::OcelLog) -> OcelPetriNet {
        let view = OcelView::apply(log, &OcelFilter::default()).unwrap();
        discover_ocel_petri_net(log, &view, OcelDiscoveryConfig::default()).unwrap()
    }

    #[test]
    fn discovered_model_replays_its_source_objects_and_bindings_perfectly() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let replay = replay_ocel_tokens(&log, &view, &baseline(&log)).unwrap();
        assert_eq!(replay.replayed_objects, 4);
        assert_eq!(replay.lifecycle_fit_objects, 4);
        assert_eq!(replay.replayed_events, 3);
        assert_eq!(replay.fit_events, 3);
        assert_eq!(replay.fitness_millionths, FITNESS_SCALE);
        assert_eq!(replay.binding_violations, 0);
        assert!(replay.deviations.is_empty());
    }

    #[test]
    fn discovered_model_aligns_its_source_objects_at_zero_cost() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let alignment = align_ocel(&log, &view, &baseline(&log)).unwrap();
        assert_eq!(alignment.aligned_objects, 4);
        assert_eq!(alignment.lifecycle_fit_objects, 4);
        assert_eq!(alignment.aligned_events, 3);
        assert_eq!(alignment.fit_events, 3);
        assert_eq!(alignment.total_cost, 0);
        assert_eq!(alignment.fitness_millionths, FITNESS_SCALE);
        assert_eq!(alignment.binding_violations, 0);
        assert!(alignment.binding_deviations.is_empty());
        assert!(alignment.objects.iter().all(|object| object.fit));
    }

    #[test]
    fn alignment_explains_a_missing_lifecycle_event_as_a_model_move() {
        let baseline_log = log();
        let model = baseline(&baseline_log);
        let mut observed = log();
        observed.event_object_relations.retain(|relation| {
            !(relation.event == OcelEventId(1) && relation.object == ObjectId(3))
        });
        let view = OcelView::apply(&observed, &OcelFilter::default()).unwrap();
        let alignment = align_ocel(&observed, &view, &model).unwrap();
        let package = alignment
            .objects
            .iter()
            .find(|object| object.object == ObjectId(3))
            .unwrap();
        assert_eq!(package.cost, 1);
        assert_eq!(package.visible_model_moves, 1);
        assert!(package.moves.iter().any(|movement| matches!(
            movement,
            OcelAlignmentMove::VisibleModel {
                event_type: EventTypeId(1),
                ..
            }
        )));
        assert_eq!(alignment.binding_violations, 1);
    }

    #[test]
    fn object_type_filter_replays_only_that_typed_slice() {
        let log = log();
        let model = baseline(&log);
        let view = OcelView::apply(
            &log,
            &OcelFilter {
                object_types: vec!["package".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let replay = replay_ocel_tokens(&log, &view, &model).unwrap();
        assert_eq!(replay.replayed_objects, 1);
        assert_eq!(replay.lifecycle_fit_objects, 1);
        assert_eq!(replay.replayed_events, 2);
        assert_eq!(replay.fit_events, 2);
        assert_eq!(replay.binding_violations, 0);
        assert!(replay.deviations.is_empty());
    }

    #[test]
    fn fixed_arc_multiplicity_is_a_first_class_event_deviation() {
        let baseline_log = log();
        let model = baseline(&baseline_log);
        let mut observed = log();
        let extra = ObjectId(observed.object_ids.intern("o2"));
        observed.objects.push(crate::Object {
            object_type: ObjectTypeId(0),
        });
        observed
            .event_object_relations
            .push(crate::EventObjectRelation {
                event: OcelEventId(0),
                object: extra,
                qualifier: observed.event_object_relations[0].qualifier,
            });
        let view = OcelView::apply(&observed, &OcelFilter::default()).unwrap();
        let replay = replay_ocel_tokens(&observed, &view, &model).unwrap();
        assert_eq!(replay.binding_violations, 1);
        assert_eq!(replay.fit_events, 2);
        assert!(matches!(
            replay.deviations[0],
            OcelReplayDeviation::FixedArcMultiplicity {
                event: OcelEventId(0),
                object_type: ObjectTypeId(0),
                observed_objects: 2,
                ..
            }
        ));
    }

    #[test]
    fn missing_shared_object_participation_is_not_hidden_by_lifecycle_replay() {
        let baseline_log = log();
        let model = baseline(&baseline_log);
        let mut observed = log();
        observed.event_object_relations.retain(|relation| {
            !(relation.event == OcelEventId(1) && relation.object == ObjectId(3))
        });
        let view = OcelView::apply(&observed, &OcelFilter::default()).unwrap();
        let replay = replay_ocel_tokens(&observed, &view, &model).unwrap();
        assert!(replay.binding_violations > 0);
        assert!(replay.deviations.iter().any(|deviation| matches!(
            deviation,
            OcelReplayDeviation::MissingObjectParticipation {
                event: OcelEventId(1),
                object_type: ObjectTypeId(2),
                ..
            }
        )));
    }

    #[test]
    fn modelled_activity_without_any_objects_remains_in_scope_as_a_deviation() {
        let baseline_log = log();
        let model = baseline(&baseline_log);
        let mut observed = log();
        observed
            .event_object_relations
            .retain(|relation| relation.event != OcelEventId(1));
        let other_type = ObjectTypeId(observed.object_types.len() as u32);
        observed.object_types.push(crate::ObjectType {
            name: "other".to_owned(),
            attributes: Vec::new(),
        });
        let other = ObjectId(observed.object_ids.intern("x1"));
        observed.objects.push(crate::Object {
            object_type: other_type,
        });
        observed
            .event_object_relations
            .push(crate::EventObjectRelation {
                event: OcelEventId(1),
                object: other,
                qualifier: observed.event_object_relations[0].qualifier,
            });
        let view = OcelView::apply(&observed, &OcelFilter::default()).unwrap();
        let replay = replay_ocel_tokens(&observed, &view, &model).unwrap();
        assert_eq!(replay.replayed_events, 3);
        assert_eq!(replay.fit_events, 1);
        assert_eq!(
            replay
                .deviations
                .iter()
                .filter(|deviation| matches!(
                    deviation,
                    OcelReplayDeviation::MissingObjectParticipation {
                        event: OcelEventId(1),
                        ..
                    }
                ))
                .count(),
            2,
            "{:?}",
            replay.deviations
        );
    }

    #[test]
    fn simultaneous_events_on_one_object_are_not_ordered_for_replay() {
        let baseline_log = log();
        let model = baseline(&baseline_log);
        let mut observed = log();
        observed.events[1].at = observed.events[0].at;
        let view = OcelView::apply(&observed, &OcelFilter::default()).unwrap();
        assert!(matches!(
            replay_ocel_tokens(&observed, &view, &model),
            Err(OcelReplayError::SimultaneousEvents {
                object: ObjectId(1),
                at: 1
            })
        ));
    }

    #[test]
    fn malformed_arc_multiplicity_is_rejected_before_replay() {
        let log = log();
        let mut model = baseline(&log);
        let transition = model
            .transitions
            .iter_mut()
            .find(|transition| transition.event_type == Some(EventTypeId(1)))
            .unwrap();
        let variable_input = transition
            .inputs
            .iter()
            .find(|arc| arc.variable)
            .unwrap()
            .place;
        let object_type = model.places[variable_input.0 as usize].object_type;
        transition
            .outputs
            .iter_mut()
            .find(|arc| model.places[arc.place.0 as usize].object_type == object_type)
            .unwrap()
            .variable = false;
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        assert!(matches!(
            replay_ocel_tokens(&log, &view, &model),
            Err(OcelReplayError::InconsistentArcMultiplicity)
        ));
    }
}
