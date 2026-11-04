//! The facts about each step that decide whether it can be automated: how
//! often it happens, how often it is redone, who does it (people, systems, or
//! nobody recorded), and how predictable what comes next is.
//!
//! These are measurements only. Which bucket a step belongs in (already
//! automated, rule candidate, agent candidate, stays human) is a judgement an
//! agent suggests and a person confirms; nothing here decides it.
//!
//! Traces are read one at a time ([`StepFactsFold`]), so the facts are the
//! same whether the log is in memory or on disk.

use crate::automation::ResourceKind;
use crate::declare::Step;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Cases kept as examples for each step.
const STEP_EXAMPLES: usize = 3;

/// The facts about one step.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepFacts {
    pub activity: u32,
    pub occurrences: u64,
    /// Cases with the step at least once.
    pub cases: u64,
    /// Occurrences after the first in a case: rework.
    pub repeats: u64,
    /// Distinct performers seen doing the step.
    pub performers: usize,
    pub by_people: u64,
    pub by_systems: u64,
    /// Occurrences with no performer recorded.
    pub unrecorded: u64,
    /// Occurrences by performers nobody has classified yet.
    pub not_classified: u64,
    /// The step that most often comes next, and in what share of the
    /// occurrences followed by any step, 0 to 1. A decision that always
    /// leads the same way is a candidate for a simple rule.
    pub usual_next: Option<u32>,
    pub usual_next_share: f64,
    /// Occurrences that are the last step of their case.
    pub ends_case: u64,
    pub examples: Vec<u32>,
}

#[derive(Default)]
struct Tally {
    occurrences: u64,
    cases: u64,
    repeats: u64,
    performers: BTreeSet<u32>,
    by_people: u64,
    by_systems: u64,
    unrecorded: u64,
    not_classified: u64,
    next: BTreeMap<u32, u64>,
    ends_case: u64,
    examples: Vec<u32>,
}

/// Collects [`StepFacts`]. `kinds[p]` is what performer `p` is, when someone
/// has decided it.
pub struct StepFactsFold<'a> {
    kinds: &'a dyn Fn(u32) -> Option<ResourceKind>,
    steps: BTreeMap<u32, Tally>,
}

impl<'a> StepFactsFold<'a> {
    pub fn new(kinds: &'a dyn Fn(u32) -> Option<ResourceKind>) -> Self {
        Self {
            kinds,
            steps: BTreeMap::new(),
        }
    }

    pub fn trace(&mut self, case: u32, steps: &[Step]) {
        let mut seen = BTreeSet::new();
        for (index, step) in steps.iter().enumerate() {
            let tally = self.steps.entry(step.activity).or_default();
            tally.occurrences += 1;
            if seen.insert(step.activity) {
                tally.cases += 1;
                if tally.examples.len() < STEP_EXAMPLES {
                    tally.examples.push(case);
                }
            } else {
                tally.repeats += 1;
            }
            match step.performer {
                None => tally.unrecorded += 1,
                Some(performer) => {
                    tally.performers.insert(performer);
                    match (self.kinds)(performer) {
                        Some(ResourceKind::Human) => tally.by_people += 1,
                        Some(ResourceKind::System) => tally.by_systems += 1,
                        Some(ResourceKind::Unrecorded) => tally.unrecorded += 1,
                        None => tally.not_classified += 1,
                    }
                }
            }
            match steps.get(index + 1) {
                Some(next) => *tally.next.entry(next.activity).or_insert(0) += 1,
                None => tally.ends_case += 1,
            }
        }
    }

    /// The facts, most frequent step first.
    pub fn finish(self) -> Vec<StepFacts> {
        let mut facts = self
            .steps
            .into_iter()
            .map(|(activity, t)| {
                let followed: u64 = t.next.values().sum();
                let usual = t
                    .next
                    .iter()
                    .max_by(|x, y| x.1.cmp(y.1).then(y.0.cmp(x.0)))
                    .map(|(a, n)| (*a, *n));
                StepFacts {
                    activity,
                    occurrences: t.occurrences,
                    cases: t.cases,
                    repeats: t.repeats,
                    performers: t.performers.len(),
                    by_people: t.by_people,
                    by_systems: t.by_systems,
                    unrecorded: t.unrecorded,
                    not_classified: t.not_classified,
                    usual_next: usual.map(|u| u.0),
                    usual_next_share: match usual {
                        Some((_, n)) if followed > 0 => n as f64 / followed as f64,
                        _ => 0.0,
                    },
                    ends_case: t.ends_case,
                    examples: t.examples,
                }
            })
            .collect::<Vec<_>>();
        facts.sort_by(|x, y| {
            y.occurrences
                .cmp(&x.occurrences)
                .then(x.activity.cmp(&y.activity))
        });
        facts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(activity: u32, performer: Option<u32>) -> Step {
        Step {
            activity,
            performer,
        }
    }

    #[test]
    fn counts_who_does_each_step_and_what_follows() {
        // Performer 0 is a person, 1 a system, 2 not classified yet.
        let kinds = |p: u32| match p {
            0 => Some(ResourceKind::Human),
            1 => Some(ResourceKind::System),
            _ => None,
        };
        let mut fold = StepFactsFold::new(&kinds);
        fold.trace(10, &[step(0, Some(1)), step(1, Some(0)), step(2, Some(0))]);
        fold.trace(
            11,
            &[
                step(0, Some(1)),
                step(1, Some(2)),
                step(1, Some(0)),
                step(3, None),
            ],
        );
        let facts = fold.finish();
        let check = facts.iter().find(|f| f.activity == 1).unwrap();
        assert_eq!((check.occurrences, check.cases, check.repeats), (3, 2, 1));
        assert_eq!((check.by_people, check.not_classified), (2, 1));
        assert_eq!(check.performers, 2);
        let created = facts.iter().find(|f| f.activity == 0).unwrap();
        assert_eq!(created.by_systems, 2);
        // Always followed by step 1.
        assert_eq!(
            (created.usual_next, created.usual_next_share),
            (Some(1), 1.0)
        );
        let last = facts.iter().find(|f| f.activity == 3).unwrap();
        assert_eq!((last.unrecorded, last.ends_case), (1, 1));
        // Most frequent first.
        assert_eq!(facts[0].activity, 1);
    }
}
