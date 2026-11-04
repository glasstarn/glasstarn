//! Amounts carried by cases: totals, averages and medians, overall or by group.
//!
//! "How much money is waiting at each step?" and "which route carries the most
//! value?" are counts weighted by a number the case holds. The number is a
//! case attribute read as described in [`pm_core::number`]. Cases without a
//! readable number are counted beside the figures, never treated as zero.

use pm_core::{CaseTrace, EventLog, case_number, number_fields};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// How cases are grouped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "by", content = "attribute", rename_all = "snake_case")]
pub enum ValueGrouping {
    /// One figure for the whole view.
    #[default]
    None,
    /// By route: the case's full sequence of steps.
    Variant,
    /// By the last recorded step: where each case is, or where it stopped.
    LastStep,
    /// By the value of another case attribute.
    Attribute(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValueError {
    #[error("'{0}' is not a case attribute in this view")]
    UnknownField(String),
    #[error(
        "'{0}' does not hold numbers in this view; fields that do: {1}. Counts and times are still available."
    )]
    NotNumeric(String, String),
}

/// Figures for one set of cases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueStats {
    pub cases: usize,
    /// Cases whose value reads as a number; the figures cover these.
    pub cases_with_value: usize,
    pub total: f64,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    /// Nearest-rank 90th percentile.
    pub p90: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Cases not at a confirmed finishing step, and their total. Absent when
    /// no finishing steps are confirmed, because openness is then unknown.
    pub open_cases: Option<usize>,
    pub open_total: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueGroup {
    pub label: String,
    pub stats: ValueStats,
    /// A case in the group, for drill-down.
    pub example_case: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueSummary {
    pub field: String,
    pub grouping: ValueGrouping,
    pub overall: ValueStats,
    /// Largest total first, at most the requested number.
    pub groups: Vec<ValueGroup>,
    pub groups_available: usize,
    /// Figures for the groups left out by the limit, if any.
    pub other_groups: Option<ValueStats>,
    pub semantics: Vec<String>,
}

/// Sums and distributions of `field` over the cases of `log`.
///
/// `finishing` lists the steps that finish a case; when it is empty, open
/// amounts are not reported.
pub fn summarize_values(
    log: &EventLog,
    field: &str,
    grouping: &ValueGrouping,
    finishing: &BTreeSet<String>,
    limit: usize,
) -> Result<ValueSummary, ValueError> {
    let index = log
        .attribute_index(field)
        .ok_or_else(|| ValueError::UnknownField(field.to_owned()))?;
    let numeric = number_fields(log);
    if !numeric.iter().any(|candidate| candidate.name == field) {
        let names = numeric
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>();
        return Err(ValueError::NotNumeric(
            field.to_owned(),
            if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            },
        ));
    }
    let group_index = match grouping {
        ValueGrouping::Attribute(name) => Some(
            log.attribute_index(name)
                .ok_or_else(|| ValueError::UnknownField(name.clone()))?,
        ),
        _ => None,
    };
    let finishing_ids = finishing
        .iter()
        .filter_map(|name| log.activities.get(name))
        .collect::<BTreeSet<_>>();
    let measure_open = !finishing.is_empty();

    let mut all = Accumulator::default();
    let mut groups: BTreeMap<String, (Accumulator, String)> = BTreeMap::new();
    for trace in &log.traces {
        let value = case_number(log, trace, index);
        let open = measure_open
            && trace
                .events
                .last()
                .is_none_or(|event| !finishing_ids.contains(&event.activity.0));
        all.add(value, open);
        if let Some(label) = group_label(log, trace, grouping, group_index) {
            groups
                .entry(label)
                .or_insert_with(|| {
                    (
                        Accumulator::default(),
                        log.case_label(trace.case).to_owned(),
                    )
                })
                .0
                .add(value, open);
        }
    }

    let mut ranked = groups
        .into_iter()
        .map(|(label, (accumulator, example))| (label, accumulator, example))
        .collect::<Vec<_>>();
    // Largest total first; ties by more cases, then label, so output is stable.
    ranked.sort_by(|a, b| {
        b.1.total
            .total_cmp(&a.1.total)
            .then(b.1.cases.cmp(&a.1.cases))
            .then(a.0.cmp(&b.0))
    });
    let groups_available = ranked.len();
    let mut other = Accumulator::default();
    for (_, accumulator, _) in ranked.iter_mut().skip(limit) {
        other.merge(std::mem::take(accumulator));
    }
    let other_groups = (groups_available > limit).then(|| other.stats(measure_open));
    let groups = ranked
        .into_iter()
        .take(limit)
        .map(|(label, accumulator, example)| ValueGroup {
            label,
            stats: accumulator.stats(measure_open),
            example_case: Some(example),
        })
        .collect();

    let mut semantics = vec![
        "Figures cover cases whose value reads as a number; cases without one are counted, not treated as zero.".to_owned(),
        "Each case counts its value once, whatever its number of steps.".to_owned(),
    ];
    if matches!(grouping, ValueGrouping::LastStep) {
        semantics.push(if measure_open {
            "Open amounts are cases whose last recorded step is not a confirmed finishing step.".to_owned()
        } else {
            "Finishing steps are not confirmed, so a case's last step may be where it finished rather than where it waits.".to_owned()
        });
    }
    Ok(ValueSummary {
        field: field.to_owned(),
        grouping: grouping.clone(),
        overall: all.stats(measure_open),
        groups,
        groups_available,
        other_groups,
        semantics,
    })
}

fn group_label(
    log: &EventLog,
    trace: &CaseTrace,
    grouping: &ValueGrouping,
    group_index: Option<usize>,
) -> Option<String> {
    let activity = |id| log.activities.label(id).unwrap_or_default().to_owned();
    match grouping {
        ValueGrouping::None => None,
        ValueGrouping::Variant => Some(
            trace
                .events
                .iter()
                .map(|event| activity(event.activity.0))
                .collect::<Vec<_>>()
                .join(" → "),
        ),
        ValueGrouping::LastStep => trace.events.last().map(|event| activity(event.activity.0)),
        ValueGrouping::Attribute(_) => Some(
            group_index
                .and_then(|index| trace.attribute(index))
                .and_then(|value| log.attribute_values.label(value))
                .unwrap_or("(no value)")
                .to_owned(),
        ),
    }
}

#[derive(Default)]
struct Accumulator {
    cases: usize,
    values: Vec<f64>,
    total: f64,
    open_cases: usize,
    open_total: f64,
}

impl Accumulator {
    fn add(&mut self, value: Option<f64>, open: bool) {
        self.cases += 1;
        self.open_cases += usize::from(open);
        if let Some(value) = value {
            self.values.push(value);
            self.total += value;
            if open {
                self.open_total += value;
            }
        }
    }

    fn merge(&mut self, other: Accumulator) {
        self.cases += other.cases;
        self.values.extend(other.values);
        self.total += other.total;
        self.open_cases += other.open_cases;
        self.open_total += other.open_total;
    }

    fn stats(mut self, measure_open: bool) -> ValueStats {
        self.values.sort_by(f64::total_cmp);
        let n = self.values.len();
        let median = (n > 0).then(|| {
            if n % 2 == 1 {
                self.values[n / 2]
            } else {
                (self.values[n / 2 - 1] + self.values[n / 2]) / 2.0
            }
        });
        ValueStats {
            cases: self.cases,
            cases_with_value: n,
            total: self.total,
            mean: (n > 0).then(|| self.total / n as f64),
            median,
            p90: (n > 0).then(|| self.values[(n * 9).div_ceil(10).max(1) - 1]),
            min: self.values.first().copied(),
            max: self.values.last().copied(),
            open_cases: measure_open.then_some(self.open_cases),
            open_total: measure_open.then_some(self.open_total),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::{Dictionary, Event, NO_ATTRIBUTE};

    /// Cases of (id, steps, amount, region); an empty amount is missing.
    fn log(spec: &[(&str, &[&str], &str, &str)]) -> EventLog {
        let mut log = EventLog {
            attribute_names: vec!["amount".into(), "region".into()],
            ..EventLog::default()
        };
        let mut cases = Dictionary::new();
        let mut row = 2;
        for (id, steps, amount, region) in spec {
            let case = pm_core::CaseId(cases.intern(id));
            let events = steps
                .iter()
                .enumerate()
                .map(|(at, step)| {
                    row += 1;
                    Event::new(
                        pm_core::ActivityId(log.activities.intern(step)),
                        at as i64,
                        row,
                    )
                })
                .collect();
            let attributes = [amount, region]
                .iter()
                .map(|value| {
                    if value.is_empty() {
                        NO_ATTRIBUTE
                    } else {
                        log.attribute_values.intern(value)
                    }
                })
                .collect();
            log.traces.push(CaseTrace {
                case,
                events,
                attributes,
                incomplete: false,
            });
        }
        log.cases = cases;
        log
    }

    fn sample() -> EventLog {
        log(&[
            ("1", &["Order", "Ship", "Pay"], "100", "EU"),
            ("2", &["Order", "Ship"], "60,000", "US"),
            ("3", &["Order", "Ship"], "40", "EU"),
            ("4", &["Order"], "", "US"),
        ])
    }

    #[test]
    fn overall_figures_skip_missing_values() {
        let summary = summarize_values(
            &sample(),
            "amount",
            &ValueGrouping::None,
            &BTreeSet::new(),
            10,
        )
        .unwrap();
        let stats = summary.overall;
        assert_eq!((stats.cases, stats.cases_with_value), (4, 3));
        assert_eq!(stats.total, 60_140.0);
        assert_eq!(stats.median, Some(100.0));
        assert_eq!(stats.p90, Some(60_000.0));
        assert_eq!((stats.min, stats.max), (Some(40.0), Some(60_000.0)));
        assert_eq!(stats.open_cases, None);
        assert!(summary.groups.is_empty());
    }

    #[test]
    fn value_waiting_at_each_step_uses_confirmed_finishing_steps() {
        let finishing = BTreeSet::from(["Pay".to_owned()]);
        let summary = summarize_values(
            &sample(),
            "amount",
            &ValueGrouping::LastStep,
            &finishing,
            10,
        )
        .unwrap();
        let ship = &summary.groups[0];
        assert_eq!(ship.label, "Ship");
        assert_eq!(ship.stats.total, 60_040.0);
        assert_eq!(ship.stats.open_cases, Some(2));
        assert_eq!(ship.stats.open_total, Some(60_040.0));
        assert_eq!(summary.overall.open_total, Some(60_040.0));
        let paid = summary.groups.iter().find(|g| g.label == "Pay").unwrap();
        assert_eq!(paid.stats.open_cases, Some(0));
    }

    #[test]
    fn groups_by_route_or_attribute_with_a_remainder() {
        let by_route = summarize_values(
            &sample(),
            "amount",
            &ValueGrouping::Variant,
            &BTreeSet::new(),
            1,
        )
        .unwrap();
        assert_eq!(by_route.groups_available, 3);
        assert_eq!(by_route.groups[0].label, "Order → Ship");
        assert_eq!(by_route.groups[0].stats.mean, Some(30_020.0));
        let other = by_route.other_groups.unwrap();
        assert_eq!((other.cases, other.total), (2, 100.0));

        let by_region = summarize_values(
            &sample(),
            "amount",
            &ValueGrouping::Attribute("region".into()),
            &BTreeSet::new(),
            10,
        )
        .unwrap();
        assert_eq!(by_region.groups[0].label, "US");
        assert_eq!(by_region.groups[0].stats.cases_with_value, 1);
    }

    #[test]
    fn a_field_without_numbers_says_so_and_names_the_ones_that_have_them() {
        let error = summarize_values(
            &sample(),
            "region",
            &ValueGrouping::None,
            &BTreeSet::new(),
            10,
        )
        .unwrap_err();
        assert_eq!(
            error,
            ValueError::NotNumeric("region".into(), "amount".into())
        );
        assert!(
            error
                .to_string()
                .contains("Counts and times are still available")
        );
        assert!(matches!(
            summarize_values(
                &sample(),
                "nope",
                &ValueGrouping::None,
                &BTreeSet::new(),
                10
            ),
            Err(ValueError::UnknownField(_))
        ));
    }

    #[test]
    fn grouping_reads_and_writes_as_json() {
        let grouping: ValueGrouping =
            serde_json::from_str(r#"{"by":"attribute","attribute":"region"}"#).unwrap();
        assert_eq!(grouping, ValueGrouping::Attribute("region".into()));
        let grouping: ValueGrouping = serde_json::from_str(r#"{"by":"last_step"}"#).unwrap();
        assert_eq!(grouping, ValueGrouping::LastStep);
    }

    #[test]
    fn a_saved_filter_reads_number_ranges() {
        let filter: pm_core::Filter = serde_json::from_str(
            r#"{"attribute_between":[{"attribute":"amount","at_least":50000}]}"#,
        )
        .unwrap();
        let sample = sample();
        let kept = filter.apply(&sample);
        assert_eq!(kept.log.case_count(), 1);
        let old: pm_core::Filter = serde_json::from_str(r#"{"complete_only":true}"#).unwrap();
        assert!(old.attribute_between.is_empty());
    }
}
