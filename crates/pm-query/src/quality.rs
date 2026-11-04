//! Structured data-quality reporting.
//!
//! Validation already knows where the source is ambiguous or unreliable. This
//! module turns those findings into a stable, serializable query result so the
//! CLI, browser, and MCP surface can present the same facts and decisions.

use pm_ingest::{Mapping, Policy, Report, Severity};
use serde::Serialize;

/// Attention level of a data-quality finding.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QualitySeverity {
    Information,
    Warning,
    Decision,
}

impl From<Severity> for QualitySeverity {
    fn from(value: Severity) -> Self {
        match value {
            Severity::Info => Self::Information,
            Severity::Warning => Self::Warning,
            Severity::Decision => Self::Decision,
        }
    }
}

/// One available answer to a data interpretation decision.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QualityOption {
    pub id: String,
    pub label: String,
    pub effect: String,
}

/// A choice associated with a finding.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QualityDecision {
    pub question: String,
    pub options: Vec<QualityOption>,
    pub risk: String,
    /// Answer supplied by the current policy, if any.
    pub answer: Option<String>,
    /// Whether the supplied answer is one of the offered options.
    pub answered: bool,
    /// Whether analysis requires an answer before it can proceed.
    pub required: bool,
}

/// One validation finding with evidence and actionable decision metadata.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QualityFinding {
    pub kind: pm_ingest::Kind,
    pub title: String,
    pub severity: QualitySeverity,
    pub detail: String,
    pub affected_rows: usize,
    pub affected_cases: usize,
    pub examples: Vec<String>,
    pub decision: Option<QualityDecision>,
}

/// Counts for quickly judging the state of a source.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct QualitySummary {
    pub findings: usize,
    pub decisions: usize,
    pub open_decisions: usize,
    pub warnings: usize,
    pub information: usize,
}

/// First-class data-quality result for one source and mapping.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DataQualityReport {
    pub source: String,
    pub mapping: Mapping,
    /// True only when every required interpretation decision has a valid answer.
    pub ready_for_analysis: bool,
    pub summary: QualitySummary,
    pub findings: Vec<QualityFinding>,
}

impl DataQualityReport {
    pub(crate) fn from_validation(
        source: String,
        mapping: Mapping,
        report: &Report,
        policy: &Policy,
    ) -> Self {
        let findings: Vec<QualityFinding> = report
            .at_least(Severity::Info)
            .into_iter()
            .map(|finding| {
                let decision = finding.decision.as_ref().map(|decision| {
                    let answer = policy.get(&finding.kind).map(str::to_owned);
                    let answered = answer
                        .as_deref()
                        .is_some_and(|answer| decision.accepts(answer));
                    QualityDecision {
                        question: decision.question.clone(),
                        options: decision
                            .options
                            .iter()
                            .map(|option| QualityOption {
                                id: option.id.to_owned(),
                                label: option.label.clone(),
                                effect: option.effect.clone(),
                            })
                            .collect(),
                        risk: decision.risk.clone(),
                        answer,
                        answered,
                        required: finding.is_blocking(),
                    }
                });
                QualityFinding {
                    kind: finding.kind.clone(),
                    title: finding.kind.title().to_owned(),
                    severity: finding.severity.into(),
                    detail: finding.detail.clone(),
                    affected_rows: finding.affected_rows,
                    affected_cases: finding.affected_cases,
                    examples: finding.examples.clone(),
                    decision,
                }
            })
            .collect();

        let mut summary = QualitySummary {
            findings: findings.len(),
            ..Default::default()
        };
        for finding in &findings {
            match finding.severity {
                QualitySeverity::Information => summary.information += 1,
                QualitySeverity::Warning => summary.warnings += 1,
                QualitySeverity::Decision => summary.decisions += 1,
            }
            if finding
                .decision
                .as_ref()
                .is_some_and(|decision| decision.required && !decision.answered)
            {
                summary.open_decisions += 1;
            }
        }

        Self {
            source,
            mapping,
            ready_for_analysis: policy.resolves(report),
            summary,
            findings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_ingest::finding::blank_timestamp_decision;
    use pm_ingest::{Finding, Kind};

    fn report() -> Report {
        let mut report = Report::default();
        report.push(
            Finding::decision(
                Kind::BlankTimestamps,
                "one timestamp is blank",
                blank_timestamp_decision(),
            )
            .counts(1, 1)
            .examples(vec!["row 3: Review".to_owned()]),
        );
        report.push(Finding::warning(
            Kind::HighActivityCardinality,
            "the map may be unreadable",
        ));
        report
    }

    #[test]
    fn reports_open_decisions_and_evidence() {
        let quality = DataQualityReport::from_validation(
            "events.csv".to_owned(),
            Mapping::new("case", "activity", "time"),
            &report(),
            &Policy::new(),
        );
        assert!(!quality.ready_for_analysis);
        assert_eq!(quality.summary.findings, 2);
        assert_eq!(quality.summary.open_decisions, 1);
        assert_eq!(quality.summary.warnings, 1);
        assert_eq!(quality.findings[0].examples, vec!["row 3: Review"]);
    }

    #[test]
    fn records_when_a_required_decision_was_answered() {
        let policy = Policy::new().answer(Kind::BlankTimestamps, "missing_data");
        let quality = DataQualityReport::from_validation(
            "events.csv".to_owned(),
            Mapping::new("case", "activity", "time"),
            &report(),
            &policy,
        );
        assert!(quality.ready_for_analysis);
        assert_eq!(quality.summary.open_decisions, 0);
        let decision = quality.findings[0].decision.as_ref().unwrap();
        assert_eq!(decision.answer.as_deref(), Some("missing_data"));
        assert!(decision.answered);
    }
}
