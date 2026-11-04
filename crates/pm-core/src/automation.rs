//! Automation targeting — finding the work worth automating.
//!
//! # The premise
//!
//! Where a system already runs a process straight through, there is nothing to
//! automate. Automation value lives exactly where straight-through processing
//! **breaks**: exceptions, rework, manual queues, and work that leaves the
//! system entirely and comes back.
//!
//! That question is answerable from an event log alone. Locating the boundary
//! between machine and human does not require watching anyone's screen.
//!
//! # Candidate discovery is evidence, not an automation verdict
//!
//! The metric this module is built around, and the one most easily got wrong:
//!
//! - A manual step appearing in one stable routing context five thousand times
//!   is a strong target for investigation.
//! - A manual step scattered across many routing contexts needs broader
//!   investigation before anyone proposes an intervention.
//!
//! Ranking by volume alone finds the second as readily as the first. So
//! manual steps carry a routing-context concentration derived from their
//! surrounding activities. This is useful evidence for investigation, but it
//! does not establish that the work itself is repetitive or rules-based.
//!
//! # What this cannot tell you
//!
//! Where the manual work is, how much of it there is, and how repeatable it
//! looks — yes. **How to do it** — no. That needs to see the screen, which
//! this product deliberately does not. Any report built on this module should
//! say so rather than imply a procedure it cannot supply.

use crate::dict::{ActivityId, CaseId};
use crate::log::{EventLog, Micros, ResourceId};
use crate::variants::Variants;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Whether a resource is a person or a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// A named individual or team.
    Human,
    /// A batch job, service account, or robot.
    System,
    /// No resource was recorded.
    Unrecorded,
}

/// Everything the engine can say about a resource without judging it.
///
/// Handed to a person or an assistant so they can decide what it is. The
/// engine describes; it does not conclude.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceProfile {
    pub resource: ResourceId,
    pub label: String,
    /// Events performed.
    pub events: u64,
    /// Cases worked on.
    pub cases: u64,
    /// Distinct activities performed.
    pub activities: usize,
    /// Times it began a case — machine accounts often start everything.
    pub starts: u64,
    /// Times it took over directly from a different resource.
    pub receives: u64,
    /// What the built-in heuristic would guess, and why.
    ///
    /// A starting point for a conversation, not an answer.
    pub suggested: ResourceKind,
    pub suggested_because: String,
}

/// How resources are divided into people and machines.
///
/// **An input, not a conclusion.** Deciding whether `EDI_GATEWAY` is a system
/// or `credit-desk` is a team is a judgement about someone's organisation, and
/// a name-matching table gets it wrong in the one direction that matters: a
/// person misread as a machine hides the manual work this analysis exists to
/// find.
///
/// So the engine takes the classification as given, records it with the
/// project, and applies it exactly. Producing one is a job for a person, or an
/// assistant reading [`ResourceProfile`]s — the same seam preparation rules
/// use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceClassification {
    assignments: BTreeMap<String, ResourceKind>,
}

impl ResourceClassification {
    /// An empty classification. Everything counts as human until told
    /// otherwise.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records what a resource is.
    pub fn assign(mut self, label: &str, kind: ResourceKind) -> Self {
        self.assignments.insert(label.to_owned(), kind);
        self
    }

    /// Marks a resource as a machine.
    pub fn system(self, label: &str) -> Self {
        self.assign(label, ResourceKind::System)
    }

    /// Marks a resource as a person or team.
    pub fn human(self, label: &str) -> Self {
        self.assign(label, ResourceKind::Human)
    }

    /// What `label` has been classified as.
    ///
    /// Unassigned labels read as [`ResourceKind::Human`], deliberately: the
    /// error then *overstates* manual work, which a user will notice and
    /// correct. Defaulting the other way would quietly shrink the number the
    /// whole analysis is about.
    pub fn kind(&self, label: &str) -> ResourceKind {
        self.assignments
            .get(label)
            .copied()
            .unwrap_or(ResourceKind::Human)
    }

    /// Whether `label` has been decided explicitly.
    pub fn is_assigned(&self, label: &str) -> bool {
        self.assignments.contains_key(label)
    }

    /// Number of explicit assignments.
    pub fn len(&self) -> usize {
        self.assignments.len()
    }

    /// Whether nothing has been decided.
    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }

    /// Every assignment, for recording with a project.
    pub fn assignments(&self) -> &BTreeMap<String, ResourceKind> {
        &self.assignments
    }

    /// Builds a classification from the built-in heuristic.
    ///
    /// A convenience for getting started, and the thing an assistant should
    /// improve on rather than the thing the engine relies on.
    pub fn from_suggestions(log: &EventLog) -> Self {
        let mut out = Self::new();
        for profile in profile_resources(log) {
            out = out.assign(&profile.label, profile.suggested);
        }
        out
    }
}

/// Name fragments that make a resource *look* like a machine.
///
/// Used only to seed a suggestion. Deliberately narrow, and matched on word
/// boundaries so `Roberta` is not a robot.
const SYSTEM_MARKERS: &[&str] = &[
    "system",
    "batch",
    "bot",
    "rpa",
    "robot",
    "svc",
    "service",
    "daemon",
    "scheduler",
    "cron",
    "job",
    "interface",
    "integration",
    "api",
    "wf",
];

/// Guesses what a resource label looks like.
///
/// **A suggestion for a human to confirm**, never applied on its own. Returns
/// the guess and the reason for it, so the reason can be shown alongside.
pub fn suggest(label: &str) -> (ResourceKind, String) {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return (ResourceKind::Unrecorded, "no resource recorded".to_owned());
    }
    let lower = trimmed.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    for marker in SYSTEM_MARKERS {
        if words.iter().any(|w| w == marker) {
            return (
                ResourceKind::System,
                format!("the name contains '{marker}', which usually marks a service account"),
            );
        }
    }
    (
        ResourceKind::Human,
        "the name does not look like a service account".to_owned(),
    )
}

/// Describes every resource in the log, so a person or an assistant can
/// classify them.
pub fn profile_resources(log: &EventLog) -> Vec<ResourceProfile> {
    let mut events: BTreeMap<ResourceId, u64> = BTreeMap::new();
    let mut cases: BTreeMap<ResourceId, BTreeSet<CaseId>> = BTreeMap::new();
    let mut activities: BTreeMap<ResourceId, BTreeSet<ActivityId>> = BTreeMap::new();
    let mut starts: BTreeMap<ResourceId, u64> = BTreeMap::new();
    let mut receives: BTreeMap<ResourceId, u64> = BTreeMap::new();

    for trace in &log.traces {
        let mut previous: Option<ResourceId> = None;
        for (index, event) in trace.events.iter().enumerate() {
            let Some(r) = event.resource() else {
                previous = None;
                continue;
            };
            *events.entry(r).or_insert(0) += 1;
            cases.entry(r).or_default().insert(trace.case);
            activities.entry(r).or_default().insert(event.activity);
            if index == 0 {
                *starts.entry(r).or_insert(0) += 1;
            }
            if let Some(p) = previous
                && p != r
            {
                *receives.entry(r).or_insert(0) += 1;
            }
            previous = Some(r);
        }
    }

    events
        .keys()
        .map(|r| {
            let label = log.resource_label(*r).to_owned();
            let (suggested, why) = suggest(&label);
            ResourceProfile {
                resource: *r,
                events: events.get(r).copied().unwrap_or(0),
                cases: cases.get(r).map(|c| c.len() as u64).unwrap_or(0),
                activities: activities.get(r).map(BTreeSet::len).unwrap_or(0),
                starts: starts.get(r).copied().unwrap_or(0),
                receives: receives.get(r).copied().unwrap_or(0),
                suggested,
                suggested_because: why,
                label,
            }
        })
        .collect()
}

/// A manual step worth considering for automation.
#[derive(Debug, Clone, PartialEq)]
pub struct Opportunity {
    pub activity: ActivityId,
    /// Times a human performed it.
    pub occurrences: u64,
    /// Cases it touched.
    pub cases: u64,
    /// Observed execution time of the activity itself.
    pub total_processing: Micros,
    pub processing_observations: u64,
    /// True waiting immediately before this step, where the preceding event
    /// had an end timestamp.
    pub total_waiting_before: Micros,
    pub waiting_observations: u64,
    /// Start-to-start elapsed time before this step when true waiting could not
    /// be measured. Kept separate because it includes preceding processing.
    pub total_start_to_start_before: Micros,
    pub start_to_start_observations: u64,
    /// Recorded cost across those events, when a cost column was mapped.
    pub total_cost: i128,
    /// Concentration of observed predecessor/successor contexts, 0–100.
    ///
    /// This describes routing regularity only. It does not show whether the
    /// task inputs or decisions were themselves consistent.
    pub routing_context_concentration: f64,
    /// Distinct (predecessor, successor) contexts observed.
    pub distinct_contexts: usize,
    pub contexts: Vec<OpportunityContext>,
    pub example_cases: Vec<CaseId>,
}

impl Opportunity {
    fn score(&self, basis: RankingBasis) -> f64 {
        let scale = match basis {
            RankingBasis::ProcessingTime => self.total_processing as f64 / 3_600_000_000.0,
            RankingBasis::Occurrences => self.occurrences as f64,
        };
        scale * (self.routing_context_concentration / 100.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpportunityContext {
    pub before: Option<ActivityId>,
    pub after: Option<ActivityId>,
    pub occurrences: u64,
    pub share_percent: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankingBasis {
    ProcessingTime,
    #[default]
    Occurrences,
}

/// Where cases leave the dominant path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakPoint {
    /// The activity after which cases diverge.
    pub after: Option<ActivityId>,
    /// What they did instead of the dominant path.
    pub instead_of: Option<ActivityId>,
    pub took: Option<ActivityId>,
    pub cases: u64,
    pub example_cases: Vec<CaseId>,
}

/// The automation view of a log.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Automation {
    /// Cases no human touched.
    pub straight_through_cases: usize,
    /// Cases at least one human touched.
    pub touched_cases: usize,
    /// Human-performed events.
    pub manual_events: u64,
    /// Machine-performed events.
    pub system_events: u64,
    /// Events with no recorded resource.
    pub unrecorded_events: u64,
    /// Cases containing no confirmed human event but at least one event whose
    /// performer was not recorded.
    pub undetermined_cases: usize,
    /// Manual steps ranked by score, best candidate first.
    pub opportunities: Vec<Opportunity>,
    pub ranking_basis: RankingBasis,
    /// Where cases leave the dominant path, most frequent first.
    pub break_points: Vec<BreakPoint>,
    pub dominant_variant_cases: usize,
    /// Whether a resource column was mapped at all.
    pub resources_known: bool,
    /// Resources the classification did not decide, so they were counted as
    /// human. Reported because they are the margin of error on every figure
    /// above.
    pub unclassified: Vec<String>,
}

impl Automation {
    /// Builds the automation view under a given classification.
    ///
    /// The classification is an **input**: deciding what counts as a machine
    /// is a judgement about someone's organisation, and getting it wrong hides
    /// the manual work this analysis exists to find. Resources it does not
    /// decide are counted as human and listed in
    /// [`Automation::unclassified`], so the margin of error is visible rather
    /// than buried.
    ///
    /// Without a resource column the machine/human boundary is unknowable, so
    /// straight-through counts stay unset and only break points are reported.
    /// Inferring which activities are manual from their names would be exactly
    /// the kind of invention this product refuses.
    pub fn build(log: &EventLog, classification: &ResourceClassification) -> Automation {
        let resources_known = log.has_resources();
        let kinds: BTreeMap<ResourceId, ResourceKind> = (0..log.resources.len() as u32)
            .map(|id| {
                let label = log.resources.label(id).unwrap_or_default();
                (ResourceId(id), classification.kind(label))
            })
            .collect();

        let mut view = Automation {
            resources_known,
            unclassified: (0..log.resources.len() as u32)
                .filter_map(|id| log.resources.label(id))
                .filter(|label| !classification.is_assigned(label))
                .map(str::to_owned)
                .collect(),
            ..Default::default()
        };

        // Per-activity accumulation for manual work.
        let mut manual: BTreeMap<ActivityId, ManualTally> = BTreeMap::new();

        for trace in &log.traces {
            let mut saw_human = false;
            let mut saw_system = false;
            let mut saw_unrecorded = false;

            for (index, event) in trace.events.iter().enumerate() {
                let kind = event
                    .resource()
                    .map(|r| kinds.get(&r).copied().unwrap_or(ResourceKind::Human))
                    .unwrap_or(ResourceKind::Unrecorded);

                match kind {
                    ResourceKind::Human => {
                        view.manual_events += 1;
                        saw_human = true;

                        let tally = manual.entry(event.activity).or_default();
                        tally.occurrences += 1;
                        tally.cases.insert(trace.case);
                        if tally.examples.len() < 5 && !tally.examples.contains(&trace.case) {
                            tally.examples.push(trace.case);
                        }

                        if !trace.incomplete {
                            tally.duration_eligible_occurrences += 1;
                            if let Some(processing) = event.processing_time() {
                                tally.processing += processing.max(0);
                                tally.processing_observations += 1;
                            }
                            if index > 0 {
                                let previous = &trace.events[index - 1];
                                if let Some(end) = previous.end() {
                                    tally.waiting_before += (event.at - end).max(0);
                                    tally.waiting_observations += 1;
                                } else {
                                    tally.start_to_start_before += (event.at - previous.at).max(0);
                                    tally.start_to_start_observations += 1;
                                }
                            }
                        }
                        tally.cost += log
                            .event_cost(trace.case, event)
                            .map(i128::from)
                            .unwrap_or(0);

                        let before = index.checked_sub(1).map(|i| trace.events[i].activity);
                        let after = trace.events.get(index + 1).map(|e| e.activity);
                        *tally.contexts.entry((before, after)).or_insert(0) += 1;
                    }
                    ResourceKind::System => {
                        view.system_events += 1;
                        saw_system = true;
                    }
                    ResourceKind::Unrecorded => {
                        view.unrecorded_events += 1;
                        saw_unrecorded = true;
                    }
                }
            }

            if saw_human {
                view.touched_cases += 1;
            } else if resources_known && saw_system && !saw_unrecorded {
                view.straight_through_cases += 1;
            } else {
                view.undetermined_cases += 1;
            }
        }

        let duration_eligible: u64 = manual
            .values()
            .map(|tally| tally.duration_eligible_occurrences)
            .sum();
        let processing_observations: u64 = manual
            .values()
            .map(|tally| tally.processing_observations)
            .sum();
        view.ranking_basis =
            if duration_eligible > 0 && processing_observations == duration_eligible {
                RankingBasis::ProcessingTime
            } else {
                RankingBasis::Occurrences
            };
        view.opportunities = manual
            .into_iter()
            .map(|(activity, tally)| tally.into_opportunity(activity))
            .collect();
        // Best candidate first; ties broken by activity so the order is stable.
        view.opportunities.sort_by(|a, b| {
            b.score(view.ranking_basis)
                .partial_cmp(&a.score(view.ranking_basis))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.activity.cmp(&b.activity))
        });

        let (break_points, dominant_variant_cases) = break_points(log);
        view.break_points = break_points;
        view.dominant_variant_cases = dominant_variant_cases;
        view
    }

    /// Share of cases running straight through, as a percentage.
    ///
    /// `None` when no resource column was mapped, because the answer is then
    /// genuinely unknown rather than zero.
    pub fn straight_through_percent(&self) -> Option<f64> {
        if !self.resources_known {
            return None;
        }
        let known = self.straight_through_cases + self.touched_cases;
        if known == 0 {
            return None;
        }
        Some(self.straight_through_cases as f64 / known as f64 * 100.0)
    }

    pub fn total_processing_time(&self) -> Micros {
        self.opportunities.iter().map(|o| o.total_processing).sum()
    }

    pub fn total_waiting_before(&self) -> Micros {
        self.opportunities
            .iter()
            .map(|o| o.total_waiting_before)
            .sum()
    }

    pub fn total_start_to_start_before(&self) -> Micros {
        self.opportunities
            .iter()
            .map(|o| o.total_start_to_start_before)
            .sum()
    }
}

#[derive(Default)]
struct ManualTally {
    occurrences: u64,
    cases: BTreeSet<CaseId>,
    duration_eligible_occurrences: u64,
    processing: Micros,
    processing_observations: u64,
    waiting_before: Micros,
    waiting_observations: u64,
    start_to_start_before: Micros,
    start_to_start_observations: u64,
    cost: i128,
    contexts: BTreeMap<(Option<ActivityId>, Option<ActivityId>), u64>,
    examples: Vec<CaseId>,
}

impl ManualTally {
    fn into_opportunity(self, activity: ActivityId) -> Opportunity {
        let total: u64 = self.contexts.values().sum();
        let mut contexts: Vec<_> = self
            .contexts
            .iter()
            .map(|((before, after), occurrences)| OpportunityContext {
                before: *before,
                after: *after,
                occurrences: *occurrences,
                share_percent: if total == 0 {
                    0.0
                } else {
                    *occurrences as f64 / total as f64 * 100.0
                },
            })
            .collect();
        contexts.sort_by(|left, right| {
            right
                .occurrences
                .cmp(&left.occurrences)
                .then(left.before.cmp(&right.before))
                .then(left.after.cmp(&right.after))
        });

        Opportunity {
            activity,
            occurrences: self.occurrences,
            cases: self.cases.len() as u64,
            total_processing: self.processing,
            processing_observations: self.processing_observations,
            total_waiting_before: self.waiting_before,
            waiting_observations: self.waiting_observations,
            total_start_to_start_before: self.start_to_start_before,
            start_to_start_observations: self.start_to_start_observations,
            total_cost: self.cost,
            routing_context_concentration: context_concentration(&self.contexts),
            distinct_contexts: self.contexts.len(),
            contexts,
            example_cases: self.examples,
        }
    }
}

/// How concentrated a distribution of contexts is, 0–100.
///
/// Simpson concentration: one context scores 100, two equally common contexts
/// score 50, and fifty equally common contexts score 2. This is a routing fact,
/// not a claim that the work performed inside the activity is repetitive.
fn context_concentration(
    contexts: &BTreeMap<(Option<ActivityId>, Option<ActivityId>), u64>,
) -> f64 {
    let total: u64 = contexts.values().sum();
    if total == 0 {
        return 0.0;
    }
    contexts
        .values()
        .map(|count| {
            let p = *count as f64 / total as f64;
            p * p
        })
        .sum::<f64>()
        * 100.0
}

/// Where a case diverged: the activity before, what the dominant path did
/// there, and what this case did instead.
type Divergence = (Option<ActivityId>, Option<ActivityId>, Option<ActivityId>);

/// Finds where cases leave the dominant path.
fn break_points(log: &EventLog) -> (Vec<BreakPoint>, usize) {
    let variants = Variants::build(log);
    let Some(dominant) = variants.variants.first() else {
        return (Vec::new(), 0);
    };
    let happy = &dominant.sequence;

    let mut found: BTreeMap<Divergence, (u64, Vec<CaseId>)> = BTreeMap::new();

    for trace in &log.traces {
        let actual: Vec<ActivityId> = trace.events.iter().map(|e| e.activity).collect();
        if actual == *happy {
            continue;
        }
        // The first position where this case differs from the dominant path.
        let divergence = actual
            .iter()
            .zip(happy.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(actual.len().min(happy.len()));

        let after = divergence
            .checked_sub(1)
            .and_then(|i| actual.get(i))
            .copied();
        let took = actual.get(divergence).copied();
        let instead_of = happy.get(divergence).copied();

        let entry = found
            .entry((after, instead_of, took))
            .or_insert((0, Vec::new()));
        entry.0 += 1;
        if entry.1.len() < 5 {
            entry.1.push(trace.case);
        }
    }

    let mut points: Vec<BreakPoint> = found
        .into_iter()
        .map(
            |((after, instead_of, took), (cases, example_cases))| BreakPoint {
                after,
                instead_of,
                took,
                cases,
                example_cases,
            },
        )
        .collect();
    points.sort_by(|a, b| {
        b.cases
            .cmp(&a.cases)
            .then(a.after.cmp(&b.after))
            .then(a.took.cmp(&b.took))
    });
    (points, dominant.cases.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::Dictionary;
    use crate::log::{CaseTrace, Event};

    const HOUR: Micros = 3_600_000_000;

    type Step<'a> = (&'a str, i64, &'a str);
    type Case<'a> = (&'a str, &'a [Step<'a>]);

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
            costs: BTreeMap::new(),
            step_values: Default::default(),
            traces,
        }
    }

    #[test]
    fn suggests_machine_accounts() {
        for name in [
            "SYSTEM",
            "WF-BATCH",
            "batch_job",
            "RPA_BOT",
            "svc-invoicing",
            "SCHEDULER",
            "integration",
            "api_user",
        ] {
            assert_eq!(suggest(name).0, ResourceKind::System, "{name}");
        }
    }

    #[test]
    fn suggests_people() {
        for name in ["alice", "Bob Smith", "j.doe", "credit-desk", "Team Nordics"] {
            assert_eq!(suggest(name).0, ResourceKind::Human, "{name}");
        }
    }

    /// A conservative classifier matters: reading a person as a machine would
    /// hide the very manual work being measured.
    #[test]
    fn does_not_mistake_names_containing_marker_substrings() {
        for name in ["Roberta", "Autoline Ltd", "Robotham", "Serviceable Ltd"] {
            assert_eq!(suggest(name).0, ResourceKind::Human, "{name}");
        }
    }

    #[test]
    fn an_empty_resource_is_unrecorded() {
        assert_eq!(suggest("").0, ResourceKind::Unrecorded);
        assert_eq!(suggest("   ").0, ResourceKind::Unrecorded);
    }

    fn mixed() -> EventLog {
        log(&[
            // Straight through: only machine accounts.
            ("C1", &[("Receive", 0, "SYSTEM"), ("Post", 1, "WF-BATCH")]),
            ("C2", &[("Receive", 0, "SYSTEM"), ("Post", 1, "WF-BATCH")]),
            // A human intervened.
            (
                "C3",
                &[
                    ("Receive", 0, "SYSTEM"),
                    ("Fix Data", 4, "alice"),
                    ("Post", 5, "WF-BATCH"),
                ],
            ),
            (
                "C4",
                &[
                    ("Receive", 0, "SYSTEM"),
                    ("Fix Data", 6, "bob"),
                    ("Post", 7, "WF-BATCH"),
                ],
            ),
        ])
    }

    #[test]
    fn separates_straight_through_from_touched_cases() {
        let a = Automation::build(
            &mixed(),
            &ResourceClassification::from_suggestions(&mixed()),
        );
        assert_eq!(a.straight_through_cases, 2);
        assert_eq!(a.touched_cases, 2);
        assert_eq!(a.straight_through_percent(), Some(50.0));
        assert_eq!(a.manual_events, 2, "alice and bob, once each");
        assert_eq!(
            a.system_events, 8,
            "two machine events in each of four cases"
        );
    }

    #[test]
    fn finds_the_manual_step_as_an_opportunity() {
        let l = mixed();
        let a = Automation::build(&l, &ResourceClassification::from_suggestions(&l));
        assert_eq!(a.opportunities.len(), 1);
        let o = &a.opportunities[0];
        assert_eq!(l.activity_label(o.activity), "Fix Data");
        assert_eq!(o.occurrences, 2);
        assert_eq!(o.cases, 2);
    }

    /// The core claim of the module: a step always done the same way scores
    /// high, and one done differently every time scores low — regardless of
    /// how often either happens.
    #[test]
    fn repeatable_work_outranks_more_frequent_but_varied_work() {
        type OwnedStep = (String, i64, String);
        let mut spec: Vec<(String, Vec<OwnedStep>)> = Vec::new();

        // Consistent: always between Receive and Post, 10 cases.
        for i in 0..10 {
            spec.push((
                format!("R{i}"),
                vec![
                    ("Receive".to_owned(), 0, "SYSTEM".to_owned()),
                    ("Consistent".to_owned(), 2, "alice".to_owned()),
                    ("Post".to_owned(), 3, "SYSTEM".to_owned()),
                ],
            ));
        }
        // Varied: 20 cases, every one a different context.
        for i in 0..20 {
            spec.push((
                format!("V{i}"),
                vec![
                    (format!("Before {i}"), 0, "SYSTEM".to_owned()),
                    ("Varied".to_owned(), 2, "alice".to_owned()),
                    (format!("After {i}"), 3, "SYSTEM".to_owned()),
                ],
            ));
        }

        let owned: Vec<(&str, Vec<Step<'_>>)> = spec
            .iter()
            .map(|(c, events)| {
                (
                    c.as_str(),
                    events
                        .iter()
                        .map(|(a, h, r)| (a.as_str(), *h, r.as_str()))
                        .collect(),
                )
            })
            .collect();
        let cases: Vec<Case<'_>> = owned.iter().map(|(c, e)| (*c, e.as_slice())).collect();
        let l = log(&cases);
        let a = Automation::build(&l, &ResourceClassification::from_suggestions(&l));

        let consistent = a
            .opportunities
            .iter()
            .find(|o| l.activity_label(o.activity) == "Consistent")
            .unwrap();
        let varied = a
            .opportunities
            .iter()
            .find(|o| l.activity_label(o.activity) == "Varied")
            .unwrap();

        assert_eq!(
            consistent.routing_context_concentration, 100.0,
            "one context every time"
        );
        assert!(
            varied.routing_context_concentration < 10.0,
            "twenty distinct contexts: {}",
            varied.routing_context_concentration
        );
        assert!(
            varied.occurrences > consistent.occurrences,
            "the varied step is more frequent"
        );
        assert!(
            consistent.score(a.ranking_basis) > varied.score(a.ranking_basis),
            "yet the routing-concentrated one must rank higher"
        );
        assert_eq!(a.opportunities[0].activity, consistent.activity);
    }

    #[test]
    fn counts_distinct_contexts() {
        let l = mixed();
        let a = Automation::build(&l, &ResourceClassification::from_suggestions(&l));
        assert_eq!(a.opportunities[0].distinct_contexts, 1);
    }

    /// Without a resource column the machine/human split is unknowable, and
    /// must be reported as unknown rather than guessed from activity names.
    #[test]
    fn reports_unknown_when_no_resource_column_was_mapped() {
        let l = log(&[("C1", &[("A", 0, ""), ("B", 1, "")])]);
        let a = Automation::build(&l, &ResourceClassification::from_suggestions(&l));
        assert!(!a.resources_known);
        assert_eq!(a.straight_through_percent(), None);
        assert_eq!(a.straight_through_cases, 0);
        assert!(a.opportunities.is_empty());
    }

    #[test]
    fn finds_where_cases_leave_the_dominant_path() {
        let l = mixed();
        let a = Automation::build(&l, &ResourceClassification::from_suggestions(&l));
        // Two cases diverge after Receive by doing Fix Data instead of Post.
        assert_eq!(a.break_points.len(), 1);
        let b = &a.break_points[0];
        assert_eq!(b.cases, 2);
        assert_eq!(b.after.map(|x| l.activity_label(x)), Some("Receive"));
        assert_eq!(b.took.map(|x| l.activity_label(x)), Some("Fix Data"));
        assert_eq!(b.instead_of.map(|x| l.activity_label(x)), Some("Post"));
    }

    #[test]
    fn a_log_that_never_deviates_has_no_break_points() {
        let l = log(&[
            ("C1", &[("A", 0, "SYSTEM"), ("B", 1, "SYSTEM")]),
            ("C2", &[("A", 0, "SYSTEM"), ("B", 1, "SYSTEM")]),
        ]);
        let c = ResourceClassification::from_suggestions(&l);
        assert!(Automation::build(&l, &c).break_points.is_empty());
    }

    #[test]
    fn keeps_start_to_start_elapsed_separate_from_manual_processing() {
        let l = mixed();
        let a = Automation::build(&l, &ResourceClassification::from_suggestions(&l));
        assert_eq!(a.opportunities[0].total_start_to_start_before, 10 * HOUR);
        assert_eq!(a.total_start_to_start_before(), 10 * HOUR);
        assert_eq!(a.total_processing_time(), 0);
        assert_eq!(a.ranking_basis, RankingBasis::Occurrences);
    }

    #[test]
    fn ranks_by_processing_only_when_complete_processing_evidence_exists() {
        let mut l = mixed();
        for trace in &mut l.traces {
            for event in &mut trace.events {
                event.end_at = event.at + HOUR / 2;
            }
        }
        let a = Automation::build(&l, &ResourceClassification::from_suggestions(&l));
        assert_eq!(a.ranking_basis, RankingBasis::ProcessingTime);
        assert_eq!(a.opportunities[0].total_processing, HOUR);
        assert_eq!(a.opportunities[0].processing_observations, 2);
        assert_eq!(a.opportunities[0].total_waiting_before, 9 * HOUR);
        assert_eq!(a.opportunities[0].waiting_observations, 2);
        assert_eq!(a.opportunities[0].total_start_to_start_before, 0);
    }

    #[test]
    fn a_missing_performer_prevents_a_straight_through_claim() {
        let mut l = log(&[("C1", &[("Receive", 0, "SYSTEM"), ("Post", 1, "")])]);
        let system = ResourceClassification::new().system("SYSTEM");
        let a = Automation::build(&l, &system);
        assert_eq!(a.straight_through_cases, 0);
        assert_eq!(a.undetermined_cases, 1);
        assert_eq!(a.unrecorded_events, 1);
        // A confirmed human still makes a case touched even if another event
        // has no performer.
        l.traces[0].events[1].resource = l.resources.intern("alice");
        let a = Automation::build(&l, &system.human("alice"));
        assert_eq!(a.touched_cases, 1);
    }

    #[test]
    fn context_concentration_distinguishes_two_from_fifty_uniform_contexts() {
        let two = BTreeMap::from([
            ((Some(ActivityId(0)), Some(ActivityId(1))), 50),
            ((Some(ActivityId(2)), Some(ActivityId(3))), 50),
        ]);
        let fifty: BTreeMap<_, _> = (0..50)
            .map(|id| ((Some(ActivityId(id)), None), 2))
            .collect();
        assert_eq!(context_concentration(&two), 50.0);
        assert!((context_concentration(&fifty) - 2.0).abs() < 0.000_001);
    }

    /// The whole point of the redesign: the engine applies the classification
    /// it is given. Calling a service account human moves the numbers, and it
    /// is the user's call to make.
    #[test]
    fn the_classification_is_an_input_not_a_verdict() {
        let l = mixed();

        let suggested = Automation::build(&l, &ResourceClassification::from_suggestions(&l));
        assert_eq!(suggested.straight_through_cases, 2);

        // Treat WF-BATCH as a person: nothing runs straight through any more.
        let overridden = ResourceClassification::from_suggestions(&l).human("WF-BATCH");
        let a = Automation::build(&l, &overridden);
        assert_eq!(a.straight_through_cases, 0);
        assert_eq!(a.touched_cases, 4);
    }

    /// An empty classification counts everyone as human — overstating manual
    /// work rather than hiding it — and says which resources it had to assume.
    #[test]
    fn unclassified_resources_default_to_human_and_are_reported() {
        let l = mixed();
        let a = Automation::build(&l, &ResourceClassification::new());
        assert_eq!(
            a.straight_through_cases, 0,
            "nothing is known to be a machine"
        );
        assert_eq!(a.manual_events, 10);
        assert_eq!(
            a.unclassified,
            vec!["SYSTEM", "WF-BATCH", "alice", "bob"],
            "the margin of error is visible"
        );
    }

    /// Profiles describe resources without judging them, so a person or an
    /// assistant can decide.
    #[test]
    fn profiles_describe_resources_for_someone_else_to_classify() {
        let l = mixed();
        let profiles = profile_resources(&l);
        let system = profiles.iter().find(|p| p.label == "SYSTEM").unwrap();
        assert_eq!(system.events, 4);
        assert_eq!(system.cases, 4);
        assert_eq!(system.starts, 4, "it begins every case");
        assert_eq!(system.suggested, ResourceKind::System);
        assert!(system.suggested_because.contains("system"));

        let alice = profiles.iter().find(|p| p.label == "alice").unwrap();
        assert_eq!(alice.suggested, ResourceKind::Human);
        assert_eq!(alice.receives, 1, "took over from SYSTEM once");
    }

    #[test]
    fn is_deterministic() {
        let l = mixed();
        let c = ResourceClassification::from_suggestions(&l);
        assert_eq!(Automation::build(&l, &c), Automation::build(&l, &c));
    }

    #[test]
    fn an_empty_log_produces_an_empty_view() {
        let a = Automation::build(&EventLog::default(), &ResourceClassification::new());
        assert_eq!(a.straight_through_percent(), None);
        assert!(a.opportunities.is_empty());
        assert!(a.break_points.is_empty());
    }
}
