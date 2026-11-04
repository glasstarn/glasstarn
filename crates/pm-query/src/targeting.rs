//! Automation targeting, as the front ends see it.
//!
//! Two questions, in order:
//!
//! 1. **Which accounts are machines?** Nobody can measure the boundary between
//!    system and human work until that is settled, so [`resource_report`]
//!    describes every account and leaves the decision to a person or an
//!    assistant.
//! 2. **Where is the manual work, and how repeatable is it?**
//!    [`automation_report`] answers that under a settled classification.
//!
//! The ordering is not a convenience. Running the second without the first
//! produces numbers that look authoritative and rest on a name-matching guess.

use pm_core::automation::{Automation, RankingBasis, ResourceKind, profile_resources, suggest};
use pm_core::log::{EventLog, Micros};
use pm_ingest::Rules;
use pm_ingest::finding::Policy;
use pm_ingest::validate::Mapping;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::analysis::QueryError;

const MICROS_PER_SECOND: Micros = 1_000_000;

/// What is known about one account, and what it looks like.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceSummary {
    pub resource: String,
    pub events: u64,
    pub cases: u64,
    /// Distinct activities performed — a breadth measure, not a skill measure.
    pub activities: usize,
    /// Times it began a case. Machine accounts often start everything.
    pub starts_cases: u64,
    /// Times it took over directly from a different account.
    pub takes_over: u64,
    /// What the name suggests, and why.
    pub suggested: ResourceKind,
    pub suggested_because: String,
    /// What the user has actually decided, if anything.
    pub decided: Option<ResourceKind>,
}

/// Every account, for classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceReport {
    pub resources: Vec<ResourceSummary>,
    /// Accounts with no decision recorded yet.
    pub undecided: usize,
    /// Whether a resource column was mapped at all.
    pub resources_mapped: bool,
}

impl ResourceReport {
    /// Whether every account has been decided.
    pub fn is_settled(&self) -> bool {
        self.resources_mapped && self.undecided == 0
    }
}

/// One manual step, sized and scored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManualStep {
    pub activity: String,
    pub occurrences: u64,
    pub cases: u64,
    pub processing_seconds: i64,
    pub processing_observations: u64,
    pub waiting_before_seconds: i64,
    pub waiting_observations: u64,
    pub start_to_start_before_seconds: i64,
    pub start_to_start_observations: u64,
    pub total_cost_microunits: i128,
    /// Simpson concentration of predecessor/successor contexts, 0–100. This
    /// says nothing about consistency of task inputs or decisions.
    pub routing_context_concentration: f64,
    pub distinct_contexts: usize,
    pub routing_contexts: Vec<RoutingContext>,
    pub example_cases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingContext {
    pub before: Option<String>,
    pub after: Option<String>,
    pub occurrences: u64,
    pub share_percent: f64,
}

/// Where cases leave the dominant path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Divergence {
    pub after: Option<String>,
    /// Next activity in the most frequent observed variant. This is not an
    /// approved or normative process model.
    pub dominant_path_next: Option<String>,
    pub took: Option<String>,
    pub cases: u64,
    pub example_cases: Vec<String>,
}

/// Where the machine stops and people start.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutomationReport {
    pub source: String,
    /// Share of cases no person touched. Absent when unknowable.
    pub straight_through_percent: Option<f64>,
    pub straight_through_cases: usize,
    pub touched_cases: usize,
    pub undetermined_cases: usize,
    pub classification_coverage_percent: f64,
    pub manual_events: u64,
    pub system_events: u64,
    pub unrecorded_events: u64,
    pub manual_processing_seconds: i64,
    pub waiting_before_manual_seconds: i64,
    pub start_to_start_before_manual_seconds: i64,
    pub ranking_basis: RankingBasis,
    /// Manual steps ordered by processing time × routing concentration when
    /// complete processing durations exist, otherwise occurrences × routing
    /// concentration.
    ///
    /// **A heuristic ordering, not a recommendation.** Whether any of these is
    /// worth automating depends on effort, risk and regulation, none of which
    /// are in the log.
    pub manual_steps_ranked: Vec<ManualStep>,
    pub divergences: Vec<Divergence>,
    pub dominant_variant_cases: usize,
    pub dominant_variant_percent: f64,
    /// Accounts counted as human because nothing said otherwise. The margin of
    /// error on every figure above.
    pub unclassified_resources: Vec<String>,
    /// Steps done by those accounts. They are inside `manual_events`, but
    /// nobody has said whether a person did them: "not yet classified".
    #[serde(default)]
    pub unclassified_events: u64,
    /// Undecided accounts whose name looks like a service account, such as
    /// `batch_07`. A suggestion for a person to confirm, never applied.
    #[serde(default)]
    pub suggested_system_resources: Vec<String>,
    pub resources_mapped: bool,
    /// Stable constraints an agent must preserve when interpreting this
    /// deterministic result.
    pub interpretation_limits: Vec<String>,
}

impl AutomationReport {
    /// Whether the figures rest on a complete classification.
    pub fn is_settled(&self) -> bool {
        self.resources_mapped && self.unclassified_resources.is_empty()
    }
}

/// Describes every account in a file.
pub fn resource_report(
    path: impl AsRef<Path>,
    mapping: Option<Mapping>,
    policy: &Policy,
    rules: Rules,
) -> Result<Result<ResourceReport, Box<crate::analysis::Blocked>>, QueryError> {
    let log = match crate::load_log(path, mapping, policy, rules.clone())? {
        Ok(log) => log,
        Err(blocked) => return Ok(Err(blocked)),
    };
    Ok(Ok(build_resource_report(&log.log, &rules)))
}

/// Builds the account report from a loaded log.
pub fn build_resource_report(log: &EventLog, rules: &Rules) -> ResourceReport {
    let resources: Vec<ResourceSummary> = profile_resources(log)
        .into_iter()
        .map(|p| ResourceSummary {
            decided: rules.resource_kinds.get(&p.label).copied(),
            resource: p.label,
            events: p.events,
            cases: p.cases,
            activities: p.activities,
            starts_cases: p.starts,
            takes_over: p.receives,
            suggested: p.suggested,
            suggested_because: p.suggested_because,
        })
        .collect();

    ResourceReport {
        undecided: resources.iter().filter(|r| r.decided.is_none()).count(),
        resources_mapped: log.has_resources(),
        resources,
    }
}

/// Locates and sizes the manual work in a file.
pub fn automation_report(
    path: impl AsRef<Path>,
    mapping: Option<Mapping>,
    policy: &Policy,
    rules: Rules,
) -> Result<Result<AutomationReport, Box<crate::analysis::Blocked>>, QueryError> {
    let path = path.as_ref();
    let source = path.display().to_string();
    let log = match crate::load_log(path, mapping, policy, rules.clone())? {
        Ok(log) => log,
        Err(blocked) => return Ok(Err(blocked)),
    };
    Ok(Ok(build_automation_report(&log.log, &rules, &source)))
}

/// Builds the automation report from a loaded log.
pub fn build_automation_report(log: &EventLog, rules: &Rules, source: &str) -> AutomationReport {
    let view = Automation::build(log, &rules.classification());
    let total_cases = log.case_count();
    let classified_cases = view.straight_through_cases + view.touched_cases;
    let undecided: std::collections::BTreeSet<&str> =
        view.unclassified.iter().map(String::as_str).collect();
    let unclassified_events = log
        .traces
        .iter()
        .flat_map(|trace| &trace.events)
        .filter_map(|event| event.resource())
        .filter(|r| {
            log.resources
                .label(r.0)
                .is_some_and(|label| undecided.contains(label))
        })
        .count() as u64;
    let suggested_system_resources = view
        .unclassified
        .iter()
        .filter(|label| suggest(label).0 == ResourceKind::System)
        .cloned()
        .collect();

    AutomationReport {
        source: source.to_owned(),
        straight_through_percent: view.straight_through_percent(),
        straight_through_cases: view.straight_through_cases,
        touched_cases: view.touched_cases,
        undetermined_cases: view.undetermined_cases,
        classification_coverage_percent: if total_cases == 0 {
            0.0
        } else {
            classified_cases as f64 / total_cases as f64 * 100.0
        },
        manual_events: view.manual_events,
        system_events: view.system_events,
        unrecorded_events: view.unrecorded_events,
        manual_processing_seconds: view.total_processing_time() / MICROS_PER_SECOND,
        waiting_before_manual_seconds: view.total_waiting_before() / MICROS_PER_SECOND,
        start_to_start_before_manual_seconds: view.total_start_to_start_before()
            / MICROS_PER_SECOND,
        ranking_basis: view.ranking_basis,
        manual_steps_ranked: view
            .opportunities
            .iter()
            .take(20)
            .map(|o| ManualStep {
                activity: log.activity_label(o.activity).to_owned(),
                occurrences: o.occurrences,
                cases: o.cases,
                processing_seconds: o.total_processing / MICROS_PER_SECOND,
                processing_observations: o.processing_observations,
                waiting_before_seconds: o.total_waiting_before / MICROS_PER_SECOND,
                waiting_observations: o.waiting_observations,
                start_to_start_before_seconds: o.total_start_to_start_before / MICROS_PER_SECOND,
                start_to_start_observations: o.start_to_start_observations,
                total_cost_microunits: o.total_cost,
                routing_context_concentration: o.routing_context_concentration,
                distinct_contexts: o.distinct_contexts,
                routing_contexts: o
                    .contexts
                    .iter()
                    .take(5)
                    .map(|context| RoutingContext {
                        before: context.before.map(|a| log.activity_label(a).to_owned()),
                        after: context.after.map(|a| log.activity_label(a).to_owned()),
                        occurrences: context.occurrences,
                        share_percent: context.share_percent,
                    })
                    .collect(),
                example_cases: o
                    .example_cases
                    .iter()
                    .map(|c| log.case_label(*c).to_owned())
                    .collect(),
            })
            .collect(),
        divergences: view
            .break_points
            .iter()
            .take(10)
            .map(|b| Divergence {
                after: b.after.map(|a| log.activity_label(a).to_owned()),
                dominant_path_next: b.instead_of.map(|a| log.activity_label(a).to_owned()),
                took: b.took.map(|a| log.activity_label(a).to_owned()),
                cases: b.cases,
                example_cases: b
                    .example_cases
                    .iter()
                    .map(|c| log.case_label(*c).to_owned())
                    .collect(),
            })
            .collect(),
        dominant_variant_cases: view.dominant_variant_cases,
        dominant_variant_percent: if total_cases == 0 {
            0.0
        } else {
            view.dominant_variant_cases as f64 / total_cases as f64 * 100.0
        },
        unclassified_events,
        suggested_system_resources,
        unclassified_resources: view.unclassified,
        resources_mapped: view.resources_known,
        interpretation_limits: vec![
            "Routing-context concentration does not establish that task inputs or decisions are rules-based."
                .to_owned(),
            "High routing concentration from only a few observations is weak evidence; inspect occurrence counts and representative cases."
                .to_owned(),
            "Waiting before a manual step is separate from the step's processing time and is not manual effort."
                .to_owned(),
            "The dominant observed variant is not an approved or compliant process model."
                .to_owned(),
            "Automation suitability still requires task inputs, decision rules, exceptions, risk, regulation, and implementation effort."
                .to_owned(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("pm_targeting_{name}.csv"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    /// Two cases run straight through; two need a person to fix data.
    const LOG: &str = "case_id,activity,timestamp,resource\n\
        C1,Receive,2026-03-02 09:00:00,SYSTEM\n\
        C1,Post,2026-03-02 10:00:00,WF-BATCH\n\
        C2,Receive,2026-03-02 09:00:00,SYSTEM\n\
        C2,Post,2026-03-02 10:00:00,WF-BATCH\n\
        C3,Receive,2026-03-02 09:00:00,SYSTEM\n\
        C3,Fix Data,2026-03-02 13:00:00,alice\n\
        C3,Post,2026-03-02 14:00:00,WF-BATCH\n\
        C4,Receive,2026-03-02 09:00:00,SYSTEM\n\
        C4,Fix Data,2026-03-02 15:00:00,bob\n\
        C4,Post,2026-03-02 16:00:00,WF-BATCH\n";

    fn settled() -> Rules {
        Rules::new()
            .classify_resource("SYSTEM", ResourceKind::System)
            .classify_resource("WF-BATCH", ResourceKind::System)
            .classify_resource("alice", ResourceKind::Human)
            .classify_resource("bob", ResourceKind::Human)
    }

    /// Each test writes its own file: these run in parallel, and sharing one
    /// path lets a reader see a half-truncated write.
    fn report(name: &str, rules: Rules) -> AutomationReport {
        let p = write_temp(name, LOG);
        automation_report(&p, None, &Policy::new(), rules)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn describes_every_account_without_judging_it() {
        let p = write_temp("resources", LOG);
        let r = resource_report(&p, None, &Policy::new(), Rules::new())
            .unwrap()
            .unwrap();
        assert_eq!(r.resources.len(), 4);
        assert_eq!(r.undecided, 4, "nothing has been decided yet");
        assert!(!r.is_settled());

        let system = r.resources.iter().find(|x| x.resource == "SYSTEM").unwrap();
        assert_eq!(system.starts_cases, 4, "it begins every case");
        assert_eq!(system.suggested, ResourceKind::System);
        assert!(!system.suggested_because.is_empty(), "the reason is shown");
        assert_eq!(system.decided, None);
    }

    #[test]
    fn shows_what_the_user_has_decided() {
        let p = write_temp("decided", LOG);
        let r = resource_report(&p, None, &Policy::new(), settled())
            .unwrap()
            .unwrap();
        assert_eq!(r.undecided, 0);
        assert!(r.is_settled());
        let alice = r.resources.iter().find(|x| x.resource == "alice").unwrap();
        assert_eq!(alice.decided, Some(ResourceKind::Human));
    }

    #[test]
    fn measures_the_boundary_once_classification_is_settled() {
        let r = report("boundary", settled());
        assert_eq!(r.straight_through_percent, Some(50.0));
        assert_eq!(r.straight_through_cases, 2);
        assert_eq!(r.touched_cases, 2);
        assert_eq!(r.undetermined_cases, 0);
        assert_eq!(r.classification_coverage_percent, 100.0);
        assert!(r.is_settled());
    }

    #[test]
    fn finds_and_sizes_the_manual_step() {
        let r = report("sizes", settled());
        assert_eq!(r.manual_steps_ranked.len(), 1);
        let step = &r.manual_steps_ranked[0];
        assert_eq!(step.activity, "Fix Data");
        assert_eq!(step.occurrences, 2);
        // Start-only logs cannot distinguish waiting from prior processing.
        assert_eq!(step.processing_seconds, 0);
        assert_eq!(step.start_to_start_before_seconds, 10 * 3600);
        assert_eq!(
            step.routing_context_concentration, 100.0,
            "always between Receive and Post"
        );
        assert_eq!(step.routing_contexts[0].before.as_deref(), Some("Receive"));
        assert_eq!(step.routing_contexts[0].after.as_deref(), Some("Post"));
        assert_eq!(step.example_cases, vec!["C3", "C4"]);
    }

    /// Without a settled classification the figures still appear, but the
    /// report says what it had to assume.
    #[test]
    fn reports_what_it_assumed_when_classification_is_incomplete() {
        let r = report("assumed", Rules::new());
        assert!(!r.is_settled());
        assert_eq!(
            r.unclassified_resources,
            vec!["SYSTEM", "WF-BATCH", "alice", "bob"]
        );
        assert_eq!(
            r.straight_through_cases, 0,
            "counting everything as human overstates manual work"
        );
        assert_eq!(r.unclassified_events, r.manual_events);
        assert_eq!(
            r.suggested_system_resources,
            vec!["SYSTEM", "WF-BATCH"],
            "names that look like service accounts are offered for confirmation"
        );
    }

    #[test]
    fn finds_where_cases_leave_the_dominant_path() {
        let r = report("divergence", settled());
        assert_eq!(r.divergences.len(), 1);
        let d = &r.divergences[0];
        assert_eq!(d.after.as_deref(), Some("Receive"));
        assert_eq!(d.took.as_deref(), Some("Fix Data"));
        assert_eq!(d.dominant_path_next.as_deref(), Some("Post"));
        assert_eq!(d.cases, 2);
    }

    /// Without a resource column the machine/human split is unknowable and
    /// must be reported as unknown rather than guessed.
    #[test]
    fn reports_unknown_without_a_resource_column() {
        let p = write_temp(
            "noresource",
            "case_id,activity,timestamp\nC1,A,2026-03-02 09:00:00\nC1,B,2026-03-02 10:00:00\n",
        );
        let r = automation_report(&p, None, &Policy::new(), Rules::new())
            .unwrap()
            .unwrap();
        assert!(!r.resources_mapped);
        assert_eq!(r.straight_through_percent, None);
        assert!(r.manual_steps_ranked.is_empty());
    }

    #[test]
    fn partial_resource_data_reduces_classification_coverage() {
        let p = write_temp(
            "partial_resource",
            "case_id,activity,timestamp,resource\nC1,A,2026-03-02 09:00:00,SYSTEM\nC1,B,2026-03-02 10:00:00,\n",
        );
        let r = automation_report(
            &p,
            None,
            &Policy::new(),
            Rules::new().classify_resource("SYSTEM", ResourceKind::System),
        )
        .unwrap()
        .unwrap();
        assert_eq!(r.straight_through_cases, 0);
        assert_eq!(r.undetermined_cases, 1);
        assert_eq!(r.classification_coverage_percent, 0.0);
    }

    #[test]
    fn json_carries_agent_interpretation_boundaries() {
        let r = report("agent_contract", settled());
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["ranking_basis"], "occurrences");
        assert!(json["manual_steps_ranked"][0]["routing_contexts"].is_array());
        assert!(json["interpretation_limits"].as_array().unwrap().len() >= 4);
        assert!(json["divergences"][0].get("dominant_path_next").is_some());
        assert!(json["divergences"][0].get("expected").is_none());
    }

    #[test]
    fn is_deterministic() {
        assert_eq!(
            report("determinism", settled()),
            report("determinism", settled())
        );
    }
}
