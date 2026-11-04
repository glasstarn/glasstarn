//! Deterministic Heuristics Miner dependency and binding discovery.
//!
//! Scores use integer millionths. The result is deliberately a heuristic net,
//! not mislabeled as a sound accepting Petri net.

use crate::{ActivityId, EventLog};
use std::collections::{BTreeMap, BTreeSet};

const SCALE: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HeuristicsMinerConfig {
    pub dependency_threshold_millionths: u64,
    pub relative_to_best_millionths: u64,
    pub and_threshold_millionths: u64,
    pub minimum_observations: u64,
}

impl Default for HeuristicsMinerConfig {
    fn default() -> Self {
        Self {
            dependency_threshold_millionths: 500_000,
            relative_to_best_millionths: 50_000,
            and_threshold_millionths: 650_000,
            minimum_observations: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    Direct,
    LengthOneLoop,
    LengthTwoLoop,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HeuristicArc {
    pub from: ActivityId,
    pub to: ActivityId,
    pub observations: u64,
    pub dependency_millionths: u64,
    pub kind: DependencyKind,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AndBinding {
    pub anchor: ActivityId,
    pub activities: [ActivityId; 2],
    pub score_millionths: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HeuristicsNet {
    pub activities: Vec<ActivityId>,
    pub starts: Vec<(ActivityId, u64)>,
    pub ends: Vec<(ActivityId, u64)>,
    pub arcs: Vec<HeuristicArc>,
    pub and_splits: Vec<AndBinding>,
    pub and_joins: Vec<AndBinding>,
    pub config: HeuristicsMinerConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum HeuristicsError {
    #[error("Heuristics Miner needs at least one event")]
    EmptyLog,
    #[error("all thresholds must be at most 1,000,000 millionths")]
    InvalidThreshold,
    #[error("minimum observations must be at least one")]
    InvalidMinimumObservations,
}

pub fn discover_heuristics(
    log: &EventLog,
    config: HeuristicsMinerConfig,
) -> Result<HeuristicsNet, HeuristicsError> {
    if config.dependency_threshold_millionths > SCALE
        || config.relative_to_best_millionths > SCALE
        || config.and_threshold_millionths > SCALE
    {
        return Err(HeuristicsError::InvalidThreshold);
    }
    if config.minimum_observations == 0 {
        return Err(HeuristicsError::InvalidMinimumObservations);
    }
    let activities: BTreeSet<_> = log
        .traces
        .iter()
        .flat_map(|trace| trace.events.iter().map(|event| event.activity))
        .collect();
    if activities.is_empty() {
        return Err(HeuristicsError::EmptyLog);
    }
    let mut direct = BTreeMap::<(ActivityId, ActivityId), u64>::new();
    let mut length_two = BTreeMap::<(ActivityId, ActivityId), u64>::new();
    let mut starts = BTreeMap::new();
    let mut ends = BTreeMap::new();
    for trace in &log.traces {
        if let Some(first) = trace.events.first() {
            *starts.entry(first.activity).or_insert(0) += 1;
        }
        if let Some(last) = trace.events.last() {
            *ends.entry(last.activity).or_insert(0) += 1;
        }
        for pair in trace.events.windows(2) {
            *direct
                .entry((pair[0].activity, pair[1].activity))
                .or_insert(0) += 1;
        }
        for triple in trace.events.windows(3) {
            if triple[0].activity == triple[2].activity && triple[0].activity != triple[1].activity
            {
                *length_two
                    .entry((triple[0].activity, triple[1].activity))
                    .or_insert(0) += 1;
            }
        }
    }

    let mut candidates = Vec::new();
    for (&(from, to), &observations) in &direct {
        if observations < config.minimum_observations {
            continue;
        }
        let (score, kind) = if from == to {
            (
                ratio(observations, observations + 1),
                DependencyKind::LengthOneLoop,
            )
        } else {
            let reverse = direct.get(&(to, from)).copied().unwrap_or(0);
            let loops = length_two.get(&(from, to)).copied().unwrap_or(0)
                + length_two.get(&(to, from)).copied().unwrap_or(0);
            if loops >= config.minimum_observations {
                (ratio(loops, loops + 1), DependencyKind::LengthTwoLoop)
            } else if observations > reverse {
                (
                    ratio(observations - reverse, observations + reverse + 1),
                    DependencyKind::Direct,
                )
            } else {
                continue;
            }
        };
        candidates.push(HeuristicArc {
            from,
            to,
            observations,
            dependency_millionths: score,
            kind,
        });
    }
    let best_by_source: BTreeMap<_, _> =
        candidates.iter().fold(BTreeMap::new(), |mut best, arc| {
            best.entry(arc.from)
                .and_modify(|score: &mut u64| *score = (*score).max(arc.dependency_millionths))
                .or_insert(arc.dependency_millionths);
            best
        });
    let arcs: Vec<_> = candidates
        .into_iter()
        .filter(|arc| {
            arc.dependency_millionths >= config.dependency_threshold_millionths
                && best_by_source[&arc.from].saturating_sub(arc.dependency_millionths)
                    <= config.relative_to_best_millionths
        })
        .collect();
    let and_splits = bindings(&arcs, &direct, config.and_threshold_millionths, false);
    let and_joins = bindings(&arcs, &direct, config.and_threshold_millionths, true);
    Ok(HeuristicsNet {
        activities: activities.into_iter().collect(),
        starts: starts.into_iter().collect(),
        ends: ends.into_iter().collect(),
        arcs,
        and_splits,
        and_joins,
        config,
    })
}

fn bindings(
    arcs: &[HeuristicArc],
    direct: &BTreeMap<(ActivityId, ActivityId), u64>,
    threshold: u64,
    incoming: bool,
) -> Vec<AndBinding> {
    let mut neighbours = BTreeMap::<ActivityId, BTreeSet<ActivityId>>::new();
    for arc in arcs {
        let (anchor, neighbour) = if incoming {
            (arc.to, arc.from)
        } else {
            (arc.from, arc.to)
        };
        neighbours.entry(anchor).or_default().insert(neighbour);
    }
    let mut result = Vec::new();
    for (anchor, adjacent) in neighbours {
        let adjacent: Vec<_> = adjacent.into_iter().collect();
        for left in 0..adjacent.len() {
            for right in left + 1..adjacent.len() {
                let a = adjacent[left];
                let b = adjacent[right];
                let cross = direct.get(&(a, b)).copied().unwrap_or(0)
                    + direct.get(&(b, a)).copied().unwrap_or(0);
                let anchor_edges = if incoming {
                    direct.get(&(a, anchor)).copied().unwrap_or(0)
                        + direct.get(&(b, anchor)).copied().unwrap_or(0)
                } else {
                    direct.get(&(anchor, a)).copied().unwrap_or(0)
                        + direct.get(&(anchor, b)).copied().unwrap_or(0)
                };
                let score = ratio(cross, anchor_edges + 1);
                if score >= threshold {
                    result.push(AndBinding {
                        anchor,
                        activities: [a, b],
                        score_millionths: score,
                    });
                }
            }
        }
    }
    result
}

fn ratio(numerator: u64, denominator: u64) -> u64 {
    numerator
        .saturating_mul(SCALE)
        .checked_div(denominator)
        .unwrap_or(0)
        .min(SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaseId, CaseTrace, Dictionary, Event};

    fn log(spec: &[&[&str]]) -> EventLog {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let traces = spec
            .iter()
            .enumerate()
            .map(|(case_index, labels)| CaseTrace {
                case: CaseId(cases.intern(&format!("C{case_index}"))),
                events: labels
                    .iter()
                    .enumerate()
                    .map(|(index, label)| {
                        Event::new(ActivityId(activities.intern(label)), index as i64, 0)
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
    fn retains_strong_dependencies_and_filters_weak_reverse_edges() {
        let log = log(&[&["A", "B"], &["A", "B"], &["A", "B"], &["B", "A"]]);
        let config = HeuristicsMinerConfig {
            dependency_threshold_millionths: 300_000,
            relative_to_best_millionths: 1_000_000,
            ..Default::default()
        };
        let model = discover_heuristics(&log, config).unwrap();
        assert_eq!(model.arcs.len(), 1);
        assert_eq!((model.arcs[0].from.0, model.arcs[0].to.0), (0, 1));
        assert_eq!(model.arcs[0].dependency_millionths, 400_000);
    }

    #[test]
    fn detects_length_one_and_length_two_loops() {
        let log = log(&[&["A", "A", "B", "A", "B", "C"]]);
        let config = HeuristicsMinerConfig {
            dependency_threshold_millionths: 300_000,
            relative_to_best_millionths: 1_000_000,
            ..Default::default()
        };
        let model = discover_heuristics(&log, config).unwrap();
        assert!(
            model
                .arcs
                .iter()
                .any(|arc| arc.kind == DependencyKind::LengthOneLoop)
        );
        assert!(
            model
                .arcs
                .iter()
                .any(|arc| arc.kind == DependencyKind::LengthTwoLoop)
        );
    }

    #[test]
    fn finds_parallel_split_and_join_bindings() {
        let log = log(&[
            &["A", "B", "C", "D"],
            &["A", "C", "B", "D"],
            &["A", "B", "C", "D"],
            &["A", "C", "B", "D"],
        ]);
        let config = HeuristicsMinerConfig {
            and_threshold_millionths: 300_000,
            ..Default::default()
        };
        let model = discover_heuristics(&log, config).unwrap();
        assert!(!model.and_splits.is_empty());
        assert!(!model.and_joins.is_empty());
    }

    #[test]
    fn validates_configuration() {
        assert!(matches!(
            discover_heuristics(
                &log(&[&["A"]]),
                HeuristicsMinerConfig {
                    minimum_observations: 0,
                    ..Default::default()
                }
            ),
            Err(HeuristicsError::InvalidMinimumObservations)
        ));
    }
}
