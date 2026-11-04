//! Variants — distinct end-to-end paths through the process.
//!
//! The cheapest high-value artifact in process mining. Most real processes
//! have a handful of variants covering the majority of cases, and a long tail
//! of near-unique ones. That concentration is often the single most
//! informative number in an analysis.
//!
//! Variant coverage is also what keeps a directly-follows graph honest: it
//! tells the user how much of observed reality the displayed map accounts for.

use crate::dict::{ActivityId, CaseId};
use crate::log::{EventLog, Micros};
use std::collections::BTreeMap;

/// One distinct path, and the cases that followed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// The activity sequence defining this variant.
    pub sequence: Vec<ActivityId>,
    /// Cases that followed it, in first-seen order.
    pub cases: Vec<CaseId>,
    /// Total throughput across those cases.
    pub total_throughput: Micros,
    /// Cases eligible for duration statistics.
    pub duration_cases: usize,
}

impl Variant {
    /// Number of cases following this variant.
    pub fn case_count(&self) -> usize {
        self.cases.len()
    }

    /// Events contributed by all its cases.
    pub fn event_count(&self) -> usize {
        self.sequence.len() * self.cases.len()
    }

    /// Mean throughput across its cases.
    pub fn mean_throughput(&self) -> Micros {
        if self.duration_cases == 0 {
            0
        } else {
            self.total_throughput / self.duration_cases as i64
        }
    }
}

/// All variants in a log, ranked by case count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Variants {
    /// Variants, most frequent first. Ties broken by sequence for stability.
    pub variants: Vec<Variant>,
    /// Cases the set was built from.
    pub case_count: usize,
}

impl Variants {
    /// Computes variants from a normalized log.
    pub fn build(log: &EventLog) -> Variants {
        let mut groups: BTreeMap<Vec<ActivityId>, Variant> = BTreeMap::new();

        for trace in &log.traces {
            let sequence: Vec<ActivityId> = trace.events.iter().map(|e| e.activity).collect();
            let entry = groups.entry(sequence.clone()).or_insert_with(|| Variant {
                sequence,
                cases: Vec::new(),
                total_throughput: 0,
                duration_cases: 0,
            });
            entry.cases.push(trace.case);
            if !trace.incomplete {
                entry.total_throughput += trace.throughput();
                entry.duration_cases += 1;
            }
        }

        let mut variants: Vec<Variant> = groups.into_values().collect();
        // Most frequent first; ties broken by sequence so the order is stable
        // rather than dependent on iteration.
        variants.sort_by(|a, b| {
            b.cases
                .len()
                .cmp(&a.cases.len())
                .then(a.sequence.cmp(&b.sequence))
        });

        Variants {
            variants,
            case_count: log.case_count(),
        }
    }

    /// Number of distinct variants.
    pub fn len(&self) -> usize {
        self.variants.len()
    }

    /// Whether there are no variants.
    pub fn is_empty(&self) -> bool {
        self.variants.is_empty()
    }

    /// Share of cases covered by the top `n` variants, as a percentage.
    ///
    /// The headline concentration number: "the top 5 paths cover 82% of cases".
    pub fn coverage(&self, n: usize) -> f64 {
        if self.case_count == 0 {
            return 0.0;
        }
        let covered: usize = self.variants.iter().take(n).map(Variant::case_count).sum();
        covered as f64 / self.case_count as f64 * 100.0
    }

    /// Fewest variants needed to cover at least `percent` of cases.
    ///
    /// Answers "how many paths do I have to understand to understand most of
    /// this process?"
    pub fn variants_needed_for(&self, percent: f64) -> usize {
        if self.case_count == 0 {
            return 0;
        }
        let target = self.case_count as f64 * percent / 100.0;
        let mut covered = 0usize;
        for (i, v) in self.variants.iter().enumerate() {
            covered += v.case_count();
            if covered as f64 >= target {
                return i + 1;
            }
        }
        self.variants.len()
    }

    /// Variants followed by exactly one case.
    ///
    /// A large count here means a process with little standardisation — or a
    /// data-quality problem.
    pub fn unique_variants(&self) -> usize {
        self.variants.iter().filter(|v| v.case_count() == 1).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::Dictionary;
    use crate::log::{CaseTrace, Event};

    const HOUR: Micros = 3_600_000_000;

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

    /// Mirrors `testdata/canonical/05-choice.csv`: 5 cases, 2 variants, 3 accept
    /// and 2 reject.
    fn choice() -> EventLog {
        log(&[
            (
                "A1",
                &[("Submit", 0), ("Assess", 5), ("Accept", 24), ("Notify", 25)],
            ),
            (
                "A2",
                &[
                    ("Submit", 1),
                    ("Assess", 26),
                    ("Reject", 30),
                    ("Notify", 31),
                ],
            ),
            (
                "A3",
                &[
                    ("Submit", 23),
                    ("Assess", 28),
                    ("Accept", 48),
                    ("Notify", 50),
                ],
            ),
            (
                "A4",
                &[
                    ("Submit", 24),
                    ("Assess", 49),
                    ("Reject", 53),
                    ("Notify", 54),
                ],
            ),
            (
                "A5",
                &[
                    ("Submit", 47),
                    ("Assess", 51),
                    ("Accept", 72),
                    ("Notify", 73),
                ],
            ),
        ])
    }

    #[test]
    fn groups_identical_paths() {
        let v = Variants::build(&choice());
        assert_eq!(v.len(), 2);
        assert_eq!(v.variants[0].case_count(), 3);
        assert_eq!(v.variants[1].case_count(), 2);
    }

    /// The invariant: variant case counts must sum to the total case count.
    /// If this fails, a case was lost or double-counted.
    #[test]
    fn case_counts_sum_to_the_total() {
        let v = Variants::build(&choice());
        let total: usize = v.variants.iter().map(Variant::case_count).sum();
        assert_eq!(total, v.case_count);
        assert_eq!(total, 5);
    }

    #[test]
    fn ranks_most_frequent_first() {
        let v = Variants::build(&choice());
        let counts: Vec<usize> = v.variants.iter().map(Variant::case_count).collect();
        let mut sorted = counts.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(counts, sorted);
    }

    #[test]
    fn computes_coverage() {
        let v = Variants::build(&choice());
        assert_eq!(v.coverage(1), 60.0);
        assert_eq!(v.coverage(2), 100.0);
        // Asking for more variants than exist is not an error.
        assert_eq!(v.coverage(99), 100.0);
    }

    #[test]
    fn computes_variants_needed_for_a_target() {
        let v = Variants::build(&choice());
        assert_eq!(v.variants_needed_for(50.0), 1);
        assert_eq!(v.variants_needed_for(80.0), 2);
        assert_eq!(v.variants_needed_for(100.0), 2);
    }

    #[test]
    fn counts_unique_variants() {
        let v = Variants::build(&log(&[
            ("A", &[("X", 0)]),
            ("B", &[("Y", 0)]),
            ("C", &[("X", 0)]),
        ]));
        assert_eq!(v.len(), 2);
        assert_eq!(v.unique_variants(), 1);
    }

    #[test]
    fn computes_mean_throughput_per_variant() {
        let v = Variants::build(&choice());
        // Accept cases run 0→25, 23→50, 47→73: 25, 27, 26 hours. Mean 26.
        let accept = &v.variants[0];
        assert_eq!(accept.total_throughput, 78 * HOUR);
        assert_eq!(accept.mean_throughput(), 26 * HOUR);
    }

    #[test]
    fn incomplete_cases_do_not_bias_variant_duration() {
        let mut l = log(&[
            ("complete", &[("A", 0), ("B", 2)]),
            ("incomplete", &[("A", 0), ("B", 100)]),
        ]);
        l.traces[1].incomplete = true;
        let variants = Variants::build(&l);
        let variant = &variants.variants[0];
        assert_eq!(variant.case_count(), 2);
        assert_eq!(variant.duration_cases, 1);
        assert_eq!(variant.mean_throughput(), 2 * HOUR);
    }

    /// Order within a case matters: the same activities in a different order
    /// are a different variant. This is what makes concurrency visible.
    #[test]
    fn different_order_is_a_different_variant() {
        let v = Variants::build(&log(&[
            ("O1", &[("Pick", 0), ("Print", 1)]),
            ("O2", &[("Print", 0), ("Pick", 1)]),
        ]));
        assert_eq!(v.len(), 2);
        assert_eq!(v.coverage(2), 100.0);
    }

    #[test]
    fn single_event_cases_form_their_own_variant() {
        let v = Variants::build(&log(&[
            ("A", &[("Created", 0)]),
            ("B", &[("Created", 0)]),
            ("C", &[("Created", 0), ("Qualified", 1)]),
        ]));
        assert_eq!(v.len(), 2);
        assert_eq!(v.variants[0].case_count(), 2);
        assert_eq!(v.variants[0].sequence.len(), 1);
    }

    #[test]
    fn is_deterministic() {
        let l = choice();
        assert_eq!(Variants::build(&l), Variants::build(&l));
    }

    #[test]
    fn empty_log_has_no_variants() {
        let v = Variants::build(&EventLog::default());
        assert!(v.is_empty());
        assert_eq!(v.coverage(1), 0.0);
        assert_eq!(v.variants_needed_for(80.0), 0);
    }
}
