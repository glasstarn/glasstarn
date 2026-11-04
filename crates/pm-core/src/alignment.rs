//! Optimal, deterministic trace/model alignments.
//!
//! A* searches the synchronous product of a trace position and Petri-net
//! marking. Synchronous and invisible-model moves cost zero; log-only and
//! visible-model moves cost one. The admissible heuristic counts remaining
//! events whose labels do not occur anywhere in the model. Equal-cost paths
//! prefer synchronous, invisible-model, log-only, then visible-model moves,
//! with transition identifiers as the final stable tie-break.

use crate::{ActivityId, CaseId, Event, EventLog, ModelError, PetriNet, TransitionId};
use std::collections::{BTreeMap, BTreeSet};

const FITNESS_SCALE: u64 = 1_000_000;
const MAX_EXPANDED_STATES: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AlignmentMove {
    Synchronous {
        event_index: usize,
        source_row: u32,
        activity: ActivityId,
        transition: TransitionId,
    },
    InvisibleModel {
        transition: TransitionId,
    },
    LogOnly {
        event_index: usize,
        source_row: u32,
        activity: ActivityId,
    },
    VisibleModel {
        transition: TransitionId,
        activity: ActivityId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CaseAlignment {
    pub case: CaseId,
    pub cost: u64,
    /// Baseline model-completion moves plus observed trace events.
    pub denominator: u64,
    pub fitness_millionths: u64,
    pub fit: bool,
    pub moves: Vec<AlignmentMove>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LogAlignment {
    pub cases: Vec<CaseAlignment>,
    pub total_cost: u64,
    pub denominator: u64,
    pub fitness_millionths: u64,
    pub fit_cases: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum AlignmentError {
    #[error(transparent)]
    InvalidModel(#[from] ModelError),
    #[error("alignment search exceeded {MAX_EXPANDED_STATES} states for case {case:?}")]
    SearchLimit { case: CaseId },
    #[error("the model has no executable path from its initial to final marking")]
    NoModelCompletion,
    #[error("no complete alignment exists for case {case:?}")]
    NoAlignment { case: CaseId },
}

/// Aligns every case optimally against an accepting Petri net.
pub fn align_log(log: &EventLog, net: &PetriNet) -> Result<LogAlignment, AlignmentError> {
    net.validate()?;
    let baseline = search(net, &[], CaseId(0))
        .map_err(|error| match error {
            AlignmentError::NoAlignment { .. } => AlignmentError::NoModelCompletion,
            other => other,
        })?
        .0;
    let mut cases = Vec::with_capacity(log.case_count());
    let mut denominator = 0u64;
    // Cases with the same steps align the same way: search each variant once
    // and only re-point its moves at the case's own source rows.
    let mut aligned: std::collections::HashMap<Vec<ActivityId>, (u64, Vec<AlignmentMove>)> =
        std::collections::HashMap::new();
    for trace in &log.traces {
        let steps: Vec<ActivityId> = trace.events.iter().map(|event| event.activity).collect();
        let (cost, mut moves) = match aligned.get(&steps) {
            Some(found) => found.clone(),
            None => {
                let found = search(net, &trace.events, trace.case)?;
                aligned.insert(steps, found.clone());
                found
            }
        };
        for movement in &mut moves {
            if let AlignmentMove::Synchronous {
                event_index,
                source_row,
                ..
            }
            | AlignmentMove::LogOnly {
                event_index,
                source_row,
                ..
            } = movement
            {
                *source_row = trace.events[*event_index].source_row;
            }
        }
        let case_denominator = baseline.saturating_add(trace.events.len() as u64);
        denominator = denominator.saturating_add(case_denominator);
        cases.push(CaseAlignment {
            case: trace.case,
            cost,
            denominator: case_denominator,
            fitness_millionths: alignment_fitness(cost, case_denominator),
            fit: cost == 0,
            moves,
        });
    }
    let total_cost = cases.iter().map(|case| case.cost).sum();
    Ok(LogAlignment {
        fitness_millionths: alignment_fitness(total_cost, denominator),
        fit_cases: cases.iter().filter(|case| case.fit).count(),
        cases,
        total_cost,
        denominator,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StepKey {
    rank: u8,
    transition: u32,
    event_index: usize,
}

type State = (usize, Vec<u32>);
type BestPath = (u64, Vec<StepKey>, Vec<AlignmentMove>);
type QueueEntry = (u64, u64, Vec<StepKey>, usize, Vec<u32>);

fn search(
    net: &PetriNet,
    trace: &[Event],
    case: CaseId,
) -> Result<(u64, Vec<AlignmentMove>), AlignmentError> {
    let model_activities: BTreeSet<_> = net
        .transitions
        .iter()
        .filter_map(|transition| transition.activity)
        .collect();
    let mut heuristic = vec![0u64; trace.len() + 1];
    for index in (0..trace.len()).rev() {
        heuristic[index] =
            heuristic[index + 1] + u64::from(!model_activities.contains(&trace[index].activity));
    }

    let initial_state = (0, net.initial_marking.clone());
    let mut best = BTreeMap::from([(initial_state.clone(), (0, Vec::new(), Vec::new()))]);
    let mut queue = BTreeSet::from([(
        heuristic[0],
        0,
        Vec::new(),
        initial_state.0,
        initial_state.1,
    )]);
    let mut expanded = 0usize;

    while let Some((_, cost, path_key, position, marking)) = queue.pop_first() {
        let state = (position, marking.clone());
        let Some((best_cost, best_key, moves)) = best.get(&state) else {
            continue;
        };
        if *best_cost != cost || *best_key != path_key {
            continue;
        }
        if position == trace.len() && marking == net.final_marking {
            return Ok((cost, moves.clone()));
        }
        expanded += 1;
        if expanded > MAX_EXPANDED_STATES {
            return Err(AlignmentError::SearchLimit { case });
        }

        if let Some(event) = trace.get(position) {
            for transition in &net.transitions {
                if transition.activity == Some(event.activity)
                    && net.is_enabled(&marking, transition.id)
                {
                    let next = net.fire(&marking, transition.id)?;
                    relax(
                        &mut best,
                        &mut queue,
                        heuristic[position + 1],
                        state.clone(),
                        (position + 1, next),
                        0,
                        StepKey {
                            rank: 0,
                            transition: transition.id.0,
                            event_index: position,
                        },
                        AlignmentMove::Synchronous {
                            event_index: position,
                            source_row: event.source_row,
                            activity: event.activity,
                            transition: transition.id,
                        },
                    );
                }
            }
        }
        for transition in &net.transitions {
            if !net.is_enabled(&marking, transition.id) {
                continue;
            }
            let next = net.fire(&marking, transition.id)?;
            let (step_cost, rank, movement) = match transition.activity {
                None => (
                    0,
                    1,
                    AlignmentMove::InvisibleModel {
                        transition: transition.id,
                    },
                ),
                Some(activity) => (
                    1,
                    3,
                    AlignmentMove::VisibleModel {
                        transition: transition.id,
                        activity,
                    },
                ),
            };
            relax(
                &mut best,
                &mut queue,
                heuristic[position],
                state.clone(),
                (position, next),
                step_cost,
                StepKey {
                    rank,
                    transition: transition.id.0,
                    event_index: position,
                },
                movement,
            );
        }
        if let Some(event) = trace.get(position) {
            relax(
                &mut best,
                &mut queue,
                heuristic[position + 1],
                state.clone(),
                (position + 1, marking),
                1,
                StepKey {
                    rank: 2,
                    transition: u32::MAX,
                    event_index: position,
                },
                AlignmentMove::LogOnly {
                    event_index: position,
                    source_row: event.source_row,
                    activity: event.activity,
                },
            );
        }
    }
    Err(AlignmentError::NoAlignment { case })
}

#[allow(clippy::too_many_arguments)]
fn relax(
    best: &mut BTreeMap<State, BestPath>,
    queue: &mut BTreeSet<QueueEntry>,
    heuristic: u64,
    from: State,
    to: State,
    step_cost: u64,
    step_key: StepKey,
    movement: AlignmentMove,
) {
    let (cost, path_key, moves) = best.get(&from).expect("expanded state has a path");
    let next_cost = cost.saturating_add(step_cost);
    let mut next_key = path_key.clone();
    next_key.push(step_key);
    let improve = best
        .get(&to)
        .is_none_or(|(known_cost, known_key, _)| (next_cost, &next_key) < (*known_cost, known_key));
    if !improve {
        return;
    }
    let mut next_moves = moves.clone();
    next_moves.push(movement);
    best.insert(to.clone(), (next_cost, next_key.clone(), next_moves));
    queue.insert((
        next_cost.saturating_add(heuristic),
        next_cost,
        next_key,
        to.0,
        to.1,
    ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaseTrace, Dictionary, ProcessTree};

    fn fixture(labels: &[&str]) -> (EventLog, PetriNet) {
        let mut cases = Dictionary::new();
        let case = CaseId(cases.intern("C1"));
        let mut activities = Dictionary::new();
        let a = ActivityId(activities.intern("A"));
        let b = ActivityId(activities.intern("B"));
        let c = ActivityId(activities.intern("C"));
        let events = labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                Event::new(
                    ActivityId(activities.intern(label)),
                    index as i64,
                    index as u32 + 2,
                )
            })
            .collect();
        let log = EventLog {
            cases,
            activities,
            traces: vec![CaseTrace {
                case,
                events,
                incomplete: false,
                attributes: Vec::new(),
            }],
            ..EventLog::default()
        };
        let net = ProcessTree::sequence(vec![
            ProcessTree::activity(a),
            ProcessTree::activity(b),
            ProcessTree::activity(c),
        ])
        .to_petri_net()
        .unwrap();
        (log, net)
    }

    #[test]
    fn exact_trace_uses_only_synchronous_moves() {
        let (log, net) = fixture(&["A", "B", "C"]);
        let result = align_log(&log, &net).unwrap();
        assert_eq!(result.total_cost, 0);
        assert_eq!(result.fitness_millionths, FITNESS_SCALE);
        assert_eq!(result.cases[0].moves.len(), 3);
        assert!(
            result.cases[0]
                .moves
                .iter()
                .all(|movement| matches!(movement, AlignmentMove::Synchronous { .. }))
        );
    }

    #[test]
    fn skipped_model_activity_is_a_visible_model_move() {
        let (log, net) = fixture(&["A", "C"]);
        let result = align_log(&log, &net).unwrap();
        assert_eq!(result.total_cost, 1);
        assert!(matches!(
            result.cases[0].moves[1],
            AlignmentMove::VisibleModel {
                activity: ActivityId(1),
                ..
            }
        ));
    }

    #[test]
    fn unexpected_event_is_a_log_move_with_source_evidence() {
        let (log, net) = fixture(&["A", "Noise", "B", "C"]);
        let result = align_log(&log, &net).unwrap();
        assert_eq!(result.total_cost, 1);
        assert!(matches!(
            result.cases[0].moves[1],
            AlignmentMove::LogOnly { source_row: 3, .. }
        ));
    }

    #[test]
    fn parallel_interleavings_align_at_zero_cost() {
        let (log, _) = fixture(&["B", "A"]);
        let a = ActivityId(log.activities.get("A").unwrap());
        let b = ActivityId(log.activities.get("B").unwrap());
        let net = ProcessTree::parallel(vec![ProcessTree::activity(a), ProcessTree::activity(b)])
            .to_petri_net()
            .unwrap();
        let result = align_log(&log, &net).unwrap();
        assert_eq!(result.total_cost, 0);
        assert_eq!(result.fit_cases, 1);
    }

    #[test]
    fn equal_cost_alignment_uses_fixed_move_preference() {
        let (log, _) = fixture(&["Noise"]);
        let a = ActivityId(log.activities.get("A").unwrap());
        let b = ActivityId(log.activities.get("B").unwrap());
        let net = ProcessTree::exclusive(vec![ProcessTree::activity(a), ProcessTree::activity(b)])
            .to_petri_net()
            .unwrap();
        let first = align_log(&log, &net).unwrap();
        let second = align_log(&log, &net).unwrap();
        assert_eq!(first, second);
        assert!(matches!(
            first.cases[0].moves.as_slice(),
            [
                AlignmentMove::LogOnly { .. },
                AlignmentMove::VisibleModel {
                    transition: TransitionId(0),
                    ..
                }
            ]
        ));
    }
}
