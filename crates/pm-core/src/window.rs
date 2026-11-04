//! The period an extract was taken for, and where each case sits against it.
//!
//! An extract selects the items with activity inside a window and keeps their
//! full history, so some cases began before the window and some had not
//! finished by its end. Dropping them silently biases every duration: the slow
//! items still open at the end disappear and the process looks faster than it
//! is. So nothing is dropped. Each case is labelled instead, and keeping only
//! the cases that ran their whole course inside the window is an explicit
//! filter a person chooses.
//!
//! "Still open" needs to know which steps finish an item. Until someone
//! confirms those steps, openness is reported as not determined rather than
//! guessed from the data.

use crate::log::{CaseTrace, EventLog, Micros};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The observed period, with the steps that finish an item when confirmed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationWindow {
    /// Inclusive start, in microseconds since the Unix epoch.
    pub start_micros: Micros,
    /// Exclusive end, in microseconds since the Unix epoch.
    pub end_micros: Micros,
    /// Steps that finish an item. Empty means not yet confirmed.
    #[serde(default)]
    pub end_activities: BTreeSet<String>,
}

/// Where one case sits against the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaseWindow {
    /// The first recorded step is before the window starts.
    pub started_before: bool,
    /// The last recorded step is not a confirmed finishing step. `None` when
    /// no finishing steps are confirmed.
    pub still_open: Option<bool>,
}

impl CaseWindow {
    /// Began inside the window and, when finishing steps are confirmed,
    /// reached one of them.
    pub fn complete(&self) -> bool {
        !self.started_before && self.still_open != Some(true)
    }
}

/// Case counts against the window, for the view they were computed on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSummary {
    pub start_micros: Micros,
    pub end_micros: Micros,
    pub end_activities: Vec<String>,
    /// Whether finishing steps are confirmed, so `still_open` is measured.
    pub end_defined: bool,
    pub cases: usize,
    /// Began and, when measurable, finished inside the window.
    pub complete: usize,
    pub started_before: usize,
    /// Zero when `end_defined` is false.
    pub still_open: usize,
}

impl ObservationWindow {
    /// Labels one case of `log`.
    pub fn label(&self, log: &EventLog, trace: &CaseTrace) -> CaseWindow {
        let started_before = trace
            .events
            .first()
            .is_some_and(|first| first.at < self.start_micros);
        let still_open = (!self.end_activities.is_empty()).then(|| {
            trace.events.last().is_none_or(|last| {
                !self
                    .end_activities
                    .contains(log.activity_label(last.activity))
            })
        });
        CaseWindow {
            started_before,
            still_open,
        }
    }

    /// Counts every case of `log` against the window.
    pub fn summarize(&self, log: &EventLog) -> WindowSummary {
        let mut summary = WindowSummary {
            start_micros: self.start_micros,
            end_micros: self.end_micros,
            end_activities: self.end_activities.iter().cloned().collect(),
            end_defined: !self.end_activities.is_empty(),
            cases: log.case_count(),
            ..WindowSummary::default()
        };
        for trace in &log.traces {
            let label = self.label(log, trace);
            summary.started_before += usize::from(label.started_before);
            summary.still_open += usize::from(label.still_open == Some(true));
            summary.complete += usize::from(label.complete());
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::{ActivityId, CaseId, Dictionary};
    use crate::log::Event;

    fn log(cases: &[(&str, &[(&str, Micros)])]) -> EventLog {
        let mut labels = Dictionary::default();
        let mut activities = Dictionary::default();
        let traces = cases
            .iter()
            .map(|(case, steps)| CaseTrace {
                case: CaseId(labels.intern(case)),
                events: steps
                    .iter()
                    .enumerate()
                    .map(|(row, (step, at))| {
                        Event::new(ActivityId(activities.intern(step)), *at, row as u32)
                    })
                    .collect(),
                incomplete: false,
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

    fn cases() -> EventLog {
        log(&[
            ("inside", &[("Open", 10), ("Close", 20)]),
            ("earlier", &[("Open", 5), ("Close", 20)]),
            ("open", &[("Open", 10), ("Review", 20)]),
        ])
    }

    #[test]
    fn openness_is_not_guessed_without_confirmed_finishing_steps() {
        let window = ObservationWindow {
            start_micros: 10,
            end_micros: 30,
            end_activities: BTreeSet::new(),
        };
        let summary = window.summarize(&cases());
        assert!(!summary.end_defined);
        assert_eq!(
            (summary.complete, summary.started_before, summary.still_open),
            (2, 1, 0)
        );
    }

    #[test]
    fn confirmed_finishing_steps_mark_open_cases() {
        let window = ObservationWindow {
            start_micros: 10,
            end_micros: 30,
            end_activities: BTreeSet::from(["Close".to_owned()]),
        };
        let log = cases();
        let summary = window.summarize(&log);
        assert_eq!(
            (summary.complete, summary.started_before, summary.still_open),
            (1, 1, 1)
        );
        assert!(window.label(&log, &log.traces[0]).complete());
        assert!(!window.label(&log, &log.traces[2]).complete());
    }
}
