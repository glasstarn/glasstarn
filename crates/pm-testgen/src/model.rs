//! Process models used to generate logs with known ground truth.
//!
//! The model is a **process tree**: a block-structured composition of
//! sequence, exclusive choice, parallelism, and loops. This shape is chosen
//! deliberately — it is the same representation the Inductive Miner produces,
//! so a generated log has a directly comparable expected discovery result.
//!
//! Generating a log from a known model is the strongest test oracle available:
//! the correct answer is known by construction, so no human judgment and no
//! reference tool is required.

use crate::rng::Rng;

/// A block-structured process model.
#[derive(Debug, Clone, PartialEq)]
pub enum Model {
    /// A single observable activity.
    Activity(String),

    /// A silent step. Produces no event — models a skippable branch.
    Silent,

    /// Children execute in order.
    Sequence(Vec<Model>),

    /// Exactly one child executes, chosen by relative weight.
    ///
    /// Weights need not sum to one; they are normalized. A branch with weight
    /// zero is never taken, which is how "rare path" logs are built.
    Choice(Vec<(f64, Model)>),

    /// All children execute, interleaved in an arbitrary order.
    ///
    /// Each child's own internal order is preserved; only the interleaving
    /// between children varies. This is true concurrency, and it is what a
    /// directly-follows graph cannot represent faithfully.
    Parallel(Vec<Model>),

    /// `body` executes once, then with probability `repeat` the `redo` branch
    /// executes and `body` runs again.
    ///
    /// This is the rework construct — the source of self-loops and cycles.
    Loop {
        body: Box<Model>,
        redo: Box<Model>,
        repeat: f64,
        /// Hard cap on repetitions, so generation always terminates.
        max_repeats: usize,
    },
}

impl Model {
    /// Convenience constructor for a sequence of plain activities.
    pub fn seq(activities: &[&str]) -> Model {
        Model::Sequence(
            activities
                .iter()
                .map(|a| Model::Activity((*a).to_owned()))
                .collect(),
        )
    }

    /// Convenience constructor for a single activity.
    pub fn act(name: &str) -> Model {
        Model::Activity(name.to_owned())
    }

    /// Every distinct activity label the model can produce, in sorted order.
    ///
    /// This is part of the ground truth: a miner that invents an activity not
    /// in this set, or omits one reachable by a taken branch, is wrong.
    pub fn activities(&self) -> Vec<String> {
        let mut found = Vec::new();
        self.collect_activities(&mut found);
        found.sort();
        found.dedup();
        found
    }

    fn collect_activities(&self, out: &mut Vec<String>) {
        match self {
            Model::Activity(name) => out.push(name.clone()),
            Model::Silent => {}
            Model::Sequence(children) | Model::Parallel(children) => {
                for c in children {
                    c.collect_activities(out);
                }
            }
            Model::Choice(branches) => {
                for (_, c) in branches {
                    c.collect_activities(out);
                }
            }
            Model::Loop { body, redo, .. } => {
                body.collect_activities(out);
                redo.collect_activities(out);
            }
        }
    }

    /// Produces one trace by executing the model.
    ///
    /// The returned sequence contains only observable activities; silent steps
    /// contribute nothing, exactly as they would be unobservable in real data.
    pub fn simulate(&self, rng: &mut Rng) -> Vec<String> {
        let mut trace = Vec::new();
        self.execute(rng, &mut trace);
        trace
    }

    fn execute(&self, rng: &mut Rng, out: &mut Vec<String>) {
        match self {
            Model::Activity(name) => out.push(name.clone()),

            Model::Silent => {}

            Model::Sequence(children) => {
                for c in children {
                    c.execute(rng, out);
                }
            }

            Model::Choice(branches) => {
                if let Some(chosen) = pick_weighted(branches, rng) {
                    chosen.execute(rng, out);
                }
            }

            Model::Parallel(children) => {
                // Execute each child into its own buffer, then interleave the
                // buffers while preserving each child's internal order. This
                // is a random shuffle of the multiset of "next step" choices.
                let mut streams: Vec<std::collections::VecDeque<String>> = children
                    .iter()
                    .map(|c| {
                        let mut buf = Vec::new();
                        c.execute(rng, &mut buf);
                        buf.into()
                    })
                    .collect();

                loop {
                    let live: Vec<usize> = streams
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| !s.is_empty())
                        .map(|(i, _)| i)
                        .collect();
                    if live.is_empty() {
                        break;
                    }
                    let pick = live[rng.below(live.len() as u64) as usize];
                    if let Some(step) = streams[pick].pop_front() {
                        out.push(step);
                    }
                }
            }

            Model::Loop {
                body,
                redo,
                repeat,
                max_repeats,
            } => {
                body.execute(rng, out);
                for _ in 0..*max_repeats {
                    if !rng.chance(*repeat) {
                        break;
                    }
                    redo.execute(rng, out);
                    body.execute(rng, out);
                }
            }
        }
    }
}

/// Selects a branch by relative weight. Returns `None` only if every weight is
/// zero or the branch list is empty.
fn pick_weighted<'a>(branches: &'a [(f64, Model)], rng: &mut Rng) -> Option<&'a Model> {
    let total: f64 = branches.iter().map(|(w, _)| w.max(0.0)).sum();
    if total <= 0.0 {
        return None;
    }
    let mut target = rng.unit() * total;
    for (weight, model) in branches {
        let w = weight.max(0.0);
        if target < w {
            return Some(model);
        }
        target -= w;
    }
    // Floating-point drift can leave a tiny remainder; fall back to the last
    // branch with a non-zero weight rather than returning nothing.
    branches
        .iter()
        .rev()
        .find(|(w, _)| *w > 0.0)
        .map(|(_, m)| m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_produces_exact_order() {
        let m = Model::seq(&["A", "B", "C"]);
        let mut rng = Rng::new(1);
        assert_eq!(m.simulate(&mut rng), vec!["A", "B", "C"]);
    }

    #[test]
    fn silent_step_produces_no_event() {
        let m = Model::Sequence(vec![Model::act("A"), Model::Silent, Model::act("B")]);
        let mut rng = Rng::new(1);
        assert_eq!(m.simulate(&mut rng), vec!["A", "B"]);
    }

    #[test]
    fn choice_takes_exactly_one_branch() {
        let m = Model::Choice(vec![(1.0, Model::act("Left")), (1.0, Model::act("Right"))]);
        let mut rng = Rng::new(7);
        for _ in 0..100 {
            let t = m.simulate(&mut rng);
            assert_eq!(t.len(), 1);
            assert!(t[0] == "Left" || t[0] == "Right");
        }
    }

    #[test]
    fn zero_weight_branch_is_never_taken() {
        let m = Model::Choice(vec![
            (0.0, Model::act("Never")),
            (1.0, Model::act("Always")),
        ]);
        let mut rng = Rng::new(3);
        for _ in 0..200 {
            assert_eq!(m.simulate(&mut rng), vec!["Always"]);
        }
    }

    #[test]
    fn parallel_preserves_each_branch_internal_order() {
        // A→B must stay in order, X→Y must stay in order, but they interleave.
        let m = Model::Parallel(vec![Model::seq(&["A", "B"]), Model::seq(&["X", "Y"])]);
        let mut rng = Rng::new(11);
        for _ in 0..200 {
            let t = m.simulate(&mut rng);
            assert_eq!(t.len(), 4);
            let pos = |s: &str| t.iter().position(|x| x == s).unwrap();
            assert!(pos("A") < pos("B"), "A must precede B in {t:?}");
            assert!(pos("X") < pos("Y"), "X must precede Y in {t:?}");
        }
    }

    #[test]
    fn parallel_actually_interleaves() {
        let m = Model::Parallel(vec![Model::seq(&["A", "B"]), Model::seq(&["X", "Y"])]);
        let mut rng = Rng::new(11);
        let seen: std::collections::HashSet<Vec<String>> =
            (0..500).map(|_| m.simulate(&mut rng)).collect();
        assert!(
            seen.len() > 1,
            "parallel branches produced only one ordering"
        );
    }

    #[test]
    fn loop_repeats_and_terminates() {
        let m = Model::Loop {
            body: Box::new(Model::act("Review")),
            redo: Box::new(Model::act("Rework")),
            repeat: 0.5,
            max_repeats: 3,
        };
        let mut rng = Rng::new(5);
        for _ in 0..300 {
            let t = m.simulate(&mut rng);
            let reviews = t.iter().filter(|a| *a == "Review").count();
            let reworks = t.iter().filter(|a| *a == "Rework").count();
            assert!((1..=4).contains(&reviews), "unexpected repeat count: {t:?}");
            assert_eq!(reworks, reviews - 1, "each redo precedes one more body");
        }
    }

    #[test]
    fn never_repeating_loop_runs_body_once() {
        let m = Model::Loop {
            body: Box::new(Model::act("Do")),
            redo: Box::new(Model::act("Again")),
            repeat: 0.0,
            max_repeats: 5,
        };
        let mut rng = Rng::new(2);
        assert_eq!(m.simulate(&mut rng), vec!["Do"]);
    }

    #[test]
    fn reports_all_activities_sorted_and_deduped() {
        let m = Model::Sequence(vec![
            Model::act("B"),
            Model::Choice(vec![(1.0, Model::act("A")), (1.0, Model::act("B"))]),
            Model::Silent,
        ]);
        assert_eq!(m.activities(), vec!["A", "B"]);
    }

    /// Same seed, same model, same log — the property the whole oracle rests on.
    #[test]
    fn simulation_is_deterministic() {
        let m = Model::Sequence(vec![
            Model::seq(&["Receive", "Check"]),
            Model::Choice(vec![
                (3.0, Model::act("Approve")),
                (1.0, Model::act("Reject")),
            ]),
            Model::Loop {
                body: Box::new(Model::act("Ship")),
                redo: Box::new(Model::act("Return")),
                repeat: 0.3,
                max_repeats: 2,
            },
        ]);
        let run = || {
            let mut rng = Rng::new(2026);
            (0..100).map(|_| m.simulate(&mut rng)).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
