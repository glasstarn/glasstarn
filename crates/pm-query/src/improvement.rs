//! The improvement view: the rules the process follows and the cases that
//! break them, who does each step (people or systems), and where each step
//! stands for automation.
//!
//! The engine measures; it does not decide. Mined rules are offered as
//! "found in the data"; buckets, performer kinds and rules the company keeps
//! come from [`Improvement`], where every entry says whether it is the
//! agent's suggestion or the person's decision.

use pm_core::ResourceKind;
use pm_core::declare::{DeclareFold, MiningSettings, Rule, RuleCheckFold, Step, Template};
use pm_core::steps::{StepFacts, StepFactsFold};
use pm_project::improvement::{Improvement, Status};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// Mined rules shown beside the ones the company keeps.
const MINED_SHOWN: usize = 40;
/// Names listed when performers still need a decision.
const NAMES_SHOWN: usize = 30;

/// A log read as cases of steps, wherever it lives.
pub trait Traces {
    /// Activity names by identifier.
    fn activities(&self) -> Vec<String>;
    /// Performer names by identifier; empty when none were recorded.
    fn performers(&self) -> Vec<String>;
    /// Calls `f` with each case and its steps, in a fixed order.
    fn each(&self, f: &mut dyn FnMut(u32, &[Step])) -> Result<(), String>;
    fn case_label(&self, case: u32) -> String;
    /// The case's value in `field` (an amount), when it has a number there.
    fn case_value(&self, _case: u32, _field: &str) -> Option<f64> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImprovementReport {
    /// What one case is (for example "purchase order line").
    pub follows: String,
    pub cases: u64,
    pub performers: PerformerSummary,
    pub rules: Vec<RuleView>,
    pub steps: Vec<StepView>,
    /// The decisions' revision: a page that shows an older one is stale.
    pub revision: u64,
    pub value_field: Option<String>,
    /// What the numbers do and do not say.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PerformerSummary {
    pub recorded: bool,
    pub total: usize,
    pub people: usize,
    pub systems: usize,
    pub not_classified: usize,
    /// Not yet decided, and whose name looks like a service account.
    pub look_like_systems: Vec<String>,
    /// Not yet decided, first few by name.
    pub undecided: Vec<String>,
    pub undecided_omitted: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleView {
    pub template: Template,
    pub a: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b: Option<String>,
    /// The rule in words.
    pub text: String,
    /// "found in the data", "suggested", "confirmed" or "rejected".
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Cases the rule applies to, and how many break it.
    pub applies_to: u64,
    pub broken: u64,
    /// Share of the cases it applies to that keep it, in percent.
    pub kept_percent: f64,
    /// The total value of the cases that break it, when a value field is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broken_value: Option<f64>,
    pub examples: Vec<String>,
    /// Set when a named step is not in this data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepView {
    pub activity: String,
    pub occurrences: u64,
    pub cases: u64,
    pub repeats: u64,
    pub performers: usize,
    pub by_people: u64,
    pub by_systems: u64,
    pub unrecorded: u64,
    pub not_classified: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usual_next: Option<String>,
    pub usual_next_percent: f64,
    pub examples: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bucket: Option<BucketView>,
    /// Minutes of a person's work per occurrence, as the company estimated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort_minutes: Option<f64>,
    /// Hours of people's work the step took in this data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub people_hours: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub people_cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BucketView {
    pub bucket: pm_project::improvement::Bucket,
    pub label: &'static str,
    pub status: Status,
    pub by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn status_word(status: Status) -> &'static str {
    match status {
        Status::Suggested => "suggested",
        Status::Confirmed => "confirmed",
        Status::Rejected => "rejected",
    }
}

/// Builds the view from a log and the company's decisions. `known` holds
/// performer kinds decided elsewhere (a case log's rules), which the
/// project's own decisions override.
pub fn improvement_report(
    traces: &dyn Traces,
    follows: &str,
    decisions: &Improvement,
    known: &BTreeMap<String, ResourceKind>,
    settings: &MiningSettings,
) -> Result<ImprovementReport, String> {
    let activities = traces.activities();
    let performers = traces.performers();
    let mut kinds_by_name = known.clone();
    kinds_by_name.extend(decisions.confirmed_performers());
    let kinds = performers
        .iter()
        .map(|name| kinds_by_name.get(name).copied())
        .collect::<Vec<_>>();
    let kind = |p: u32| kinds.get(p as usize).copied().flatten();

    // Pass one: mine rules and measure the steps.
    let mut mining = DeclareFold::new();
    let mut facts = StepFactsFold::new(&kind);
    let mut cases = 0u64;
    traces.each(&mut |case, steps| {
        cases += 1;
        mining.trace(steps);
        facts.trace(case, steps);
    })?;
    let facts = facts.finish();
    // Four eyes is about people: offer it only for steps mostly done by
    // people or by performers not classified yet.
    let human = facts
        .iter()
        .filter(|f| 2 * (f.by_people + f.not_classified) >= f.occurrences)
        .map(|f| f.activity)
        .collect::<std::collections::BTreeSet<_>>();
    let mined = mining.finish(settings, &|a| human.contains(&a));

    // The rules to check: the company's, then mined ones not yet saved.
    let index = activities
        .iter()
        .enumerate()
        .map(|(i, name)| (name.as_str(), i as u32))
        .collect::<HashMap<_, _>>();
    let name = |id: u32| activities.get(id as usize).cloned().unwrap_or_default();
    let mut views = Vec::new();
    let mut checked = Vec::new();
    for saved in &decisions.rules {
        let a = index.get(saved.a.as_str()).copied();
        let b = match &saved.b {
            Some(b) => index.get(b.as_str()).copied().map(Some),
            None => Some(None),
        };
        let mut view = RuleView {
            template: saved.template,
            a: saved.a.clone(),
            b: saved.b.clone(),
            text: saved
                .template
                .describe(&saved.a, saved.b.as_deref().unwrap_or(&saved.a)),
            status: status_word(saved.said.status),
            by: Some(saved.said.by.clone()),
            reason: saved.said.reason.clone(),
            applies_to: 0,
            broken: 0,
            kept_percent: 100.0,
            broken_value: None,
            examples: Vec::new(),
            problem: None,
        };
        match (a, b) {
            (Some(a), Some(b)) => {
                checked.push((views.len(), Rule::binary(saved.template, a, b.unwrap_or(a))));
            }
            _ => view.problem = Some("a step this rule names is not in this data".into()),
        }
        views.push(view);
    }
    let saved_rules = checked.iter().map(|(_, r)| *r).collect::<Vec<_>>();
    for rule in mined
        .into_iter()
        .filter(|r| !saved_rules.contains(r))
        .take(MINED_SHOWN)
    {
        let (a, b) = (name(rule.a), name(rule.b));
        checked.push((views.len(), rule));
        views.push(RuleView {
            template: rule.template,
            text: rule.template.describe(&a, &b),
            b: rule.template.is_binary().then(|| b.clone()),
            a,
            status: "found in the data",
            by: None,
            reason: None,
            applies_to: 0,
            broken: 0,
            kept_percent: 100.0,
            broken_value: None,
            examples: Vec::new(),
            problem: None,
        });
    }

    // Pass two: which cases break them, and what they are worth.
    let value_field = decisions.value_field.clone();
    let mut check = RuleCheckFold::new(checked.iter().map(|(_, r)| *r).collect());
    traces.each(&mut |case, steps| {
        let value = value_field
            .as_deref()
            .and_then(|field| traces.case_value(case, field));
        check.trace_with_value(case, steps, value);
    })?;
    for ((view, _), result) in checked.iter().zip(check.finish()) {
        let view = &mut views[*view];
        view.applies_to = result.activated;
        view.broken = result.violated;
        view.kept_percent = result.confidence * 100.0;
        view.broken_value = value_field.as_ref().map(|_| result.violated_value);
        view.examples = result
            .examples
            .iter()
            .map(|c| traces.case_label(*c))
            .collect();
    }

    let steps = facts
        .into_iter()
        .map(|f| step_view(f, &name, traces, decisions))
        .collect();

    let undecided = performers
        .iter()
        .filter(|p| !kinds_by_name.contains_key(*p))
        .cloned()
        .collect::<Vec<_>>();
    let summary = PerformerSummary {
        recorded: !performers.is_empty(),
        total: performers.len(),
        people: kinds
            .iter()
            .filter(|k| **k == Some(ResourceKind::Human))
            .count(),
        systems: kinds
            .iter()
            .filter(|k| **k == Some(ResourceKind::System))
            .count(),
        not_classified: undecided.len(),
        look_like_systems: undecided
            .iter()
            .filter(|p| pm_core::suggest(p).0 == ResourceKind::System)
            .take(NAMES_SHOWN)
            .cloned()
            .collect(),
        undecided: undecided.iter().take(NAMES_SHOWN).cloned().collect(),
        undecided_omitted: undecided.len().saturating_sub(NAMES_SHOWN),
    };
    let mut notes = vec![
        "Rules found in the data hold in at least 95% of the cases they apply to; that they hold does not make them policy until the company confirms them.".to_owned(),
        "Buckets are suggestions until a person confirms them. The log shows when steps were recorded, not how long people worked on them: hours and money need the company's estimate of minutes per step.".to_owned(),
    ];
    if !summary.recorded {
        notes.push("No performer was recorded, so people and systems cannot be told apart.".into());
    } else if summary.not_classified > 0 {
        notes.push(format!(
            "{} performers are not classified yet; their steps count as not classified, not as manual.",
            summary.not_classified
        ));
    }
    Ok(ImprovementReport {
        follows: follows.to_owned(),
        cases,
        performers: summary,
        rules: views,
        steps,
        revision: decisions.revision,
        value_field,
        notes,
    })
}

fn step_view(
    f: StepFacts,
    name: &dyn Fn(u32) -> String,
    traces: &dyn Traces,
    decisions: &Improvement,
) -> StepView {
    let activity = name(f.activity);
    let bucket = decisions.buckets.get(&activity).and_then(|b| {
        (b.said.status != Status::Rejected).then(|| BucketView {
            bucket: b.bucket,
            label: b.bucket.label(),
            status: b.said.status,
            by: b.said.by.clone(),
            reason: b.said.reason.clone(),
        })
    });
    let effort = decisions
        .efforts
        .get(&activity)
        .filter(|e| e.said.status == Status::Confirmed);
    // People's hours count the occurrences people did, and those nobody has
    // classified yet (they may be people too); systems' work is free here.
    let manual = f.by_people + f.not_classified;
    let people_hours = effort.map(|e| manual as f64 * e.minutes / 60.0);
    StepView {
        usual_next: f.usual_next.map(name),
        usual_next_percent: f.usual_next_share * 100.0,
        examples: f.examples.iter().map(|c| traces.case_label(*c)).collect(),
        effort_minutes: effort.map(|e| e.minutes),
        people_cost: effort
            .and_then(|e| e.cost_per_hour)
            .zip(people_hours)
            .map(|(rate, hours)| rate * hours),
        people_hours,
        activity,
        occurrences: f.occurrences,
        cases: f.cases,
        repeats: f.repeats,
        performers: f.performers,
        by_people: f.by_people,
        by_systems: f.by_systems,
        unrecorded: f.unrecorded,
        not_classified: f.not_classified,
        bucket,
    }
}

/// A case log as cases of steps.
pub struct CaseLog<'a>(pub &'a pm_core::EventLog);

impl Traces for CaseLog<'_> {
    fn activities(&self) -> Vec<String> {
        self.0
            .activities
            .labels()
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }
    fn performers(&self) -> Vec<String> {
        self.0
            .resources
            .labels()
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }
    fn each(&self, f: &mut dyn FnMut(u32, &[Step])) -> Result<(), String> {
        let mut steps = Vec::new();
        for trace in &self.0.traces {
            steps.clear();
            steps.extend(trace.events.iter().map(|e| Step {
                activity: e.activity.0,
                performer: e.resource().map(|r| r.0),
            }));
            f(trace.case.0, &steps);
        }
        Ok(())
    }
    fn case_label(&self, case: u32) -> String {
        self.0.case_label(pm_core::CaseId(case)).to_owned()
    }
    fn case_value(&self, case: u32, field: &str) -> Option<f64> {
        let trace = self.0.traces.get(case as usize)?;
        debug_assert_eq!(trace.case.0, case);
        parse_amount(self.0.attribute_value(trace, field)?)
    }
}

/// A number as exports write amounts: `1234.5`, `1,234.50`, `-12`.
pub fn parse_amount(text: &str) -> Option<f64> {
    let cleaned = text.trim().replace(',', "");
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Who did each event of a loaded connected log, numbered as on disk: the
/// performer names in first-seen event order, and each event's performer.
pub fn ocel_performers(log: &pm_core::OcelLog) -> (Vec<String>, Vec<Option<u32>>) {
    use pm_core::AttributeValue;
    let names = pm_ingest::ocel_sqlite::PERFORMER_ATTRIBUTES
        .iter()
        .filter_map(|name| log.attribute_names.get(name))
        .collect::<Vec<_>>();
    let mut text = vec![None::<String>; log.events.len()];
    // The first performer attribute in order of preference wins.
    for preferred in names.iter().rev() {
        for attribute in &log.event_attributes {
            if attribute.name.0 != *preferred {
                continue;
            }
            let value = match attribute.value {
                AttributeValue::String(id) => log.attribute_values.label(id.0).map(str::to_owned),
                AttributeValue::Integer(n) => Some(n.to_string()),
                AttributeValue::Float(x) => Some(x.to_string()),
                AttributeValue::Boolean(b) => Some(b.to_string()),
                AttributeValue::Time(_) => None,
            };
            text[attribute.event.0 as usize] = value.filter(|v| !v.trim().is_empty());
        }
    }
    let mut labels = Vec::new();
    let mut ids = HashMap::<String, u32>::new();
    let performers = text
        .into_iter()
        .map(|name| {
            name.map(|name| {
                *ids.entry(name.clone()).or_insert_with(|| {
                    labels.push(name);
                    (labels.len() - 1) as u32
                })
            })
        })
        .collect();
    (labels, performers)
}

/// A loaded connected log, one case per item of the followed type.
pub struct OcelCases<'a> {
    pub log: &'a pm_core::OcelLog,
    pub view: pm_core::OcelView,
    pub item_type: pm_core::ObjectTypeId,
    pub performer_names: Vec<String>,
    pub performers: Vec<Option<u32>>,
}

impl Traces for OcelCases<'_> {
    fn activities(&self) -> Vec<String> {
        self.log
            .event_types
            .iter()
            .map(|t| t.name.clone())
            .collect()
    }
    fn performers(&self) -> Vec<String> {
        self.performer_names.clone()
    }
    fn each(&self, f: &mut dyn FnMut(u32, &[Step])) -> Result<(), String> {
        let mut steps = Vec::new();
        pm_core::ocel_graph::for_each_timeline(self.log, &self.view, |object, kind, timeline| {
            if kind != self.item_type {
                return;
            }
            steps.clear();
            steps.extend(timeline.iter().map(|(_, event, kind)| Step {
                activity: kind.0,
                performer: self.performers.get(event.0 as usize).copied().flatten(),
            }));
            f(object.0, &steps);
        });
        Ok(())
    }
    fn case_label(&self, case: u32) -> String {
        self.log
            .object_ids
            .label(case)
            .unwrap_or("<unknown>")
            .to_owned()
    }
}

/// A connected log on disk, one case per item of the followed type.
pub struct DiskCases<'a> {
    pub log: &'a pm_ingest::DiskLog,
    pub view: pm_ingest::DiskView,
    pub performer_names: Vec<String>,
    pub performers: Vec<Option<u32>>,
}

impl Traces for DiskCases<'_> {
    fn activities(&self) -> Vec<String> {
        self.log.event_types.clone()
    }
    fn performers(&self) -> Vec<String> {
        self.performer_names.clone()
    }
    fn each(&self, f: &mut dyn FnMut(u32, &[Step])) -> Result<(), String> {
        let mut steps = Vec::new();
        self.log
            .for_each_timeline(&self.view, |object, _, timeline| {
                steps.clear();
                steps.extend(timeline.iter().map(|(_, event, kind)| Step {
                    activity: kind.0,
                    performer: self.performers.get(event.0 as usize).copied().flatten(),
                }));
                f(object.0, &steps);
                Ok(())
            })
            .map_err(|e| e.to_string())
    }
    fn case_label(&self, case: u32) -> String {
        self.log
            .object_label(pm_core::ObjectId(case))
            .unwrap_or_else(|_| "<unknown>".to_owned())
    }
}

/// The improvement view of a connected log file, following `item_type`;
/// read from disk for large saved logs, as every connected question is.
pub fn improvement_ocel(
    path: &std::path::Path,
    filter: &pm_core::OcelFilter,
    item_type: &str,
    decisions: &Improvement,
) -> Result<ImprovementReport, String> {
    if crate::follow::from_disk(path) {
        return improvement_ocel_disk(path, filter, item_type, decisions);
    }
    let log = pm_ingest::ocel_read::read_ocel(path).map_err(|e| e.to_string())?;
    improvement_ocel_log(&log, filter, item_type, decisions)
}

/// The improvement view of a connected log read from disk.
pub fn improvement_ocel_disk(
    path: &std::path::Path,
    filter: &pm_core::OcelFilter,
    item_type: &str,
    decisions: &Improvement,
) -> Result<ImprovementReport, String> {
    let log = pm_ingest::DiskLog::open_cached(path, &std::env::temp_dir().join("glasstarn-disk"))
        .map_err(|e| e.to_string())?;
    let kind = log
        .object_types
        .iter()
        .position(|t| t == item_type)
        .ok_or_else(|| format!("the log has no item type '{item_type}'"))?;
    let view = log.view(filter).map_err(|e| e.to_string())?;
    let (performer_names, performers) = log.event_performers().map_err(|e| e.to_string())?;
    let cases = DiskCases {
        log: &log,
        view: view.only_type(pm_core::ObjectTypeId(kind as u32)),
        performer_names,
        performers,
    };
    improvement_report(
        &cases,
        item_type,
        decisions,
        &BTreeMap::new(),
        &MiningSettings::default(),
    )
}

/// The improvement view of a loaded connected log.
pub fn improvement_ocel_log(
    log: &pm_core::OcelLog,
    filter: &pm_core::OcelFilter,
    item_type: &str,
    decisions: &Improvement,
) -> Result<ImprovementReport, String> {
    let kind = log
        .object_types
        .iter()
        .position(|t| t.name == item_type)
        .ok_or_else(|| format!("the log has no item type '{item_type}'"))?;
    let view = pm_core::OcelView::apply(log, filter).map_err(|e| e.to_string())?;
    let (performer_names, performers) = ocel_performers(log);
    let cases = OcelCases {
        log,
        view,
        item_type: pm_core::ObjectTypeId(kind as u32),
        performer_names,
        performers,
    };
    improvement_report(
        &cases,
        item_type,
        decisions,
        &BTreeMap::new(),
        &MiningSettings::default(),
    )
}

/// The improvement view of a case log file under its preparation.
pub fn improvement_case_log(
    path: &std::path::Path,
    mapping: Option<pm_ingest::validate::Mapping>,
    policy: &pm_ingest::Policy,
    rules: pm_ingest::Rules,
    decisions: &Improvement,
) -> Result<ImprovementReport, String> {
    let known = rules.resource_kinds.clone();
    let loaded = match crate::load_log(path, mapping, policy, rules).map_err(|e| e.to_string())? {
        Ok(loaded) => loaded,
        Err(_) => return Err("the data has open questions; answer them before this view".into()),
    };
    improvement_report(
        &CaseLog(&loaded.log),
        "case",
        decisions,
        &known,
        &MiningSettings::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_project::improvement::{
        Bucket, Said, SavedBucket, SavedEffort, SavedPerformer, SavedRule,
    };

    /// Three steps per case; one case in ten pays before approval, and the
    /// performers are a person, a batch job and nobody.
    struct Planted;

    impl Traces for Planted {
        fn activities(&self) -> Vec<String> {
            ["Receive invoice", "Approve invoice", "Pay invoice"]
                .map(String::from)
                .to_vec()
        }
        fn performers(&self) -> Vec<String> {
            ["anna", "batch_07"].map(String::from).to_vec()
        }
        fn each(&self, f: &mut dyn FnMut(u32, &[Step])) -> Result<(), String> {
            for case in 0..100u32 {
                let step = |activity, performer| Step {
                    activity,
                    performer,
                };
                let steps = if case % 10 == 0 {
                    vec![step(0, Some(1)), step(2, Some(1)), step(1, Some(0))]
                } else {
                    vec![step(0, Some(1)), step(1, Some(0)), step(2, Some(1))]
                };
                f(case, &steps);
            }
            Ok(())
        }
        fn case_label(&self, case: u32) -> String {
            format!("INV-{case}")
        }
        fn case_value(&self, _case: u32, _field: &str) -> Option<f64> {
            Some(250.0)
        }
    }

    fn said(status: Status) -> Said {
        Said {
            status,
            by: "Accounts payable lead".into(),
            at: "2026-10-14T00:00:00Z".into(),
            reason: None,
        }
    }

    #[test]
    fn checks_the_company_rules_and_totals_the_broken_cases() {
        let mut decisions = Improvement {
            value_field: Some("amount".into()),
            ..Improvement::default()
        };
        decisions.rules.push(SavedRule {
            template: Template::Precedence,
            a: "Approve invoice".into(),
            b: Some("Pay invoice".into()),
            said: said(Status::Confirmed),
        });
        decisions.rules.push(SavedRule {
            template: Template::Response,
            a: "Receive invoice".into(),
            b: Some("Book invoice".into()),
            said: said(Status::Suggested),
        });
        let report = improvement_report(
            &Planted,
            "invoice",
            &decisions,
            &BTreeMap::new(),
            &MiningSettings::default(),
        )
        .unwrap();
        let paid_first = &report.rules[0];
        assert_eq!(paid_first.status, "confirmed");
        assert_eq!((paid_first.applies_to, paid_first.broken), (100, 10));
        assert_eq!(paid_first.broken_value, Some(2500.0));
        assert_eq!(paid_first.examples[0], "INV-0");
        assert_eq!(
            paid_first.text,
            "“Pay invoice” happens only after “Approve invoice”"
        );
        // A step not in the data is said so, not silently passed.
        assert!(report.rules[1].problem.is_some());
        // Mined rules follow, labelled as found in the data.
        assert!(
            report.rules[2..]
                .iter()
                .all(|r| r.status == "found in the data")
        );
        assert!(
            report
                .rules
                .iter()
                .any(|r| r.template == Template::Init && r.a == "Receive invoice")
        );
    }

    #[test]
    fn steps_count_people_systems_and_hours() {
        let mut decisions = Improvement::default();
        decisions.performers.insert(
            "batch_07".into(),
            SavedPerformer {
                kind: ResourceKind::System,
                said: said(Status::Confirmed),
            },
        );
        decisions.buckets.insert(
            "Approve invoice".into(),
            SavedBucket {
                bucket: Bucket::RuleCandidate,
                said: said(Status::Suggested),
            },
        );
        decisions.efforts.insert(
            "Approve invoice".into(),
            SavedEffort {
                minutes: 6.0,
                cost_per_hour: Some(40.0),
                said: said(Status::Confirmed),
            },
        );
        let report = improvement_report(
            &Planted,
            "invoice",
            &decisions,
            &BTreeMap::new(),
            &MiningSettings::default(),
        )
        .unwrap();
        let approve = report
            .steps
            .iter()
            .find(|s| s.activity == "Approve invoice")
            .unwrap();
        // anna is not classified yet: her steps are not counted as manual.
        assert_eq!((approve.by_people, approve.not_classified), (0, 100));
        assert_eq!(approve.people_hours, Some(10.0));
        assert_eq!(approve.people_cost, Some(400.0));
        assert_eq!(approve.bucket.as_ref().unwrap().label, "Rule candidate");
        let pay = report
            .steps
            .iter()
            .find(|s| s.activity == "Pay invoice")
            .unwrap();
        assert_eq!(pay.by_systems, 100);
        assert_eq!(report.performers.systems, 1);
        assert_eq!(report.performers.undecided, vec!["anna"]);
        assert!(report.performers.look_like_systems.is_empty());
    }
}
