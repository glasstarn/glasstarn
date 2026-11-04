//! Declarative rules: which rules a process follows, and which cases break
//! them.
//!
//! A rule names one or two activities and a template from the Declare
//! language (Pesic and van der Aalst, "A Declarative Approach for Flexible
//! Business Processes Management", BPM Workshops 2006): for example
//! *response(A, B)*, "every A is eventually followed by B", or
//! *precedence(A, B)*, "B happens only after A". Rules are mined as in Maggi,
//! Mooij and van der Aalst, "User-Guided Discovery of Declarative Process
//! Models" (CIDM 2011): every candidate rule is checked against every trace,
//! and a rule is kept when it holds in nearly all the traces that *activate*
//! it. Traces that do not contain the activating activity satisfy a rule
//! vacuously; they count towards support but not towards confidence, so a rule
//! about a rare step is not inflated by the cases that never reach it.
//!
//! One template here is not part of the original language: *different
//! performers*, the four-eyes rule that the same person never performs both A
//! and B in one case (separation of duties). It needs the performer of each
//! step.
//!
//! The traces are read one at a time ([`DeclareFold`], [`RuleCheckFold`]),
//! so the same rules come out whether the log is in memory or on disk.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Cases kept as evidence for each broken rule.
pub const RULE_EXAMPLES: usize = 5;

/// A rule template. Unary templates use only the first activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Template {
    /// A happens in every case.
    Existence,
    /// A happens at most once in a case.
    AtMostOnce,
    /// Every case starts with A.
    Init,
    /// Every case ends with A.
    End,
    /// If A happens, B happens too (before or after).
    RespondedExistence,
    /// Every A is eventually followed by B.
    Response,
    /// Every A is directly followed by B.
    ChainResponse,
    /// B happens only after A has happened.
    Precedence,
    /// B happens only directly after A.
    ChainPrecedence,
    /// Every A is followed by B, and B happens only after A.
    Succession,
    /// A and B never both happen in a case.
    NotCoexistence,
    /// B never happens after A.
    NotSuccession,
    /// A and B are never performed by the same performer in one case.
    DifferentPerformers,
}

impl Template {
    /// Whether the template names a second activity.
    pub fn is_binary(self) -> bool {
        !matches!(
            self,
            Self::Existence | Self::AtMostOnce | Self::Init | Self::End
        )
    }

    /// The rule in words, with the activities' names.
    pub fn describe(self, a: &str, b: &str) -> String {
        match self {
            Self::Existence => format!("“{a}” happens in every case"),
            Self::AtMostOnce => format!("“{a}” happens at most once per case"),
            Self::Init => format!("every case starts with “{a}”"),
            Self::End => format!("every case ends with “{a}”"),
            Self::RespondedExistence => format!("if “{a}” happens, “{b}” happens too"),
            Self::Response => format!("“{a}” is always followed by “{b}”"),
            Self::ChainResponse => format!("“{a}” is always directly followed by “{b}”"),
            Self::Precedence => format!("“{b}” happens only after “{a}”"),
            Self::ChainPrecedence => format!("“{b}” happens only directly after “{a}”"),
            Self::Succession => {
                format!("“{a}” is always followed by “{b}”, and “{b}” happens only after “{a}”")
            }
            Self::NotCoexistence => format!("“{a}” and “{b}” never happen in the same case"),
            Self::NotSuccession => format!("“{b}” never happens after “{a}”"),
            Self::DifferentPerformers => {
                format!("the same person never does both “{a}” and “{b}” in a case")
            }
        }
    }
}

/// One rule over activity identifiers. For a unary template `b` equals `a`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Rule {
    pub template: Template,
    pub a: u32,
    pub b: u32,
}

impl Rule {
    pub fn unary(template: Template, a: u32) -> Self {
        Self { template, a, b: a }
    }
    pub fn binary(template: Template, a: u32, b: u32) -> Self {
        Self { template, a, b }
    }
}

/// How a rule fared on the log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleResult {
    pub rule: Rule,
    /// Cases read.
    pub cases: u64,
    /// Cases in which the rule had something to say (contain the activating
    /// activity); the rest satisfy it vacuously.
    pub activated: u64,
    /// Activated cases that break the rule.
    pub violated: u64,
    /// Share of activated cases that keep the rule, 0 to 1.
    pub confidence: f64,
    /// Share of all cases that keep the rule, vacuously or not, 0 to 1.
    pub support: f64,
    /// A few cases that break it, in reading order.
    pub examples: Vec<u32>,
    /// The total of the cases' value (an amount) over the cases that break
    /// it, when values were given.
    pub violated_value: f64,
}

/// One step of a case: its activity and, when recorded, its performer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub activity: u32,
    pub performer: Option<u32>,
}

/// Where each activity sits in one trace.
struct Positions {
    first: usize,
    last: usize,
    count: u32,
}

fn positions(steps: &[Step]) -> BTreeMap<u32, Positions> {
    let mut map = BTreeMap::new();
    for (index, step) in steps.iter().enumerate() {
        map.entry(step.activity)
            .and_modify(|p: &mut Positions| {
                p.last = index;
                p.count += 1;
            })
            .or_insert(Positions {
                first: index,
                last: index,
                count: 1,
            });
    }
    map
}

/// Whether `rule` holds in `steps`, and whether the trace activates it.
/// Returns (activated, satisfied).
fn evaluate(rule: Rule, steps: &[Step], at: &BTreeMap<u32, Positions>) -> (bool, bool) {
    let a = at.get(&rule.a);
    let b = at.get(&rule.b);
    let every_a = |f: &dyn Fn(usize) -> bool| {
        steps
            .iter()
            .enumerate()
            .filter(|(_, s)| s.activity == rule.a)
            .all(|(i, _)| f(i))
    };
    let every_b = |f: &dyn Fn(usize) -> bool| {
        steps
            .iter()
            .enumerate()
            .filter(|(_, s)| s.activity == rule.b)
            .all(|(i, _)| f(i))
    };
    match rule.template {
        Template::Existence => (true, a.is_some()),
        Template::AtMostOnce => (a.is_some(), a.is_none_or(|p| p.count <= 1)),
        Template::Init => (true, steps.first().is_some_and(|s| s.activity == rule.a)),
        Template::End => (true, steps.last().is_some_and(|s| s.activity == rule.a)),
        Template::RespondedExistence => (a.is_some(), a.is_none() || b.is_some()),
        Template::Response => (
            a.is_some(),
            match (a, b) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(a), Some(b)) => b.last > a.last,
            },
        ),
        Template::ChainResponse => (
            a.is_some(),
            a.is_none() || every_a(&|i| steps.get(i + 1).is_some_and(|s| s.activity == rule.b)),
        ),
        Template::Precedence => (
            b.is_some(),
            match (a, b) {
                (_, None) => true,
                (None, Some(_)) => false,
                (Some(a), Some(b)) => a.first < b.first,
            },
        ),
        Template::ChainPrecedence => (
            b.is_some(),
            b.is_none() || every_b(&|i| i > 0 && steps[i - 1].activity == rule.a),
        ),
        Template::Succession => {
            let response = evaluate(
                Rule {
                    template: Template::Response,
                    ..rule
                },
                steps,
                at,
            );
            let precedence = evaluate(
                Rule {
                    template: Template::Precedence,
                    ..rule
                },
                steps,
                at,
            );
            (response.0 || precedence.0, response.1 && precedence.1)
        }
        Template::NotCoexistence => (a.is_some() || b.is_some(), a.is_none() || b.is_none()),
        Template::NotSuccession => (
            a.is_some(),
            match (a, b) {
                (Some(a), Some(b)) => b.last < a.first,
                _ => true,
            },
        ),
        Template::DifferentPerformers => {
            let both = a.is_some() && b.is_some();
            if !both {
                return (false, true);
            }
            let performers = |activity: u32| {
                steps
                    .iter()
                    .filter(move |s| s.activity == activity)
                    .filter_map(|s| s.performer)
            };
            let shared = performers(rule.a).any(|p| performers(rule.b).any(|q| q == p));
            (true, !shared)
        }
    }
}

/// Checks a fixed set of rules, one trace at a time.
pub struct RuleCheckFold {
    rules: Vec<Rule>,
    cases: u64,
    activated: Vec<u64>,
    violated: Vec<u64>,
    examples: Vec<Vec<u32>>,
    values: Vec<f64>,
}

impl RuleCheckFold {
    pub fn new(rules: Vec<Rule>) -> Self {
        let n = rules.len();
        Self {
            rules,
            cases: 0,
            activated: vec![0; n],
            violated: vec![0; n],
            examples: vec![Vec::new(); n],
            values: vec![0.0; n],
        }
    }

    pub fn trace(&mut self, case: u32, steps: &[Step]) {
        self.trace_with_value(case, steps, None);
    }

    /// As [`Self::trace`], with the case's value (an amount) to total over
    /// the cases that break each rule.
    pub fn trace_with_value(&mut self, case: u32, steps: &[Step], value: Option<f64>) {
        self.cases += 1;
        let at = positions(steps);
        for (index, rule) in self.rules.iter().enumerate() {
            let (activated, satisfied) = evaluate(*rule, steps, &at);
            if activated {
                self.activated[index] += 1;
                if !satisfied {
                    self.violated[index] += 1;
                    self.values[index] += value.unwrap_or(0.0);
                    if self.examples[index].len() < RULE_EXAMPLES {
                        self.examples[index].push(case);
                    }
                }
            }
        }
    }

    pub fn finish(self) -> Vec<RuleResult> {
        let cases = self.cases;
        self.rules
            .into_iter()
            .enumerate()
            .map(|(index, rule)| {
                let activated = self.activated[index];
                let violated = self.violated[index];
                RuleResult {
                    rule,
                    cases,
                    activated,
                    violated,
                    confidence: if activated == 0 {
                        1.0
                    } else {
                        (activated - violated) as f64 / activated as f64
                    },
                    support: if cases == 0 {
                        1.0
                    } else {
                        (cases - violated) as f64 / cases as f64
                    },
                    examples: self.examples[index].clone(),
                    violated_value: self.values[index],
                }
            })
            .collect()
    }
}

/// How rules are mined.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiningSettings {
    /// Keep rules that hold in at least this share of the cases that
    /// activate them.
    pub min_confidence: f64,
    /// Consider only activities in at least this share of cases.
    pub min_activity_share: f64,
    /// Keep at most this many rules, strongest first.
    pub max_rules: usize,
}

impl Default for MiningSettings {
    fn default() -> Self {
        Self {
            min_confidence: 0.95,
            min_activity_share: 0.05,
            max_rules: 60,
        }
    }
}

#[derive(Default, Clone, Copy)]
struct PairCounts {
    /// Cases with both activities.
    both: u64,
    response: u64,
    chain_response: u64,
    precedence: u64,
    chain_precedence: u64,
    /// Cases with both activities whose performers are all recorded.
    performed: u64,
    different_performers: u64,
}

#[derive(Default, Clone, Copy)]
struct ActivityCounts {
    cases: u64,
    at_most_once: u64,
    first: u64,
    last: u64,
    fully_performed: u64,
}

/// Mines rules: counts, per trace, what each candidate rule needs, without
/// keeping the traces. Only pairs that occur together are stored; a rule
/// with B missing from a case is derived from the per-activity counts.
#[derive(Default)]
pub struct DeclareFold {
    cases: u64,
    activities: HashMap<u32, ActivityCounts>,
    pairs: HashMap<(u32, u32), PairCounts>,
}

impl DeclareFold {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trace(&mut self, steps: &[Step]) {
        self.cases += 1;
        if steps.is_empty() {
            return;
        }
        let at = positions(steps);
        let performed = |activity: u32| {
            steps
                .iter()
                .filter(|s| s.activity == activity)
                .all(|s| s.performer.is_some())
        };
        for (activity, p) in &at {
            let counts = self.activities.entry(*activity).or_default();
            counts.cases += 1;
            counts.at_most_once += u64::from(p.count <= 1);
            counts.first += u64::from(steps[0].activity == *activity);
            counts.last += u64::from(steps[steps.len() - 1].activity == *activity);
            counts.fully_performed += u64::from(performed(*activity));
        }
        for (a, pa) in &at {
            for (b, pb) in &at {
                if a == b {
                    continue;
                }
                let rule = |template| Rule::binary(template, *a, *b);
                let counts = self.pairs.entry((*a, *b)).or_default();
                counts.both += 1;
                counts.response += u64::from(pb.last > pa.last);
                counts.precedence += u64::from(pa.first < pb.first);
                counts.chain_response +=
                    u64::from(evaluate(rule(Template::ChainResponse), steps, &at).1);
                counts.chain_precedence +=
                    u64::from(evaluate(rule(Template::ChainPrecedence), steps, &at).1);
                if performed(*a) && performed(*b) {
                    counts.performed += 1;
                    counts.different_performers +=
                        u64::from(evaluate(rule(Template::DifferentPerformers), steps, &at).1);
                }
            }
        }
    }

    /// The rules that hold, strongest first. Their violating cases come from
    /// a [`RuleCheckFold`] pass over the same traces.
    ///
    /// Rules another kept rule already implies are left out: a step that
    /// starts every case precedes everything, and every step "co-occurs"
    /// with one that happens in every case. Four-eyes rules are offered only
    /// for steps `four_eyes` accepts (steps people do), and only the
    /// [`FOUR_EYES_SHOWN`] most common, since steps done by different teams
    /// trivially have different performers.
    pub fn finish(self, settings: &MiningSettings, four_eyes: &dyn Fn(u32) -> bool) -> Vec<Rule> {
        let cases = self.cases;
        if cases == 0 {
            return Vec::new();
        }
        let min_cases = ((cases as f64 * settings.min_activity_share).ceil() as u64).max(1);
        let frequent = |a: &u32| self.activities.get(a).is_some_and(|c| c.cases >= min_cases);
        let holds = |kept: u64, activated: u64| {
            activated >= min_cases && kept as f64 >= settings.min_confidence * activated as f64
        };
        // (rule, confidence, activated)
        let mut found: Vec<(Rule, f64, u64)> = Vec::new();
        let mut separated: Vec<(Rule, f64, u64)> = Vec::new();
        let mut activities = self
            .activities
            .keys()
            .copied()
            .filter(frequent)
            .collect::<Vec<_>>();
        activities.sort_unstable();
        let mut always = Vec::new();
        let mut starts = Vec::new();
        {
            let mut add = |rule: Rule, kept: u64, activated: u64| {
                if holds(kept, activated) {
                    found.push((rule, kept as f64 / activated as f64, activated));
                    true
                } else {
                    false
                }
            };
            for a in &activities {
                let c = self.activities[a];
                if add(Rule::unary(Template::Existence, *a), c.cases, cases) {
                    always.push(*a);
                }
                add(
                    Rule::unary(Template::AtMostOnce, *a),
                    c.at_most_once,
                    c.cases,
                );
                if add(Rule::unary(Template::Init, *a), c.first, cases) {
                    starts.push(*a);
                }
                add(Rule::unary(Template::End, *a), c.last, cases);
            }
        }
        let add = |list: &mut Vec<(Rule, f64, u64)>, rule: Rule, kept: u64, activated: u64| {
            if holds(kept, activated) {
                list.push((rule, kept as f64 / activated as f64, activated));
            }
        };
        for a in &activities {
            for b in &activities {
                if a == b {
                    continue;
                }
                let ca = self.activities[a];
                let cb = self.activities[b];
                let p = self.pairs.get(&(*a, *b)).copied().unwrap_or_default();
                let response = holds(p.response, ca.cases);
                // A step that starts every case precedes every other.
                let precedence = holds(p.precedence, cb.cases) && !starts.contains(a);
                let chain_response = holds(p.chain_response, ca.cases);
                let chain_precedence = holds(p.chain_precedence, cb.cases);
                if response && precedence {
                    let kept = p.response.min(p.precedence);
                    add(
                        &mut found,
                        Rule::binary(Template::Succession, *a, *b),
                        kept,
                        ca.cases.max(cb.cases),
                    );
                } else {
                    if chain_response {
                        add(
                            &mut found,
                            Rule::binary(Template::ChainResponse, *a, *b),
                            p.chain_response,
                            ca.cases,
                        );
                    } else if response {
                        add(
                            &mut found,
                            Rule::binary(Template::Response, *a, *b),
                            p.response,
                            ca.cases,
                        );
                    } else if !always.contains(b) {
                        add(
                            &mut found,
                            Rule::binary(Template::RespondedExistence, *a, *b),
                            p.both,
                            ca.cases,
                        );
                    }
                    if chain_precedence {
                        add(
                            &mut found,
                            Rule::binary(Template::ChainPrecedence, *a, *b),
                            p.chain_precedence,
                            cb.cases,
                        );
                    } else if precedence {
                        add(
                            &mut found,
                            Rule::binary(Template::Precedence, *a, *b),
                            p.precedence,
                            cb.cases,
                        );
                    }
                }
                if a < b {
                    let either = ca.cases + cb.cases - p.both;
                    add(
                        &mut found,
                        Rule::binary(Template::NotCoexistence, *a, *b),
                        either - p.both,
                        either,
                    );
                    // Four eyes only where both steps' performers are known.
                    if four_eyes(*a)
                        && four_eyes(*b)
                        && p.performed >= min_cases
                        && p.performed * 10 >= p.both * 9
                    {
                        add(
                            &mut separated,
                            Rule::binary(Template::DifferentPerformers, *a, *b),
                            p.different_performers,
                            p.performed,
                        );
                    }
                }
            }
        }
        separated.sort_by(|x, y| y.2.cmp(&x.2).then(x.0.cmp(&y.0)));
        separated.truncate(FOUR_EYES_SHOWN);
        found.extend(separated);
        found.sort_by(|x, y| y.1.total_cmp(&x.1).then(y.2.cmp(&x.2)).then(x.0.cmp(&y.0)));
        found.truncate(settings.max_rules);
        found.into_iter().map(|(rule, _, _)| rule).collect()
    }
}

/// Four-eyes rules offered at most.
pub const FOUR_EYES_SHOWN: usize = 10;

#[cfg(test)]
mod tests {
    use super::*;

    const A: u32 = 0;
    const B: u32 = 1;
    const C: u32 = 2;
    const D: u32 = 3;

    fn trace(activities: &[u32]) -> Vec<Step> {
        activities
            .iter()
            .map(|a| Step {
                activity: *a,
                performer: None,
            })
            .collect()
    }

    fn check(rule: Rule, traces: &[&[u32]]) -> RuleResult {
        let mut fold = RuleCheckFold::new(vec![rule]);
        for (case, t) in traces.iter().enumerate() {
            fold.trace(case as u32, &trace(t));
        }
        fold.finish().remove(0)
    }

    #[test]
    fn response_and_precedence_follow_their_definitions() {
        let traces: &[&[u32]] = &[&[A, B], &[A, C], &[C], &[B, A, B], &[B]];
        let response = check(Rule::binary(Template::Response, A, B), traces);
        // Activated by the traces with A; the second has no B after A.
        assert_eq!((response.activated, response.violated), (3, 1));
        assert_eq!(response.examples, vec![1]);
        let precedence = check(Rule::binary(Template::Precedence, A, B), traces);
        // Activated by the traces with B; the last two start with B.
        assert_eq!((precedence.activated, precedence.violated), (3, 2));
        assert_eq!(precedence.examples, vec![3, 4]);
        // Traces without the activating step satisfy the rule vacuously.
        assert_eq!(precedence.support, 3.0 / 5.0);
    }

    #[test]
    fn chain_rules_need_the_step_directly_next() {
        let traces: &[&[u32]] = &[&[A, B], &[A, C, B]];
        let chain = check(Rule::binary(Template::ChainResponse, A, B), traces);
        assert_eq!(chain.violated, 1);
        let chain_precedence = check(Rule::binary(Template::ChainPrecedence, A, B), traces);
        assert_eq!(chain_precedence.violated, 1);
    }

    #[test]
    fn unary_and_negative_rules() {
        let traces: &[&[u32]] = &[&[A, B, D], &[A, A, D], &[A, C, D]];
        assert_eq!(check(Rule::unary(Template::Init, A), traces).violated, 0);
        assert_eq!(check(Rule::unary(Template::End, D), traces).violated, 0);
        assert_eq!(
            check(Rule::unary(Template::AtMostOnce, A), traces).violated,
            1
        );
        assert_eq!(
            check(Rule::unary(Template::Existence, B), traces).violated,
            2
        );
        let not_both = check(Rule::binary(Template::NotCoexistence, B, C), traces);
        assert_eq!((not_both.activated, not_both.violated), (2, 0));
        let never_after = check(Rule::binary(Template::NotSuccession, D, A), traces);
        assert_eq!(never_after.violated, 0);
    }

    #[test]
    fn four_eyes_compares_the_performers() {
        let step = |activity, performer| Step {
            activity,
            performer: Some(performer),
        };
        let mut fold = RuleCheckFold::new(vec![Rule::binary(Template::DifferentPerformers, A, B)]);
        fold.trace(0, &[step(A, 7), step(B, 8)]);
        fold.trace(1, &[step(A, 7), step(B, 7)]);
        fold.trace(2, &[step(A, 7)]);
        let result = fold.finish().remove(0);
        assert_eq!((result.activated, result.violated), (2, 1));
        assert_eq!(result.examples, vec![1]);
    }

    /// The mined rules are the ones the planted process obeys: requests are
    /// always approved before ordering, and orders are always followed by a
    /// receipt; the rare rush order skips approval.
    #[test]
    fn mining_finds_the_planted_rules_and_not_the_broken_ones() {
        let mut fold = DeclareFold::new();
        for n in 0..200 {
            let t: &[u32] = if n % 50 == 0 {
                &[A, C, D]
            } else {
                &[A, B, C, D]
            };
            fold.trace(&trace(t));
        }
        let rules = fold.finish(&MiningSettings::default(), &|_| true);
        assert!(rules.contains(&Rule::unary(Template::Init, A)));
        assert!(rules.contains(&Rule::unary(Template::End, D)));
        assert!(rules.contains(&Rule::binary(Template::Succession, C, D)));
        // Approval is skipped in 2% of cases, which still clears 95%.
        assert!(rules.contains(&Rule::unary(Template::Existence, B)));
        // B is always followed by C, and C comes after B in 98% of cases.
        assert!(rules.contains(&Rule::binary(Template::Succession, B, C)));
        // D never comes before A.
        assert!(
            !rules
                .iter()
                .any(|r| r.template == Template::Precedence && r.a == D)
        );
    }

    #[test]
    fn mining_respects_the_confidence_threshold() {
        let mut fold = DeclareFold::new();
        for n in 0..100 {
            let t: &[u32] = if n % 5 == 0 { &[A, C] } else { &[A, B, C] };
            fold.trace(&trace(t));
        }
        // B is in 80% of cases: not an "every case" rule at 95%.
        let rules = fold.finish(&MiningSettings::default(), &|_| true);
        assert!(!rules.contains(&Rule::unary(Template::Existence, B)));
        let mut fold = DeclareFold::new();
        for n in 0..100 {
            let t: &[u32] = if n % 5 == 0 { &[A, C] } else { &[A, B, C] };
            fold.trace(&trace(t));
        }
        let loose = MiningSettings {
            min_confidence: 0.75,
            ..MiningSettings::default()
        };
        assert!(
            fold.finish(&loose, &|_| true)
                .contains(&Rule::unary(Template::Existence, B))
        );
    }
}
