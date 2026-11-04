//! Labelled, bounded conformance reporting for every front end.

use crate::{LabelledPetriNet, View};
use pm_core::{
    ActivityId, AlignmentMove, EventLog, Filter, PetriNet, PetriTransition, PlaceId,
    ReplayDeviation, TransitionId, align_log, replay_tokens,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviationEvidence {
    ActivityNotInModel {
        event_index: usize,
        source_row: u32,
        activity: String,
    },
    MissingTokens {
        event_index: usize,
        source_row: u32,
        activity: String,
        count: u64,
    },
    FinalTokensMissing {
        count: u64,
    },
    TokensRemaining {
        count: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviatingCase {
    pub case: String,
    pub fitness_millionths: u64,
    pub missing_tokens: u64,
    pub remaining_tokens: u64,
    pub unmapped_events: u64,
    pub deviations: Vec<DeviationEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConformanceReport {
    pub source: String,
    pub model_source: String,
    pub algorithm: String,
    pub view: View,
    pub fitness_millionths: u64,
    pub fitness_percent: f64,
    pub fit_cases: usize,
    pub deviating_cases: usize,
    pub fit_case_percent: f64,
    pub missing_tokens: u64,
    pub remaining_tokens: u64,
    pub unmapped_events: u64,
    pub deviations_shown: usize,
    pub deviation_limit: usize,
    pub cases: Vec<DeviatingCase>,
    /// Every deviating case counted by what went wrong at which step, most
    /// common first, so a reader sees "41 cases: Pay claim before its
    /// preceding steps" without reading cases one by one. Absent from older
    /// results.
    #[serde(default)]
    pub by_step: Vec<StepDeviation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepDeviation {
    /// The step, or empty for a case that ended early.
    pub activity: String,
    /// `out_of_order` (the step happened without the steps expected before
    /// it), `not_expected` (the step is not in the expected process) or
    /// `unfinished` (the case ended before the expected end).
    pub kind: String,
    /// Cases with this deviation.
    pub cases: usize,
    pub meaning: String,
    pub example_cases: Vec<String>,
    /// The step sequences of every case counted here, so a reader can open
    /// exactly these cases with a variant filter. Empty from older results.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Vec<String>>,
}

fn summarise_by_step(cases: &[DeviatingCase]) -> Vec<StepDeviation> {
    let mut counts: std::collections::BTreeMap<(String, String), (usize, Vec<String>)> =
        std::collections::BTreeMap::new();
    for case in cases {
        let mut seen = std::collections::BTreeSet::new();
        // Tokens left over after a step out of order are that same deviation,
        // not a second one: a case is "unfinished" only when nothing else
        // went wrong.
        let explained = case.deviations.iter().any(|deviation| {
            matches!(
                deviation,
                DeviationEvidence::MissingTokens { .. }
                    | DeviationEvidence::ActivityNotInModel { .. }
            )
        });
        for deviation in &case.deviations {
            let key = match deviation {
                DeviationEvidence::MissingTokens { activity, .. } => {
                    ("out_of_order", activity.clone())
                }
                DeviationEvidence::ActivityNotInModel { activity, .. } => {
                    ("not_expected", activity.clone())
                }
                _ if explained => continue,
                _ => ("unfinished", String::new()),
            };
            if seen.insert(key.clone()) {
                let entry = counts.entry((key.0.to_owned(), key.1)).or_default();
                entry.0 += 1;
                if entry.1.len() < 5 {
                    entry.1.push(case.case.clone());
                }
            }
        }
    }
    let mut out: Vec<StepDeviation> = counts
        .into_iter()
        .map(|((kind, activity), (cases, example_cases))| StepDeviation {
            meaning: match kind.as_str() {
                "out_of_order" => format!(
                    "{activity} happened without the steps the expected process puts before it"
                ),
                "not_expected" => format!("{activity} is not part of the expected process"),
                _ => "the case ended before the expected process was complete".to_owned(),
            },
            activity,
            kind,
            cases,
            example_cases,
            variants: Vec::new(),
        })
        .collect();
    out.sort_by(|a, b| b.cases.cmp(&a.cases).then(a.activity.cmp(&b.activity)));
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AlignmentDeviation {
    LogOnly {
        event_index: usize,
        source_row: u32,
        activity: String,
    },
    VisibleModel {
        transition: u32,
        activity: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AlignedCase {
    pub case: String,
    pub cost: u64,
    pub fitness_millionths: u64,
    pub synchronous_moves: usize,
    pub invisible_moves: usize,
    pub deviations: Vec<AlignmentDeviation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlignmentReport {
    pub source: String,
    pub model_source: String,
    pub algorithm: String,
    pub view: View,
    pub total_cost: u64,
    pub fitness_millionths: u64,
    pub fitness_percent: f64,
    pub fit_cases: usize,
    pub deviating_cases: usize,
    pub fit_case_percent: f64,
    pub deviations_shown: usize,
    pub deviation_limit: usize,
    pub cases: Vec<AlignedCase>,
    /// Every deviating case counted by step: a required step that was
    /// `skipped`, or a step recorded where the expected process has no place
    /// for it (`extra`). Absent from older results.
    #[serde(default)]
    pub by_step: Vec<StepDeviation>,
}

/// The executable portion shared by discovered and imported formal-model JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortableFormalModel {
    pub source: String,
    pub net: LabelledPetriNet,
}

#[derive(Debug, thiserror::Error)]
pub enum FormalModelError {
    #[error("model is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model has no string 'source' field")]
    MissingSource,
    #[error("model has no executable 'net' field")]
    MissingNet,
    #[error("model has an invalid executable net: {0}")]
    InvalidNet(serde_json::Error),
    #[error("model has an invalid process tree: {0}")]
    InvalidTree(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ConformanceError {
    #[error("formal model contains too many distinct activity labels")]
    TooManyActivities,
    #[error(transparent)]
    Model(#[from] pm_core::ModelError),
    #[error(transparent)]
    Replay(#[from] pm_core::ReplayError),
    #[error(transparent)]
    Alignment(#[from] pm_core::AlignmentError),
}

/// Validates the complete executable-net structure before a front end accepts
/// a portable model as a baseline.
pub fn validate_formal_model(model: &LabelledPetriNet) -> Result<(), ConformanceError> {
    let mut labels = EventLog::default();
    resolve_model(model, &mut labels)?.validate()?;
    Ok(())
}

/// Reads the common `source` and executable `net` contract emitted by formal
/// discovery and BPMN import. Heuristic nets intentionally have no executable
/// Petri net and are therefore rejected as conformance baselines.
pub fn parse_formal_model_json(bytes: &[u8]) -> Result<PortableFormalModel, FormalModelError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let source = value
        .get("source")
        .and_then(serde_json::Value::as_str)
        .ok_or(FormalModelError::MissingSource)?
        .to_owned();
    // An expected process written as steps (a process tree with step names)
    // is turned into its executable net here.
    if value.get("net").is_none()
        && let Some(tree) = value.get("tree")
    {
        let tree: crate::LabelledProcessTree =
            serde_json::from_value(tree.clone()).map_err(FormalModelError::InvalidNet)?;
        let net = crate::discovery::tree_to_net(&tree).map_err(FormalModelError::InvalidTree)?;
        return Ok(PortableFormalModel { source, net });
    }
    let net = value
        .get("net")
        .cloned()
        .ok_or(FormalModelError::MissingNet)?;
    let net = serde_json::from_value(net).map_err(FormalModelError::InvalidNet)?;
    Ok(PortableFormalModel { source, net })
}

/// Replays a filtered log against an explicit labelled accepting Petri net.
///
/// Only the worst `deviation_limit` cases are returned, while all headline
/// counts and token totals cover the complete filtered view.
pub fn check_conformance(
    source: &str,
    model_source: &str,
    log: &EventLog,
    filter: &Filter,
    model: &LabelledPetriNet,
    deviation_limit: usize,
) -> Result<ConformanceReport, ConformanceError> {
    let filtered = filter.apply(log);
    let view = View {
        cases_shown: filtered.log.case_count(),
        cases_total: filtered.cases_before,
        events_total: filtered.events_before,
        percent_shown: filtered.cases_kept_percent(),
        narrowed: filtered.narrowed(),
    };
    // Resolving the model may add its activities to the log, so it needs its own copy.
    let mut replay_log = filtered.log.into_owned();
    let net = resolve_model(model, &mut replay_log)?;
    let replay = replay_tokens(&replay_log, &net)?;
    let deviating_cases = replay.cases.len().saturating_sub(replay.fit_cases);
    let fit_case_percent = if replay.cases.is_empty() {
        100.0
    } else {
        replay.fit_cases as f64 / replay.cases.len() as f64 * 100.0
    };
    let mut cases: Vec<_> = replay
        .cases
        .iter()
        .filter(|case| !case.fit)
        .map(|case| DeviatingCase {
            case: replay_log.case_label(case.case).to_owned(),
            fitness_millionths: case.fitness_millionths,
            missing_tokens: case.missing_tokens,
            remaining_tokens: case.remaining_tokens,
            unmapped_events: case.unmapped_events,
            deviations: case
                .deviations
                .iter()
                .map(|deviation| label_deviation(deviation, &replay_log))
                .collect(),
        })
        .collect();
    cases.sort_by(|left, right| {
        (
            left.fitness_millionths,
            std::cmp::Reverse(left.missing_tokens + left.remaining_tokens),
            &left.case,
        )
            .cmp(&(
                right.fitness_millionths,
                std::cmp::Reverse(right.missing_tokens + right.remaining_tokens),
                &right.case,
            ))
    });
    let by_step = summarise_by_step(&cases);
    cases.truncate(deviation_limit);

    Ok(ConformanceReport {
        by_step,
        source: source.to_owned(),
        model_source: model_source.to_owned(),
        algorithm: "token_based_replay".to_owned(),
        view,
        fitness_millionths: replay.fitness_millionths,
        fitness_percent: replay.fitness_millionths as f64 / 10_000.0,
        fit_cases: replay.fit_cases,
        deviating_cases,
        fit_case_percent,
        missing_tokens: replay.missing_tokens,
        remaining_tokens: replay.remaining_tokens,
        unmapped_events: replay.unmapped_events,
        deviations_shown: cases.len(),
        deviation_limit,
        cases,
    })
}

/// Computes optimal A* alignments against an explicit accepting Petri net.
///
/// Headline metrics cover the complete filtered view. Detailed move evidence
/// is bounded to the worst `deviation_limit` cases.
pub fn align_conformance(
    source: &str,
    model_source: &str,
    log: &EventLog,
    filter: &Filter,
    model: &LabelledPetriNet,
    deviation_limit: usize,
) -> Result<AlignmentReport, ConformanceError> {
    let filtered = filter.apply(log);
    let view = View {
        cases_shown: filtered.log.case_count(),
        cases_total: filtered.cases_before,
        events_total: filtered.events_before,
        percent_shown: filtered.cases_kept_percent(),
        narrowed: filtered.narrowed(),
    };
    // Resolving the model may add its activities to the log, so it needs its own copy.
    let mut replay_log = filtered.log.into_owned();
    let net = resolve_model(model, &mut replay_log)?;
    let alignment = align_log(&replay_log, &net)?;
    let deviating_cases = alignment.cases.len().saturating_sub(alignment.fit_cases);
    let fit_case_percent = if alignment.cases.is_empty() {
        100.0
    } else {
        alignment.fit_cases as f64 / alignment.cases.len() as f64 * 100.0
    };
    let mut cases: Vec<_> = alignment
        .cases
        .iter()
        .filter(|case| !case.fit)
        .map(|case| {
            let synchronous_moves = case
                .moves
                .iter()
                .filter(|movement| matches!(movement, AlignmentMove::Synchronous { .. }))
                .count();
            let invisible_moves = case
                .moves
                .iter()
                .filter(|movement| matches!(movement, AlignmentMove::InvisibleModel { .. }))
                .count();
            let deviations = case
                .moves
                .iter()
                .filter_map(|movement| label_alignment_deviation(movement, &replay_log))
                .collect();
            AlignedCase {
                case: replay_log.case_label(case.case).to_owned(),
                cost: case.cost,
                fitness_millionths: case.fitness_millionths,
                synchronous_moves,
                invisible_moves,
                deviations,
            }
        })
        .collect();
    cases.sort_by(|left, right| {
        (std::cmp::Reverse(left.cost), &left.case)
            .cmp(&(std::cmp::Reverse(right.cost), &right.case))
    });
    let by_step = summarise_alignment_by_step(&cases, &replay_log);
    cases.truncate(deviation_limit);

    Ok(AlignmentReport {
        source: source.to_owned(),
        model_source: model_source.to_owned(),
        algorithm: "astar_alignment".to_owned(),
        view,
        total_cost: alignment.total_cost,
        fitness_millionths: alignment.fitness_millionths,
        fitness_percent: alignment.fitness_millionths as f64 / 10_000.0,
        fit_cases: alignment.fit_cases,
        deviating_cases,
        fit_case_percent,
        deviations_shown: cases.len(),
        deviation_limit,
        cases,
        by_step,
    })
}

/// Cases counted, example cases, and the step sequences of every case.
type GroupTally = (usize, Vec<String>, std::collections::BTreeSet<Vec<String>>);

/// Each skipped or extra step with the cases it affects, most common first.
fn summarise_alignment_by_step(cases: &[AlignedCase], log: &EventLog) -> Vec<StepDeviation> {
    use std::collections::{BTreeMap, BTreeSet};
    let sequences: BTreeMap<&str, Vec<String>> = log
        .traces
        .iter()
        .map(|trace| {
            (
                log.case_label(trace.case),
                trace
                    .events
                    .iter()
                    .map(|event| log.activity_label(event.activity).to_owned())
                    .collect(),
            )
        })
        .collect();
    let mut counts: BTreeMap<(&str, String), GroupTally> = BTreeMap::new();
    for case in cases {
        let mut seen = BTreeSet::new();
        for deviation in &case.deviations {
            let key = match deviation {
                AlignmentDeviation::VisibleModel { activity, .. } => ("skipped", activity.clone()),
                AlignmentDeviation::LogOnly { activity, .. } => ("extra", activity.clone()),
            };
            if seen.insert(key.clone()) {
                let entry = counts.entry(key).or_default();
                entry.0 += 1;
                if entry.1.len() < 5 {
                    entry.1.push(case.case.clone());
                }
                if let Some(sequence) = sequences.get(case.case.as_str()) {
                    entry.2.insert(sequence.clone());
                }
            }
        }
    }
    let mut out: Vec<StepDeviation> = counts
        .into_iter()
        .map(
            |((kind, activity), (cases, example_cases, variants))| StepDeviation {
                meaning: match kind {
                    "skipped" => format!(
                        "{activity} is required by the expected process but was not recorded"
                    ),
                    _ => format!(
                        "{activity} was recorded where the expected process does not allow it"
                    ),
                },
                activity,
                kind: kind.to_owned(),
                cases,
                example_cases,
                variants: variants.into_iter().collect(),
            },
        )
        .collect();
    out.sort_by(|a, b| b.cases.cmp(&a.cases).then(a.activity.cmp(&b.activity)));
    out
}

/// True when the model has steps that are not recorded (choices of "or
/// nothing", loops that may not run). Token replay guesses around those and
/// can mark compliant cases as deviating, so such a model is checked by
/// alignment, which is exact.
pub fn has_silent_steps(model: &LabelledPetriNet) -> bool {
    model
        .transitions
        .iter()
        .any(|transition| transition.activity.is_none())
}

/// One step of an expected process, for showing it as a line of steps.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpectedStep {
    pub step: String,
    /// The process can finish without this step (an "or nothing" choice, one
    /// of several alternatives, or inside a loop that need not run).
    pub optional: bool,
}

/// The expected process's steps in order of first appearance. A model
/// written as steps (a process tree) also says which steps are optional; a
/// model given only as a net lists its labelled steps.
pub fn expected_steps(model_json: &[u8], model: &LabelledPetriNet) -> Vec<ExpectedStep> {
    let tree = serde_json::from_slice::<serde_json::Value>(model_json)
        .ok()
        .and_then(|value| value.get("tree").cloned())
        .and_then(|tree| serde_json::from_value::<crate::LabelledProcessTree>(tree).ok());
    let mut seen = std::collections::BTreeSet::new();
    let Some(tree) = tree else {
        return model
            .transitions
            .iter()
            .filter_map(|transition| transition.activity.clone())
            .filter(|label| seen.insert(label.clone()))
            .map(|step| ExpectedStep {
                step,
                optional: false,
            })
            .collect();
    };
    let required = always_occurs(&tree);
    let mut order = Vec::new();
    tree_steps(&tree, &mut order);
    order
        .into_iter()
        .filter(|label| seen.insert(label.clone()))
        .map(|step| ExpectedStep {
            optional: !required.contains(&step),
            step,
        })
        .collect()
}

fn tree_steps(tree: &crate::LabelledProcessTree, out: &mut Vec<String>) {
    use crate::LabelledProcessTree as Tree;
    match tree {
        Tree::Activity { activity } => out.push(activity.clone()),
        Tree::Tau => {}
        Tree::Sequence { children }
        | Tree::Exclusive { children }
        | Tree::Parallel { children } => {
            children.iter().for_each(|child| tree_steps(child, out));
        }
        Tree::Loop { body, redo } => {
            tree_steps(body, out);
            tree_steps(redo, out);
        }
    }
}

/// The steps every complete run of the tree records at least once.
fn always_occurs(tree: &crate::LabelledProcessTree) -> std::collections::BTreeSet<String> {
    use crate::LabelledProcessTree as Tree;
    match tree {
        Tree::Activity { activity } => std::collections::BTreeSet::from([activity.clone()]),
        Tree::Tau => std::collections::BTreeSet::new(),
        Tree::Sequence { children } | Tree::Parallel { children } => {
            children.iter().flat_map(always_occurs).collect()
        }
        Tree::Exclusive { children } => {
            let mut sets = children.iter().map(always_occurs);
            let first = sets.next().unwrap_or_default();
            sets.fold(first, |kept, next| {
                kept.intersection(&next).cloned().collect()
            })
        }
        // The body runs at least once; the redo part may never run.
        Tree::Loop { body, .. } => always_occurs(body),
    }
}

fn resolve_model(
    model: &LabelledPetriNet,
    log: &mut EventLog,
) -> Result<PetriNet, ConformanceError> {
    let transitions = model
        .transitions
        .iter()
        .map(|transition| {
            let activity = transition
                .activity
                .as_ref()
                .map(|label| {
                    u32::try_from(log.activities.len())
                        .map_err(|_| ConformanceError::TooManyActivities)?;
                    Ok::<ActivityId, ConformanceError>(ActivityId(log.activities.intern(label)))
                })
                .transpose()?;
            Ok(PetriTransition {
                id: TransitionId(transition.id),
                activity,
                inputs: transition.inputs.iter().copied().map(PlaceId).collect(),
                outputs: transition.outputs.iter().copied().map(PlaceId).collect(),
            })
        })
        .collect::<Result<Vec<_>, ConformanceError>>()?;
    Ok(PetriNet {
        place_count: model.place_count,
        transitions,
        initial_marking: model.initial_marking.clone(),
        final_marking: model.final_marking.clone(),
    })
}

fn label_deviation(deviation: &ReplayDeviation, log: &EventLog) -> DeviationEvidence {
    match deviation {
        ReplayDeviation::ActivityNotInModel {
            event_index,
            source_row,
            activity,
        } => DeviationEvidence::ActivityNotInModel {
            event_index: *event_index,
            source_row: *source_row,
            activity: log.activity_label(*activity).to_owned(),
        },
        ReplayDeviation::MissingTokens {
            event_index,
            source_row,
            activity,
            count,
        } => DeviationEvidence::MissingTokens {
            event_index: *event_index,
            source_row: *source_row,
            activity: log.activity_label(*activity).to_owned(),
            count: *count,
        },
        ReplayDeviation::FinalTokensMissing { count } => {
            DeviationEvidence::FinalTokensMissing { count: *count }
        }
        ReplayDeviation::TokensRemaining { count } => {
            DeviationEvidence::TokensRemaining { count: *count }
        }
    }
}

fn label_alignment_deviation(
    movement: &AlignmentMove,
    log: &EventLog,
) -> Option<AlignmentDeviation> {
    match movement {
        AlignmentMove::LogOnly {
            event_index,
            source_row,
            activity,
        } => Some(AlignmentDeviation::LogOnly {
            event_index: *event_index,
            source_row: *source_row,
            activity: log.activity_label(*activity).to_owned(),
        }),
        AlignmentMove::VisibleModel {
            transition,
            activity,
        } => Some(AlignmentDeviation::VisibleModel {
            transition: transition.0,
            activity: log.activity_label(*activity).to_owned(),
        }),
        AlignmentMove::Synchronous { .. } | AlignmentMove::InvisibleModel { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FormalDiscovery, LabelledProcessTree, discover_formal};
    use pm_core::{CaseId, CaseTrace, Dictionary, Event, InductiveMinerConfig};

    fn log(spec: &[(&str, &[&str])]) -> EventLog {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let mut traces = Vec::new();
        let mut row = 2;
        for (case_label, labels) in spec {
            let case = CaseId(cases.intern(case_label));
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

    fn sequence_model() -> FormalDiscovery {
        let baseline = log(&[("BASE", &["A", "B", "C"])]);
        discover_formal(
            "baseline.csv",
            &baseline,
            &Filter::new(),
            InductiveMinerConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn reports_deviating_cases_with_source_rows() {
        let model = sequence_model();
        let actual = log(&[
            ("FIT", &["A", "B", "C"]),
            ("SKIP", &["A", "C"]),
            ("NOISE", &["A", "Noise", "B", "C"]),
        ]);
        let report = check_conformance(
            "actual.csv",
            &model.source,
            &actual,
            &Filter::new(),
            &model.net,
            20,
        )
        .unwrap();
        assert_eq!(report.fit_cases, 1);
        assert_eq!(report.deviating_cases, 2);
        assert_eq!(report.cases.len(), 2);
        assert_eq!(report.cases[0].case, "SKIP");
        assert!(
            report
                .cases
                .iter()
                .any(|case| case.deviations.iter().any(|deviation| matches!(
                    deviation,
                    DeviationEvidence::MissingTokens { source_row: 6, .. }
                )))
        );
        assert!(report.cases.iter().any(|case| case.deviations.iter().any(
            |deviation| matches!(deviation, DeviationEvidence::ActivityNotInModel { activity, .. } if activity == "Noise")
        )));
    }

    #[test]
    fn alignment_reports_exact_log_and_model_moves() {
        let model = sequence_model();
        let actual = log(&[("SKIP", &["A", "C"]), ("NOISE", &["A", "Noise", "B", "C"])]);
        let report = align_conformance(
            "actual.csv",
            &model.source,
            &actual,
            &Filter::new(),
            &model.net,
            20,
        )
        .unwrap();
        assert_eq!(report.algorithm, "astar_alignment");
        assert_eq!(report.total_cost, 2);
        assert_eq!(report.fit_cases, 0);
        assert_eq!(report.deviating_cases, 2);
        assert!(report.cases.iter().any(|case| case.deviations.iter().any(
            |deviation| matches!(deviation, AlignmentDeviation::VisibleModel { activity, .. } if activity == "B")
        )));
        assert!(report.cases.iter().any(|case| case.deviations.iter().any(
            |deviation| matches!(deviation, AlignmentDeviation::LogOnly { activity, source_row: 5, .. } if activity == "Noise")
        )));
    }

    #[test]
    fn alignment_detail_is_bounded_but_cost_is_complete() {
        let model = sequence_model();
        let actual = log(&[
            ("C1", &["A", "C"]),
            ("C2", &["A", "C"]),
            ("C3", &["A", "C"]),
        ]);
        let report = align_conformance(
            "actual.csv",
            &model.source,
            &actual,
            &Filter::new(),
            &model.net,
            1,
        )
        .unwrap();
        assert_eq!(report.total_cost, 3);
        assert_eq!(report.deviating_cases, 3);
        assert_eq!(report.deviations_shown, 1);
        assert_eq!(report.cases[0].case, "C1");
    }

    #[test]
    fn deviation_detail_is_bounded_but_totals_are_complete() {
        let model = sequence_model();
        let actual = log(&[
            ("C1", &["A", "C"]),
            ("C2", &["A", "C"]),
            ("C3", &["A", "C"]),
        ]);
        let report = check_conformance(
            "actual.csv",
            &model.source,
            &actual,
            &Filter::new(),
            &model.net,
            1,
        )
        .unwrap();
        assert_eq!(report.deviating_cases, 3);
        assert_eq!(report.deviations_shown, 1);
        assert_eq!(report.cases[0].case, "C1");
    }

    #[test]
    fn uses_labels_not_dictionary_identifier_coincidence() {
        let model = sequence_model();
        let actual = log(&[("C1", &["C", "A", "B"])]);
        let report = check_conformance(
            "actual.csv",
            &model.source,
            &actual,
            &Filter::new(),
            &model.net,
            20,
        )
        .unwrap();
        assert_eq!(report.fit_cases, 0);
        assert!(report.missing_tokens > 0 || report.remaining_tokens > 0);
    }

    #[test]
    fn filtered_scope_is_disclosed() {
        let model = sequence_model();
        let actual = log(&[("FIT", &["A", "B", "C"]), ("SKIP", &["A", "C"])]);
        let report = check_conformance(
            "actual.csv",
            &model.source,
            &actual,
            &Filter::new().with_activity("B"),
            &model.net,
            20,
        )
        .unwrap();
        assert_eq!(report.view.cases_shown, 1);
        assert_eq!(report.view.cases_total, 2);
        assert_eq!(report.fit_cases, 1);
        assert_eq!(report.fit_case_percent, 100.0);
    }

    #[test]
    fn model_json_round_trips_for_cli_use() {
        let model = sequence_model();
        let json = serde_json::to_string(&model).unwrap();
        let restored = parse_formal_model_json(json.as_bytes()).unwrap();
        assert_eq!(restored.net, model.net);
        assert_eq!(restored.source, "baseline.csv");
        assert!(matches!(model.tree, LabelledProcessTree::Sequence { .. }));
    }

    #[test]
    fn portable_model_parser_rejects_heuristic_and_malformed_contracts() {
        assert!(matches!(
            parse_formal_model_json(br#"{"source":"baseline.csv"}"#),
            Err(FormalModelError::MissingNet)
        ));
        assert!(matches!(
            parse_formal_model_json(br#"{"net":{}}"#),
            Err(FormalModelError::MissingSource)
        ));
        assert!(matches!(
            parse_formal_model_json(br#"{"source":"baseline.csv","net":{}}"#),
            Err(FormalModelError::InvalidNet(_))
        ));
        let mut invalid = sequence_model().net;
        invalid.initial_marking.pop();
        assert!(matches!(
            validate_formal_model(&invalid),
            Err(ConformanceError::Model(_))
        ));
    }

    // An SOP written as steps, with the shapes that broke token replay on the
    // claims check in testing: optional steps, a choice, and a trailing loop
    // whose body is silent. Ground truth by construction: the compliant cases
    // are generated from the SOP, the deviations are planted.
    const CLAIMS_SOP: &str = r#"{"source":"Claims SOP v2.1; appeals after closure","tree":{"kind":"sequence","children":[
        {"kind":"activity","activity":"Submit"},
        {"kind":"loop","body":{"kind":"activity","activity":"Assess"},"redo":{"kind":"activity","activity":"Request documents"}},
        {"kind":"exclusive","children":[{"kind":"activity","activity":"Senior review"},{"kind":"tau"}]},
        {"kind":"exclusive","children":[
          {"kind":"sequence","children":[{"kind":"activity","activity":"Approve claim"},{"kind":"activity","activity":"Approve payment"},{"kind":"activity","activity":"Pay"}]},
          {"kind":"activity","activity":"Reject"}]},
        {"kind":"activity","activity":"Notify"},
        {"kind":"loop","body":{"kind":"tau"},"redo":{"kind":"sequence","children":[
          {"kind":"activity","activity":"Appeal"},{"kind":"activity","activity":"Senior review"},{"kind":"activity","activity":"Notify"}]}},
        {"kind":"activity","activity":"Close"}]}}"#;

    fn claims_log() -> EventLog {
        log(&[
            (
                "OK-1",
                &[
                    "Submit",
                    "Assess",
                    "Approve claim",
                    "Approve payment",
                    "Pay",
                    "Notify",
                    "Close",
                ],
            ),
            (
                "OK-2",
                &[
                    "Submit",
                    "Assess",
                    "Request documents",
                    "Assess",
                    "Senior review",
                    "Reject",
                    "Notify",
                    "Close",
                ],
            ),
            (
                "OK-3",
                &[
                    "Submit",
                    "Assess",
                    "Reject",
                    "Notify",
                    "Appeal",
                    "Senior review",
                    "Notify",
                    "Close",
                ],
            ),
            (
                "OK-4",
                &[
                    "Submit",
                    "Assess",
                    "Approve claim",
                    "Approve payment",
                    "Pay",
                    "Notify",
                    "Close",
                ],
            ),
            (
                "NO-NOTIFY-1",
                &[
                    "Submit",
                    "Assess",
                    "Approve claim",
                    "Approve payment",
                    "Pay",
                    "Close",
                ],
            ),
            ("NO-NOTIFY-2", &["Submit", "Assess", "Reject", "Close"]),
            (
                "NO-NOTIFY-3",
                &[
                    "Submit",
                    "Assess",
                    "Approve claim",
                    "Approve payment",
                    "Pay",
                    "Close",
                ],
            ),
            (
                "NO-APPROVAL-1",
                &[
                    "Submit",
                    "Assess",
                    "Approve claim",
                    "Pay",
                    "Notify",
                    "Close",
                ],
            ),
            (
                "BOTH",
                &["Submit", "Assess", "Approve claim", "Pay", "Close"],
            ),
        ])
    }

    #[test]
    fn sop_check_counts_planted_breaches_by_step_and_fits_every_compliant_case() {
        let model = parse_formal_model_json(CLAIMS_SOP.as_bytes()).unwrap();
        assert!(has_silent_steps(&model.net));
        let report = align_conformance(
            "claims.csv",
            &model.source,
            &claims_log(),
            &Filter::new(),
            &model.net,
            2,
        )
        .unwrap();
        assert_eq!(
            report.fit_cases, 4,
            "every case generated from the SOP fits it"
        );
        assert_eq!(report.deviating_cases, 5);
        let counted: Vec<_> = report
            .by_step
            .iter()
            .map(|step| (step.kind.as_str(), step.activity.as_str(), step.cases))
            .collect();
        assert_eq!(
            counted,
            vec![("skipped", "Notify", 4), ("skipped", "Approve payment", 2)]
        );
        // The counts and the variants cover every case, not just the two
        // examples kept in detail.
        assert_eq!(report.cases.len(), 2);
        let notify = &report.by_step[0];
        assert_eq!(
            notify.variants.len(),
            3,
            "NO-NOTIFY-1 and -3 share one variant"
        );
        let approval = &report.by_step[1];
        assert!(approval.variants.contains(&vec![
            "Submit".to_owned(),
            "Assess".to_owned(),
            "Approve claim".to_owned(),
            "Pay".to_owned(),
            "Notify".to_owned(),
            "Close".to_owned()
        ]));
    }

    #[test]
    fn token_replay_does_not_count_left_over_tokens_as_a_second_deviation() {
        let model = sequence_model();
        let actual = log(&[("SKIP", &["A", "C"]), ("EARLY", &["A", "B"])]);
        let report = check_conformance(
            "actual.csv",
            &model.source,
            &actual,
            &Filter::new(),
            &model.net,
            20,
        )
        .unwrap();
        let counted: Vec<_> = report
            .by_step
            .iter()
            .map(|step| (step.kind.as_str(), step.activity.as_str(), step.cases))
            .collect();
        // SKIP counts once, for C; only EARLY, which skipped nothing, is unfinished.
        assert_eq!(
            counted,
            vec![("unfinished", "", 1), ("out_of_order", "C", 1)]
        );
    }

    #[test]
    fn expected_steps_follow_the_sop_and_mark_optional_ones() {
        let model = parse_formal_model_json(CLAIMS_SOP.as_bytes()).unwrap();
        let steps = expected_steps(CLAIMS_SOP.as_bytes(), &model.net);
        let shown: Vec<_> = steps
            .iter()
            .map(|step| (step.step.as_str(), step.optional))
            .collect();
        assert_eq!(
            shown,
            vec![
                ("Submit", false),
                ("Assess", false),
                ("Request documents", true),
                ("Senior review", true),
                ("Approve claim", true),
                ("Approve payment", true),
                ("Pay", true),
                ("Reject", true),
                ("Notify", false),
                ("Appeal", true),
                ("Close", false),
            ]
        );
    }

    #[test]
    fn cases_with_the_same_steps_align_identically_but_cite_their_own_rows() {
        let model = parse_formal_model_json(CLAIMS_SOP.as_bytes()).unwrap();
        let actual = claims_log();
        let report = align_conformance(
            "claims.csv",
            &model.source,
            &actual,
            &Filter::new(),
            &model.net,
            20,
        )
        .unwrap();
        let alone = log(&[(
            "NO-NOTIFY-3",
            &[
                "Submit",
                "Assess",
                "Approve claim",
                "Approve payment",
                "Pay",
                "Close",
            ],
        )]);
        let single = align_conformance(
            "one.csv",
            &model.source,
            &alone,
            &Filter::new(),
            &model.net,
            20,
        )
        .unwrap();
        let shared = report
            .cases
            .iter()
            .find(|case| case.case == "NO-NOTIFY-3")
            .unwrap();
        assert_eq!(shared.cost, single.cases[0].cost);
        assert_eq!(shared.deviations, single.cases[0].deviations);
        // A log-only move keeps the row of its own case, not the first case's.
        let noisy = log(&[
            (
                "X",
                &["Submit", "Noise", "Assess", "Reject", "Notify", "Close"],
            ),
            (
                "Y",
                &["Submit", "Noise", "Assess", "Reject", "Notify", "Close"],
            ),
        ]);
        let report = align_conformance(
            "noisy.csv",
            &model.source,
            &noisy,
            &Filter::new(),
            &model.net,
            20,
        )
        .unwrap();
        let rows: Vec<_> = report
            .cases
            .iter()
            .flat_map(|case| case.deviations.iter())
            .filter_map(|deviation| match deviation {
                AlignmentDeviation::LogOnly { source_row, .. } => Some(*source_row),
                AlignmentDeviation::VisibleModel { .. } => None,
            })
            .collect();
        assert_eq!(rows, vec![3, 9]);
    }
}
