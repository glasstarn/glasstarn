//! Rework and observed loop detection.
//!
//! A cycle in a directly-follows graph is not evidence that any case looped:
//! parallel activities can create both `A → B` and `B → A` across different
//! cases. This module therefore works from each complete ordered trace.

use crate::dfg::EVIDENCE_EXAMPLES;
use crate::dict::{ActivityId, CaseId};
use crate::log::{EventLog, Micros};
use std::collections::BTreeMap;

/// Bounds path materialization under deeply nested or adversarial traces.
const MAX_LOOP_EVENTS: usize = 64;
/// Keeps detailed output and working memory independent of event volume.
const MAX_LOOP_PATTERNS: usize = 10_000;

/// Repetition statistics for one activity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityRework {
    /// Occurrences after the first occurrence in a case.
    pub repetitions: u64,
    /// Repetitions immediately following the same activity.
    pub immediate_repetitions: u64,
    /// Repetitions reached after one or more intervening activities.
    pub returns_after_intervening: u64,
    /// Cases in which the activity repeated.
    pub case_frequency: u64,
    /// Repetitions eligible for interval measurement.
    pub duration_frequency: u64,
    /// Time from the previous occurrence's start to this occurrence's start.
    pub total_repeat_interval: Micros,
    /// A bounded set of evidencing cases.
    pub examples: Vec<CaseId>,
}

impl ActivityRework {
    pub fn mean_repeat_interval(&self) -> Micros {
        if self.duration_frequency == 0 {
            0
        } else {
            self.total_repeat_interval / self.duration_frequency as Micros
        }
    }
}

/// Statistics for one loop path, including its repeated anchor at both ends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopStats {
    pub occurrences: u64,
    pub case_frequency: u64,
    pub duration_frequency: u64,
    pub total_duration: Micros,
    pub examples: Vec<CaseId>,
}

impl LoopStats {
    pub fn mean_duration(&self) -> Micros {
        if self.duration_frequency == 0 {
            0
        } else {
            self.total_duration / self.duration_frequency as Micros
        }
    }
}

/// Rework observed in the current event-log view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rework {
    pub cases_with_rework: u64,
    pub activities: BTreeMap<ActivityId, ActivityRework>,
    pub loops: BTreeMap<Vec<ActivityId>, LoopStats>,
    /// Candidates omitted from path detail by the safety limits. Activity-level
    /// repetition counts remain complete.
    pub omitted_loop_candidates: u64,
}

impl Rework {
    pub fn build(log: &EventLog) -> Self {
        let mut result = Self::default();
        let activity_count = log.activity_count();
        let mut last_seen_generation = vec![0_usize; activity_count];
        let mut last_position = vec![0_usize; activity_count];
        let mut anchor_generation = vec![0_usize; activity_count];
        let mut anchor_position = vec![0_usize; activity_count];
        let mut rework_case_generation = vec![0_usize; activity_count];
        let mut loop_last_case: BTreeMap<Vec<ActivityId>, CaseId> = BTreeMap::new();

        for (case_index, trace) in log.traces.iter().enumerate() {
            let generation = case_index + 1;
            let mut case_has_rework = false;

            for (index, event) in trace.events.iter().enumerate() {
                let activity_index = event.activity.0 as usize;
                let previous = (last_seen_generation[activity_index] == generation)
                    .then_some(last_position[activity_index]);
                last_seen_generation[activity_index] = generation;
                last_position[activity_index] = index;
                if let Some(previous) = previous {
                    case_has_rework = true;
                    let stats = result.activities.entry(event.activity).or_default();
                    stats.repetitions += 1;
                    if previous + 1 == index {
                        stats.immediate_repetitions += 1;
                    } else {
                        stats.returns_after_intervening += 1;
                    }
                    if !trace.incomplete {
                        stats.duration_frequency += 1;
                        stats.total_repeat_interval +=
                            (event.at - trace.events[previous].at).max(0);
                    }
                    if stats.examples.len() < EVIDENCE_EXAMPLES
                        && !stats.examples.contains(&trace.case)
                    {
                        stats.examples.push(trace.case);
                    }
                    if rework_case_generation[activity_index] != generation {
                        stats.case_frequency += 1;
                        rework_case_generation[activity_index] = generation;
                    }
                }

                let previous_anchor = (anchor_generation[activity_index] == generation)
                    .then_some(anchor_position[activity_index]);
                if let Some(previous) = previous_anchor {
                    let path_len = index - previous + 1;
                    if path_len > MAX_LOOP_EVENTS {
                        result.omitted_loop_candidates += 1;
                        anchor_generation[activity_index] = generation;
                        anchor_position[activity_index] = index;
                        continue;
                    }
                    let sequence: Vec<ActivityId> = trace.events[previous..=index]
                        .iter()
                        .map(|event| event.activity)
                        .collect();
                    let can_record = result.loops.contains_key(&sequence)
                        || result.loops.len() < MAX_LOOP_PATTERNS;
                    if can_record {
                        let stats = result.loops.entry(sequence.clone()).or_default();
                        stats.occurrences += 1;
                        if !trace.incomplete {
                            stats.duration_frequency += 1;
                            stats.total_duration += (event.at - trace.events[previous].at).max(0);
                        }
                        if stats.examples.len() < EVIDENCE_EXAMPLES
                            && !stats.examples.contains(&trace.case)
                        {
                            stats.examples.push(trace.case);
                        }
                        if loop_last_case.get(&sequence) != Some(&trace.case) {
                            stats.case_frequency += 1;
                            loop_last_case.insert(sequence, trace.case);
                        }
                    } else {
                        result.omitted_loop_candidates += 1;
                    }

                    // Interior activities must be seen again before they can
                    // anchor another loop. This prevents one traversal from
                    // being counted again as a rotated version of itself.
                    for interior in &trace.events[previous + 1..index] {
                        anchor_generation[interior.activity.0 as usize] = 0;
                    }
                }
                anchor_generation[activity_index] = generation;
                anchor_position[activity_index] = index;
            }

            if case_has_rework {
                result.cases_with_rework += 1;
            }
        }
        result
    }

    pub fn repeated_activity_occurrences(&self) -> u64 {
        self.activities
            .values()
            .map(|stats| stats.repetitions)
            .sum()
    }

    pub fn loop_traversals(&self) -> u64 {
        self.loops.values().map(|stats| stats.occurrences).sum()
    }

    pub fn immediate_repetitions(&self) -> u64 {
        self.activities
            .values()
            .map(|stats| stats.immediate_repetitions)
            .sum()
    }

    pub fn returns_after_intervening(&self) -> u64 {
        self.activities
            .values()
            .map(|stats| stats.returns_after_intervening)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::Dictionary;
    use crate::log::{CaseTrace, Event};

    fn log(spec: &[(&str, &[(&str, i64)])]) -> EventLog {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let traces = spec
            .iter()
            .enumerate()
            .map(|(case_index, (case, events))| CaseTrace {
                case: CaseId(cases.intern(case)),
                events: events
                    .iter()
                    .enumerate()
                    .map(|(event_index, (activity, time))| {
                        Event::new(
                            ActivityId(activities.intern(activity)),
                            *time,
                            (case_index * 10 + event_index + 1) as u32,
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
            ..Default::default()
        }
    }

    #[test]
    fn counts_immediate_repetition_once_per_extra_occurrence() {
        let log = log(&[("C1", &[("A", 0), ("B", 1), ("B", 2), ("B", 3)])]);
        let rework = Rework::build(&log);
        let b = ActivityId(log.activities.get("B").unwrap());
        assert_eq!(rework.activities[&b].repetitions, 2);
        assert_eq!(rework.immediate_repetitions(), 2);
        assert_eq!(rework.loop_traversals(), 2);
    }

    #[test]
    fn repeated_long_loop_is_not_double_counted_by_rotation() {
        let log = log(&[(
            "C1",
            &[
                ("Review", 0),
                ("Documents", 1),
                ("Review", 2),
                ("Documents", 3),
                ("Review", 4),
            ],
        )]);
        let rework = Rework::build(&log);
        assert_eq!(rework.loop_traversals(), 2);
        assert_eq!(rework.loops.len(), 1);
        assert_eq!(rework.loops.values().next().unwrap().occurrences, 2);
    }

    #[test]
    fn aggregate_graph_cycle_without_trace_repetition_is_not_a_loop() {
        let log = log(&[("C1", &[("A", 0), ("B", 1)]), ("C2", &[("B", 0), ("A", 1)])]);
        let rework = Rework::build(&log);
        assert_eq!(rework.cases_with_rework, 0);
        assert!(rework.activities.is_empty());
        assert!(rework.loops.is_empty());
    }

    #[test]
    fn oversized_loop_paths_are_omitted_without_losing_rework_counts() {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let case = CaseId(cases.intern("C1"));
        let anchor = ActivityId(activities.intern("Anchor"));
        let mut events = vec![Event::new(anchor, 0, 1)];
        for index in 0..MAX_LOOP_EVENTS {
            let activity = ActivityId(activities.intern(&format!("Step {index}")));
            events.push(Event::new(activity, index as i64 + 1, index as u32 + 2));
        }
        events.push(Event::new(anchor, 100, 100));
        let log = EventLog {
            cases,
            activities,
            traces: vec![CaseTrace {
                case,
                events,
                incomplete: false,
                attributes: Vec::new(),
            }],
            ..Default::default()
        };

        let rework = Rework::build(&log);
        assert_eq!(rework.activities[&anchor].repetitions, 1);
        assert!(rework.loops.is_empty());
        assert_eq!(rework.omitted_loop_candidates, 1);
    }
}
