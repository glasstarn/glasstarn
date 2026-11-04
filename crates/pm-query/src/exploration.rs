//! Analysis of an explicit interval, grouped populations and range cohorts.
//! The browser, CLI and agent use this same definition and denominator.
use pm_core::{EventLog, segment::Segment};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExplorationRequest {
    pub segment: Option<Segment>,
    pub group_by: Option<String>,
    pub min_micros: Option<i64>,
    pub max_micros: Option<i64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Population {
    pub items: usize,
    pub measurable: usize,
    pub missing_endpoints_or_incomplete: usize,
    pub mean_seconds: Option<i64>,
    pub median_seconds: Option<i64>,
    pub p90_seconds: Option<i64>,
    pub repeated_items: usize,
    pub examples: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplorationGroup {
    pub value: Option<String>,
    pub population: Population,
    pub selected_items: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplorationBin {
    pub from_micros: i64,
    pub to_micros: Option<i64>,
    pub items: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityContrast {
    pub activity: String,
    pub selected_percent: f64,
    pub remainder_percent: f64,
    pub difference_points: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exploration {
    pub request: ExplorationRequest,
    pub overall: Population,
    pub selected: Population,
    pub remainder: Population,
    pub bins: Vec<ExplorationBin>,
    pub groups: Vec<ExplorationGroup>,
    pub groups_total: usize,
    pub contrasts: Vec<ActivityContrast>,
    pub semantics: String,
    /// Field values that go with the slowest fifth of this measure.
    #[serde(default)]
    pub drivers: crate::drivers::Drivers,
    /// The amount carried by the items in the chosen range, when the items
    /// have an amount: (column, total).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_value: Option<(String, f64)>,
    /// Set when the field changes between steps, so an item that had several
    /// values is counted in each of their groups and the groups overlap.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub groups_overlap: bool,
}
#[derive(Default)]
struct Acc {
    items: usize,
    times: Vec<i64>,
    repeats: usize,
    examples: Vec<String>,
}
impl Acc {
    fn add(&mut self, duration: Option<i64>, repeated: bool, label: &str) {
        self.items += 1;
        self.repeats += usize::from(repeated);
        if let Some(time) = duration {
            self.times.push(time);
        }
        if self.examples.len() < 5 {
            self.examples.push(label.to_owned());
        }
    }
    fn finish(mut self) -> Population {
        self.times.sort_unstable();
        let n = self.times.len();
        Population {
            items: self.items,
            measurable: n,
            missing_endpoints_or_incomplete: self.items - n,
            mean_seconds: (n > 0).then(|| {
                (self.times.iter().map(|t| i128::from(*t)).sum::<i128>() / n as i128 / 1_000_000)
                    as i64
            }),
            median_seconds: (n > 0).then(|| self.times[n / 2] / 1_000_000),
            p90_seconds: (n > 0).then(|| self.times[(n * 9).div_ceil(10) - 1] / 1_000_000),
            repeated_items: self.repeats,
            examples: self.examples,
        }
    }
}
/// All summary counts cover the active log. Missing/reversed endpoints are
/// excluded from duration distributions, never silently replaced with zero.
pub fn explore(log: &EventLog, request: &ExplorationRequest) -> Result<Exploration, String> {
    explore_with_durations(log, request, None)
}
pub(crate) fn explore_with_durations(
    log: &EventLog,
    request: &ExplorationRequest,
    external: Option<&std::collections::HashMap<String, Option<i64>>>,
) -> Result<Exploration, String> {
    if request.min_micros.is_some_and(|n| n < 0)
        || request.max_micros.is_some_and(|n| n < 0)
        || request
            .min_micros
            .zip(request.max_micros)
            .is_some_and(|(a, b)| a > b)
    {
        return Err("interval bounds must be non-negative and lower must not exceed upper".into());
    }
    if let Some(segment) = &request.segment {
        for activity in [&segment.from, &segment.to] {
            if external.is_none() && log.activities.get(activity).is_none() {
                return Err(format!("unknown endpoint activity: {activity}"));
            }
        }
        if segment.min_micros.is_some_and(|n| n < 0)
            || segment.max_micros.is_some_and(|n| n < 0)
            || segment
                .min_micros
                .zip(segment.max_micros)
                .is_some_and(|(a, b)| a > b)
        {
            return Err(
                "interval bounds must be non-negative and lower must not exceed upper".into(),
            );
        }
    }
    let field = match &request.group_by {
        Some(name) => Some(
            log.attribute_index(name)
                .ok_or_else(|| format!("unknown field: {name}"))?,
        ),
        None => None,
    };
    let mut overall = Acc::default();
    let mut selected = Acc::default();
    let mut remainder = Acc::default();
    let mut groups = BTreeMap::<Option<String>, (Acc, usize)>::new();
    let mut activities = BTreeMap::<String, (usize, usize)>::new();
    let mut times = Vec::new();
    let mut per_item = Vec::with_capacity(log.traces.len());
    let amount = pm_core::main_number_field(&pm_core::number_fields(log)).and_then(|field| {
        log.attribute_index(&field.name)
            .map(|index| (field.name.clone(), index))
    });
    let mut selected_amount = 0.0;
    for trace in &log.traces {
        let duration = if let Some(external) = external {
            external
                .get(log.cases.label(trace.case.0).unwrap_or(""))
                .copied()
                .flatten()
        } else {
            match &request.segment {
                Some(segment) => segment.duration(log, trace),
                None => (!trace.incomplete).then(|| trace.throughput()),
            }
        };
        let chosen = match &request.segment {
            Some(segment) => duration.is_some_and(|t| segment.contains(t)),
            None => {
                duration.is_some_and(|time| {
                    request.min_micros.is_none_or(|min| time >= min)
                        && request.max_micros.is_none_or(|max| time <= max)
                }) || (request.min_micros.is_none() && request.max_micros.is_none())
            }
        };
        let unique = trace
            .events
            .iter()
            .map(|event| event.activity)
            .collect::<BTreeSet<_>>();
        let repeated = unique.len() < trace.events.len();
        let label = log.cases.label(trace.case.0).unwrap_or("");
        per_item.push(duration);
        overall.add(duration, repeated, label);
        if chosen && let Some((_, index)) = &amount {
            selected_amount += pm_core::case_number(log, trace, *index).unwrap_or(0.0);
        }
        if chosen {
            selected.add(duration, repeated, label);
        } else {
            remainder.add(duration, repeated, label);
        }
        if let Some(time) = duration {
            times.push(time);
        }
        if let Some(index) = field {
            // A column that changes between steps puts the item in the group
            // of every value it had.
            let values = log.case_values(trace, index);
            let mut values = values
                .iter()
                .filter_map(|id| log.attribute_values.label(*id))
                .map(|value| Some(value.to_owned()))
                .collect::<Vec<_>>();
            if values.is_empty() {
                values.push(None);
            }
            for value in values {
                let group = groups.entry(value).or_default();
                group.0.add(duration, repeated, label);
                group.1 += usize::from(chosen);
            }
        }
        // Compare measured intervals against other measured intervals. Missing
        // endpoints stay in the population, but cannot be a slow/fast baseline.
        for activity in unique.into_iter().filter(|_| duration.is_some()) {
            let counts = activities
                .entry(log.activities.label(activity.0).unwrap_or("").to_owned())
                .or_default();
            if chosen {
                counts.0 += 1;
            } else {
                counts.1 += 1;
            }
        }
    }
    times.sort_unstable();
    let mut bins = Vec::new();
    if !times.is_empty() {
        let p99 = times[(times.len() * 99).div_ceil(100) - 1];
        let widths = [
            60_i64, 300, 900, 3600, 14400, 43200, 86400, 172800, 604800, 2592000, 31536000,
        ];
        let width = widths
            .into_iter()
            .map(|s| s * 1_000_000)
            .find(|width| p99 / width < 11)
            .unwrap_or_else(|| (p99 / 11).saturating_add(1))
            .max(1);
        for i in 0_i64..12 {
            let to = (i < 11).then(|| (i + 1).checked_mul(width)).flatten();
            bins.push(ExplorationBin {
                from_micros: i.saturating_mul(width),
                to_micros: to,
                items: 0,
            });
            if to.is_none() {
                break;
            }
        }
        for time in times {
            let index = (time / width).min((bins.len() - 1) as i64) as usize;
            bins[index].items += 1;
        }
        while bins.last().is_some_and(|b| b.items == 0) {
            bins.pop();
        }
    }
    let sn = selected.times.len();
    let rn = remainder.times.len();
    let mut contrasts = activities
        .into_iter()
        .map(|(activity, (s, r))| {
            let sp = 100.0 * s as f64 / sn.max(1) as f64;
            let rp = 100.0 * r as f64 / rn.max(1) as f64;
            ActivityContrast {
                activity,
                selected_percent: sp,
                remainder_percent: rp,
                difference_points: sp - rp,
            }
        })
        .collect::<Vec<_>>();
    if sn == 0 || rn == 0 {
        contrasts.clear();
    }
    contrasts.sort_by(|a, b| {
        b.difference_points
            .abs()
            .total_cmp(&a.difference_points.abs())
            .then(a.activity.cmp(&b.activity))
    });
    contrasts.truncate(15);
    let groups_total = groups.len();
    let mut groups = groups
        .into_iter()
        .map(|(value, (acc, count))| ExplorationGroup {
            value,
            population: acc.finish(),
            selected_items: count,
        })
        .collect::<Vec<_>>();
    groups.sort_by(|a, b| {
        b.population
            .items
            .cmp(&a.population.items)
            .then(a.value.cmp(&b.value))
    });
    groups.truncate(100);
    let drivers = crate::drivers::drivers(log, &per_item, 3);
    let selected_value = amount.map(|(name, _)| (name, selected_amount));
    let groups_overlap = field.is_some_and(|index| log.is_step_level(index));
    Ok(Exploration{request:request.clone(),overall:overall.finish(),selected:selected.finish(),remainder:remainder.finish(),bins,groups,groups_total,contrasts,drivers,selected_value,groups_overlap,
        semantics:"Recorded elapsed time, not work effort. Each active item is counted once. Endpoints use explicit first/last occurrences; reversed, missing and incomplete histories have no duration. Equal timestamps yield zero elapsed time and do not establish event order. Group values are recorded case attributes, including a separate missing-value group. Contrasts compare only measurable items in range versus measurable items outside it; missing intervals are excluded from this comparison. Associations are not proof of cause. Groups are ranked by item count; up to 100 are listed. For connected data, this calculation describes the followed object history; events absent from that history cannot be measured here.".into()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::{
        ActivityId, CaseId, CaseTrace, Event, Filter, dict::Dictionary, segment::Occurrence,
    };
    const DAY: i64 = 86_400_000_000;
    fn fixture() -> EventLog {
        let mut log = EventLog {
            cases: Dictionary::new(),
            activities: Dictionary::new(),
            resources: Dictionary::new(),
            attribute_names: vec!["vendor".into()],
            attribute_values: Dictionary::new(),
            costs: Default::default(),
            step_values: Default::default(),
            traces: vec![],
        };
        let routes = [
            (
                vec![("Start", 0), ("Review", 2), ("End", 10), ("End", 12)],
                Some("ABC"),
                false,
            ),
            (
                vec![("Start", 0), ("Start", 3), ("End", 5)],
                Some("XYZ"),
                false,
            ),
            (vec![("Start", 0), ("Review", 4)], Some("ABC"), false),
            (vec![("End", 0), ("Start", 1)], Some("XYZ"), false),
            (vec![("Start", 0), ("End", 8)], None, true),
            (vec![("Start", 0), ("End", 0)], None, false),
        ];
        for (index, (events, vendor, incomplete)) in routes.into_iter().enumerate() {
            let case = CaseId(log.cases.intern(&format!("item-{index}")));
            let events = events
                .into_iter()
                .enumerate()
                .map(|(row, (name, day))| {
                    Event::new(
                        ActivityId(log.activities.intern(name)),
                        day * DAY,
                        row as u32 + 1,
                    )
                })
                .collect();
            log.traces.push(CaseTrace {
                case,
                events,
                incomplete,
                attributes: vec![vendor.map_or(pm_core::log::NO_ATTRIBUTE, |name| {
                    log.attribute_values.intern(name)
                })],
            });
        }
        log
    }
    fn interval() -> Segment {
        Segment {
            from: "Start".into(),
            to: "End".into(),
            to_occurrence: Occurrence::Last,
            min_micros: Some(10 * DAY),
            max_micros: Some(18 * DAY),
            ..Default::default()
        }
    }
    #[test]
    fn first_last_range_groups_and_denominators_match_constructed_facts() {
        let log = fixture();
        let report = explore(
            &log,
            &ExplorationRequest {
                segment: Some(interval()),
                group_by: Some("vendor".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            (
                report.overall.items,
                report.overall.measurable,
                report.overall.missing_endpoints_or_incomplete
            ),
            (6, 3, 3)
        );
        assert_eq!(report.overall.mean_seconds, Some(489600));
        assert_eq!(report.overall.median_seconds, Some(5 * 86400));
        assert_eq!(report.overall.p90_seconds, Some(12 * 86400));
        assert_eq!((report.selected.items, report.remainder.items), (1, 5));
        assert_eq!(report.selected.examples, ["item-0"]);
        assert_eq!(report.groups_total, 3);
        assert!(
            report
                .groups
                .iter()
                .all(|group| group.population.items == 2)
        );
        assert_eq!(report.bins.iter().map(|bin| bin.items).sum::<usize>(), 3);
        let review = report
            .contrasts
            .iter()
            .find(|item| item.activity == "Review")
            .unwrap();
        assert_eq!(
            (review.selected_percent, review.remainder_percent),
            (100.0, 0.0)
        );
    }
    #[test]
    fn occurrence_rules_and_whole_process_are_different_measurements() {
        let log = fixture();
        let mut segment = interval();
        segment.from_occurrence = Occurrence::Last;
        segment.to_occurrence = Occurrence::First;
        assert_eq!(segment.duration(&log, &log.traces[0]), Some(10 * DAY));
        assert_eq!(segment.duration(&log, &log.traces[1]), Some(2 * DAY));
        let whole = explore(&log, &Default::default()).unwrap();
        assert_eq!(whole.overall.measurable, 5);
        assert_eq!(whole.overall.median_seconds, Some(4 * 86400));
        assert_eq!(whole.selected.items, 6);
    }
    #[test]
    fn applying_the_range_keeps_complete_histories_and_or_values_are_scoped() {
        let log = fixture();
        let filter = Filter {
            segment: Some(interval()),
            ..Default::default()
        };
        let filtered = filter.apply(&log);
        assert_eq!(filtered.log.case_count(), 1);
        assert_eq!(filtered.log.traces[0].events.len(), 4);
        let filter = Filter {
            attribute_in: BTreeMap::from([(
                "vendor".into(),
                BTreeSet::from(["ABC".into(), "XYZ".into()]),
            )]),
            ..Default::default()
        };
        assert_eq!(filter.apply(&log).log.case_count(), 4);
        let missing = Filter {
            attribute_missing: BTreeSet::from(["vendor".into()]),
            ..Default::default()
        };
        assert_eq!(missing.apply(&log).log.case_count(), 2);
        let empty = Filter {
            attribute_in: BTreeMap::from([("vendor".into(), BTreeSet::new())]),
            ..Default::default()
        };
        assert_eq!(empty.apply(&log).log.case_count(), 0);
    }
    #[test]
    fn unknown_endpoints_and_invalid_ranges_fail_without_fabricated_zeroes() {
        let log = fixture();
        let mut segment = interval();
        segment.from = "Unknown".into();
        assert!(
            explore(
                &log,
                &ExplorationRequest {
                    segment: Some(segment),
                    ..Default::default()
                }
            )
            .is_err()
        );
        let mut segment = interval();
        segment.max_micros = Some(DAY);
        assert!(
            explore(
                &log,
                &ExplorationRequest {
                    segment: Some(segment),
                    ..Default::default()
                }
            )
            .is_err()
        );
        let empty = Filter::new().with_activity("Unknown").apply(&log);
        let report = explore(&empty.log, &Default::default()).unwrap();
        assert_eq!(report.overall.mean_seconds, None);
        assert!(report.bins.is_empty());
    }
}
