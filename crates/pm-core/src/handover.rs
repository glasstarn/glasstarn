//! Handover of work — who passes work to whom.
//!
//! Cheap to compute once a resource column is mapped, and often the finding
//! people act on fastest: a queue between two teams is easier to fix than a
//! slow activity, and easier to argue about with names attached.
//!
//! # What it is and is not
//!
//! A handover is one resource performing an event immediately after a
//! different resource, within the same case. That is an **observation about
//! sequence**, not evidence that work was deliberately passed, that either
//! person was slow, or that the transition caused the delay.
//!
//! Resource data names individuals. Treating a slow handover as a judgement of
//! a person, rather than of a process boundary, is the obvious misuse — so the
//! waiting time is attributed to the *transition*, never to a resource.

use crate::dict::ActivityId;
use crate::dict::CaseId;
use crate::log::{EventLog, Micros, ResourceId};
use std::collections::{BTreeMap, BTreeSet};

/// Work passing from one resource to another.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Handover {
    /// Times this transition was observed.
    pub frequency: u64,
    /// Observations eligible for duration statistics.
    pub duration_frequency: u64,
    /// Cases in which it occurred at least once.
    pub case_frequency: u64,
    /// Total elapsed time across all observations.
    pub total_duration: Micros,
    /// A bounded set of cases evidencing this handover.
    pub examples: Vec<CaseId>,
}

impl Handover {
    /// Mean elapsed time across observations.
    pub fn mean_duration(&self) -> Micros {
        if self.duration_frequency == 0 {
            0
        } else {
            self.total_duration / self.duration_frequency as i64
        }
    }
}

/// What one resource did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResourceActivity {
    /// Events performed.
    pub events: u64,
    /// Cases worked on.
    pub cases: u64,
    /// Distinct activities performed — a breadth measure, not a skill measure.
    pub activities: u64,
    /// Times this resource received work from a different one.
    pub received: u64,
    /// Times this resource passed work to a different one.
    pub passed: u64,
}

/// Organizational view of a log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Organization {
    /// Handovers, keyed by (from, to). Excludes a resource following itself.
    pub handovers: BTreeMap<(ResourceId, ResourceId), Handover>,
    /// Per-resource activity.
    pub resources: BTreeMap<ResourceId, ResourceActivity>,
    /// Events where the same resource continued the work.
    pub continuations: u64,
}

impl Organization {
    /// Builds the organizational view.
    ///
    /// Events without a resource are skipped rather than grouped under an
    /// "unknown" bucket, which would invent handovers that were never
    /// observed.
    pub fn build(log: &EventLog) -> Organization {
        let mut org = Organization::default();
        let mut per_resource_activities: BTreeMap<ResourceId, BTreeSet<ActivityId>> =
            BTreeMap::new();

        for trace in &log.traces {
            let mut resources_in_case: BTreeSet<ResourceId> = BTreeSet::new();
            let mut handovers_in_case: BTreeSet<(ResourceId, ResourceId)> = BTreeSet::new();

            for event in &trace.events {
                if let Some(r) = event.resource() {
                    let entry = org.resources.entry(r).or_default();
                    entry.events += 1;
                    per_resource_activities
                        .entry(r)
                        .or_default()
                        .insert(event.activity);
                    resources_in_case.insert(r);
                }
            }

            for pair in trace.events.windows(2) {
                let (Some(from), Some(to)) = (pair[0].resource(), pair[1].resource()) else {
                    continue;
                };
                if from == to {
                    org.continuations += 1;
                    continue;
                }
                let handover = org.handovers.entry((from, to)).or_default();
                handover.frequency += 1;
                if handover.examples.len() < crate::dfg::EVIDENCE_EXAMPLES
                    && !handover.examples.contains(&trace.case)
                {
                    handover.examples.push(trace.case);
                }
                if !trace.incomplete {
                    let duration = (pair[1].at - pair[0].end().unwrap_or(pair[0].at)).max(0);
                    handover.duration_frequency += 1;
                    handover.total_duration += duration;
                }
                handovers_in_case.insert((from, to));

                org.resources.entry(from).or_default().passed += 1;
                org.resources.entry(to).or_default().received += 1;
            }

            for r in resources_in_case {
                org.resources.entry(r).or_default().cases += 1;
            }
            for key in handovers_in_case {
                org.handovers.entry(key).or_default().case_frequency += 1;
            }
        }

        for (r, activities) in per_resource_activities {
            org.resources.entry(r).or_default().activities = activities.len() as u64;
        }

        org
    }

    /// Handovers ranked by total elapsed time, longest first.
    ///
    /// Total rather than mean, for the same reason as bottleneck ranking: a
    /// frequent moderate wait costs more than a single long one.
    pub fn slowest_handovers(&self) -> Vec<((ResourceId, ResourceId), Handover)> {
        let mut ranked: Vec<_> = self
            .handovers
            .iter()
            .filter(|(_, v)| v.duration_frequency > 0)
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        ranked.sort_by(|a, b| {
            b.1.total_duration
                .cmp(&a.1.total_duration)
                .then(a.0.cmp(&b.0))
        });
        ranked
    }

    /// Total elapsed time spent across all handovers.
    pub fn total_handover_time(&self) -> Micros {
        self.handovers.values().map(|h| h.total_duration).sum()
    }

    /// Number of distinct resources seen.
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    /// Share of consecutive event pairs that changed hands, as a percentage.
    ///
    /// High fragmentation means work rarely stays with one person, which is
    /// often where queueing time accumulates.
    pub fn fragmentation_percent(&self) -> f64 {
        let handovers: u64 = self.handovers.values().map(|h| h.frequency).sum();
        let total = handovers + self.continuations;
        if total == 0 {
            0.0
        } else {
            handovers as f64 / total as f64 * 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::{CaseId, Dictionary};
    use crate::log::{CaseTrace, Event};

    const HOUR: Micros = 3_600_000_000;

    /// One event: activity, hour offset, and resource. An empty resource
    /// string means none was recorded.
    type Step<'a> = (&'a str, i64, &'a str);
    /// One case: its identifier and its events.
    type Case<'a> = (&'a str, &'a [Step<'a>]);

    /// Builds a log from case specifications.
    fn log(spec: &[Case<'_>]) -> EventLog {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let mut resources = Dictionary::new();
        let mut traces = Vec::new();
        let mut row = 2u32;
        for (case, events) in spec {
            let case_id = CaseId(cases.intern(case));
            let mut list = Vec::new();
            for (activity, hour, resource) in *events {
                let mut event =
                    Event::new(ActivityId(activities.intern(activity)), hour * HOUR, row);
                if !resource.is_empty() {
                    event.resource = resources.intern(resource);
                }
                list.push(event);
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
            resources,
            attribute_names: Vec::new(),
            attribute_values: Dictionary::new(),
            costs: Default::default(),
            step_values: Default::default(),
            traces,
        }
    }

    fn resource(log: &EventLog, label: &str) -> ResourceId {
        ResourceId(log.resources.get(label).expect("resource exists"))
    }

    fn sample() -> EventLog {
        log(&[
            (
                "C1",
                &[
                    ("Receive", 0, "alice"),
                    ("Check", 2, "alice"),
                    ("Approve", 26, "bob"),
                    ("Ship", 27, "carol"),
                ],
            ),
            (
                "C2",
                &[
                    ("Receive", 1, "alice"),
                    ("Check", 3, "alice"),
                    ("Approve", 51, "bob"),
                    ("Ship", 52, "carol"),
                ],
            ),
        ])
    }

    #[test]
    fn identifies_handovers_between_resources() {
        let l = sample();
        let o = Organization::build(&l);
        let (alice, bob, carol) = (
            resource(&l, "alice"),
            resource(&l, "bob"),
            resource(&l, "carol"),
        );
        assert_eq!(o.handovers.len(), 2);
        assert_eq!(o.handovers[&(alice, bob)].frequency, 2);
        assert_eq!(o.handovers[&(bob, carol)].frequency, 2);
    }

    /// A resource continuing its own work is not a handover.
    #[test]
    fn a_resource_following_itself_is_not_a_handover() {
        let l = sample();
        let o = Organization::build(&l);
        let alice = resource(&l, "alice");
        assert!(!o.handovers.contains_key(&(alice, alice)));
        assert_eq!(o.continuations, 2, "alice → alice in each case");
    }

    /// Hand-derived: alice → bob waits 24h and 48h, so 72h total, 36h mean.
    #[test]
    fn computes_handover_durations() {
        let l = sample();
        let o = Organization::build(&l);
        let h = &o.handovers[&(resource(&l, "alice"), resource(&l, "bob"))];
        assert_eq!(h.total_duration, 72 * HOUR);
        assert_eq!(h.mean_duration(), 36 * HOUR);
        assert_eq!(h.case_frequency, 2);
    }

    #[test]
    fn incomplete_cases_keep_handovers_but_not_durations() {
        let mut l = log(&[
            ("complete", &[("A", 0, "alice"), ("B", 1, "bob")]),
            ("incomplete", &[("A", 0, "alice"), ("B", 100, "bob")]),
        ]);
        l.traces[1].incomplete = true;
        let o = Organization::build(&l);
        let h = &o.handovers[&(resource(&l, "alice"), resource(&l, "bob"))];
        assert_eq!(h.frequency, 2);
        assert_eq!(h.duration_frequency, 1);
        assert_eq!(h.mean_duration(), HOUR);
        assert_eq!(h.examples.len(), 2);
    }

    #[test]
    fn ranks_the_slowest_handover_first() {
        let l = sample();
        let o = Organization::build(&l);
        assert_eq!(
            o.slowest_handovers()[0].0,
            (resource(&l, "alice"), resource(&l, "bob"))
        );
    }

    #[test]
    fn summarises_each_resource() {
        let l = sample();
        let o = Organization::build(&l);
        let alice = o.resources[&resource(&l, "alice")];
        assert_eq!(alice.events, 4);
        assert_eq!(alice.cases, 2);
        assert_eq!(alice.activities, 2, "Receive and Check");
        assert_eq!(alice.passed, 2);
        assert_eq!(alice.received, 0, "alice always starts");
    }

    #[test]
    fn computes_fragmentation() {
        let l = sample();
        let o = Organization::build(&l);
        // Six consecutive pairs; four change hands.
        assert!((o.fragmentation_percent() - 66.666).abs() < 0.01);
    }

    /// Events without a resource must not become an "unknown" participant,
    /// which would invent handovers that were never observed.
    #[test]
    fn events_without_a_resource_are_skipped_not_bucketed() {
        let l = log(&[("C1", &[("A", 0, "alice"), ("B", 1, ""), ("C", 2, "bob")])]);
        let o = Organization::build(&l);
        assert!(o.handovers.is_empty(), "no handover can be inferred: {o:?}");
        assert_eq!(o.resource_count(), 2);
    }

    #[test]
    fn a_log_without_resources_yields_nothing() {
        let l = log(&[("C1", &[("A", 0, ""), ("B", 1, "")])]);
        let o = Organization::build(&l);
        assert_eq!(o.resource_count(), 0);
        assert_eq!(o.fragmentation_percent(), 0.0);
        assert!(o.handovers.is_empty());
    }

    #[test]
    fn is_deterministic() {
        let l = sample();
        assert_eq!(Organization::build(&l), Organization::build(&l));
    }

    #[test]
    fn empty_log_produces_an_empty_organization() {
        let o = Organization::build(&EventLog::default());
        assert_eq!(o.resource_count(), 0);
        assert_eq!(o.total_handover_time(), 0);
    }
}
