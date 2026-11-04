//! Deterministic comparison of two case cohorts.
//!
//! A comparison reports associations in the observed log. It never claims
//! that the cohort attribute caused a difference.

use pm_core::{EventLog, Filter};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MICROS_PER_SECOND: i64 = 1_000_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ComparisonError {
    #[error("case attribute '{0}' is not mapped in the active view")]
    UnknownAttribute(String),
    #[error("cohort value '{value}' does not occur for case attribute '{attribute}'")]
    UnknownValue { attribute: String, value: String },
    #[error("the two cohort values must be different")]
    SameValue,
    #[error("{period} period starts after it ends")]
    InvalidPeriod { period: &'static str },
    #[error("the baseline period must end before the comparison period starts")]
    PeriodsOverlapOrReversed,
    #[error("the {period} period contains no cases in the active view")]
    EmptyPeriod { period: &'static str },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CohortSummary {
    pub value: String,
    pub cases: usize,
    pub events: usize,
    pub duration_eligible_cases: usize,
    pub mean_throughput_seconds: i64,
    pub median_throughput_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityDifference {
    pub rank: usize,
    pub activity: String,
    pub left_occurrences: u64,
    pub right_occurrences: u64,
    pub left_cases: u64,
    pub right_cases: u64,
    pub left_case_percent: f64,
    pub right_case_percent: f64,
    /// Right minus left, in percentage points.
    pub difference_points: f64,
    pub left_example_cases: Vec<String>,
    pub right_example_cases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CohortComparison {
    pub attribute: String,
    pub left: CohortSummary,
    pub right: CohortSummary,
    /// Items in both groups: for a field that changes between steps, an item
    /// that had both values is on each side.
    #[serde(default)]
    pub shared_items: usize,
    /// Right minus left. This is an observed association, not a causal effect.
    pub mean_throughput_difference_seconds: i64,
    pub activity_differences: Vec<ActivityDifference>,
    pub interpretation_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PeriodSummary {
    pub started_at_or_after: i64,
    pub started_at_or_before: i64,
    pub cases: usize,
    pub events: usize,
    pub duration_eligible_cases: usize,
    pub mean_throughput_seconds: i64,
    pub median_throughput_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PeriodComparison {
    pub baseline: PeriodSummary,
    pub comparison: PeriodSummary,
    /// Comparison minus baseline. This is an observed change, not causation.
    pub mean_throughput_difference_seconds: i64,
    pub activity_differences: Vec<ActivityDifference>,
    pub interpretation_limits: Vec<String>,
}

pub fn compare_cohorts(
    log: &EventLog,
    attribute: &str,
    left: &str,
    right: &str,
) -> Result<CohortComparison, ComparisonError> {
    if left == right {
        return Err(ComparisonError::SameValue);
    }
    if log.attribute_index(attribute).is_none() {
        return Err(ComparisonError::UnknownAttribute(attribute.to_owned()));
    }
    let options = log.attribute_options(attribute);
    for value in [left, right] {
        if !options.contains(&value) {
            return Err(ComparisonError::UnknownValue {
                attribute: attribute.to_owned(),
                value: value.to_owned(),
            });
        }
    }

    let left_log = Filter::new().attribute(attribute, left).apply(log).log;
    let right_log = Filter::new().attribute(attribute, right).apply(log).log;
    let left_items: std::collections::BTreeSet<_> =
        left_log.traces.iter().map(|trace| trace.case).collect();
    let shared_items = right_log
        .traces
        .iter()
        .filter(|trace| left_items.contains(&trace.case))
        .count();
    let left_summary = summarize(&left_log, left);
    let right_summary = summarize(&right_log, right);
    let mut differences = activity_differences(&left_log, &right_log);
    rank_differences(&mut differences);

    Ok(CohortComparison {
        attribute: attribute.to_owned(),
        mean_throughput_difference_seconds: right_summary.mean_throughput_seconds
            - left_summary.mean_throughput_seconds,
        left: left_summary,
        right: right_summary,
        shared_items,
        activity_differences: differences,
        interpretation_limits: vec![
            "Differences are statistical associations in the active view, not evidence that the cohort attribute caused them."
                .to_owned(),
            "Cohort size and duration-eligibility denominators must be considered before interpreting a difference."
                .to_owned(),
            "Case-mix, selection, missing attributes, and other confounders may explain an observed difference."
                .to_owned(),
        ],
    })
}

/// Compares cases by their first-event timestamp in two non-overlapping,
/// chronologically ordered inclusive windows.
pub fn compare_periods(
    log: &EventLog,
    baseline_start: i64,
    baseline_end: i64,
    comparison_start: i64,
    comparison_end: i64,
) -> Result<PeriodComparison, ComparisonError> {
    if baseline_start > baseline_end {
        return Err(ComparisonError::InvalidPeriod { period: "baseline" });
    }
    if comparison_start > comparison_end {
        return Err(ComparisonError::InvalidPeriod {
            period: "comparison",
        });
    }
    if baseline_end >= comparison_start {
        return Err(ComparisonError::PeriodsOverlapOrReversed);
    }
    let baseline_log = Filter::new()
        .started_after(baseline_start)
        .started_before(baseline_end)
        .apply(log)
        .log;
    let comparison_log = Filter::new()
        .started_after(comparison_start)
        .started_before(comparison_end)
        .apply(log)
        .log;
    if baseline_log.traces.is_empty() {
        return Err(ComparisonError::EmptyPeriod { period: "baseline" });
    }
    if comparison_log.traces.is_empty() {
        return Err(ComparisonError::EmptyPeriod {
            period: "comparison",
        });
    }
    let baseline_metrics = summary_metrics(&baseline_log);
    let comparison_metrics = summary_metrics(&comparison_log);
    let mut differences = activity_differences(&baseline_log, &comparison_log);
    rank_differences(&mut differences);
    Ok(PeriodComparison {
        baseline: PeriodSummary::new(baseline_start, baseline_end, baseline_metrics),
        comparison: PeriodSummary::new(comparison_start, comparison_end, comparison_metrics),
        mean_throughput_difference_seconds: comparison_metrics.mean_throughput_seconds
            - baseline_metrics.mean_throughput_seconds,
        activity_differences: differences,
        interpretation_limits: vec![
            "Period differences are observed associations, not evidence that time or an intervention caused them.".to_owned(),
            "Seasonality, case mix, selection, missing cases, and other contemporaneous changes may explain a difference.".to_owned(),
            "Windows include cases according to their first event and include both boundary instants.".to_owned(),
        ],
    })
}

fn summarize(log: &EventLog, value: &str) -> CohortSummary {
    let metrics = summary_metrics(log);
    CohortSummary {
        value: value.to_owned(),
        cases: metrics.cases,
        events: metrics.events,
        duration_eligible_cases: metrics.duration_eligible_cases,
        mean_throughput_seconds: metrics.mean_throughput_seconds,
        median_throughput_seconds: metrics.median_throughput_seconds,
    }
}

#[derive(Clone, Copy)]
struct SummaryMetrics {
    cases: usize,
    events: usize,
    duration_eligible_cases: usize,
    mean_throughput_seconds: i64,
    median_throughput_seconds: i64,
}

impl PeriodSummary {
    fn new(start: i64, end: i64, metrics: SummaryMetrics) -> Self {
        Self {
            started_at_or_after: start,
            started_at_or_before: end,
            cases: metrics.cases,
            events: metrics.events,
            duration_eligible_cases: metrics.duration_eligible_cases,
            mean_throughput_seconds: metrics.mean_throughput_seconds,
            median_throughput_seconds: metrics.median_throughput_seconds,
        }
    }
}

fn summary_metrics(log: &EventLog) -> SummaryMetrics {
    let mut durations = log
        .traces
        .iter()
        .filter(|trace| !trace.incomplete && trace.len() >= 2)
        .map(|trace| trace.throughput())
        .collect::<Vec<_>>();
    durations.sort_unstable();
    let eligible = durations.len();
    let mean = if eligible == 0 {
        0
    } else {
        let total = durations
            .iter()
            .map(|duration| i128::from(*duration))
            .sum::<i128>();
        // An average of i64 observations remains within the i64 range.
        (total / eligible as i128) as i64
    };
    let median = if eligible == 0 {
        0
    } else if eligible % 2 == 1 {
        durations[eligible / 2]
    } else {
        let upper = durations[eligible / 2];
        let lower = durations[eligible / 2 - 1];
        lower + (upper - lower) / 2
    };
    SummaryMetrics {
        cases: log.case_count(),
        events: log.event_count(),
        duration_eligible_cases: eligible,
        mean_throughput_seconds: mean / MICROS_PER_SECOND,
        median_throughput_seconds: median / MICROS_PER_SECOND,
    }
}

fn rank_differences(differences: &mut [ActivityDifference]) {
    differences.sort_by(|a, b| {
        b.difference_points
            .abs()
            .total_cmp(&a.difference_points.abs())
            .then_with(|| a.activity.cmp(&b.activity))
    });
    for (index, difference) in differences.iter_mut().enumerate() {
        difference.rank = index + 1;
    }
}

fn activity_differences(left: &EventLog, right: &EventLog) -> Vec<ActivityDifference> {
    let left_stats = activity_stats(left);
    let right_stats = activity_stats(right);
    let activities = left_stats
        .keys()
        .chain(right_stats.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    activities
        .into_iter()
        .map(|activity| {
            let left_stat = left_stats.get(&activity).cloned().unwrap_or_default();
            let right_stat = right_stats.get(&activity).cloned().unwrap_or_default();
            let left_percent = percent(left_stat.cases.len(), left.case_count());
            let right_percent = percent(right_stat.cases.len(), right.case_count());
            ActivityDifference {
                rank: 0,
                activity,
                left_occurrences: left_stat.occurrences,
                right_occurrences: right_stat.occurrences,
                left_cases: left_stat.cases.len() as u64,
                right_cases: right_stat.cases.len() as u64,
                left_case_percent: left_percent,
                right_case_percent: right_percent,
                difference_points: right_percent - left_percent,
                left_example_cases: left_stat.examples,
                right_example_cases: right_stat.examples,
            }
        })
        .collect()
}

#[derive(Clone, Default)]
struct ActivityStats {
    occurrences: u64,
    cases: BTreeSet<String>,
    examples: Vec<String>,
}

fn activity_stats(log: &EventLog) -> BTreeMap<String, ActivityStats> {
    let mut out: BTreeMap<String, ActivityStats> = BTreeMap::new();
    for trace in &log.traces {
        let case = log.case_label(trace.case).to_owned();
        for event in &trace.events {
            let stat = out
                .entry(log.activity_label(event.activity).to_owned())
                .or_default();
            stat.occurrences += 1;
            if stat.cases.insert(case.clone()) && stat.examples.len() < 3 {
                stat.examples.push(case.clone());
            }
        }
    }
    out
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64 * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::{ActivityId, CaseId, CaseTrace, Dictionary, Event};

    fn fixture() -> EventLog {
        let mut activities = Dictionary::new();
        for label in ["Start", "Review", "Escalate", "End"] {
            activities.intern(label);
        }
        let mut cases = Dictionary::new();
        for label in ["E1", "E2", "W1", "W2"] {
            cases.intern(label);
        }
        let mut attribute_values = Dictionary::new();
        for label in ["East", "West"] {
            attribute_values.intern(label);
        }
        let trace = |case, region, activities: &[u32], hours: &[i64]| CaseTrace {
            case: CaseId(case),
            events: activities
                .iter()
                .zip(hours)
                .enumerate()
                .map(|(row, (activity, hour))| {
                    Event::new(ActivityId(*activity), hour * 3_600_000_000, row as u32 + 2)
                })
                .collect(),
            incomplete: false,
            attributes: vec![region],
        };
        EventLog {
            activities,
            cases,
            attribute_names: vec!["region".to_owned()],
            attribute_values,
            step_values: Default::default(),
            traces: vec![
                trace(0, 0, &[0, 1, 3], &[0, 1, 2]),
                trace(1, 0, &[0, 1, 3], &[0, 1, 3]),
                trace(2, 1, &[0, 2, 3], &[24, 26, 30]),
                trace(3, 1, &[0, 2, 3], &[48, 50, 56]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn compares_cohorts_and_ranks_largest_activity_differences() {
        let result = compare_cohorts(&fixture(), "region", "East", "West").unwrap();
        assert_eq!(result.left.cases, 2);
        assert_eq!(result.right.mean_throughput_seconds, 7 * 3600);
        assert_eq!(result.mean_throughput_difference_seconds, 4 * 3600 + 1800);
        assert_eq!(result.activity_differences[0].rank, 1);
        assert_eq!(
            result.activity_differences[0].difference_points.abs(),
            100.0
        );
        assert!(result.interpretation_limits[0].contains("not evidence"));
    }

    #[test]
    fn rejects_unknown_and_identical_cohorts() {
        let log = fixture();
        assert_eq!(
            compare_cohorts(&log, "missing", "East", "West"),
            Err(ComparisonError::UnknownAttribute("missing".to_owned()))
        );
        assert_eq!(
            compare_cohorts(&log, "region", "East", "East"),
            Err(ComparisonError::SameValue)
        );
    }

    #[test]
    fn compares_non_overlapping_periods_by_case_start() {
        let hour = 3_600_000_000;
        let result = compare_periods(&fixture(), 0, 10 * hour, 20 * hour, 60 * hour).unwrap();
        assert_eq!(result.baseline.cases, 2);
        assert_eq!(result.comparison.cases, 2);
        assert_eq!(result.comparison.mean_throughput_seconds, 7 * 3600);
        assert_eq!(result.mean_throughput_difference_seconds, 4 * 3600 + 1800);
        assert_eq!(
            result.activity_differences[0].difference_points.abs(),
            100.0
        );
        assert!(result.interpretation_limits[0].contains("not evidence"));
    }

    #[test]
    fn rejects_invalid_overlapping_and_empty_periods() {
        let hour = 3_600_000_000;
        let log = fixture();
        assert_eq!(
            compare_periods(&log, 10, 0, 20, 30),
            Err(ComparisonError::InvalidPeriod { period: "baseline" })
        );
        assert_eq!(
            compare_periods(&log, 0, 20 * hour, 20 * hour, 60 * hour),
            Err(ComparisonError::PeriodsOverlapOrReversed)
        );
        assert_eq!(
            compare_periods(&log, 5 * hour, 10 * hour, 20 * hour, 60 * hour),
            Err(ComparisonError::EmptyPeriod { period: "baseline" })
        );
    }

    #[test]
    fn is_deterministic() {
        let log = fixture();
        assert_eq!(
            compare_cohorts(&log, "region", "East", "West").unwrap(),
            compare_cohorts(&log, "region", "East", "West").unwrap()
        );
    }
}
