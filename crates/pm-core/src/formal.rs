//! Formal process models shared by discovery and conformance.
//!
//! Discovery produces a block-structured [`ProcessTree`]. Conformance executes
//! an accepting [`PetriNet`]. Keeping the conversion here gives every front end
//! one deterministic model semantics rather than separate CLI, browser, and
//! replay interpretations.

use crate::ActivityId;
use std::collections::BTreeSet;

/// A block-structured process tree.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessTree {
    /// One observable activity.
    Activity { activity: ActivityId },
    /// An unobservable step, used for optional paths.
    Tau,
    /// Children execute from left to right.
    Sequence { children: Vec<ProcessTree> },
    /// Exactly one child executes.
    Exclusive { children: Vec<ProcessTree> },
    /// Every child executes, in any valid interleaving.
    Parallel { children: Vec<ProcessTree> },
    /// The body executes once; each redo is followed by another body execution.
    Loop {
        body: Box<ProcessTree>,
        redo: Box<ProcessTree>,
    },
}

impl ProcessTree {
    pub fn activity(activity: ActivityId) -> Self {
        Self::Activity { activity }
    }

    pub fn sequence(children: Vec<Self>) -> Self {
        Self::Sequence { children }
    }

    pub fn exclusive(children: Vec<Self>) -> Self {
        Self::Exclusive { children }
    }

    pub fn parallel(children: Vec<Self>) -> Self {
        Self::Parallel { children }
    }

    pub fn loop_of(body: Self, redo: Self) -> Self {
        Self::Loop {
            body: Box::new(body),
            redo: Box::new(redo),
        }
    }

    /// Rejects malformed operator nodes before conversion or execution.
    pub fn validate(&self) -> Result<(), ModelError> {
        match self {
            Self::Activity { .. } | Self::Tau => Ok(()),
            Self::Sequence { children }
            | Self::Exclusive { children }
            | Self::Parallel { children } => {
                if children.len() < 2 {
                    return Err(ModelError::OperatorArity);
                }
                for child in children {
                    child.validate()?;
                }
                Ok(())
            }
            Self::Loop { body, redo } => {
                body.validate()?;
                redo.validate()
            }
        }
    }

    /// Distinct observable activities in identifier order.
    pub fn activities(&self) -> Vec<ActivityId> {
        let mut activities = BTreeSet::new();
        self.collect_activities(&mut activities);
        activities.into_iter().collect()
    }

    fn collect_activities(&self, activities: &mut BTreeSet<ActivityId>) {
        match self {
            Self::Activity { activity } => {
                activities.insert(*activity);
            }
            Self::Tau => {}
            Self::Sequence { children }
            | Self::Exclusive { children }
            | Self::Parallel { children } => {
                for child in children {
                    child.collect_activities(activities);
                }
            }
            Self::Loop { body, redo } => {
                body.collect_activities(activities);
                redo.collect_activities(activities);
            }
        }
    }

    /// Converts this tree into a sound accepting workflow net.
    pub fn to_petri_net(&self) -> Result<PetriNet, ModelError> {
        self.validate()?;
        let mut builder = NetBuilder::default();
        let source = builder.place();
        let sink = builder.place();
        builder.compile(self, source, sink);
        let mut initial_marking = vec![0; builder.place_count];
        initial_marking[source.0 as usize] = 1;
        let mut final_marking = vec![0; builder.place_count];
        final_marking[sink.0 as usize] = 1;
        let net = PetriNet {
            place_count: builder.place_count,
            transitions: builder.transitions,
            initial_marking,
            final_marking,
        };
        net.validate()?;
        Ok(net)
    }
}

/// A place identifier local to one Petri net.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct PlaceId(pub u32);

/// A transition identifier local to one Petri net.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct TransitionId(pub u32);

/// One Petri-net transition. `None` is an invisible routing transition.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PetriTransition {
    pub id: TransitionId,
    pub activity: Option<ActivityId>,
    pub inputs: Vec<PlaceId>,
    pub outputs: Vec<PlaceId>,
}

/// An accepting place/transition Petri net with explicit initial and final markings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PetriNet {
    pub place_count: usize,
    pub transitions: Vec<PetriTransition>,
    /// Token counts indexed by [`PlaceId`].
    pub initial_marking: Vec<u32>,
    /// The only marking considered complete.
    pub final_marking: Vec<u32>,
}

impl PetriNet {
    /// Checks identifiers, markings, and the bipartite transition boundary.
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.place_count == 0
            || self.initial_marking.len() != self.place_count
            || self.final_marking.len() != self.place_count
        {
            return Err(ModelError::InvalidMarking);
        }
        for (index, transition) in self.transitions.iter().enumerate() {
            if transition.id.0 as usize != index {
                return Err(ModelError::InvalidTransitionId);
            }
            if transition.inputs.is_empty() || transition.outputs.is_empty() {
                return Err(ModelError::DisconnectedTransition);
            }
            let distinct_inputs: BTreeSet<_> = transition.inputs.iter().collect();
            let distinct_outputs: BTreeSet<_> = transition.outputs.iter().collect();
            if distinct_inputs.len() != transition.inputs.len()
                || distinct_outputs.len() != transition.outputs.len()
            {
                return Err(ModelError::WeightedArcUnsupported);
            }
            if transition
                .inputs
                .iter()
                .chain(&transition.outputs)
                .any(|place| place.0 as usize >= self.place_count)
            {
                return Err(ModelError::UnknownPlace);
            }
        }
        Ok(())
    }

    pub fn transition(&self, id: TransitionId) -> Option<&PetriTransition> {
        self.transitions.get(id.0 as usize).filter(|t| t.id == id)
    }

    /// Whether every input place contains the token this transition consumes.
    pub fn is_enabled(&self, marking: &[u32], id: TransitionId) -> bool {
        marking.len() == self.place_count
            && self.transition(id).is_some_and(|transition| {
                transition.inputs.iter().all(|p| marking[p.0 as usize] > 0)
            })
    }

    /// Fires one enabled transition and returns the next marking.
    pub fn fire(&self, marking: &[u32], id: TransitionId) -> Result<Vec<u32>, ModelError> {
        if marking.len() != self.place_count {
            return Err(ModelError::InvalidMarking);
        }
        let transition = self.transition(id).ok_or(ModelError::UnknownTransition)?;
        if !self.is_enabled(marking, id) {
            return Err(ModelError::TransitionNotEnabled);
        }
        let mut next = marking.to_vec();
        for place in &transition.inputs {
            next[place.0 as usize] -= 1;
        }
        for place in &transition.outputs {
            next[place.0 as usize] += 1;
        }
        Ok(next)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("sequence, exclusive-choice, and parallel operators need at least two children")]
    OperatorArity,
    #[error("the initial or final marking does not match the net's places")]
    InvalidMarking,
    #[error("transition identifiers must be contiguous and in storage order")]
    InvalidTransitionId,
    #[error("every transition needs at least one input and one output place")]
    DisconnectedTransition,
    #[error("weighted arcs are not supported; a place may appear only once on each side")]
    WeightedArcUnsupported,
    #[error("a transition refers to a place outside this net")]
    UnknownPlace,
    #[error("the transition does not exist in this net")]
    UnknownTransition,
    #[error("the transition is not enabled in this marking")]
    TransitionNotEnabled,
}

#[derive(Default)]
struct NetBuilder {
    place_count: usize,
    transitions: Vec<PetriTransition>,
}

impl NetBuilder {
    fn place(&mut self) -> PlaceId {
        let place =
            PlaceId(u32::try_from(self.place_count).expect("Petri net exceeded u32::MAX places"));
        self.place_count += 1;
        place
    }

    fn transition(
        &mut self,
        activity: Option<ActivityId>,
        inputs: Vec<PlaceId>,
        outputs: Vec<PlaceId>,
    ) {
        let id = TransitionId(
            u32::try_from(self.transitions.len()).expect("Petri net exceeded u32::MAX transitions"),
        );
        self.transitions.push(PetriTransition {
            id,
            activity,
            inputs,
            outputs,
        });
    }

    fn compile(&mut self, tree: &ProcessTree, source: PlaceId, sink: PlaceId) {
        match tree {
            ProcessTree::Activity { activity } => {
                self.transition(Some(*activity), vec![source], vec![sink]);
            }
            ProcessTree::Tau => self.transition(None, vec![source], vec![sink]),
            ProcessTree::Sequence { children } => {
                let mut input = source;
                for (index, child) in children.iter().enumerate() {
                    let output = if index + 1 == children.len() {
                        sink
                    } else {
                        self.place()
                    };
                    self.compile(child, input, output);
                    input = output;
                }
            }
            ProcessTree::Exclusive { children } => {
                for child in children {
                    self.compile(child, source, sink);
                }
            }
            ProcessTree::Parallel { children } => {
                let inputs: Vec<_> = children.iter().map(|_| self.place()).collect();
                let outputs: Vec<_> = children.iter().map(|_| self.place()).collect();
                self.transition(None, vec![source], inputs.clone());
                for ((child, input), output) in children.iter().zip(&inputs).zip(&outputs) {
                    self.compile(child, *input, *output);
                }
                self.transition(None, outputs, vec![sink]);
            }
            ProcessTree::Loop { body, redo } => {
                let completed = self.place();
                self.compile(body, source, completed);
                self.transition(None, vec![completed], vec![sink]);
                self.compile(redo, completed, source);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity(id: u32) -> ProcessTree {
        ProcessTree::activity(ActivityId(id))
    }

    #[test]
    fn rejects_degenerate_operators() {
        assert_eq!(
            ProcessTree::sequence(vec![activity(1)]).validate(),
            Err(ModelError::OperatorArity)
        );
    }

    #[test]
    fn activity_inventory_is_sorted_and_unique() {
        let tree = ProcessTree::exclusive(vec![
            activity(3),
            ProcessTree::sequence(vec![activity(1), activity(3)]),
        ]);
        assert_eq!(tree.activities(), vec![ActivityId(1), ActivityId(3)]);
    }

    #[test]
    fn sequence_becomes_a_single_token_path() {
        let net = ProcessTree::sequence(vec![activity(0), activity(1), activity(2)])
            .to_petri_net()
            .unwrap();
        assert_eq!(net.place_count, 4);
        assert_eq!(net.transitions.len(), 3);
        assert_eq!(net.initial_marking, vec![1, 0, 0, 0]);
        assert_eq!(net.final_marking, vec![0, 1, 0, 0]);
        assert_eq!(net.transitions[0].activity, Some(ActivityId(0)));
        assert_eq!(net.transitions[1].activity, Some(ActivityId(1)));
        assert_eq!(net.transitions[2].activity, Some(ActivityId(2)));

        let mut marking = net.initial_marking.clone();
        for transition in &net.transitions {
            marking = net.fire(&marking, transition.id).unwrap();
        }
        assert_eq!(marking, net.final_marking);
    }

    #[test]
    fn exclusive_branches_share_entry_and_exit_places() {
        let net = ProcessTree::exclusive(vec![activity(0), activity(1)])
            .to_petri_net()
            .unwrap();
        assert_eq!(net.place_count, 2);
        assert_eq!(net.transitions[0].inputs, net.transitions[1].inputs);
        assert_eq!(net.transitions[0].outputs, net.transitions[1].outputs);
    }

    #[test]
    fn parallelism_has_explicit_split_and_join_transitions() {
        let net = ProcessTree::parallel(vec![activity(0), activity(1)])
            .to_petri_net()
            .unwrap();
        assert_eq!(net.place_count, 6);
        assert_eq!(net.transitions.len(), 4);
        assert_eq!(net.transitions[0].activity, None);
        assert_eq!(net.transitions[0].outputs.len(), 2);
        assert_eq!(net.transitions[3].activity, None);
        assert_eq!(net.transitions[3].inputs.len(), 2);

        let after_split = net.fire(&net.initial_marking, TransitionId(0)).unwrap();
        let after_right = net.fire(&after_split, TransitionId(2)).unwrap();
        let after_left = net.fire(&after_right, TransitionId(1)).unwrap();
        assert_eq!(
            net.fire(&after_left, TransitionId(3)).unwrap(),
            net.final_marking
        );
    }

    #[test]
    fn loop_routes_redo_back_before_the_body() {
        let net = ProcessTree::loop_of(activity(0), activity(1))
            .to_petri_net()
            .unwrap();
        let body = &net.transitions[0];
        let exit = &net.transitions[1];
        let redo = &net.transitions[2];
        assert_eq!(body.outputs, exit.inputs);
        assert_eq!(redo.inputs, body.outputs);
        assert_eq!(redo.outputs, body.inputs);
        assert_eq!(exit.activity, None);

        let once = net.fire(&net.initial_marking, body.id).unwrap();
        let redone = net.fire(&once, redo.id).unwrap();
        let twice = net.fire(&redone, body.id).unwrap();
        assert_eq!(net.fire(&twice, exit.id).unwrap(), net.final_marking);
    }

    #[test]
    fn model_conversion_is_byte_deterministic() {
        let tree = ProcessTree::parallel(vec![
            ProcessTree::sequence(vec![activity(0), activity(1)]),
            ProcessTree::exclusive(vec![activity(2), ProcessTree::Tau]),
        ]);
        assert_eq!(tree.to_petri_net(), tree.to_petri_net());
    }
}
