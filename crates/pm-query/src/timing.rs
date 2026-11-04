//! How long cases take: the completion-time distribution and the slowest
//! cases, for the timing and individual-item views.
//!
//! Durations are first-to-last recorded event, so they are elapsed time.
//! Cases marked incomplete are left out, as everywhere else durations appear.

use pm_core::log::{EventLog, Micros};
use pm_core::variants::Variants;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

/// Slowest cases listed with an analysis.
pub const SLOWEST_LISTED: usize = 50;

/// Most bins drawn before the open-ended last bin.
const MAX_BINS: usize = 12;

/// Bin widths a reader can take in at a glance, in seconds.
const NICE_WIDTHS: [i64; 17] = [
    60, 300, 600, 1_800, 3_600, 7_200, 14_400, 21_600, 43_200, 86_400, 172_800, 259_200, 604_800,
    1_209_600, 2_592_000, 7_776_000, 31_536_000,
];

const MICROS_PER_SECOND: Micros = 1_000_000;

/// One histogram bar: cases whose completion time falls in `[from, to)`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurationBin {
    pub from_seconds: i64,
    /// `None` for the last bar, which holds every longer case.
    pub to_seconds: Option<i64>,
    pub cases: usize,
}

/// One of the slowest cases.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlowCase {
    pub case: String,
    pub started_at_micros: i64,
    pub duration_seconds: i64,
    pub events: usize,
    /// One-based rank of the route this case followed.
    pub route_rank: usize,
    /// The case records some step more than once.
    pub repeats_step: bool,
}

/// Completion times for the active view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingSummary {
    pub duration_eligible_cases: usize,
    pub incomplete_cases: usize,
    pub median_seconds: i64,
    pub p90_seconds: i64,
    pub bin_width_seconds: i64,
    pub bins: Vec<DurationBin>,
    /// Longest first, up to [`SLOWEST_LISTED`]; ties by case label.
    pub slowest_cases: Vec<SlowCase>,
}

/// Builds the timing summary of a log whose variants are already known.
pub fn summarize_timing(log: &EventLog, variants: &Variants) -> TimingSummary {
    let mut eligible: Vec<_> = log.traces.iter().filter(|t| !t.incomplete).collect();
    let mut seconds: Vec<i64> = eligible
        .iter()
        .map(|t| t.throughput() / MICROS_PER_SECOND)
        .collect();
    seconds.sort_unstable();
    let rank = |share_tenths: usize| {
        if seconds.is_empty() {
            0
        } else {
            seconds[(seconds.len() * share_tenths).div_ceil(10).max(1) - 1]
        }
    };
    let median = if seconds.is_empty() {
        0
    } else {
        seconds[seconds.len() / 2]
    };
    let p90 = rank(9);
    let p99 = if seconds.is_empty() {
        0
    } else {
        seconds[(seconds.len() * 99).div_ceil(100).max(1) - 1]
    };

    let width = NICE_WIDTHS
        .iter()
        .copied()
        .find(|width| (p99 / width + 1) as usize <= MAX_BINS)
        .unwrap_or(*NICE_WIDTHS.last().expect("widths"));
    let drawn = ((p99 / width + 1) as usize).min(MAX_BINS);
    let mut bins: Vec<DurationBin> = (0..drawn)
        .map(|index| DurationBin {
            from_seconds: index as i64 * width,
            to_seconds: Some((index as i64 + 1) * width),
            cases: 0,
        })
        .collect();
    let limit = drawn as i64 * width;
    if seconds.iter().any(|value| *value >= limit) {
        bins.push(DurationBin {
            from_seconds: limit,
            to_seconds: None,
            cases: 0,
        });
    }
    for value in &seconds {
        let index = ((value / width) as usize).min(bins.len().saturating_sub(1));
        if let Some(bin) = bins.get_mut(index) {
            bin.cases += 1;
        }
    }

    let routes: HashMap<_, usize> = variants
        .variants
        .iter()
        .enumerate()
        .flat_map(|(index, variant)| variant.cases.iter().map(move |case| (*case, index + 1)))
        .collect();
    eligible.sort_by(|left, right| {
        right
            .throughput()
            .cmp(&left.throughput())
            .then_with(|| log.case_label(left.case).cmp(log.case_label(right.case)))
    });
    let slowest_cases = eligible
        .iter()
        .take(SLOWEST_LISTED)
        .map(|trace| {
            let distinct: BTreeSet<_> = trace.events.iter().map(|e| e.activity).collect();
            SlowCase {
                case: log.case_label(trace.case).to_owned(),
                started_at_micros: trace.start(),
                duration_seconds: trace.throughput() / MICROS_PER_SECOND,
                events: trace.len(),
                route_rank: routes.get(&trace.case).copied().unwrap_or(0),
                repeats_step: distinct.len() < trace.len(),
            }
        })
        .collect();

    TimingSummary {
        duration_eligible_cases: seconds.len(),
        incomplete_cases: log.traces.len() - seconds.len(),
        median_seconds: median,
        p90_seconds: p90,
        bin_width_seconds: width,
        bins,
        slowest_cases,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::dict::{ActivityId, CaseId, Dictionary};
    use pm_core::log::{CaseTrace, Event};

    const DAY: i64 = 86_400 * 1_000_000;

    fn log(cases: &[(&str, &[&str], i64, bool)]) -> EventLog {
        let mut labels = Dictionary::default();
        let mut activities = Dictionary::default();
        let traces = cases
            .iter()
            .enumerate()
            .map(|(row, (case, steps, days, incomplete))| CaseTrace {
                case: CaseId(labels.intern(case)),
                events: steps
                    .iter()
                    .enumerate()
                    .map(|(index, step)| {
                        let at = if index + 1 == steps.len() {
                            days * DAY
                        } else {
                            index as i64
                        };
                        Event::new(ActivityId(activities.intern(step)), at, row as u32)
                    })
                    .collect(),
                incomplete: *incomplete,
                attributes: Vec::new(),
            })
            .collect();
        EventLog {
            cases: labels,
            activities,
            step_values: Default::default(),
            traces,
            ..EventLog::default()
        }
    }

    #[test]
    fn bins_completion_times_and_lists_the_slowest_first() {
        let log = log(&[
            ("A", &["Start", "End"], 1, false),
            ("B", &["Start", "End"], 2, false),
            ("C", &["Start", "Check", "Start", "End"], 9, false),
            ("D", &["Start", "End"], 3, false),
            ("E", &["Start", "End"], 30, true),
        ]);
        let timing = summarize_timing(&log, &Variants::build(&log));
        assert_eq!(
            (timing.duration_eligible_cases, timing.incomplete_cases),
            (4, 1)
        );
        assert_eq!(timing.median_seconds, 3 * 86_400);
        assert_eq!(timing.p90_seconds, 9 * 86_400);
        // Nine days fit twelve one-day bars; every eligible case is counted once.
        assert_eq!(timing.bin_width_seconds, 86_400);
        assert_eq!(timing.bins.iter().map(|b| b.cases).sum::<usize>(), 4);
        assert_eq!(timing.bins[9].cases, 1);
        let slowest: Vec<_> = timing
            .slowest_cases
            .iter()
            .map(|c| c.case.as_str())
            .collect();
        assert_eq!(slowest, ["C", "D", "B", "A"]);
        assert!(timing.slowest_cases[0].repeats_step);
        assert!(!timing.slowest_cases[1].repeats_step);
        assert_eq!(timing.slowest_cases[1].route_rank, 1);
    }

    #[test]
    fn long_tails_go_to_an_open_last_bar() {
        let mut cases: Vec<(String, i64)> = (0..200).map(|i| (format!("c{i}"), 1)).collect();
        cases.push(("slow".into(), 400));
        let owned: Vec<_> = cases
            .iter()
            .map(|(case, days)| (case.as_str(), &["Start", "End"][..], *days, false))
            .collect();
        let log = log(&owned);
        let timing = summarize_timing(&log, &Variants::build(&log));
        let last = timing.bins.last().unwrap();
        assert_eq!(last.to_seconds, None);
        assert_eq!(last.cases, 1);
        assert!(timing.bins.len() <= MAX_BINS + 1);
    }
}
