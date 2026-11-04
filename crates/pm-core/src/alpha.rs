//! Classic Alpha Miner for teaching and compatibility.
//!
//! This intentionally implements the original footprint-based algorithm, not
//! a repaired variant. It therefore inherits the classic miner's inability to
//! distinguish concurrency from length-two loops and its poor treatment of
//! length-one loops. The product never selects it by default.

use crate::{ActivityId, EventLog, ModelError, PetriNet, PetriTransition, PlaceId, TransitionId};
use std::collections::BTreeSet;

const MAX_CANDIDATES: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AlphaModel {
    pub net: PetriNet,
    pub places_discovered: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum AlphaError {
    #[error("Alpha Miner needs at least one event")]
    EmptyLog,
    #[error("Alpha Miner candidate-place search exceeded {MAX_CANDIDATES} candidates")]
    CandidateLimit,
    #[error(transparent)]
    InvalidModel(#[from] ModelError),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Candidate {
    left: BTreeSet<ActivityId>,
    right: BTreeSet<ActivityId>,
}

/// Discovers a Petri net using the deterministic classic Alpha algorithm.
pub fn discover_alpha(log: &EventLog) -> Result<AlphaModel, AlphaError> {
    let activities: BTreeSet<_> = log
        .traces
        .iter()
        .flat_map(|trace| trace.events.iter().map(|event| event.activity))
        .collect();
    if activities.is_empty() {
        return Err(AlphaError::EmptyLog);
    }
    let direct: BTreeSet<_> = log
        .traces
        .iter()
        .flat_map(|trace| {
            trace
                .events
                .windows(2)
                .map(|pair| (pair[0].activity, pair[1].activity))
        })
        .collect();
    let causal = |a, b| direct.contains(&(a, b)) && !direct.contains(&(b, a));
    let unrelated = |a, b| !direct.contains(&(a, b)) && !direct.contains(&(b, a));

    let mut candidates = BTreeSet::new();
    for &left in &activities {
        for &right in &activities {
            if causal(left, right) {
                candidates.insert(Candidate {
                    left: BTreeSet::from([left]),
                    right: BTreeSet::from([right]),
                });
            }
        }
    }
    loop {
        let snapshot: Vec<_> = candidates.iter().cloned().collect();
        let mut additions = BTreeSet::new();
        for first in &snapshot {
            for second in &snapshot {
                if first.left == second.left
                    && mutually_unrelated(&first.right, &second.right, &unrelated)
                {
                    additions.insert(Candidate {
                        left: first.left.clone(),
                        right: first.right.union(&second.right).copied().collect(),
                    });
                }
                if first.right == second.right
                    && mutually_unrelated(&first.left, &second.left, &unrelated)
                {
                    additions.insert(Candidate {
                        left: first.left.union(&second.left).copied().collect(),
                        right: first.right.clone(),
                    });
                }
            }
        }
        let before = candidates.len();
        candidates.extend(additions);
        if candidates.len() > MAX_CANDIDATES {
            return Err(AlphaError::CandidateLimit);
        }
        if candidates.len() == before {
            break;
        }
    }
    let maximal: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            !candidates.iter().any(|other| {
                candidate != &other
                    && candidate.left.is_subset(&other.left)
                    && candidate.right.is_subset(&other.right)
            })
        })
        .cloned()
        .collect();

    let starts: BTreeSet<_> = log
        .traces
        .iter()
        .filter_map(|trace| trace.events.first().map(|event| event.activity))
        .collect();
    let ends: BTreeSet<_> = log
        .traces
        .iter()
        .filter_map(|trace| trace.events.last().map(|event| event.activity))
        .collect();
    let source = PlaceId(0);
    let sink = PlaceId(1);
    let mut transitions = Vec::new();
    for (index, activity) in activities.iter().enumerate() {
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        if starts.contains(activity) {
            inputs.push(source);
        }
        if ends.contains(activity) {
            outputs.push(sink);
        }
        for (place_index, candidate) in maximal.iter().enumerate() {
            let place = PlaceId(u32::try_from(place_index + 2).expect("too many Alpha places"));
            if candidate.right.contains(activity) {
                inputs.push(place);
            }
            if candidate.left.contains(activity) {
                outputs.push(place);
            }
        }
        transitions.push(PetriTransition {
            id: TransitionId(u32::try_from(index).expect("too many Alpha transitions")),
            activity: Some(*activity),
            inputs,
            outputs,
        });
    }
    let place_count = maximal.len() + 2;
    let mut initial_marking = vec![0; place_count];
    initial_marking[0] = 1;
    let mut final_marking = vec![0; place_count];
    final_marking[1] = 1;
    let net = PetriNet {
        place_count,
        transitions,
        initial_marking,
        final_marking,
    };
    net.validate()?;
    Ok(AlphaModel {
        net,
        places_discovered: maximal.len(),
    })
}

fn mutually_unrelated(
    first: &BTreeSet<ActivityId>,
    second: &BTreeSet<ActivityId>,
    unrelated: &impl Fn(ActivityId, ActivityId) -> bool,
) -> bool {
    first.iter().all(|&left| {
        second
            .iter()
            .all(|&right| left == right || unrelated(left, right))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaseId, CaseTrace, Dictionary, Event, replay_tokens};

    fn log(spec: &[&[&str]]) -> EventLog {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let traces = spec
            .iter()
            .enumerate()
            .map(|(case_index, labels)| CaseTrace {
                case: CaseId(cases.intern(&format!("C{}", case_index + 1))),
                events: labels
                    .iter()
                    .enumerate()
                    .map(|(index, label)| {
                        Event::new(
                            ActivityId(activities.intern(label)),
                            index as i64,
                            (case_index * 10 + index + 2) as u32,
                        )
                    })
                    .collect(),
                incomplete: false,
                attributes: Vec::new(),
            })
            .collect();
        EventLog {
            cases,
            activities,
            step_values: Default::default(),
            traces,
            ..EventLog::default()
        }
    }

    #[test]
    fn discovers_and_replays_a_sequence() {
        let log = log(&[&["A", "B", "C"]]);
        let model = discover_alpha(&log).unwrap();
        assert_eq!(model.places_discovered, 2);
        assert_eq!(replay_tokens(&log, &model.net).unwrap().fit_cases, 1);
    }

    #[test]
    fn merges_exclusive_predecessors_into_one_place() {
        let log = log(&[&["A", "B", "D"], &["A", "C", "D"]]);
        let model = discover_alpha(&log).unwrap();
        assert_eq!(replay_tokens(&log, &model.net).unwrap().fit_cases, 2);
        let d = ActivityId(log.activities.get("D").unwrap());
        let transition = model
            .net
            .transitions
            .iter()
            .find(|transition| transition.activity == Some(d))
            .unwrap();
        assert_eq!(transition.inputs.len(), 1);
    }

    #[test]
    fn models_observed_parallel_orders() {
        let log = log(&[&["A", "B", "C", "D"], &["A", "C", "B", "D"]]);
        let model = discover_alpha(&log).unwrap();
        assert_eq!(replay_tokens(&log, &model.net).unwrap().fit_cases, 2);
    }

    #[test]
    fn rejects_an_empty_log() {
        assert!(matches!(
            discover_alpha(&EventLog::default()),
            Err(AlphaError::EmptyLog)
        ));
    }
}
