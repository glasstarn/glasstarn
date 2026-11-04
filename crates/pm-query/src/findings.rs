//! Key findings: the calculated facts worth reading first.
//!
//! Each finding states what the analysis measured, with its numbers, in a
//! neutral sentence. It does not say why or what to do: that is the
//! interpretation an agent adds, labelled as such. Findings are ranked by
//! severity and then by how much of the process they touch, and each names
//! the view that holds its evidence.
//!
//! Text uses `{items}` and `{item}` for the kind of item (invoices, tickets),
//! and `{type:<name>}` for another item type in the plural; each surface
//! substitutes its own business names, as [`render`] does.

use crate::analysis::Analysis;
use crate::journey::Journey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    High,
    Medium,
    Low,
    /// Something working well.
    Good,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Bottleneck,
    DropOut,
    Rework,
    Variation,
    SlowTail,
    Open,
    WorkingWell,
    /// A field value much more common among the slow items.
    Driver,
    /// A step that almost always comes first is missing in some items.
    SkippedStep,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyFinding {
    pub kind: FindingKind,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub tags: Vec<String>,
    /// The view holding the evidence: `timing`, `map`, `variants`,
    /// `journey` or `cases`.
    pub view: String,
    /// Share of items or of waiting time involved, 0–100; ranks findings of
    /// equal severity.
    pub reach_percent: f64,
    /// The finding's size in a few characters, for a list ("14.2M",
    /// "24% of time"). Absent from older caches.
    #[serde(default)]
    pub impact: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<FindingTarget>,
    /// The money in the items this finding touches, when the items carry an
    /// amount. Summed from the data, no assumptions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<FindingValue>,
    /// The filter that selects exactly the items this finding is about, so
    /// opening it shows those items and nothing else. Absent from older caches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<pm_core::Filter>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingValue {
    /// The amount column summed.
    pub field: String,
    pub total: f64,
    /// Items with an amount among those the finding touches.
    pub items: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FindingTarget {
    Step {
        activity: String,
    },
    Edge {
        from: String,
        to: String,
    },
    Handoff {
        object_type: String,
        from: String,
        to: String,
    },
    /// Items whose field has this value, or, for a number, lies in the band.
    Field {
        field: String,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at_least: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at_most: Option<f64>,
    },
}

/// Substitutes the item names into a finding's text.
pub fn render(text: &str, singular: &str, items: &str) -> String {
    let mut out = text.replace("{items}", items).replace("{item}", singular);
    while let Some(start) = out.find("{type:") {
        let Some(length) = out[start..].find('}') else {
            break;
        };
        let name = out[start + 6..start + length].to_owned();
        out.replace_range(start..=start + length, &plural(&name));
    }
    out
}

/// Findings from a case analysis, most important first, at most `limit`.
pub fn key_findings(analysis: &Analysis, limit: usize) -> Vec<KeyFinding> {
    // A customer/product history can contain many independent transactions.
    // Their gaps and repeated labels are not process failures. Keep the raw
    // metrics available; transaction-linked measurements supply findings.
    if analysis.mapping.object_history {
        return Vec::new();
    }
    let mut findings = Vec::new();
    let summary = &analysis.summary;
    let cases = summary.cases as f64;

    // Where the waiting is.
    if let Some(slowest) = analysis.bottlenecks.first()
        && slowest.share_percent >= 10.0
        && slowest.mean_seconds > 0
    {
        findings.push(KeyFinding {
            value: None,
            filter: None,
            target: Some(FindingTarget::Edge {
                from: slowest.from.clone(),
                to: slowest.to.clone(),
            }),
            kind: FindingKind::Bottleneck,
            severity: if slowest.share_percent >= 30.0 {
                Severity::High
            } else {
                Severity::Medium
            },
            title: format!(
                "{} → {} takes {} on average",
                slowest.from,
                slowest.to,
                duration(slowest.mean_seconds)
            ),
            detail: format!(
                "{} of all recorded time between steps is spent here, over {} {}.",
                percent(slowest.share_percent),
                count(slowest.total_occurrences),
                if slowest.total_occurrences == 1 {
                    "hand-off"
                } else {
                    "hand-offs"
                }
            ),
            tags: vec![
                "Bottleneck".into(),
                format!("{} → {}", slowest.from, slowest.to),
            ],
            view: "timing".into(),
            reach_percent: slowest.share_percent,
            impact: format!("{} of time", percent(slowest.share_percent)),
        });
    }

    // Repeated work.
    if let Some(repeated) = analysis.rework.activities.first()
        && analysis.rework.case_percent >= 2.0
    {
        let share = analysis.rework.case_percent;
        findings.push(KeyFinding {
            value: None,
            filter: None,
            target: Some(FindingTarget::Step {
                activity: repeated.activity.clone(),
            }),
            kind: FindingKind::Rework,
            severity: if share >= 20.0 {
                Severity::High
            } else if share >= 5.0 {
                Severity::Medium
            } else {
                Severity::Low
            },
            title: format!("{} of {{items}} repeat a step", percent(share)),
            detail: format!(
                "“{}” is repeated most: in {} {{items}}, {} times in all{}.",
                repeated.activity,
                count(repeated.cases_affected),
                count(repeated.repetitions),
                if repeated.mean_repeat_interval_seconds > 0 {
                    format!(
                        ", {} apart on average",
                        duration(repeated.mean_repeat_interval_seconds)
                    )
                } else {
                    String::new()
                }
            ),
            tags: vec!["Rework".into(), repeated.activity.clone()],
            view: "map".into(),
            reach_percent: share,
            impact: format!("{} {{items}}", count(analysis.rework.cases_affected)),
        });
    }

    // Items off the main routes.
    let main = analysis.variants.iter().take(3).collect::<Vec<_>>();
    let main_cases = main.iter().map(|variant| variant.cases).sum::<usize>();
    let main_eligible = main
        .iter()
        .map(|variant| variant.duration_eligible_cases)
        .sum::<usize>();
    let rest_cases = summary.cases.saturating_sub(main_cases);
    let rest_eligible = summary
        .duration_eligible_cases
        .saturating_sub(main_eligible);
    if rest_eligible > 0 && main_eligible > 0 && summary.variants > 3 {
        let main_total = main
            .iter()
            .map(|variant| {
                variant.mean_throughput_seconds as f64 * variant.duration_eligible_cases as f64
            })
            .sum::<f64>();
        let all_total =
            summary.mean_throughput_seconds as f64 * summary.duration_eligible_cases as f64;
        let main_mean = main_total / main_eligible as f64;
        let rest_mean = (all_total - main_total) / rest_eligible as f64;
        let rest_share = rest_cases as f64 / cases.max(1.0) * 100.0;
        if main_mean > 0.0 && rest_mean >= main_mean * 1.3 && rest_share >= 5.0 {
            let ratio = rest_mean / main_mean;
            findings.push(KeyFinding {
                value: None,
                filter: None,
                target: None,
            kind: FindingKind::Variation,
                severity: if ratio >= 2.0 && rest_share >= 15.0 {
                    Severity::High
                } else {
                    Severity::Medium
                },
                title: format!(
                    "{{items}} off the 3 main variants take {:.1}× longer",
                    ratio
                ),
                detail: format!(
                    "{} {{items}} ({}) follow one of {} other variants: {} on average, against {} on the main variants.",
                    count(rest_cases as u64),
                    percent(rest_share),
                    count(summary.variants.saturating_sub(3) as u64),
                    duration(rest_mean as i64),
                    duration(main_mean as i64)
                ),
                tags: vec!["Variation".into(), format!("{} variants", count(summary.variants as u64))],
                view: "variants".into(),
                reach_percent: rest_share,
                impact: format!("{ratio:.1}× slower"),
            });
        }
    }

    // A long slow end.
    let (median, p90) = (
        summary.median_throughput_seconds,
        summary.p90_throughput_seconds,
    );
    if median > 0 && p90 >= median * 3 {
        findings.push(KeyFinding {
            value: None,
            filter: None,
            target: None,
            kind: FindingKind::SlowTail,
            severity: Severity::Medium,
            title: format!("1 in 10 {{items}} take more than {}", duration(p90)),
            detail: format!(
                "Half finish within {}; the slowest tenth take over {}× as long.",
                duration(median),
                p90 / median
            ),
            tags: vec!["Slow end".into()],
            view: "timing".into(),
            reach_percent: 10.0,
            impact: format!("> {}", duration(p90)),
        });
    }

    // Still open at the end of the data, when finishing steps are known.
    if let Some(window) = &analysis.window
        && window.end_defined
        && window.still_open > 0
    {
        let share = window.still_open as f64 / window.cases.max(1) as f64 * 100.0;
        findings.push(KeyFinding {
            value: None,
            filter: None,
            target: None,
            kind: FindingKind::Open,
            severity: if share >= 20.0 {
                Severity::High
            } else {
                Severity::Medium
            },
            title: format!(
                "{} {{items}} had not finished when the data was taken",
                count(window.still_open as u64)
            ),
            detail: format!(
                "{} of {{items}} in the window had not reached a finishing step ({}).",
                percent(share),
                window.end_activities.join(", ")
            ),
            tags: vec!["Still open".into()],
            view: "cases".into(),
            reach_percent: share,
            impact: format!("{} open", count(window.still_open as u64)),
        });
    }

    // A frequent, quick hand-off.
    let busiest = analysis
        .edges
        .iter()
        .filter(|edge| edge.duration_eligible_occurrences > 0)
        .max_by_key(|edge| edge.case_frequency);
    let typical = {
        let mut means = analysis
            .edges
            .iter()
            .filter(|edge| edge.duration_eligible_occurrences > 0)
            .map(|edge| edge.mean_seconds)
            .collect::<Vec<_>>();
        means.sort_unstable();
        means.get(means.len() / 2).copied()
    };
    if let (Some(edge), Some(typical)) = (busiest, typical)
        && edge.mean_seconds <= typical / 2
        && edge.case_frequency as f64 >= cases * 0.5
    {
        findings.push(KeyFinding {
            value: None,
            filter: None,
            target: Some(FindingTarget::Edge {
                from: edge.from.clone(),
                to: edge.to.clone(),
            }),
            kind: FindingKind::WorkingWell,
            severity: Severity::Good,
            title: format!(
                "{} → {} is quick: {} on average",
                edge.from,
                edge.to,
                duration(edge.mean_seconds)
            ),
            detail: format!(
                "It happens in {} of {{items}}, and takes under half the typical hand-off time.",
                percent(edge.case_frequency as f64 / cases.max(1.0) * 100.0)
            ),
            tags: vec!["Working well".into()],
            view: "map".into(),
            reach_percent: edge.case_frequency as f64 / cases.max(1.0) * 100.0,
            impact: duration(edge.mean_seconds),
        });
    }

    // The longest single wait, when it is not the hand-off with the most time
    // in all: a rarer step that takes long each time it happens.
    if let Some(edge) = slowest_wait(analysis)
        && analysis
            .bottlenecks
            .first()
            .is_none_or(|top| top.from != edge.from || top.to != edge.to)
    {
        findings.push(KeyFinding {
            value: None,
            filter: None,
            target: Some(FindingTarget::Edge {
                from: edge.from.clone(),
                to: edge.to.clone(),
            }),
            kind: FindingKind::Bottleneck,
            severity: Severity::Medium,
            title: format!(
                "{} → {} is the longest wait: {} each time on average",
                edge.from,
                edge.to,
                duration(edge.mean_seconds)
            ),
            detail: format!(
                "It happens {} times in {} {{items}} ({}).",
                count(edge.duration_eligible_occurrences),
                count(edge.case_frequency),
                percent(edge.case_frequency as f64 / cases.max(1.0) * 100.0)
            ),
            tags: vec![
                "Longest wait".into(),
                format!("{} → {}", edge.from, edge.to),
            ],
            view: "timing".into(),
            reach_percent: edge.case_frequency as f64 / cases.max(1.0) * 100.0,
            impact: duration(edge.mean_seconds),
        });
    }

    // Where one hand-off is slow for some items only: a region slow to start.
    for step in &analysis.step_drivers {
        let driver = &step.driver;
        if driver.lift < 1.5
            || (driver.median_seconds as f64) < driver.others_median_seconds as f64 * 1.5
            || driver.share_of_items > 50.0
        {
            continue;
        }
        let who = if driver.field == "Handled by" {
            format!("handled by {}", driver.value)
        } else if driver.at_least.is_some() {
            format!("with {} {}", driver.field, driver.value)
        } else {
            format!("with {} = {}", driver.field, driver.value)
        };
        findings.push(KeyFinding {
            value: None,
            filter: None,
            target: Some(FindingTarget::Field {
                field: driver.field.clone(),
                value: driver.value.clone(),
                at_least: driver.at_least,
                at_most: driver.at_most,
            }),
            kind: FindingKind::Driver,
            severity: if driver.lift >= 2.0 {
                Severity::Medium
            } else {
                Severity::Low
            },
            title: format!(
                "{} → {} takes {} for {{items}} {}, against {} for the rest",
                step.from,
                step.to,
                duration(driver.median_seconds),
                who,
                duration(driver.others_median_seconds)
            ),
            detail: format!(
                "Median times for this one hand-off, over {} {{items}}; {} of them, but {} of the slowest fifth here. A pattern, not a proven cause.",
                count(step.measured as u64),
                percent(driver.share_of_items),
                percent(driver.share_of_slow)
            ),
            tags: vec!["Slow step".into(), driver.field.clone()],
            view: "timing".into(),
            reach_percent: driver.share_of_slow,
            impact: format!("{:.1}× as likely", driver.lift),
        });
    }

    // A usual step skipped: payment without its approval.
    for skipped in &analysis.skipped_steps {
        let usual_share =
            (skipped.reaching - skipped.missing) as f64 * 100.0 / skipped.reaching.max(1) as f64;
        let shared = skipped
            .concentrated_in
            .as_ref()
            .map_or_else(String::new, |c| {
                let all = if c.items == skipped.missing {
                    "All"
                } else {
                    "Most"
                };
                if c.field == "Handled by" {
                    format!(
                        " {all} {} were handled by {}.",
                        count(c.items as u64),
                        c.value
                    )
                } else {
                    format!(
                        " {all} {} have {} = {}.",
                        count(c.items as u64),
                        c.field,
                        c.value
                    )
                }
            });
        findings.push(KeyFinding {
            value: None,
            filter: None,
            target: None,
            kind: FindingKind::SkippedStep,
            severity: if skipped.concentrated_in.is_some() {
                Severity::High
            } else {
                Severity::Medium
            },
            title: format!(
                "{} {{items}} reached {} without {}",
                count(skipped.missing as u64),
                skipped.reached,
                skipped.usual
            ),
            detail: format!(
                "{} comes before {} in {} of {{items}} that reach it.{}",
                skipped.usual,
                skipped.reached,
                percent(usual_share),
                shared
            ),
            tags: vec!["Skipped step".into(), skipped.usual.clone()],
            view: "cases".into(),
            reach_percent: skipped.missing as f64 / cases.max(1.0) * 100.0,
            impact: format!("{} {{items}}", count(skipped.missing as u64)),
        });
    }

    // What goes with the slow items: the strongest field values.
    // A value most items share says little about the slow ones.
    for driver in analysis
        .drivers
        .drivers
        .iter()
        .filter(|driver| driver.share_of_items <= 50.0)
        .take(4)
    {
        let band = driver.at_least.is_some();
        let who = if driver.field == "Handled by" {
            format!("{{items}} handled by {}", driver.value)
        } else if band {
            format!("{{items}} with {} {}", driver.field, driver.value)
        } else {
            format!("{{items}} with {} = {}", driver.field, driver.value)
        };
        findings.push(KeyFinding {
            value: None,
            filter: None,
            target: Some(FindingTarget::Field {
                field: driver.field.clone(),
                value: driver.value.clone(),
                at_least: driver.at_least,
                at_most: driver.at_most,
            }),
            kind: FindingKind::Driver,
            severity: if driver.lift >= 2.0 && driver.share_of_slow >= 10.0 {
                Severity::Medium
            } else {
                Severity::Low
            },
            title: format!("{} are {:.1}× as likely to be slow", capitalised(&who), driver.lift),
            detail: format!(
                "{} of {{items}}, but {} of the slowest fifth; their median time is {} against {} for the rest. A pattern, not a proven cause.",
                percent(driver.share_of_items),
                percent(driver.share_of_slow),
                duration(driver.median_seconds),
                duration(driver.others_median_seconds)
            ),
            tags: vec!["Driver".into(), driver.field.clone()],
            view: "map".into(),
            reach_percent: driver.share_of_slow,
            impact: format!("{:.1}× as likely", driver.lift),
        });
    }

    for finding in &mut findings {
        finding.filter = finding_filter(finding, analysis);
    }
    let emphasis = analysis
        .process
        .as_ref()
        .map(|profile| profile.emphasis.clone())
        .unwrap_or_default();
    rank_for(findings, &emphasis, limit)
}

/// The filter that selects the items a finding is about: those through its
/// hand-off, those repeating its step, those off the main routes, the slowest
/// tenth, those not finished, or those with its field value or person.
fn finding_filter(finding: &KeyFinding, analysis: &Analysis) -> Option<pm_core::Filter> {
    let mut filter = pm_core::Filter::new();
    match (&finding.kind, &finding.target) {
        (_, Some(FindingTarget::Edge { from, to })) => {
            filter.directly_follows.insert((from.clone(), to.clone()));
        }
        // The finding counts items repeating any step, so its filter does too;
        // the step it names is only the one repeated most.
        (FindingKind::Rework, Some(FindingTarget::Step { .. })) => {
            filter.repeats_any = true;
        }
        (_, Some(FindingTarget::Step { activity })) => {
            filter.with_activities.insert(activity.clone());
        }
        (
            _,
            Some(FindingTarget::Field {
                field,
                value,
                at_least,
                at_most,
            }),
        ) => {
            if field == "Handled by" {
                filter.handled_by.insert(value.clone());
            } else if let (Some(low), Some(high)) = (at_least, at_most) {
                filter.attribute_between.push(pm_core::NumberRange {
                    attribute: field.clone(),
                    at_least: Some(*low),
                    at_most: Some(*high),
                    greater_than: None,
                    less_than: None,
                });
            } else {
                filter.attribute_equals.push((field.clone(), value.clone()));
            }
        }
        (FindingKind::Variation, None) => filter.outside_top_variants = Some(3),
        (FindingKind::SkippedStep, None) => {
            let (usual, reached) = finding.tags.get(1).zip(skipped_reached(finding))?;
            filter.with_activities.insert(reached);
            filter.without_activities.insert(usual.clone());
        }
        (FindingKind::SlowTail, None) => {
            filter.min_duration = Some(analysis.summary.p90_throughput_seconds * 1_000_000);
        }
        (FindingKind::Open, None) => {
            let window = analysis.window.as_ref()?;
            filter.not_ending_with = window.end_activities.iter().cloned().collect();
        }
        _ => return None,
    }
    Some(filter)
}

/// The step a skipped-step finding's items reached, from its title.
fn skipped_reached(finding: &KeyFinding) -> Option<String> {
    let rest = finding.title.split_once(" reached ")?.1;
    Some(rest.rsplit_once(" without ")?.0.to_owned())
}

/// The hand-off that takes longest each time it happens, among those in at
/// least one item in twenty.
fn slowest_wait(analysis: &Analysis) -> Option<&crate::analysis::Edge> {
    let cases = analysis.summary.cases.max(1) as f64;
    analysis
        .edges
        .iter()
        .filter(|edge| {
            edge.duration_eligible_occurrences >= 20 && edge.case_frequency as f64 >= cases * 0.05
        })
        .max_by_key(|edge| edge.mean_seconds)
        .filter(|edge| edge.mean_seconds > 0)
}

/// The hand-offs whose slowness is worth explaining by field: the two with
/// the most time in all and the longest single wait.
pub fn edges_worth_explaining(analysis: &Analysis) -> Vec<(String, String)> {
    let mut edges: Vec<(String, String)> = analysis
        .bottlenecks
        .iter()
        .take(2)
        .map(|b| (b.from.clone(), b.to.clone()))
        .collect();
    if let Some(edge) = slowest_wait(analysis) {
        let pair = (edge.from.clone(), edge.to.clone());
        if !edges.contains(&pair) {
            edges.push(pair);
        }
    }
    edges
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Puts the money on each finding: the amount carried by the items it
/// touches (those through its step or hand-off, those that repeat its step,
/// those with its field value). Only when the items carry an amount.
pub fn attach_values(
    findings: &mut [KeyFinding],
    log: &pm_core::EventLog,
    fields: &[pm_core::NumberField],
) {
    let Some(field) = pm_core::main_number_field(fields) else {
        return;
    };
    let Some(index) = log.attribute_index(&field.name) else {
        return;
    };
    for finding in findings.iter_mut() {
        if let Some(filter) = &finding.filter {
            let chosen = filter.apply(log);
            let (mut total, mut items) = (0.0, 0usize);
            for trace in &chosen.log.traces {
                if let Some(amount) = pm_core::case_number(&chosen.log, trace, index) {
                    total += amount;
                    items += 1;
                }
            }
            if items > 0 {
                finding.value = Some(FindingValue {
                    field: field.name.clone(),
                    total,
                    items,
                });
            }
            continue;
        }
        let Some(target) = &finding.target else {
            continue;
        };
        let touches = |trace: &pm_core::CaseTrace| -> bool {
            match target {
                FindingTarget::Step { activity } => {
                    let Some(id) = log.activities.get(activity) else {
                        return false;
                    };
                    let seen = trace
                        .events
                        .iter()
                        .filter(|event| event.activity.0 == id)
                        .count();
                    if finding.kind == FindingKind::Rework {
                        seen > 1
                    } else {
                        seen > 0
                    }
                }
                FindingTarget::Edge { from, to } => {
                    let (Some(a), Some(b)) = (log.activities.get(from), log.activities.get(to))
                    else {
                        return false;
                    };
                    trace
                        .events
                        .windows(2)
                        .any(|pair| pair[0].activity.0 == a && pair[1].activity.0 == b)
                }
                FindingTarget::Field {
                    field,
                    value,
                    at_least,
                    at_most,
                } => {
                    if field == "Handled by" {
                        let Some(person) = log.resources.get(value) else {
                            return false;
                        };
                        return trace
                            .events
                            .iter()
                            .any(|event| event.resource().is_some_and(|r| r.0 == person));
                    }
                    let Some(position) = log.attribute_index(field) else {
                        return false;
                    };
                    match (at_least, at_most) {
                        (Some(low), Some(high)) => pm_core::case_number(log, trace, position)
                            .is_some_and(|n| n >= *low && n <= *high),
                        _ => log
                            .attribute_values
                            .get(value)
                            .is_some_and(|id| log.case_had(trace, position, id)),
                    }
                }
                FindingTarget::Handoff { .. } => false,
            }
        };
        let (mut total, mut items) = (0.0, 0usize);
        for trace in log.traces.iter().filter(|trace| touches(trace)) {
            if let Some(amount) = pm_core::case_number(log, trace, index) {
                total += amount;
                items += 1;
            }
        }
        if items > 0 {
            finding.value = Some(FindingValue {
                field: field.name.clone(),
                total,
                items,
            });
        }
    }
}

/// Findings from a connected journey: items that stopped short of the next
/// stage, with the amount they carry when known.
pub fn journey_findings(journey: &Journey) -> Vec<KeyFinding> {
    journey
        .handoffs
        .iter()
        .filter(|handoff| handoff.started > handoff.reached)
        .map(|handoff| {
            let stopped = handoff.started - handoff.reached;
            let share = stopped as f64 / handoff.started.max(1) as f64 * 100.0;
            let worth = handoff
                .not_reached_value
                .as_ref()
                .map(|value| format!(" They carry {} in {}.", amount(value.total), value.field))
                .unwrap_or_default();
            KeyFinding {
                value: None,
                filter: None,
                target: Some(FindingTarget::Handoff { object_type: handoff.via_object_type.clone(), from: handoff.from_activity.clone(), to: handoff.to_activity.clone() }),
            kind: FindingKind::DropOut,
                severity: if share >= 20.0 {
                    Severity::High
                } else if share >= 5.0 {
                    Severity::Medium
                } else {
                    Severity::Low
                },
                title: format!(
                    "{} {{type:{}}} with “{}” have no “{}” yet",
                    count(stopped),
                    handoff.via_object_type,
                    handoff.from_activity,
                    handoff.to_activity,
                ),
                detail: format!(
                    "{} of {} {{type:{}}} that reached “{}” went no further in this data; some may still be in progress.{}",
                    percent(share),
                    count(handoff.started),
                    handoff.via_object_type,
                    handoff.from_activity,
                    worth
                ),
                tags: vec![
                    "Drop-out".into(),
                    format!("{} → {}", handoff.from_activity, handoff.to_activity),
                ],
                view: "journey".into(),
                reach_percent: share,
                impact: handoff
                    .not_reached_value
                    .as_ref()
                    .map_or_else(|| percent(share), |value| amount(value.total)),
            }
        })
        .collect()
}

/// Most important first: severity, then reach; working-well findings last.
pub fn rank(findings: Vec<KeyFinding>, limit: usize) -> Vec<KeyFinding> {
    rank_for(findings, &[], limit)
}

/// As [`rank`], with the kinds that matter most for the process's goal first
/// among findings of equal severity.
pub fn rank_for(
    mut findings: Vec<KeyFinding>,
    emphasis: &[FindingKind],
    limit: usize,
) -> Vec<KeyFinding> {
    let weight = |kind: FindingKind| {
        emphasis
            .iter()
            .position(|wanted| *wanted == kind)
            .unwrap_or(emphasis.len())
    };
    findings.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(weight(a.kind).cmp(&weight(b.kind)))
            .then(b.reach_percent.total_cmp(&a.reach_percent))
    });
    findings.truncate(limit);
    findings
}

/// A plain English plural for an item type named in the singular; names
/// already in the plural ("orders") are kept.
pub fn plural(name: &str) -> String {
    let lower = name.to_lowercase();
    let before_y = lower.strip_suffix('y').and_then(|stem| stem.chars().last());
    if lower.ends_with('s') && !lower.ends_with("ss") {
        name.to_owned()
    } else if before_y.is_some_and(|c| !"aeiou".contains(c)) {
        format!("{}ies", &name[..name.len() - 1])
    } else if ["ss", "x", "z", "ch", "sh"]
        .iter()
        .any(|end| lower.ends_with(end))
    {
        format!("{name}es")
    } else {
        format!("{name}s")
    }
}

fn duration(seconds: i64) -> String {
    let seconds = seconds.max(0) as f64;
    let (value, unit) = if seconds >= 86_400.0 {
        (seconds / 86_400.0, "day")
    } else if seconds >= 3_600.0 {
        (seconds / 3_600.0, "hour")
    } else if seconds >= 60.0 {
        (seconds / 60.0, "minute")
    } else {
        (seconds, "second")
    };
    let shown = if value >= 10.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    };
    let shown = shown.trim_end_matches(".0").to_owned();
    format!("{shown} {unit}{}", if shown == "1" { "" } else { "s" })
}

fn percent(value: f64) -> String {
    if value >= 10.0 {
        format!("{value:.0}%")
    } else {
        format!("{value:.1}%")
    }
}

fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn amount(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if magnitude >= 10_000.0 {
        format!("{:.0}K", value / 1_000.0)
    } else {
        count(value.round().max(0.0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Outcome, analyze_filtered};
    use pm_ingest::finding::Policy;

    fn analysis(csv: &str) -> Analysis {
        let dir = std::env::temp_dir().join(format!(
            "pm-findings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log.csv");
        std::fs::write(&path, csv).unwrap();
        let Outcome::Complete(analysis) = analyze_filtered(
            &path,
            None,
            &Policy::new(),
            Default::default(),
            Default::default(),
        )
        .unwrap() else {
            panic!("blocked")
        };
        *analysis
    }

    /// Ten invoices: approval waits days, payment follows within an hour,
    /// and three invoices are approved twice.
    fn invoices() -> Analysis {
        let mut csv = String::from("case_id,activity,timestamp\n");
        for n in 0..10 {
            let wait = if n < 9 { 2 + n % 3 } else { 30 };
            csv.push_str(&format!("I{n},Receive,2026-01-01T08:00:00Z\n"));
            csv.push_str(&format!("I{n},Approve,2026-01-{:02}T08:00:00Z\n", 1 + wait));
            if n < 3 {
                csv.push_str(&format!("I{n},Approve,2026-01-{:02}T09:00:00Z\n", 1 + wait));
            }
            csv.push_str(&format!("I{n},Pay,2026-01-{:02}T09:30:00Z\n", 1 + wait));
        }
        analysis(&csv)
    }

    #[test]
    fn the_largest_waiting_and_repeated_work_come_first_with_their_numbers() {
        let findings = key_findings(&invoices(), 5);
        let bottleneck = &findings[0];
        assert_eq!(bottleneck.kind, FindingKind::Bottleneck);
        assert_eq!(bottleneck.severity, Severity::High);
        assert!(
            bottleneck.title.starts_with("Receive → Approve takes"),
            "{}",
            bottleneck.title
        );
        let rework = findings
            .iter()
            .find(|f| f.kind == FindingKind::Rework)
            .unwrap();
        assert_eq!(rework.title, "30% of {items} repeat a step");
        assert!(
            rework
                .detail
                .contains("“Approve” is repeated most: in 3 {items}")
        );
        // Neutral facts only: no advice or causes.
        for finding in &findings {
            for word in ["should", "because", "recommend"] {
                assert!(!finding.title.contains(word) && !finding.detail.contains(word));
            }
        }
    }

    #[test]
    fn severities_rank_before_reach_and_good_news_comes_last() {
        let finding = |severity, reach| KeyFinding {
            value: None,
            filter: None,
            target: None,
            kind: FindingKind::Rework,
            severity,
            title: String::new(),
            detail: String::new(),
            tags: vec![],
            view: String::new(),
            reach_percent: reach,
            impact: String::new(),
        };
        let ranked = rank(
            vec![
                finding(Severity::Good, 90.0),
                finding(Severity::Medium, 50.0),
                finding(Severity::High, 10.0),
                finding(Severity::Medium, 70.0),
            ],
            3,
        );
        let order = ranked
            .iter()
            .map(|f| (f.severity, f.reach_percent))
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            [
                (Severity::High, 10.0),
                (Severity::Medium, 70.0),
                (Severity::Medium, 50.0)
            ]
        );
    }

    #[test]
    fn the_process_goal_orders_findings_of_equal_severity() {
        let finding = |kind, reach| KeyFinding {
            value: None,
            filter: None,
            target: None,
            kind,
            severity: Severity::Medium,
            title: String::new(),
            detail: String::new(),
            tags: vec![],
            view: String::new(),
            reach_percent: reach,
            impact: String::new(),
        };
        let ranked = rank_for(
            vec![
                finding(FindingKind::Bottleneck, 40.0),
                finding(FindingKind::SlowTail, 10.0),
            ],
            &[FindingKind::SlowTail, FindingKind::Bottleneck],
            5,
        );
        assert_eq!(ranked[0].kind, FindingKind::SlowTail);
        let plain = rank(
            vec![
                finding(FindingKind::Bottleneck, 40.0),
                finding(FindingKind::SlowTail, 10.0),
            ],
            5,
        );
        assert_eq!(plain[0].kind, FindingKind::Bottleneck);
    }

    #[test]
    fn item_names_and_numbers_read_naturally() {
        assert_eq!(
            render("{items} and one {item}", "ticket", "tickets"),
            "tickets and one ticket"
        );
        assert_eq!(duration(86_400 * 19 + 70_000), "20 days");
        assert_eq!(duration(5_400), "1.5 hours");
        assert_eq!(duration(86_400), "1 day");
        assert_eq!(count(1_234_567), "1,234,567");
        for (one, many) in [
            ("invoice", "invoices"),
            ("opportunity", "opportunities"),
            ("orders", "orders"),
            ("box", "boxes"),
            ("day", "days"),
            ("address", "addresses"),
        ] {
            assert_eq!(plural(one), many);
        }
        assert_eq!(amount(1_279_130.0), "1.3M");
        assert_eq!(amount(396_870.0), "397K");
    }
}
