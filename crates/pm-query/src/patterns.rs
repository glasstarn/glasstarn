//! Patterns a first look should always cover, beyond whole-item speed:
//!
//! - **Slow-step drivers**: for the hand-offs where time goes, which field
//!   value makes that one hand-off slow. A region can be slow to start work
//!   without being slow overall.
//! - **Skipped usual steps**: a step that almost always comes before another
//!   but is missing in some items, such as payment without its approval, and
//!   whether one field value or person accounts for nearly all of them.
//!
//! Plain counts and medians, no model; patterns, not proven causes.

use crate::drivers::Driver;
use pm_core::{EventLog, Micros};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A step must come before another in at least this share of the items that
/// reach it to count as usual.
const USUAL: f64 = 0.9;
/// Fewer items than this missing the usual step is noise.
const MIN_MISSING: usize = 5;
/// A field value accounts for the missing step when it covers this share of
/// those items.
const CONCENTRATED: f64 = 0.8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepDriver {
    pub from: String,
    pub to: String,
    /// Items in which the hand-off was measured.
    pub measured: usize,
    pub driver: Driver,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkippedStep {
    /// The step that is usually there first.
    pub usual: String,
    /// The step reached without it.
    pub reached: String,
    /// Items that reach `reached`.
    pub reaching: usize,
    /// Of those, items without `usual` before it.
    pub missing: usize,
    pub example_items: Vec<String>,
    /// A field value (or "Handled by" a person) shared by most of the
    /// missing items, when one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concentrated_in: Option<Concentration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Concentration {
    pub field: String,
    pub value: String,
    /// Missing items with this value.
    pub items: usize,
    /// Share of all items with this value, 0–100, to show it is unusual.
    pub share_of_all_percent: f64,
}

/// For each hand-off in `edges`, the strongest field value that goes with it
/// being slow. The first occurrence of the hand-off in each item is measured.
pub fn step_drivers(log: &EventLog, edges: &[(String, String)]) -> Vec<StepDriver> {
    let mut found = Vec::new();
    for (from, to) in edges {
        let (Some(a), Some(b)) = (log.activities.get(from), log.activities.get(to)) else {
            continue;
        };
        let times: Vec<Option<Micros>> = log
            .traces
            .iter()
            .map(|trace| {
                if trace.incomplete {
                    return None;
                }
                trace
                    .events
                    .windows(2)
                    .find(|pair| pair[0].activity.0 == a && pair[1].activity.0 == b)
                    .map(|pair| (pair[1].at - pair[0].at).max(0))
            })
            .collect();
        let measured = times.iter().filter(|time| time.is_some()).count();
        let drivers = crate::drivers::drivers(log, &times, 1);
        if let Some(driver) = drivers.drivers.into_iter().next() {
            found.push(StepDriver {
                from: from.clone(),
                to: to.clone(),
                measured,
                driver,
            });
        }
    }
    found
}

/// Usual steps that some items skip, most skipped first, at most `limit`.
pub fn skipped_steps(log: &EventLog, limit: usize) -> Vec<SkippedStep> {
    // For each item: the position of each step's first occurrence.
    let firsts: Vec<BTreeMap<u32, usize>> = log
        .traces
        .iter()
        .map(|trace| {
            let mut first = BTreeMap::new();
            for (position, event) in trace.events.iter().enumerate() {
                first.entry(event.activity.0).or_insert(position);
            }
            first
        })
        .collect();
    let mut reaching: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (item, first) in firsts.iter().enumerate() {
        for &step in first.keys() {
            reaching.entry(step).or_default().push(item);
        }
    }
    // A closing step is reached by every path, so any alternative route
    // (a rejection) would read as a skipped step. Only steps that most items
    // carry on from are checked.
    let mut last: BTreeMap<u32, usize> = BTreeMap::new();
    for trace in &log.traces {
        if let Some(event) = trace.events.last() {
            *last.entry(event.activity.0).or_default() += 1;
        }
    }
    let mut found: Vec<(SkippedStep, Vec<usize>)> = Vec::new();
    for (&reached, items) in &reaching {
        if items.len() < MIN_MISSING * 4
            || last.get(&reached).copied().unwrap_or(0) * 2 >= items.len()
        {
            continue;
        }
        // Steps before `reached` in each item that reaches it.
        let mut before: BTreeMap<u32, usize> = BTreeMap::new();
        for &item in items {
            let at = firsts[item][&reached];
            for (&step, &position) in &firsts[item] {
                if step != reached && position < at {
                    *before.entry(step).or_default() += 1;
                }
            }
        }
        // The usual step closest to `reached`: the one that most items have
        // last before it, among those that are usual.
        let usual: Vec<u32> = before
            .iter()
            .filter(|&(_, &count)| {
                count as f64 >= items.len() as f64 * USUAL && count < items.len()
            })
            .map(|(&step, _)| step)
            .collect();
        for step in usual {
            let missing: Vec<usize> = items
                .iter()
                .copied()
                .filter(|&item| {
                    firsts[item]
                        .get(&step)
                        .is_none_or(|&position| position > firsts[item][&reached])
                })
                .collect();
            if missing.len() < MIN_MISSING {
                continue;
            }
            found.push((
                SkippedStep {
                    usual: log.activity_label(pm_core::ActivityId(step)).to_owned(),
                    reached: log.activity_label(pm_core::ActivityId(reached)).to_owned(),
                    reaching: items.len(),
                    missing: missing.len(),
                    example_items: missing
                        .iter()
                        .take(5)
                        .map(|&item| log.case_label(log.traces[item].case).to_owned())
                        .collect(),
                    concentrated_in: concentration(log, &missing),
                },
                missing,
            ));
        }
    }
    // One gap often shows as several pairs (payment and everything after it
    // without approval). Keep the first pair for each set of items: the
    // earliest step reached without its usual predecessor.
    found.sort_by(|(a, _), (b, _)| {
        b.missing
            .cmp(&a.missing)
            .then(b.reaching.cmp(&a.reaching))
            .then(a.usual.cmp(&b.usual))
            .then(a.reached.cmp(&b.reached))
    });
    let mut seen: Vec<(String, BTreeSet<usize>)> = Vec::new();
    let mut kept = Vec::new();
    for (skipped, items) in found {
        let set: BTreeSet<usize> = items.into_iter().collect();
        // One finding per missing step: the first place items go on without it.
        if seen
            .iter()
            .any(|(usual, other)| *usual == skipped.usual || overlap(other, &set) >= 0.9)
        {
            continue;
        }
        seen.push((skipped.usual.clone(), set));
        kept.push(skipped);
        if kept.len() == limit {
            break;
        }
    }
    kept
}

fn overlap(a: &BTreeSet<usize>, b: &BTreeSet<usize>) -> f64 {
    let shared = a.intersection(b).count();
    shared as f64 / a.len().min(b.len()).max(1) as f64
}

/// A field value or person that most of `items` share and few others do.
fn concentration(log: &EventLog, items: &[usize]) -> Option<Concentration> {
    let total = log.traces.len();
    let mut best: Option<(Concentration, bool)> = None;
    let step_level = log.step_level_attributes();
    let mut consider = |field: &str, value: String, inside: usize, overall: usize, steps: bool| {
        if (inside as f64) < items.len() as f64 * CONCENTRATED || overall * 2 > total {
            return;
        }
        let candidate = Concentration {
            field: field.to_owned(),
            value,
            items: inside,
            share_of_all_percent: overall as f64 * 100.0 / total.max(1) as f64,
        };
        // The value covering the most says the most; on a tie, a field that
        // belongs to the whole item (an adjuster) over one that changes
        // between steps, then the narrowest.
        let better = best.as_ref().is_none_or(|(b, b_steps)| {
            (candidate.items, !steps, -candidate.share_of_all_percent).partial_cmp(&(
                b.items,
                !*b_steps,
                -b.share_of_all_percent,
            )) == Some(std::cmp::Ordering::Greater)
        });
        if better {
            best = Some((candidate, steps));
        }
    };
    let chosen: BTreeSet<usize> = items.iter().copied().collect();
    for (index, name) in log.attribute_names.iter().enumerate() {
        let mut inside: BTreeMap<u32, usize> = BTreeMap::new();
        let mut overall: BTreeMap<u32, usize> = BTreeMap::new();
        for (item, trace) in log.traces.iter().enumerate() {
            for value in log.case_values(trace, index) {
                *overall.entry(value).or_default() += 1;
                if chosen.contains(&item) {
                    *inside.entry(value).or_default() += 1;
                }
            }
        }
        for (value, count) in inside {
            let label = log.attribute_values.label(value).unwrap_or("").to_owned();
            if !label.is_empty() {
                consider(
                    name,
                    label,
                    count,
                    overall[&value],
                    step_level.contains(&index),
                );
            }
        }
    }
    if log.has_resources() {
        let mut inside: BTreeMap<u32, usize> = BTreeMap::new();
        let mut overall: BTreeMap<u32, usize> = BTreeMap::new();
        for (item, trace) in log.traces.iter().enumerate() {
            let people: BTreeSet<u32> = trace
                .events
                .iter()
                .filter_map(|event| event.resource().map(|resource| resource.0))
                .collect();
            for person in people {
                *overall.entry(person).or_default() += 1;
                if chosen.contains(&item) {
                    *inside.entry(person).or_default() += 1;
                }
            }
        }
        for (person, count) in inside {
            let label = log.resources.label(person).unwrap_or("").to_owned();
            if !label.is_empty() {
                consider("Handled by", label, count, overall[&person], true);
            }
        }
    }
    best.map(|(found, _)| found)
}
