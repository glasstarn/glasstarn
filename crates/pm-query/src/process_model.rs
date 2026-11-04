//! Agent-authored process understanding, separate from measured event history.
//!
//! This bounded graph records provenance, uncertainty and proposed treatment.
//! It is not an event log, approved control baseline or executable workflow.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intention {
    Understand,
    Improve,
    Check,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessView {
    CombinedCurrent,
    ProposedFuture,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessModel {
    pub version: u32,
    pub title: String,
    pub view: ProcessView,
    /// Previous immutable investigation artifact, filled by the record writer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default)]
    pub sources: Vec<ProcessSource>,
    pub steps: Vec<ProcessStep>,
    pub connections: Vec<ProcessConnection>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub improvement: Option<ImprovementPlan>,
}

/// A reviewable redesign, never a deployment instruction or measured benefit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImprovementPlan {
    pub objective: String,
    pub changes: Vec<ProcessChange>,
    pub pilot: PilotPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessChange {
    pub title: String,
    /// Steps in this model whose evidence and treatment support the proposal.
    pub steps: Vec<String>,
    pub action: ChangeAction,
    pub rationale: String,
    #[serde(default)]
    pub alternatives: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    Remove,
    Simplify,
    Combine,
    Retain,
    Automate,
    Assist,
    Reorganize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PilotPlan {
    pub owner: String,
    pub scope: String,
    pub success_measures: Vec<String>,
    pub safeguards: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSource {
    pub id: String,
    pub kind: SourceKind,
    pub title: String,
    /// Version/date, or explicitly "unknown"; not a document-upload approval.
    pub revision: String,
    /// Document locator, or attribution of a supplied employee account.
    pub reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Document,
    ReportedPractice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessStep {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub evidence: Vec<ProcessEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<ProcessAssessment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConnection {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    pub evidence: Vec<ProcessEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessEvidence {
    /// Requires a current saved map query containing this exact step/edge.
    Observed {
        query: String,
    },
    Documented {
        source: String,
        section: String,
    },
    Reported {
        source: String,
    },
    Proposed {
        rationale: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessAssessment {
    pub treatment: Treatment,
    pub rationale: String,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Treatment {
    HumanLed,
    RulesBased,
    AgentAssisted,
    AgentExecuted,
}

fn text(value: &str, limit: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > limit {
        return Err(format!("process text must contain 1–{limit} characters"));
    }
    Ok(())
}
fn optional(value: &Option<String>, limit: usize) -> Result<(), String> {
    value.as_deref().map_or(Ok(()), |value| text(value, limit))
}
fn identifier(value: &str) -> Result<(), String> {
    text(value, 80)?;
    if !value
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
    {
        return Err("process identifiers use letters, numbers, underscores and hyphens".into());
    }
    Ok(())
}
fn texts(values: &[String]) -> Result<(), String> {
    if values.len() > 20 {
        return Err("use at most 20 process questions or prerequisites".into());
    }
    values.iter().try_for_each(|value| text(value, 600))
}

impl ProcessModel {
    /// References to deterministic evidence; the caller also validates identity
    /// and exact saved scope before supplying snapshots to validate().
    pub fn queries(&self) -> impl Iterator<Item = &String> {
        self.steps
            .iter()
            .flat_map(|step| &step.evidence)
            .chain(self.connections.iter().flat_map(|edge| &edge.evidence))
            .filter_map(|item| match item {
                ProcessEvidence::Observed { query } => Some(query),
                _ => None,
            })
    }

    pub fn validate(&self, queries: &BTreeMap<String, Value>) -> Result<(), String> {
        if self.version != 1
            || self.steps.is_empty()
            || self.steps.len() > 40
            || self.sources.len() > 20
            || self.connections.len() > 100
        {
            return Err(
                "process model needs version 1, 1–40 steps, at most 20 sources and 100 connections"
                    .into(),
            );
        }
        text(&self.title, 160)?;
        texts(&self.open_questions)?;
        let mut sources = BTreeMap::new();
        for source in &self.sources {
            identifier(&source.id)?;
            text(&source.title, 200)?;
            text(&source.revision, 100)?;
            text(&source.reference, 600)?;
            if sources.insert(source.id.as_str(), &source.kind).is_some() {
                return Err("duplicate process source identifier".into());
            }
        }
        let mut steps = BTreeMap::new();
        for step in &self.steps {
            identifier(&step.id)?;
            text(&step.label, 160)?;
            optional(&step.owner, 200)?;
            if steps.insert(step.id.as_str(), step).is_some() {
                return Err("duplicate process step identifier".into());
            }
            if let Some(assessment) = &step.assessment {
                text(&assessment.rationale, 1000)?;
                texts(&assessment.prerequisites)?;
                optional(&assessment.escalation, 600)?;
                optional(&assessment.validation, 600)?;
                if matches!(assessment.treatment, Treatment::AgentExecuted)
                    && (assessment.prerequisites.is_empty()
                        || assessment.escalation.is_none()
                        || assessment.validation.is_none())
                {
                    return Err("agent-executed proposals need prerequisites, escalation and a validation plan".into());
                }
            }
        }
        if let Some(plan) = &self.improvement {
            text(&plan.objective, 600)?;
            if plan.changes.is_empty() || plan.changes.len() > 20 {
                return Err("improvement needs 1–20 changes grounded in model steps".into());
            }
            for change in &plan.changes {
                text(&change.title, 160)?;
                text(&change.rationale, 1000)?;
                texts(&change.alternatives)?;
                if change.steps.is_empty()
                    || change.steps.len() > 40
                    || change
                        .steps
                        .iter()
                        .any(|id| !steps.contains_key(id.as_str()))
                {
                    return Err("improvement changes must reference existing process steps".into());
                }
            }
            text(&plan.pilot.owner, 200)?;
            text(&plan.pilot.scope, 600)?;
            texts(&plan.pilot.success_measures)?;
            texts(&plan.pilot.safeguards)?;
            if plan.pilot.success_measures.is_empty() || plan.pilot.safeguards.is_empty() {
                return Err("a pilot needs success measures and safeguards".into());
            }
        }
        let check = |evidence: &[ProcessEvidence],
                     from: &str,
                     to: Option<&str>|
         -> Result<(), String> {
            if evidence.is_empty() || evidence.len() > 8 {
                return Err(
                    "each process step and connection needs 1–8 evidence references".into(),
                );
            }
            for item in evidence {
                match item {
                    ProcessEvidence::Observed { query } => {
                        let snapshot = queries
                            .get(query)
                            .ok_or("process evidence query was not resolved")?;
                        let result = match snapshot["request"]["type"].as_str() {
                            Some("map") => &snapshot["result"],
                            Some("follow") if snapshot["request"]["detail"] == "full" => {
                                let analysis = &snapshot["result"]["analysis"];
                                if to.is_some() && analysis["mapping"]["object_history"] == true {
                                    return Err("object-history adjacency does not establish a business-process connection; document or report that relationship".into());
                                }
                                analysis
                            }
                            _ => return Err("observed process evidence requires a saved map or full followed analysis".into()),
                        };
                        let matches = if let Some(to) = to {
                            result["edges"].as_array().is_some_and(|edges| {
                                edges
                                    .iter()
                                    .any(|edge| edge["from"] == from && edge["to"] == to)
                            })
                        } else {
                            result["nodes"].as_array().is_some_and(|nodes| {
                                nodes.iter().any(|node| node["activity"] == from)
                            })
                        };
                        if !matches {
                            return Err("observed process step or connection is absent from its saved map evidence".into());
                        }
                    }
                    ProcessEvidence::Documented { source, section } => {
                        text(section, 600)?;
                        if sources.get(source.as_str()) != Some(&&SourceKind::Document) {
                            return Err(
                                "documented process evidence needs a document source".into()
                            );
                        }
                    }
                    ProcessEvidence::Reported { source } => {
                        if sources.get(source.as_str()) != Some(&&SourceKind::ReportedPractice) {
                            return Err(
                                "reported process evidence needs an attributed practice source"
                                    .into(),
                            );
                        }
                    }
                    ProcessEvidence::Proposed { rationale } => text(rationale, 1000)?,
                }
            }
            Ok(())
        };
        for step in &self.steps {
            check(&step.evidence, &step.label, None)?;
        }
        let mut seen = BTreeSet::new();
        for edge in &self.connections {
            let from = steps
                .get(edge.from.as_str())
                .ok_or("unknown process connection source")?;
            let to = steps
                .get(edge.to.as_str())
                .ok_or("unknown process connection destination")?;
            optional(&edge.condition, 300)?;
            if !seen.insert((&edge.from, &edge.to)) {
                return Err("duplicate process connection".into());
            }
            check(&edge.evidence, &from.label, Some(&to.label))?;
        }
        Ok(())
    }
}
