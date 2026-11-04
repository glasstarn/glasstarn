//! Deterministic token-based conformance replay.
//!
//! Each trace is replayed against an accepting Petri net. Invisible routing is
//! explored breadth-first with transition identifiers as the stable tie-break.
//! When reality cannot follow the model, missing tokens are inserted explicitly
//! and the deviation retains its source-row anchor.

use crate::{ActivityId, CaseId, EventLog, ModelError, PetriNet, PlaceId, TransitionId};
use std::collections::{BTreeSet, VecDeque};

const MAX_SILENT_STATES: usize = 100_000;
const FITNESS_SCALE: u64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplayDeviation {
    ActivityNotInModel {
        event_index: usize,
        source_row: u32,
        activity: ActivityId,
    },
    MissingTokens {
        event_index: usize,
        source_row: u32,
        activity: ActivityId,
        count: u64,
    },
    FinalTokensMissing {
        count: u64,
    },
    TokensRemaining {
        count: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CaseReplay {
    pub case: CaseId,
    pub fitness_millionths: u64,
    pub consumed_tokens: u64,
    pub produced_tokens: u64,
    pub missing_tokens: u64,
    pub remaining_tokens: u64,
    pub unmapped_events: u64,
    pub silent_firings: u64,
    pub fit: bool,
    pub deviations: Vec<ReplayDeviation>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TokenReplay {
    pub cases: Vec<CaseReplay>,
    pub fitness_millionths: u64,
    pub fit_cases: usize,
    pub consumed_tokens: u64,
    pub produced_tokens: u64,
    pub missing_tokens: u64,
    pub remaining_tokens: u64,
    pub unmapped_events: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error(transparent)]
    InvalidModel(#[from] ModelError),
    #[error("silent-transition search exceeded {MAX_SILENT_STATES} markings")]
    SilentSearchLimit,
}

/// Replays every case in log order against one accepting Petri net.
pub fn replay_tokens(log: &EventLog, net: &PetriNet) -> Result<TokenReplay, ReplayError> {
    net.validate()?;
    let mut cases = Vec::with_capacity(log.case_count());
    for trace in &log.traces {
        let mut marking = net.initial_marking.clone();
        let mut consumed = 0u64;
        let mut produced = token_sum(&net.initial_marking);
        let mut missing = 0u64;
        let mut unmapped = 0u64;
        let mut silent_firings = 0u64;
        let mut deviations = Vec::new();

        for (event_index, event) in trace.events.iter().enumerate() {
            let Some(candidate) = best_activity_candidate(net, &marking, event.activity)? else {
                unmapped += 1;
                deviations.push(ReplayDeviation::ActivityNotInModel {
                    event_index,
                    source_row: event.source_row,
                    activity: event.activity,
                });
                continue;
            };
            fire_path(
                net,
                &mut marking,
                &candidate.path,
                &mut consumed,
                &mut produced,
            )?;
            silent_firings += candidate.path.len() as u64;
            let transition = net.transition(candidate.transition).unwrap();
            let inserted = insert_missing(&mut marking, &transition.inputs);
            if inserted > 0 {
                missing += inserted;
                deviations.push(ReplayDeviation::MissingTokens {
                    event_index,
                    source_row: event.source_row,
                    activity: event.activity,
                    count: inserted,
                });
            }
            marking = net.fire(&marking, candidate.transition)?;
            consumed += transition.inputs.len() as u64;
            produced += transition.outputs.len() as u64;
        }

        let completion = best_completion(net, &marking)?;
        fire_path(
            net,
            &mut marking,
            &completion.path,
            &mut consumed,
            &mut produced,
        )?;
        silent_firings += completion.path.len() as u64;

        let final_missing = marking_deficit(&marking, &net.final_marking);
        let remaining = marking_excess(&marking, &net.final_marking);
        missing += final_missing;
        consumed += token_sum(&net.final_marking);
        if final_missing > 0 {
            deviations.push(ReplayDeviation::FinalTokensMissing {
                count: final_missing,
            });
        }
        if remaining > 0 {
            deviations.push(ReplayDeviation::TokensRemaining { count: remaining });
        }
        let fitness = token_fitness(consumed, produced, missing, remaining, unmapped);
        cases.push(CaseReplay {
            case: trace.case,
            fitness_millionths: fitness,
            consumed_tokens: consumed,
            produced_tokens: produced,
            missing_tokens: missing,
            remaining_tokens: remaining,
            unmapped_events: unmapped,
            silent_firings,
            fit: missing == 0 && remaining == 0 && unmapped == 0,
            deviations,
        });
    }

    let consumed_tokens = cases.iter().map(|case| case.consumed_tokens).sum();
    let produced_tokens = cases.iter().map(|case| case.produced_tokens).sum();
    let missing_tokens = cases.iter().map(|case| case.missing_tokens).sum();
    let remaining_tokens = cases.iter().map(|case| case.remaining_tokens).sum();
    let unmapped_events = cases.iter().map(|case| case.unmapped_events).sum();
    Ok(TokenReplay {
        fitness_millionths: token_fitness(
            consumed_tokens,
            produced_tokens,
            missing_tokens,
            remaining_tokens,
            unmapped_events,
        ),
        fit_cases: cases.iter().filter(|case| case.fit).count(),
        cases,
        consumed_tokens,
        produced_tokens,
        missing_tokens,
        remaining_tokens,
        unmapped_events,
    })
}

#[derive(Clone)]
struct SilentState {
    marking: Vec<u32>,
    path: Vec<TransitionId>,
}

struct ActivityCandidate {
    transition: TransitionId,
    path: Vec<TransitionId>,
}

fn best_activity_candidate(
    net: &PetriNet,
    marking: &[u32],
    activity: ActivityId,
) -> Result<Option<ActivityCandidate>, ReplayError> {
    let transitions: Vec<_> = net
        .transitions
        .iter()
        .filter(|transition| transition.activity == Some(activity))
        .collect();
    if transitions.is_empty() {
        return Ok(None);
    }
    let states = silent_closure(net, marking)?;
    let mut choices = Vec::new();
    for state in states {
        for transition in &transitions {
            choices.push((
                marking_deficit_for_places(&state.marking, &transition.inputs),
                state.path.len(),
                transition.id,
                state.marking.clone(),
                state.path.clone(),
            ));
        }
    }
    choices.sort_by(|left, right| {
        (&left.0, &left.1, &left.2, &left.3).cmp(&(&right.0, &right.1, &right.2, &right.3))
    });
    let (_, _, transition, _, path) = choices.into_iter().next().unwrap();
    Ok(Some(ActivityCandidate { transition, path }))
}

fn best_completion(net: &PetriNet, marking: &[u32]) -> Result<SilentState, ReplayError> {
    let mut states = silent_closure(net, marking)?;
    states.sort_by(|left, right| {
        let left_distance = marking_deficit(&left.marking, &net.final_marking)
            + marking_excess(&left.marking, &net.final_marking);
        let right_distance = marking_deficit(&right.marking, &net.final_marking)
            + marking_excess(&right.marking, &net.final_marking);
        (left_distance, left.path.len(), &left.marking).cmp(&(
            right_distance,
            right.path.len(),
            &right.marking,
        ))
    });
    Ok(states.remove(0))
}

fn silent_closure(net: &PetriNet, marking: &[u32]) -> Result<Vec<SilentState>, ReplayError> {
    let initial = SilentState {
        marking: marking.to_vec(),
        path: Vec::new(),
    };
    let mut found = vec![initial.clone()];
    let mut seen = BTreeSet::from([marking.to_vec()]);
    let mut queue = VecDeque::from([initial]);
    while let Some(state) = queue.pop_front() {
        for transition in net
            .transitions
            .iter()
            .filter(|transition| transition.activity.is_none())
        {
            if !net.is_enabled(&state.marking, transition.id) {
                continue;
            }
            let next = net.fire(&state.marking, transition.id)?;
            if seen.insert(next.clone()) {
                if seen.len() > MAX_SILENT_STATES {
                    return Err(ReplayError::SilentSearchLimit);
                }
                let mut path = state.path.clone();
                path.push(transition.id);
                let next_state = SilentState {
                    marking: next,
                    path,
                };
                found.push(next_state.clone());
                queue.push_back(next_state);
            }
        }
    }
    Ok(found)
}

fn fire_path(
    net: &PetriNet,
    marking: &mut Vec<u32>,
    path: &[TransitionId],
    consumed: &mut u64,
    produced: &mut u64,
) -> Result<(), ModelError> {
    for id in path {
        let transition = net.transition(*id).unwrap();
        *marking = net.fire(marking, *id)?;
        *consumed += transition.inputs.len() as u64;
        *produced += transition.outputs.len() as u64;
    }
    Ok(())
}

fn insert_missing(marking: &mut [u32], inputs: &[PlaceId]) -> u64 {
    let mut inserted = 0;
    for place in inputs {
        if marking[place.0 as usize] == 0 {
            marking[place.0 as usize] = 1;
            inserted += 1;
        }
    }
    inserted
}

fn marking_deficit_for_places(marking: &[u32], places: &[PlaceId]) -> u64 {
    places
        .iter()
        .filter(|place| marking[place.0 as usize] == 0)
        .count() as u64
}

fn marking_deficit(actual: &[u32], expected: &[u32]) -> u64 {
    actual
        .iter()
        .zip(expected)
        .map(|(actual, expected)| expected.saturating_sub(*actual) as u64)
        .sum()
}

fn marking_excess(actual: &[u32], expected: &[u32]) -> u64 {
    actual
        .iter()
        .zip(expected)
        .map(|(actual, expected)| actual.saturating_sub(*expected) as u64)
        .sum()
}

fn token_sum(marking: &[u32]) -> u64 {
    marking.iter().map(|tokens| u64::from(*tokens)).sum()
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
    let missing_part = FITNESS_SCALE.saturating_sub(missing_penalty);
    let remaining_part = FITNESS_SCALE.saturating_sub(remaining_penalty);
    (missing_part + remaining_part) / 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaseTrace, Dictionary, Event, ProcessTree};

    fn log(spec: &[&[&str]]) -> EventLog {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let mut traces = Vec::new();
        let mut row = 2;
        for (index, labels) in spec.iter().enumerate() {
            let case = CaseId(cases.intern(&format!("C{}", index + 1)));
            let events = labels
                .iter()
                .enumerate()
                .map(|(position, label)| {
                    let event =
                        Event::new(ActivityId(activities.intern(label)), position as i64, row);
                    row += 1;
                    event
                })
                .collect();
            traces.push(CaseTrace {
                case,
                events,
                incomplete: false,
                attributes: Vec::new(),
            });
        }
        EventLog {
            cases,
            activities,
            step_values: Default::default(),
            traces,
            ..EventLog::default()
        }
    }

    fn activity(log: &EventLog, name: &str) -> ProcessTree {
        ProcessTree::activity(ActivityId(log.activities.get(name).unwrap()))
    }

    #[test]
    fn exact_sequence_is_perfectly_fit() {
        let log = log(&[&["A", "B", "C"], &["A", "B", "C"]]);
        let tree = ProcessTree::sequence(vec![
            activity(&log, "A"),
            activity(&log, "B"),
            activity(&log, "C"),
        ]);
        let replay = replay_tokens(&log, &tree.to_petri_net().unwrap()).unwrap();
        assert_eq!(replay.fit_cases, 2);
        assert_eq!(replay.fitness_millionths, FITNESS_SCALE);
        assert_eq!(replay.missing_tokens, 0);
        assert_eq!(replay.remaining_tokens, 0);
        assert!(replay.cases.iter().all(|case| case.deviations.is_empty()));
    }

    #[test]
    fn skipped_activity_has_missing_and_remaining_tokens() {
        let mut log = log(&[&["A", "C"]]);
        let b = ActivityId(log.activities.intern("B"));
        let tree = ProcessTree::sequence(vec![
            activity(&log, "A"),
            ProcessTree::activity(b),
            activity(&log, "C"),
        ]);
        let replay = replay_tokens(&log, &tree.to_petri_net().unwrap()).unwrap();
        assert_eq!(replay.fit_cases, 0);
        assert_eq!(replay.missing_tokens, 1);
        assert_eq!(replay.remaining_tokens, 1);
        assert!(matches!(
            replay.cases[0].deviations[0],
            ReplayDeviation::MissingTokens {
                source_row: 3,
                count: 1,
                ..
            }
        ));
    }

    #[test]
    fn activity_absent_from_model_is_explicit() {
        let log = log(&[&["A", "Noise", "B"]]);
        let tree = ProcessTree::sequence(vec![activity(&log, "A"), activity(&log, "B")]);
        let replay = replay_tokens(&log, &tree.to_petri_net().unwrap()).unwrap();
        assert_eq!(replay.unmapped_events, 1);
        assert!(!replay.cases[0].fit);
        assert!(replay.cases[0].fitness_millionths < FITNESS_SCALE);
        assert!(matches!(
            replay.cases[0].deviations[0],
            ReplayDeviation::ActivityNotInModel { source_row: 3, .. }
        ));
    }

    #[test]
    fn both_parallel_interleavings_fit() {
        let log = log(&[&["A", "B"], &["B", "A"]]);
        let tree = ProcessTree::parallel(vec![activity(&log, "A"), activity(&log, "B")]);
        let replay = replay_tokens(&log, &tree.to_petri_net().unwrap()).unwrap();
        assert_eq!(replay.fit_cases, 2);
        assert_eq!(replay.fitness_millionths, FITNESS_SCALE);
    }

    #[test]
    fn either_exclusive_branch_fits() {
        let log = log(&[&["A"], &["B"]]);
        let tree = ProcessTree::exclusive(vec![activity(&log, "A"), activity(&log, "B")]);
        let replay = replay_tokens(&log, &tree.to_petri_net().unwrap()).unwrap();
        assert_eq!(replay.fit_cases, 2);
        assert_eq!(replay.fitness_millionths, FITNESS_SCALE);
    }

    #[test]
    fn repeated_loop_body_fits() {
        let log = log(&[&["Review"], &["Review", "Rework", "Review"]]);
        let tree = ProcessTree::loop_of(activity(&log, "Review"), activity(&log, "Rework"));
        let replay = replay_tokens(&log, &tree.to_petri_net().unwrap()).unwrap();
        assert_eq!(replay.fit_cases, 2);
        assert_eq!(replay.fitness_millionths, FITNESS_SCALE);
        assert!(replay.cases.iter().all(|case| case.silent_firings > 0));
    }

    #[test]
    fn replay_is_deterministic() {
        let log = log(&[&["A"], &["B"]]);
        let net = ProcessTree::exclusive(vec![activity(&log, "A"), activity(&log, "B")])
            .to_petri_net()
            .unwrap();
        assert_eq!(
            replay_tokens(&log, &net).unwrap(),
            replay_tokens(&log, &net).unwrap()
        );
    }
}
