//! Validation findings, and the decisions some of them demand.
//!
//! # The enforcement, not just the intention
//!
//! Some ambiguities in real data have no correct answer that software can
//! derive. Rather than pick a default and hope, the engine raises a
//! [`Finding`] carrying a [`Decision`]: the question, the options, the counts,
//! and — importantly — what goes wrong if the choice is made badly.
//!
//! Normalization then **refuses to run** until every open decision is
//! answered. That is what stops "report and ask" from decaying into "report,
//! and quietly proceed with a default anyway" the first time a deadline
//! presses.

use std::collections::BTreeMap;

/// How much attention a finding demands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Worth knowing; no action needed.
    Info,
    /// Likely to distort results; the user should look.
    Warning,
    /// Analysis cannot proceed correctly until the user chooses.
    Decision,
}

/// What was found.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The timestamp column reads validly as both day-first and month-first.
    AmbiguousDateOrder,
    /// The timestamp column contains more than one layout.
    MixedTimestampFormats,
    /// Some values could not be parsed by any known format.
    UnparseableTimestamps,
    /// Some timestamp values are blank.
    BlankTimestamps,
    /// Values declaring a UTC offset are mixed with values that do not.
    MixedZoneAndNaive,
    /// No timezone policy has been chosen.
    TimezonePolicyUnset,
    /// Rows identical in case, activity, and timestamp.
    DuplicateRows,
    /// Events within a case share a timestamp, so order is undetermined.
    TimestampTies,
    /// Cases containing exactly one event.
    SingleEventCases,
    /// Rows whose case identifier or activity label is blank.
    BlankRequiredValues,
    /// A case attribute took different values within one case.
    ConflictingCaseAttributes,
    /// A field's values within one case differ only in capitals or spaces.
    InconsistentFieldSpelling,
    /// File row order disagrees with chronological order.
    NonChronologicalFileOrder,
    /// Activity labels differing only by case or surrounding whitespace.
    NearDuplicateLabels,
    /// So many distinct activities that a process map will be unreadable.
    HighActivityCardinality,
}

impl Kind {
    /// A short human-readable title.
    pub fn title(&self) -> &'static str {
        match self {
            Kind::AmbiguousDateOrder => "Ambiguous date order",
            Kind::MixedTimestampFormats => "Mixed timestamp formats",
            Kind::UnparseableTimestamps => "Unreadable timestamps",
            Kind::BlankTimestamps => "Blank timestamps",
            Kind::MixedZoneAndNaive => "Mixed time zone information",
            Kind::TimezonePolicyUnset => "Time zone policy not set",
            Kind::DuplicateRows => "Duplicate rows",
            Kind::TimestampTies => "Tied timestamps",
            Kind::SingleEventCases => "Single-event cases",
            Kind::BlankRequiredValues => "Blank case ids or activity labels",
            Kind::ConflictingCaseAttributes => "Columns that change between steps",
            Kind::InconsistentFieldSpelling => "Field values spelled differently",
            Kind::NonChronologicalFileOrder => "Rows not in chronological order",
            Kind::NearDuplicateLabels => "Near-duplicate activity labels",
            Kind::HighActivityCardinality => "Very many distinct activities",
        }
    }
}

/// One option the user can choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Option_ {
    /// Stable identifier, recorded in the saved project. Never change these.
    pub id: &'static str,
    /// Short label for the interface.
    pub label: String,
    /// What choosing this actually does to the data.
    pub effect: String,
}

/// A question only the user can answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// The question, in plain language.
    pub question: String,
    /// The available answers. Always at least two.
    pub options: Vec<Option_>,
    /// What goes wrong if this is answered carelessly. Shown, not buried.
    pub risk: String,
}

impl Decision {
    /// Whether `id` is one of this decision's options.
    pub fn accepts(&self, id: &str) -> bool {
        self.options.iter().any(|o| o.id == id)
    }
}

/// Something validation observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: Kind,
    pub severity: Severity,
    /// Plain-language description including the counts.
    pub detail: String,
    /// Rows involved.
    pub affected_rows: usize,
    /// Cases involved.
    pub affected_cases: usize,
    /// A few real values from the data, for orientation.
    pub examples: Vec<String>,
    /// Present when the user must choose before analysis can proceed.
    pub decision: std::option::Option<Decision>,
}

impl Finding {
    /// Creates an informational finding.
    pub fn info(kind: Kind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            severity: Severity::Info,
            detail: detail.into(),
            affected_rows: 0,
            affected_cases: 0,
            examples: Vec::new(),
            decision: None,
        }
    }

    /// Creates a warning.
    pub fn warning(kind: Kind, detail: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::info(kind, detail)
        }
    }

    /// Creates a finding that blocks analysis until answered.
    pub fn decision(kind: Kind, detail: impl Into<String>, decision: Decision) -> Self {
        Self {
            severity: Severity::Decision,
            decision: Some(decision),
            ..Self::info(kind, detail)
        }
    }

    /// Attaches row and case counts.
    pub fn counts(mut self, rows: usize, cases: usize) -> Self {
        self.affected_rows = rows;
        self.affected_cases = cases;
        self
    }

    /// Attaches example values.
    pub fn examples(mut self, examples: Vec<String>) -> Self {
        self.examples = examples;
        self
    }

    /// Whether this finding blocks analysis until resolved.
    ///
    /// Determined by severity, not by carrying a decision: a `Warning` may
    /// offer a choice the user *can* make without forcing one. Only a
    /// `Decision` stops the analysis.
    pub fn is_blocking(&self) -> bool {
        self.severity == Severity::Decision && self.decision.is_some()
    }

    /// Attaches an optional choice to an advisory finding.
    ///
    /// The user may answer it, but analysis proceeds either way.
    pub fn offering(mut self, decision: Decision) -> Self {
        self.decision = Some(decision);
        self
    }

    /// Every finding that offers a choice, blocking or not.
    pub fn offers_choice(&self) -> bool {
        self.decision.is_some()
    }
}

/// Everything validation found about a log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    /// Adds a finding.
    pub fn push(&mut self, finding: Finding) {
        self.findings.push(finding);
    }

    /// Findings that require an answer, in a stable order.
    pub fn decisions(&self) -> Vec<&Finding> {
        let mut open: Vec<&Finding> = self.findings.iter().filter(|f| f.is_blocking()).collect();
        open.sort_by_key(|f| f.kind.clone());
        open
    }

    /// Whether anything blocks analysis.
    pub fn has_blocking(&self) -> bool {
        self.findings.iter().any(Finding::is_blocking)
    }

    /// Findings of at least `severity`, most severe first.
    pub fn at_least(&self, severity: Severity) -> Vec<&Finding> {
        let mut out: Vec<&Finding> = self
            .findings
            .iter()
            .filter(|f| f.severity >= severity)
            .collect();
        out.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.kind.cmp(&b.kind)));
        out
    }
}

/// The user's answers to the decisions a report raised.
///
/// Recorded with the saved project, so reopening reproduces the same results
/// without asking again.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Policy {
    answers: BTreeMap<Kind, String>,
}

/// Why normalization refused to run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("unanswered decision: {0}")]
    Unanswered(&'static str),
    #[error("'{answer}' is not a valid answer for {question}")]
    InvalidAnswer {
        question: &'static str,
        answer: String,
    },
}

impl Policy {
    /// Creates an empty policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an answer.
    pub fn answer(mut self, kind: Kind, option_id: impl Into<String>) -> Self {
        self.answers.insert(kind, option_id.into());
        self
    }

    /// The recorded answer for `kind`, if any.
    pub fn get(&self, kind: &Kind) -> std::option::Option<&str> {
        self.answers.get(kind).map(String::as_str)
    }

    /// Checks this policy against a report.
    ///
    /// Returns every reason normalization cannot proceed. An empty result
    /// means every decision has a valid answer.
    ///
    /// This is the gate that makes "never guess" structural: without a
    /// complete policy there is no path to a normalized log.
    pub fn validate_against(&self, report: &Report) -> Vec<PolicyError> {
        let mut problems = Vec::new();
        for finding in report.decisions() {
            let decision = finding
                .decision
                .as_ref()
                .expect("decisions() only yields findings with a decision");
            match self.get(&finding.kind) {
                None => problems.push(PolicyError::Unanswered(finding.kind.title())),
                Some(answer) if !decision.accepts(answer) => {
                    problems.push(PolicyError::InvalidAnswer {
                        question: finding.kind.title(),
                        answer: answer.to_owned(),
                    })
                }
                Some(_) => {}
            }
        }
        problems
    }

    /// Whether this policy fully resolves `report`.
    pub fn resolves(&self, report: &Report) -> bool {
        self.validate_against(report).is_empty()
    }
}

// ---------------------------------------------------------------------------
// The four decisions the canonical test logs expose.
// ---------------------------------------------------------------------------

/// Blank timestamps: did the step not happen, or is the data missing?
pub fn blank_timestamp_decision() -> Decision {
    Decision {
        question: "Some events have no timestamp. What does a blank mean here?".to_owned(),
        options: vec![
            Option_ {
                id: "did_not_happen",
                label: "The step did not happen".to_owned(),
                effect: "The event is excluded. The case is treated as complete and shorter."
                    .to_owned(),
            },
            Option_ {
                id: "missing_data",
                label: "The data is missing".to_owned(),
                effect: "The event is excluded but the case is flagged as incomplete and \
                         excluded from duration statistics."
                    .to_owned(),
            },
        ],
        risk: "Treating missing data as 'did not happen' makes cases look faster and simpler \
               than they are, and hides steps that did occur."
            .to_owned(),
    }
}

/// Duplicate rows: extraction artifact, or genuine repetition?
pub fn duplicate_rows_decision() -> Decision {
    Decision {
        question: "Some rows are identical in case, activity, and timestamp. Are these \
                   duplicates from the export, or did the step really repeat?"
            .to_owned(),
        options: vec![
            Option_ {
                id: "deduplicate",
                label: "Export artifacts — remove them".to_owned(),
                effect: "Identical rows collapse to one event.".to_owned(),
            },
            Option_ {
                id: "keep_as_rework",
                label: "Real repetition — keep them".to_owned(),
                effect: "Each row stays, producing self-loops and counting toward rework."
                    .to_owned(),
            },
        ],
        risk: "Removing them silently erases genuine rework. Keeping them silently invents \
               rework that never happened. Repetition at the exact same second is usually an \
               artifact; repetition minutes apart is usually real."
            .to_owned(),
    }
}

/// Ambiguous date order: day-first or month-first?
pub fn date_order_decision(example: &str, day_first: &str, month_first: &str) -> Decision {
    Decision {
        question: format!("Dates like {example} could be read two ways. Which is correct?"),
        options: vec![
            Option_ {
                id: "day_first",
                label: format!("Day first — {example} means {day_first}"),
                effect: "Common outside the United States.".to_owned(),
            },
            Option_ {
                id: "month_first",
                label: format!("Month first — {example} means {month_first}"),
                effect: "Common in the United States.".to_owned(),
            },
        ],
        risk: "Choosing wrong shifts dates by months. Every duration, bottleneck, and \
               throughput figure becomes wrong while still looking entirely plausible."
            .to_owned(),
    }
}

/// Activity labels differing only by case or whitespace: merge, or keep apart?
///
/// Offered rather than required. Keeping them distinct preserves the data
/// exactly as recorded, which is the safe default — but it does split
/// frequencies and invent variants, so the user should be able to say so.
pub fn label_merge_decision() -> Decision {
    Decision {
        question: "Some activity labels differ only by capitalisation or surrounding spaces. \
                   Are they the same activity?"
            .to_owned(),
        options: vec![
            Option_ {
                id: "keep_distinct",
                label: "Keep them separate".to_owned(),
                effect: "Labels are used exactly as recorded. This is the default: nothing is \
                         changed without your say-so."
                    .to_owned(),
            },
            Option_ {
                id: "merge",
                label: "Treat them as one activity".to_owned(),
                effect: "Labels matching after trimming spaces and ignoring case are combined, \
                         displayed under whichever spelling occurs most often."
                    .to_owned(),
            },
        ],
        risk: "Kept separate, one real activity appears as several — splitting its frequency, \
               inflating the variant count, and showing rework that is only a spelling \
               difference. Merged, a genuine distinction between two similarly named \
               activities would be lost."
            .to_owned(),
    }
}

/// Which timezone policy applies to naive timestamps?
pub fn timezone_policy_decision() -> Decision {
    Decision {
        question: "Timestamps carry no time zone. How should they be interpreted?".to_owned(),
        options: vec![
            Option_ {
                id: "as_written",
                label: "Compare wall-clock values as written".to_owned(),
                effect: "Numeric offsets are ignored when mixed with naive values. For example, \
                         09:00+02:00 is compared as 09:00. This preserves clock readings but not \
                         real elapsed time across zones or daylight-saving changes."
                    .to_owned(),
            },
            Option_ {
                id: "utc",
                label: "They are UTC".to_owned(),
                effect: "No daylight-saving effects. Correct for most system-generated logs."
                    .to_owned(),
            },
        ],
        risk: "Around daylight-saving changes, clock arithmetic and real elapsed time differ \
               by an hour, and one local hour occurs twice each autumn."
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_with_two_decisions() -> Report {
        let mut r = Report::default();
        r.push(Finding::decision(
            Kind::BlankTimestamps,
            "4 events have no timestamp",
            blank_timestamp_decision(),
        ));
        r.push(Finding::decision(
            Kind::DuplicateRows,
            "3 duplicate rows",
            duplicate_rows_decision(),
        ));
        r.push(Finding::warning(Kind::TimestampTies, "3 cases have ties"));
        r
    }

    #[test]
    fn separates_blocking_from_advisory_findings() {
        let r = report_with_two_decisions();
        assert_eq!(r.decisions().len(), 2);
        assert!(r.has_blocking());
    }

    /// The central guarantee: an empty policy cannot proceed.
    #[test]
    fn empty_policy_does_not_resolve_a_report_with_decisions() {
        let r = report_with_two_decisions();
        let problems = Policy::new().validate_against(&r);
        assert_eq!(problems.len(), 2);
        assert!(!Policy::new().resolves(&r));
    }

    #[test]
    fn partial_policy_still_blocks() {
        let r = report_with_two_decisions();
        let p = Policy::new().answer(Kind::BlankTimestamps, "did_not_happen");
        assert_eq!(p.validate_against(&r).len(), 1);
        assert!(!p.resolves(&r));
    }

    #[test]
    fn complete_policy_resolves() {
        let r = report_with_two_decisions();
        let p = Policy::new()
            .answer(Kind::BlankTimestamps, "did_not_happen")
            .answer(Kind::DuplicateRows, "deduplicate");
        assert!(p.resolves(&r), "{:?}", p.validate_against(&r));
    }

    /// A typo or a stale saved answer must not slip through as valid.
    #[test]
    fn rejects_an_answer_that_is_not_an_offered_option() {
        let r = report_with_two_decisions();
        let p = Policy::new()
            .answer(Kind::BlankTimestamps, "whatever_seems_fine")
            .answer(Kind::DuplicateRows, "deduplicate");
        let problems = p.validate_against(&r);
        assert_eq!(problems.len(), 1);
        assert!(matches!(problems[0], PolicyError::InvalidAnswer { .. }));
    }

    #[test]
    fn a_report_without_decisions_needs_no_policy() {
        let mut r = Report::default();
        r.push(Finding::warning(Kind::SingleEventCases, "3 of 5 cases"));
        assert!(!r.has_blocking());
        assert!(Policy::new().resolves(&r));
    }

    #[test]
    fn decisions_are_listed_in_a_stable_order() {
        let r = report_with_two_decisions();
        let first: Vec<Kind> = r.decisions().iter().map(|f| f.kind.clone()).collect();
        let second: Vec<Kind> = r.decisions().iter().map(|f| f.kind.clone()).collect();
        assert_eq!(first, second);
    }

    #[test]
    fn filters_by_severity_most_severe_first() {
        let r = report_with_two_decisions();
        let severe = r.at_least(Severity::Warning);
        assert_eq!(severe.len(), 3);
        assert_eq!(severe[0].severity, Severity::Decision);
        assert_eq!(severe.last().unwrap().severity, Severity::Warning);
    }

    /// Every decision must offer a real choice and say what going wrong costs.
    #[test]
    fn every_decision_is_actionable() {
        let decisions = [
            blank_timestamp_decision(),
            duplicate_rows_decision(),
            date_order_decision("05/01/2026", "5 January 2026", "1 May 2026"),
            timezone_policy_decision(),
        ];
        for d in decisions {
            assert!(d.options.len() >= 2, "a decision needs a real choice");
            assert!(
                !d.risk.is_empty(),
                "a decision must state what can go wrong"
            );
            assert!(!d.question.is_empty());
            for o in &d.options {
                assert!(!o.id.is_empty() && !o.label.is_empty() && !o.effect.is_empty());
            }
        }
    }

    #[test]
    fn option_ids_within_a_decision_are_unique() {
        for d in [
            blank_timestamp_decision(),
            duplicate_rows_decision(),
            timezone_policy_decision(),
        ] {
            let ids: std::collections::BTreeSet<&str> = d.options.iter().map(|o| o.id).collect();
            assert_eq!(ids.len(), d.options.len());
        }
    }

    #[test]
    fn date_order_decision_shows_both_readings() {
        let d = date_order_decision("05/01/2026", "5 January 2026", "1 May 2026");
        assert!(d.options[0].label.contains("5 January 2026"));
        assert!(d.options[1].label.contains("1 May 2026"));
        assert!(d.accepts("day_first") && d.accepts("month_first"));
        assert!(!d.accepts("whichever"));
    }

    #[test]
    fn timezone_decision_only_offers_policies_the_engine_can_apply() {
        let d = timezone_policy_decision();
        assert!(d.accepts("as_written"));
        assert!(d.accepts("utc"));
        assert!(!d.accepts("local_with_dst"));
    }
}
