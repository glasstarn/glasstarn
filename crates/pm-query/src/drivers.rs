//! Likely drivers: which item fields go with the slow items.
//!
//! Every field other than the item, step and time is a candidate: customer,
//! vendor, region, an amount in bands, and who worked on the item. For each
//! value the engine counts how many items carry it, how many of them are among
//! the slowest fifth, and compares their median time with everyone else's.
//! The values with the most more-slow-items-than-expected come first.
//!
//! Plain counts and medians, no model. A driver is a pattern in the data, not
//! a proven cause: other fields may travel with it.

use pm_core::{EventLog, Micros, case_number, number_fields};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One candidate: field, value label, number band if any, member items.
type Group = (String, String, Option<(f64, f64)>, Vec<usize>);

/// The slowest share of measured items that counts as "slow".
const SLOW_SHARE: f64 = 0.2;
/// Fewer measured items than this and no driver is reported.
const MIN_ITEMS: usize = 30;
/// A text field with more distinct values than this is an identifier, not a
/// grouping (order numbers, free text), and is skipped.
const MAX_GROUPS: usize = 60;
/// A driver must be at least this much more common among slow items.
const MIN_LIFT: f64 = 1.4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Driver {
    /// The column, or "Handled by" for who worked on the item.
    pub field: String,
    /// The value, or a band such as "12,400 – 25,000" for a number.
    pub value: String,
    /// For a number band, its bounds, so a filter can select it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_most: Option<f64>,
    /// Measured items with this value.
    pub items: usize,
    /// Of those, how many are among the slowest fifth.
    pub slow_items: usize,
    /// Share of all measured items that have this value, 0–100.
    pub share_of_items: f64,
    /// Share of the slow items that have this value, 0–100.
    pub share_of_slow: f64,
    /// How many times more likely an item with this value is to be slow.
    pub lift: f64,
    pub median_seconds: i64,
    pub others_median_seconds: i64,
    /// Slow items beyond what this many items would have by chance.
    pub excess_slow_items: f64,
    pub example_items: Vec<String>,
    /// The items themselves (positions in the log), to spot two drivers
    /// that pick out the same items. Not sent anywhere.
    #[serde(skip)]
    members: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Drivers {
    /// Items with a measured time.
    pub measured: usize,
    /// The time at which an item counts as slow (the slowest fifth start here).
    pub slow_from_seconds: i64,
    pub drivers: Vec<Driver>,
    pub note: String,
}

/// Drivers of slowness for whole items: time from first to last step, for
/// complete items with at least two steps.
pub fn item_drivers(log: &EventLog, limit: usize) -> Drivers {
    let times: Vec<Option<Micros>> = log
        .traces
        .iter()
        .map(|trace| (!trace.incomplete && trace.len() >= 2).then(|| trace.throughput()))
        .collect();
    drivers(log, &times, limit)
}

/// Drivers of slowness for any measure: `times` holds one entry per item, in
/// the log's order, `None` where the item was not measured.
pub fn drivers(log: &EventLog, times: &[Option<Micros>], limit: usize) -> Drivers {
    let measured: Vec<usize> = (0..log.traces.len().min(times.len()))
        .filter(|&index| times[index].is_some())
        .collect();
    let note = "Patterns in the recorded data, not proven causes: a field can travel with another that matters.".to_owned();
    if measured.len() < MIN_ITEMS {
        return Drivers {
            measured: measured.len(),
            slow_from_seconds: 0,
            drivers: Vec::new(),
            note,
        };
    }
    let mut sorted: Vec<Micros> = measured
        .iter()
        .map(|&index| times[index].unwrap_or(0))
        .collect();
    sorted.sort_unstable();
    let cut = sorted[((sorted.len() as f64) * (1.0 - SLOW_SHARE)).floor() as usize].max(1);
    let slow: BTreeSet<usize> = measured
        .iter()
        .copied()
        .filter(|&index| times[index].unwrap_or(0) >= cut)
        .collect();
    // Ties at the cut can make "slow" much more than a fifth; then the cut
    // separates nothing useful.
    if slow.len() as f64 > measured.len() as f64 * 0.5 || slow.is_empty() {
        return Drivers {
            measured: measured.len(),
            slow_from_seconds: cut / 1_000_000,
            drivers: Vec::new(),
            note,
        };
    }
    let base = slow.len() as f64 / measured.len() as f64;
    let min_group = (measured.len() as f64 * 0.02).ceil().max(15.0) as usize;

    // Groups: field -> value label -> member items (and number bands).
    let mut groups: Vec<Group> = Vec::new();
    let numeric: BTreeSet<String> = number_fields(log)
        .into_iter()
        .filter(|field| field.numbers > field.not_numbers)
        .map(|field| field.name)
        .collect();
    for (index, name) in log.attribute_names.iter().enumerate() {
        if numeric.contains(name) {
            let mut values: Vec<(f64, usize)> = measured
                .iter()
                .filter_map(|&item| {
                    case_number(log, &log.traces[item], index).map(|value| (value, item))
                })
                .collect();
            let distinct = values
                .iter()
                .map(|(value, _)| value.to_bits())
                .collect::<BTreeSet<_>>()
                .len();
            if distinct > 8 {
                // Four bands of equal size.
                values.sort_by(|a, b| a.0.total_cmp(&b.0));
                let n = values.len();
                for band in 0..4 {
                    let slice = &values[band * n / 4..(band + 1) * n / 4];
                    if slice.is_empty() {
                        continue;
                    }
                    let (low, high) = (slice[0].0, slice[slice.len() - 1].0);
                    groups.push((
                        name.clone(),
                        format!("{} – {}", round_label(low), round_label(high)),
                        Some((low, high)),
                        slice.iter().map(|(_, item)| *item).collect(),
                    ));
                }
                continue;
            }
        }
        let mut by_value: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        // A column that changes between steps puts the item in every value
        // it had, as "Handled by" does with people.
        for &item in &measured {
            for value in log.case_values(&log.traces[item], index) {
                by_value.entry(value).or_default().push(item);
            }
        }
        if by_value.len() < 2 || by_value.len() > MAX_GROUPS {
            continue;
        }
        for (value, members) in by_value {
            if members.len() == measured.len() {
                continue;
            }
            let label = log.attribute_values.label(value).unwrap_or("").to_owned();
            groups.push((name.clone(), label, None, members));
        }
    }
    // Who worked on it: an item belongs to each person who recorded a step.
    if log.has_resources() {
        let mut by_resource: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for &item in &measured {
            let people: BTreeSet<u32> = log.traces[item]
                .events
                .iter()
                .filter_map(|event| event.resource().map(|resource| resource.0))
                .collect();
            for person in people {
                by_resource.entry(person).or_default().push(item);
            }
        }
        if by_resource.len() >= 2 && by_resource.len() <= MAX_GROUPS * 4 {
            for (person, members) in by_resource {
                let label = log.resources.label(person).unwrap_or("").to_owned();
                if label.is_empty() || members.len() == measured.len() {
                    continue;
                }
                groups.push(("Handled by".to_owned(), label, None, members));
            }
        }
    }

    let mut found: Vec<Driver> = Vec::new();
    for (field, value, band, members) in groups {
        if members.len() < min_group || members.len() == measured.len() {
            continue;
        }
        let slow_items = members.iter().filter(|item| slow.contains(item)).count();
        let share = slow_items as f64 / members.len() as f64;
        let lift = share / base;
        let excess = slow_items as f64 - members.len() as f64 * base;
        if lift < MIN_LIFT || excess < 3.0 {
            continue;
        }
        let member_set: BTreeSet<usize> = members.iter().copied().collect();
        let mut inside: Vec<Micros> = members
            .iter()
            .map(|&item| times[item].unwrap_or(0))
            .collect();
        let mut outside: Vec<Micros> = measured
            .iter()
            .filter(|item| !member_set.contains(item))
            .map(|&item| times[item].unwrap_or(0))
            .collect();
        let (median, others) = (median(&mut inside), median(&mut outside));
        if median <= others {
            continue;
        }
        found.push(Driver {
            field,
            value,
            at_least: band.map(|(low, _)| low),
            at_most: band.map(|(_, high)| high),
            items: members.len(),
            slow_items,
            share_of_items: members.len() as f64 * 100.0 / measured.len() as f64,
            share_of_slow: slow_items as f64 * 100.0 / slow.len() as f64,
            lift,
            median_seconds: median / 1_000_000,
            others_median_seconds: others / 1_000_000,
            excess_slow_items: excess,
            example_items: members
                .iter()
                .filter(|item| slow.contains(item))
                .take(5)
                .map(|&item| log.case_label(log.traces[item].case).to_owned())
                .collect(),
            members,
        });
    }
    found.sort_by(|a, b| {
        b.excess_slow_items
            .total_cmp(&a.excess_slow_items)
            .then(a.field.cmp(&b.field))
            .then(a.value.cmp(&b.value))
    });
    // A person and a field value that pick out (nearly) the same items say
    // one thing twice: the Reno warehouse as a person and as a column, or
    // the one approver of all large orders and the large-amount band. Keep
    // the clearly stronger one; on a near tie keep the field, which can be
    // filtered and named in the data. A column that changes between steps
    // (a status the items passed through) only restates what happened to
    // them, so the person, who says who, is kept over it.
    let step_level: BTreeSet<String> = log
        .step_level_attributes()
        .into_iter()
        .filter_map(|index| log.attribute_names.get(index).cloned())
        .collect();
    let overlap = |a: &[usize], b: &[usize]| {
        let set: BTreeSet<usize> = b.iter().copied().collect();
        let shared = a.iter().filter(|item| set.contains(item)).count();
        shared as f64 >= 0.9 * a.len().min(b.len()) as f64
    };
    let mut dropped = BTreeSet::new();
    for (i, person) in found
        .iter()
        .enumerate()
        .filter(|(_, d)| d.field == "Handled by")
    {
        for (j, field) in found
            .iter()
            .enumerate()
            .filter(|(_, d)| d.field != "Handled by")
        {
            if dropped.contains(&i)
                || dropped.contains(&j)
                || !overlap(&person.members, &field.members)
            {
                continue;
            }
            let keep_person = step_level.contains(&field.field) || person.lift > field.lift * 1.1;
            dropped.insert(if keep_person { j } else { i });
        }
    }
    let found: Vec<Driver> = found
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !dropped.contains(index))
        .map(|(_, driver)| driver)
        .collect();
    // At most two values per field, so one busy field does not crowd out the rest.
    let mut per_field: BTreeMap<String, usize> = BTreeMap::new();
    let drivers = found
        .into_iter()
        .filter(|driver| {
            let count = per_field.entry(driver.field.clone()).or_default();
            *count += 1;
            *count <= 2
        })
        .take(limit)
        .collect();
    Drivers {
        measured: measured.len(),
        slow_from_seconds: cut / 1_000_000,
        drivers,
        note,
    }
}

fn median(values: &mut [Micros]) -> Micros {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        values[n / 2 - 1] + (values[n / 2] - values[n / 2 - 1]) / 2
    }
}

/// A band edge people can read: 12,431.77 becomes 12,400.
fn round_label(value: f64) -> String {
    let magnitude = value.abs();
    let step = if magnitude >= 100_000.0 {
        1_000.0
    } else if magnitude >= 10_000.0 {
        100.0
    } else if magnitude >= 1_000.0 {
        10.0
    } else {
        1.0
    };
    let rounded = (value / step).round() * step;
    let whole = rounded.round() as i64;
    let digits = whole.abs().to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if whole < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::{ActivityId, CaseId, CaseTrace, Dictionary, Event};

    /// `items` cases of Create → Close; two in three of one vendor's items take
    /// 20 days, everything else 1–6. Region alternates and means nothing.
    fn build(items: usize, slow_vendor: &str) -> EventLog {
        let mut activities = Dictionary::new();
        activities.intern("Create");
        activities.intern("Close");
        let mut cases = Dictionary::new();
        let mut values = Dictionary::new();
        let mut traces = Vec::new();
        for n in 0..items {
            let vendor = ["Acme", "Bolt", "Cove", slow_vendor][n % 4];
            let slow = vendor == slow_vendor && n % 12 != 3;
            // Spread-out hours, so the slowest fifth has a clean edge.
            let hours = if slow {
                480 + (n % 10) as i64
            } else {
                24 + ((n * 37) % 120) as i64
            };
            let amount = format!("{}", 100 + n * 7);
            let region = ["North", "South"][(n / 4) % 2];
            let case = cases.intern(&format!("c{n}"));
            traces.push(CaseTrace {
                case: CaseId(case),
                events: vec![
                    Event::new(ActivityId(0), 0, (n * 2) as u32),
                    Event::new(ActivityId(1), hours * 3_600_000_000, (n * 2 + 1) as u32),
                ],
                incomplete: false,
                attributes: vec![
                    values.intern(vendor),
                    values.intern(&amount),
                    values.intern(region),
                ],
            });
        }
        EventLog {
            activities,
            cases,
            attribute_names: vec!["vendor".into(), "amount".into(), "region".into()],
            attribute_values: values,
            traces,
            ..Default::default()
        }
    }

    #[test]
    fn the_slow_vendor_comes_first() {
        let log = build(400, "Delta");
        let result = item_drivers(&log, 5);
        assert_eq!(result.measured, 400);
        let first = &result.drivers[0];
        assert_eq!(
            (first.field.as_str(), first.value.as_str()),
            ("vendor", "Delta")
        );
        assert!(first.lift > 3.0, "{}", first.lift);
        assert!(first.median_seconds > first.others_median_seconds);
        assert!(
            result.drivers.iter().all(|driver| driver.field != "region"),
            "region is even, so it is no driver"
        );
    }

    #[test]
    fn too_few_items_report_nothing() {
        let log = build(20, "Delta");
        assert!(item_drivers(&log, 5).drivers.is_empty());
    }

    #[test]
    fn band_edges_read_as_round_numbers() {
        assert_eq!(round_label(12_431.77), "12,400");
        assert_eq!(round_label(123_456.0), "123,000");
        assert_eq!(round_label(42.4), "42");
    }

    #[test]
    fn a_driver_finding_carries_the_money_of_its_items() {
        use crate::findings::{FindingKind, FindingTarget, KeyFinding, Severity, attach_values};
        let log = build(400, "Delta");
        let mut findings = vec![KeyFinding {
            value: None,
            filter: None,
            target: Some(FindingTarget::Field {
                field: "vendor".into(),
                value: "Delta".into(),
                at_least: None,
                at_most: None,
            }),
            kind: FindingKind::Driver,
            severity: Severity::Medium,
            title: String::new(),
            detail: String::new(),
            tags: Vec::new(),
            view: "map".into(),
            reach_percent: 0.0,
            impact: String::new(),
        }];
        attach_values(&mut findings, &log, &pm_core::number_fields(&log));
        let value = findings[0].value.as_ref().expect("amounts are recorded");
        assert_eq!(value.field, "amount");
        assert_eq!(value.items, 100);
        // Delta is every fourth item from n = 3: amounts 100 + 7n.
        let expected: f64 = (0..400)
            .filter(|n| n % 4 == 3)
            .map(|n| (100 + n * 7) as f64)
            .sum();
        assert_eq!(value.total, expected);
    }

    #[test]
    fn the_map_knows_the_money_through_each_step_and_connection() {
        let log = build(40, "Delta");
        let values = crate::map_view::map_values(&log).expect("amounts are recorded");
        let all: f64 = (0..40).map(|n| (100 + n * 7) as f64).sum();
        assert_eq!(values.field, "amount");
        assert_eq!(values.steps["Create"], all);
        assert_eq!(values.connections["Create\u{1}Close"], all);
    }

    #[test]
    fn a_person_and_a_field_that_pick_the_same_items_are_said_once() {
        let mut log = build(400, "Delta");
        let pat = log.resources.intern("Pat");
        let sam = log.resources.intern("Sam");
        for (n, trace) in log.traces.iter_mut().enumerate() {
            trace.events[0].resource = if n % 4 == 3 { pat } else { sam };
        }
        let drivers = item_drivers(&log, 6).drivers;
        let delta = drivers
            .iter()
            .filter(|d| d.value == "Delta" || d.value == "Pat")
            .count();
        assert_eq!(
            delta,
            1,
            "{:?}",
            drivers
                .iter()
                .map(|d| (&d.field, &d.value))
                .collect::<Vec<_>>()
        );
        assert!(
            drivers
                .iter()
                .any(|d| d.field == "vendor" && d.value == "Delta"),
            "on a tie the field stays"
        );
    }
}
