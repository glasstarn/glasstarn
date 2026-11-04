//! Directly-follows graph.
//!
//! Nodes are activities, edges are observed consecutive pairs within a case,
//! with frequency and duration statistics attached.
//!
//! # What a DFG is not
//!
//! It is **not a process model**. It has no execution semantics, and it
//! aggregates away the context in which transitions happened. A DFG built from
//! genuinely concurrent activities contains a cycle between them, implying a
//! case could loop indefinitely — even when no such case exists. See
//! `testdata/canonical/04-parallel.csv`, which exists to keep this honest.
//!
//! This is why variant coverage must be displayed alongside any DFG, and why
//! the artifact is labelled a directly-follows graph rather than "the
//! process".

use crate::dict::{ActivityId, CaseId};
use crate::log::{EventLog, Micros};
use std::collections::BTreeMap;

/// How many example cases to retain per node and edge.
///
/// Every reported number must be traceable to the cases that produced it. A
/// handful is enough for a person to open and check, and bounded storage keeps
/// the cost independent of log size.
pub const EVIDENCE_EXAMPLES: usize = 5;

/// Statistics for one activity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    /// Times this activity occurred, across all cases.
    pub frequency: u64,
    /// Cases in which it occurred at least once.
    pub case_frequency: u64,
    /// Times it began a case.
    pub starts: u64,
    /// Times it ended a case.
    pub ends: u64,
    /// A few cases in which it occurred, so the figure can be checked.
    pub examples: Vec<CaseId>,
}

/// Statistics for one directly-follows transition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edge {
    /// Times this transition was observed.
    pub frequency: u64,
    /// Observations eligible for duration statistics. Incomplete cases still
    /// contribute to the process shape but never to time-based findings.
    pub duration_frequency: u64,
    /// Cases in which it occurred at least once.
    pub case_frequency: u64,
    /// Total elapsed time across all observations.
    ///
    /// Measured from the **end** of the earlier activity to the start of the
    /// later one when an end timestamp is mapped — true waiting time. With
    /// only a single timestamp per event this is start-to-start elapsed time,
    /// which *includes* however long the earlier activity took. See
    /// [`Dfg::measures_true_waiting`].
    pub total_duration: Micros,
    /// Shortest observed elapsed time.
    pub min_duration: Micros,
    /// Longest observed elapsed time.
    pub max_duration: Micros,
    /// Observations whose two steps carry the same recorded time, so their
    /// order comes from the file rather than the clock. Day-only timestamps
    /// make every same-day pair one of these.
    pub same_time: u64,
    /// A few cases in which it occurred, so the figure can be checked.
    pub examples: Vec<CaseId>,
}

impl Edge {
    /// Mean elapsed time, or zero if never observed.
    ///
    /// Integer division: exact, reproducible, and the sub-microsecond
    /// remainder is meaningless for process durations.
    pub fn mean_duration(&self) -> Micros {
        if self.duration_frequency == 0 {
            0
        } else {
            self.total_duration / self.duration_frequency as i64
        }
    }
}

/// A directly-follows graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dfg {
    /// Activity statistics, keyed by activity.
    pub nodes: BTreeMap<ActivityId, Node>,
    /// Transition statistics, keyed by (from, to).
    pub edges: BTreeMap<(ActivityId, ActivityId), Edge>,
    /// Cases the graph was built from.
    pub case_count: u64,
    /// Events the graph was built from.
    pub event_count: u64,
    /// Whether end timestamps were available, so durations exclude processing
    /// time and are genuinely waiting time.
    pub measures_true_waiting: bool,
}

impl Dfg {
    /// Builds a directly-follows graph from a normalized log.
    pub fn build(log: &EventLog) -> Dfg {
        let mut dfg = Dfg {
            case_count: log.case_count() as u64,
            event_count: log.event_count() as u64,
            ..Default::default()
        };
        let mut saw_duration = false;
        let mut every_duration_has_end = true;

        for trace in &log.traces {
            // Per-case sets so case_frequency counts cases, not occurrences.
            let mut activities_seen: std::collections::BTreeSet<ActivityId> = Default::default();
            let mut edges_seen: std::collections::BTreeSet<(ActivityId, ActivityId)> =
                Default::default();

            for event in &trace.events {
                let node = dfg.nodes.entry(event.activity).or_default();
                node.frequency += 1;
                if node.examples.len() < EVIDENCE_EXAMPLES && !node.examples.contains(&trace.case) {
                    node.examples.push(trace.case);
                }
                activities_seen.insert(event.activity);
            }

            if let Some(first) = trace.events.first() {
                dfg.nodes.entry(first.activity).or_default().starts += 1;
            }
            if let Some(last) = trace.events.last() {
                dfg.nodes.entry(last.activity).or_default().ends += 1;
            }

            for pair in trace.events.windows(2) {
                let (from, to) = (pair[0], pair[1]);
                // With an end timestamp, the gap starts when the earlier
                // activity finished — that is waiting. Without one, all we can
                // honestly measure is start-to-start elapsed time, which
                // includes the earlier activity's own duration.
                let edge = dfg
                    .edges
                    .entry((from.activity, to.activity))
                    .or_insert(Edge {
                        min_duration: Micros::MAX,
                        max_duration: Micros::MIN,
                        ..Default::default()
                    });
                edge.frequency += 1;
                if to.at == from.at {
                    edge.same_time += 1;
                }
                if !trace.incomplete {
                    saw_duration = true;
                    every_duration_has_end &= from.end().is_some();
                    // Overlapping activities have zero waiting, not negative
                    // waiting. Their overlap can be reported separately later.
                    let duration = (to.at - from.end().unwrap_or(from.at)).max(0);
                    edge.duration_frequency += 1;
                    edge.total_duration += duration;
                    edge.min_duration = edge.min_duration.min(duration);
                    edge.max_duration = edge.max_duration.max(duration);
                }
                if edge.examples.len() < EVIDENCE_EXAMPLES && !edge.examples.contains(&trace.case) {
                    edge.examples.push(trace.case);
                }
                edges_seen.insert((from.activity, to.activity));
            }

            for activity in activities_seen {
                dfg.nodes.entry(activity).or_default().case_frequency += 1;
            }
            for edge in edges_seen {
                dfg.edges.entry(edge).or_default().case_frequency += 1;
            }
        }

        dfg.measures_true_waiting = saw_duration && every_duration_has_end;
        dfg
    }

    /// Activities that ever begin a case.
    pub fn start_activities(&self) -> Vec<(ActivityId, u64)> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.starts > 0)
            .map(|(a, n)| (*a, n.starts))
            .collect()
    }

    /// Activities that ever end a case.
    pub fn end_activities(&self) -> Vec<(ActivityId, u64)> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.ends > 0)
            .map(|(a, n)| (*a, n.ends))
            .collect()
    }

    /// Case labels evidencing an activity.
    pub fn example_cases_for_activity(
        &self,
        log: &EventLog,
        activity: ActivityId,
        limit: usize,
    ) -> Vec<String> {
        self.nodes
            .get(&activity)
            .map(|n| {
                n.examples
                    .iter()
                    .take(limit)
                    .map(|c| log.case_label(*c).to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Case labels evidencing a transition.
    pub fn example_cases(
        &self,
        log: &EventLog,
        edge: (ActivityId, ActivityId),
        limit: usize,
    ) -> Vec<String> {
        self.edges
            .get(&edge)
            .map(|e| {
                e.examples
                    .iter()
                    .take(limit)
                    .map(|c| log.case_label(*c).to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Transitions ranked by total elapsed time, longest first.
    ///
    /// Total rather than mean, deliberately: a two-day gap on every one of ten
    /// thousand cases matters more than a one-week gap on a single case, and
    /// ranking by mean would report the opposite.
    pub fn bottlenecks(&self) -> Vec<((ActivityId, ActivityId), Edge)> {
        let mut ranked: Vec<_> = self
            .edges
            .iter()
            .filter(|(_, v)| v.duration_frequency > 0)
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        // Sort by total duration descending, then by the edge itself so ties
        // are broken deterministically rather than by map order.
        ranked.sort_by(|a, b| {
            b.1.total_duration
                .cmp(&a.1.total_duration)
                .then(a.0.cmp(&b.0))
        });
        ranked
    }

    /// Transitions where an activity immediately repeats.
    pub fn self_loops(&self) -> Vec<(ActivityId, Edge)> {
        self.edges
            .iter()
            .filter(|((from, to), _)| from == to)
            .map(|((from, _), e)| (*from, e.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::{CaseId, Dictionary};
    use crate::log::{CaseTrace, Event};

    const HOUR: Micros = 3_600_000_000;

    /// Builds a log from `(case, [(activity, hours_from_zero)])`.
    fn log(spec: &[(&str, &[(&str, i64)])]) -> EventLog {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let mut traces = Vec::new();
        let mut row = 2u32;
        for (case, events) in spec {
            let case_id = CaseId(cases.intern(case));
            let mut list = Vec::new();
            for (activity, hour) in *events {
                list.push(Event::new(
                    ActivityId(activities.intern(activity)),
                    hour * HOUR,
                    row,
                ));
                row += 1;
            }
            traces.push(CaseTrace {
                case: case_id,
                events: list,
                incomplete: false,
                attributes: Vec::new(),
            });
        }
        EventLog {
            cases,
            activities,
            resources: Dictionary::new(),
            attribute_names: Vec::new(),
            attribute_values: Dictionary::new(),
            costs: BTreeMap::new(),
            step_values: Default::default(),
            traces,
        }
    }

    fn id(log: &EventLog, label: &str) -> ActivityId {
        ActivityId(log.activities.get(label).expect("activity exists"))
    }

    /// Mirrors `testdata/canonical/01-sequential.csv`: three cases through one
    /// four-step path. Expected values are hand-derived.
    fn sequential() -> EventLog {
        log(&[
            (
                "O1",
                &[("Receive", 0), ("Check", 2), ("Approve", 24), ("Ship", 30)],
            ),
            (
                "O2",
                &[("Receive", 1), ("Check", 5), ("Approve", 48), ("Ship", 56)],
            ),
            (
                "O3",
                &[
                    ("Receive", 23),
                    ("Check", 24),
                    ("Approve", 31),
                    ("Ship", 49),
                ],
            ),
        ])
    }

    #[test]
    fn counts_nodes_and_edges() {
        let l = sequential();
        let d = Dfg::build(&l);
        assert_eq!(d.nodes.len(), 4);
        assert_eq!(d.edges.len(), 3);
        assert_eq!(d.case_count, 3);
        assert_eq!(d.event_count, 12);
        for edge in d.edges.values() {
            assert_eq!(edge.frequency, 3);
        }
    }

    /// The invariant that catches most counting bugs: for an activity that
    /// never ends a case, outgoing edges must account for every occurrence.
    #[test]
    fn outgoing_edges_account_for_every_occurrence() {
        let l = sequential();
        let d = Dfg::build(&l);
        for (activity, node) in &d.nodes {
            let outgoing: u64 = d
                .edges
                .iter()
                .filter(|((from, _), _)| from == activity)
                .map(|(_, e)| e.frequency)
                .sum();
            assert_eq!(
                outgoing + node.ends,
                node.frequency,
                "activity {activity} does not balance"
            );
        }
    }

    #[test]
    fn incoming_edges_account_for_every_occurrence() {
        let l = sequential();
        let d = Dfg::build(&l);
        for (activity, node) in &d.nodes {
            let incoming: u64 = d
                .edges
                .iter()
                .filter(|((_, to), _)| to == activity)
                .map(|(_, e)| e.frequency)
                .sum();
            assert_eq!(incoming + node.starts, node.frequency);
        }
    }

    #[test]
    fn identifies_start_and_end_activities() {
        let l = sequential();
        let d = Dfg::build(&l);
        assert_eq!(d.start_activities(), vec![(id(&l, "Receive"), 3)]);
        assert_eq!(d.end_activities(), vec![(id(&l, "Ship"), 3)]);
    }

    /// Hand-derived from the canonical sequential log: gaps before Approve are
    /// 22, 43, and 7 hours, so mean is 24.
    #[test]
    fn computes_edge_durations() {
        let l = sequential();
        let d = Dfg::build(&l);
        let e = &d.edges[&(id(&l, "Check"), id(&l, "Approve"))];
        assert_eq!(e.total_duration, 72 * HOUR);
        assert_eq!(e.mean_duration(), 24 * HOUR);
        assert_eq!(e.min_duration, 7 * HOUR);
        assert_eq!(e.max_duration, 43 * HOUR);
    }

    /// And that edge must therefore be the top bottleneck.
    #[test]
    fn ranks_the_correct_bottleneck() {
        let l = sequential();
        let d = Dfg::build(&l);
        let ranked = d.bottlenecks();
        let top = &ranked[0];
        assert_eq!(top.0, (id(&l, "Check"), id(&l, "Approve")));
    }

    /// Ranking uses total, not mean, so a frequent moderate wait outranks a
    /// single long one.
    #[test]
    fn ranks_by_total_not_mean_duration() {
        let l = log(&[
            ("A", &[("Start", 0), ("Frequent", 10)]),
            ("B", &[("Start", 0), ("Frequent", 10)]),
            ("C", &[("Start", 0), ("Frequent", 10)]),
            ("D", &[("Start", 0), ("Rare", 25)]),
        ]);
        let d = Dfg::build(&l);
        let ranked = d.bottlenecks();
        let top = &ranked[0];
        assert_eq!(
            top.0.1,
            id(&l, "Frequent"),
            "30h total should beat 25h total"
        );
    }

    #[test]
    fn detects_self_loops() {
        let l = log(&[("T1", &[("Open", 0), ("Work", 1), ("Work", 4), ("Close", 5)])]);
        let d = Dfg::build(&l);
        let loops = d.self_loops();
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].0, id(&l, "Work"));
        assert_eq!(loops[0].1.frequency, 1);
    }

    /// A single-event case contributes a node but no edge — and it must count
    /// as both a start and an end.
    #[test]
    fn single_event_case_contributes_node_but_no_edge() {
        let l = log(&[("A", &[("Only", 0)])]);
        let d = Dfg::build(&l);
        assert_eq!(d.nodes.len(), 1);
        assert!(d.edges.is_empty());
        let node = &d.nodes[&id(&l, "Only")];
        assert_eq!(node.starts, 1);
        assert_eq!(node.ends, 1);
        assert_eq!(node.frequency, 1);
    }

    /// Case frequency counts cases, not occurrences.
    #[test]
    fn case_frequency_counts_cases_not_repetitions() {
        let l = log(&[("T1", &[("Work", 0), ("Work", 1), ("Work", 2)])]);
        let d = Dfg::build(&l);
        let node = &d.nodes[&id(&l, "Work")];
        assert_eq!(node.frequency, 3);
        assert_eq!(node.case_frequency, 1);
        assert_eq!(d.edges[&(id(&l, "Work"), id(&l, "Work"))].case_frequency, 1);
    }

    /// The honesty test. Concurrent activities produce a cycle in the DFG that
    /// no case ever executed. This is a permanent, deliberate demonstration of
    /// the DFG's central limitation — if it ever stops holding, the graph
    /// construction changed meaning.
    #[test]
    fn concurrency_produces_a_cycle_no_case_executed() {
        let l = log(&[
            (
                "O1",
                &[("Receive", 0), ("Pick", 1), ("Print", 2), ("Ship", 3)],
            ),
            (
                "O2",
                &[("Receive", 0), ("Print", 1), ("Pick", 2), ("Ship", 3)],
            ),
        ]);
        let d = Dfg::build(&l);
        let (pick, print) = (id(&l, "Pick"), id(&l, "Print"));
        assert!(d.edges.contains_key(&(pick, print)));
        assert!(d.edges.contains_key(&(print, pick)));

        // Yet no case actually repeated either activity.
        for trace in &l.traces {
            let picks = trace.events.iter().filter(|e| e.activity == pick).count();
            assert_eq!(picks, 1, "the cycle is an artifact, not observed behavior");
        }
    }

    /// Every reported figure must be traceable to real cases.
    #[test]
    fn nodes_and_edges_carry_evidence_cases() {
        let l = sequential();
        let d = Dfg::build(&l);
        let approve = id(&l, "Approve");
        assert_eq!(
            d.example_cases_for_activity(&l, approve, 3),
            vec!["O1", "O2", "O3"]
        );
        assert_eq!(
            d.example_cases(&l, (id(&l, "Check"), approve), 2),
            vec!["O1", "O2"]
        );
    }

    /// Evidence storage must not grow with the log.
    #[test]
    fn evidence_is_capped() {
        let cases: Vec<(String, Vec<(&str, i64)>)> = (0..50)
            .map(|i| (format!("C{i}"), vec![("A", 0), ("B", 1)]))
            .collect();
        let spec: Vec<(&str, &[(&str, i64)])> = cases
            .iter()
            .map(|(name, events)| (name.as_str(), events.as_slice()))
            .collect();
        let l = log(&spec);
        let d = Dfg::build(&l);
        for node in d.nodes.values() {
            assert!(node.examples.len() <= EVIDENCE_EXAMPLES);
        }
        for edge in d.edges.values() {
            assert!(edge.examples.len() <= EVIDENCE_EXAMPLES);
        }
    }

    /// Without end timestamps we can only measure start-to-start elapsed
    /// time, and must say so rather than calling it waiting.
    #[test]
    fn reports_that_it_is_not_measuring_true_waiting() {
        let d = Dfg::build(&sequential());
        assert!(!d.measures_true_waiting);
    }

    /// With an end timestamp the gap starts when the earlier activity
    /// finished, which excludes its processing time.
    #[test]
    fn uses_end_timestamps_to_measure_real_waiting() {
        let mut l = log(&[("C1", &[("Work", 0), ("Next", 4)])]);
        // Work runs from hour 0 to hour 3; Next starts at hour 4.
        l.traces[0].events[0].end_at = 3 * HOUR;
        let d = Dfg::build(&l);
        assert!(d.measures_true_waiting);
        let e = &d.edges[&(id(&l, "Work"), id(&l, "Next"))];
        assert_eq!(
            e.total_duration, HOUR,
            "one hour of waiting, not four hours of elapsed time"
        );
    }

    #[test]
    fn incomplete_cases_keep_shape_but_not_durations() {
        let mut l = log(&[
            ("complete", &[("A", 0), ("B", 1)]),
            ("incomplete", &[("A", 0), ("B", 100)]),
        ]);
        l.traces[1].incomplete = true;
        let d = Dfg::build(&l);
        let edge = &d.edges[&(id(&l, "A"), id(&l, "B"))];
        assert_eq!(edge.frequency, 2, "both cases define the process shape");
        assert_eq!(edge.duration_frequency, 1);
        assert_eq!(edge.total_duration, HOUR);
        assert_eq!(edge.mean_duration(), HOUR);
    }

    #[test]
    fn same_time_pairs_are_counted_because_their_order_is_not_evidenced() {
        let l = log(&[
            ("tied", &[("A", 0), ("B", 0)]),
            ("timed", &[("A", 0), ("B", 1)]),
        ]);
        let d = Dfg::build(&l);
        let edge = &d.edges[&(id(&l, "A"), id(&l, "B"))];
        assert_eq!((edge.frequency, edge.same_time), (2, 1));
    }

    #[test]
    fn overlapping_activities_have_zero_waiting() {
        let mut l = log(&[("C1", &[("A", 0), ("B", 2)])]);
        l.traces[0].events[0].end_at = 3 * HOUR;
        let d = Dfg::build(&l);
        let edge = &d.edges[&(id(&l, "A"), id(&l, "B"))];
        assert_eq!(edge.total_duration, 0);
        assert_eq!(edge.min_duration, 0);
        assert_eq!(edge.max_duration, 0);
        assert!(d.measures_true_waiting);
    }

    #[test]
    fn is_deterministic() {
        let l = sequential();
        assert_eq!(Dfg::build(&l), Dfg::build(&l));
    }

    #[test]
    fn empty_log_produces_empty_graph() {
        let d = Dfg::build(&EventLog::default());
        assert!(d.nodes.is_empty());
        assert!(d.edges.is_empty());
        assert_eq!(d.case_count, 0);
    }
}
