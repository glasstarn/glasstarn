//! Case drill-down — following a number back to the cases that produced it.
//!
//! Every figure this product reports is an aggregate over cases. A user who
//! cannot open those cases has to take the aggregate on trust, which is
//! precisely what the product exists not to ask of them.
//!
//! Two directions are supported:
//!
//! - **From a case**: [`inspect_case`] returns one case in full — its ordered
//!   events, timings, resources, costs, attributes, and the source rows each
//!   event came from.
//! - **From a finding**: [`find_cases`] returns the cases behind a transition,
//!   an activity, or a variant, so a bottleneck leads to the evidence for it.
//!
//! Source row numbers are carried throughout, so evidence points at a line the
//! user can open in their own spreadsheet.

use pm_core::dict::ActivityId;
use pm_core::log::{CaseTrace, EventLog, Micros};
use serde::Serialize;

const MICROS_PER_SECOND: Micros = 1_000_000;

/// One event within a case, with everything recorded about it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CaseEvent {
    pub activity: String,
    /// When it happened, as an unambiguous ISO timestamp.
    pub timestamp: String,
    /// When it finished, if an end timestamp was mapped.
    pub end_timestamp: Option<String>,
    /// How long the activity itself took, when an end timestamp exists.
    pub processing_seconds: Option<i64>,
    /// Time since the previous event in this case.
    ///
    /// Measured from the previous activity's **end** when an end timestamp was
    /// mapped — real waiting. Otherwise from its start, which includes however
    /// long the previous activity itself took. [`CaseDetail::gap_is_waiting`]
    /// says which, so the two are never confused.
    pub since_previous_seconds: Option<i64>,
    pub resource: Option<String>,
    /// Fixed-point cost in millionths of the source unit.
    pub cost_microunits: Option<i64>,
    /// Row in the source file, matching what a spreadsheet shows.
    pub source_row: u32,
    /// Columns that change between steps, where this step's value differs
    /// from the previous step's: `status: Approved`. On the first step, every
    /// such column's starting value.
    pub changes: Vec<(String, String)>,
}

/// One case, in full.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CaseDetail {
    pub case: String,
    pub events: Vec<CaseEvent>,
    /// The activity sequence — this case's variant.
    pub variant: Vec<String>,
    pub throughput_seconds: i64,
    /// Whether this recorded span was included in aggregate duration results.
    pub throughput_eligible: bool,
    /// Set when the log had missing data for this case, so its recorded span
    /// understates reality and it was excluded from duration averages.
    pub incomplete: bool,
    /// Case attributes, in mapping order. Columns that changed between this
    /// item's steps are on the steps instead, in [`CaseEvent::changes`].
    pub attributes: Vec<(String, String)>,
    /// Total recorded cost across the case, when a cost column was mapped.
    ///
    /// Accumulated in `i128` to match the whole-log total, so a long or
    /// expensive case cannot overflow where the aggregate would not.
    pub total_cost_microunits: Option<i128>,
    /// Source rows this case occupies, so the user can find it in the file.
    pub source_rows: Vec<u32>,
    /// Whether the gaps between events exclude the previous activity's own
    /// duration. True only when end timestamps were mapped throughout.
    pub gap_is_waiting: bool,
    /// How the timestamps below should be read.
    ///
    /// Shown because `2026-03-02 09:00:00` means different instants under
    /// different readings, and a displayed value without that context invites
    /// exactly the misreading the timezone decision exists to prevent.
    pub timestamp_reading: String,
}

/// What to look for when finding the cases behind a figure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// Cases in which one activity was directly followed by another.
    Transition { from: String, to: String },
    /// Cases containing an activity.
    Activity(String),
    /// Cases following an exact path.
    Variant(Vec<String>),
    /// Cases where work passed between two resources.
    Handover { from: String, to: String },
}

/// Returns one case in full, or `None` if no such case exists.
pub fn inspect_case(log: &EventLog, case_label: &str) -> Option<CaseDetail> {
    inspect_case_read_as(log, case_label, "as recorded")
}

/// Returns one case in full, stating how its timestamps were read.
///
/// `reading` describes the timezone policy that was applied — the same value
/// changes meaning under `utc` versus `as_written`, so displaying an instant
/// without saying which was used is not actually unambiguous.
pub fn inspect_case_read_as(log: &EventLog, case_label: &str, reading: &str) -> Option<CaseDetail> {
    let trace = log
        .traces
        .iter()
        .find(|t| log.case_label(t.case) == case_label)?;
    Some(detail(log, trace, reading))
}

/// Returns up to `limit` case labels supporting a finding.
///
/// Bounded deliberately: a transition on a million-case log has a million
/// witnesses, and returning them all would be useless to a person and
/// expensive for an agent.
pub fn find_cases(log: &EventLog, evidence: &Evidence, limit: usize) -> Vec<String> {
    let activity = |name: &str| log.activities.get(name).map(ActivityId);
    let resource = |name: &str| log.resources.get(name);

    log.traces
        .iter()
        .filter(|trace| matches(trace, evidence, &activity, &resource))
        .take(limit)
        .map(|t| log.case_label(t.case).to_owned())
        .collect()
}

/// How many cases support a finding, without materializing them.
pub fn count_cases(log: &EventLog, evidence: &Evidence) -> usize {
    let activity = |name: &str| log.activities.get(name).map(ActivityId);
    let resource = |name: &str| log.resources.get(name);
    log.traces
        .iter()
        .filter(|trace| matches(trace, evidence, &activity, &resource))
        .count()
}

fn matches(
    trace: &CaseTrace,
    evidence: &Evidence,
    activity: &impl Fn(&str) -> Option<ActivityId>,
    resource: &impl Fn(&str) -> Option<u32>,
) -> bool {
    match evidence {
        Evidence::Activity(name) => match activity(name) {
            // An activity that does not occur in the log has no witnesses.
            None => false,
            Some(id) => trace.events.iter().any(|e| e.activity == id),
        },
        Evidence::Transition { from, to } => match (activity(from), activity(to)) {
            (Some(a), Some(b)) => trace
                .events
                .windows(2)
                .any(|w| w[0].activity == a && w[1].activity == b),
            _ => false,
        },
        Evidence::Variant(sequence) => {
            let ids: Option<Vec<ActivityId>> = sequence.iter().map(|n| activity(n)).collect();
            match ids {
                Some(ids) => trace.events.iter().map(|e| e.activity).eq(ids),
                None => false,
            }
        }
        Evidence::Handover { from, to } => match (resource(from), resource(to)) {
            (Some(a), Some(b)) => trace
                .events
                .windows(2)
                .any(|w| w[0].resource == a && w[1].resource == b && a != b),
            _ => false,
        },
    }
}

fn detail(log: &EventLog, trace: &CaseTrace, reading: &str) -> CaseDetail {
    let mut events = Vec::with_capacity(trace.len());
    let mut previous: Option<&pm_core::log::Event> = None;
    let mut total_cost: Option<i128> = None;

    // Gaps are real waiting only when every preceding event has an end
    // timestamp. A case where some do and some do not would otherwise mix two
    // different measurements in one column.
    let gap_is_waiting = !trace.incomplete
        && trace.events.len() >= 2
        && trace.events[..trace.events.len() - 1]
            .iter()
            .all(|e| e.end().is_some());

    // Columns that changed within this item go on its steps; a column that
    // changes in other items but not this one stays in its header.
    let step_level: std::collections::BTreeSet<usize> = log
        .step_level_attributes()
        .into_iter()
        .filter(|index| log.case_values(trace, *index).len() > 1)
        .collect();
    let mut last_values: Vec<Option<u32>> = vec![None; log.attribute_names.len()];
    for event in &trace.events {
        let mut changes = Vec::new();
        for &index in &step_level {
            let value = log.step_attribute(trace, event, index);
            if value.is_some() && value != last_values[index] {
                if let Some(label) = value.and_then(|v| log.attribute_values.label(v)) {
                    changes.push((log.attribute_names[index].clone(), label.to_owned()));
                }
                last_values[index] = value;
            }
        }
        let cost = log.event_cost(trace.case, event);
        if let Some(c) = cost {
            total_cost = Some(total_cost.unwrap_or(0) + i128::from(c));
        }
        // From the previous activity's end when we have it — that is waiting.
        // Overlapping activities yield zero rather than a negative gap.
        let since_previous = previous.map(|p| {
            // A column must contain one measurement, not a mixture. If any
            // transition cannot measure true waiting, every row falls back to
            // start-to-start elapsed time so the label describes all values.
            let from = if gap_is_waiting {
                p.end().expect("gap eligibility checked above")
            } else {
                p.at
            };
            ((event.at - from).max(0)) / MICROS_PER_SECOND
        });
        events.push(CaseEvent {
            activity: log.activity_label(event.activity).to_owned(),
            timestamp: render(event.at),
            end_timestamp: event.end().map(render),
            processing_seconds: event.processing_time().map(|d| d / MICROS_PER_SECOND),
            since_previous_seconds: since_previous,
            resource: event.resource().map(|r| log.resource_label(r).to_owned()),
            cost_microunits: cost,
            source_row: event.source_row,
            changes,
        });
        previous = Some(event);
    }

    CaseDetail {
        case: log.case_label(trace.case).to_owned(),
        variant: trace
            .events
            .iter()
            .map(|e| log.activity_label(e.activity).to_owned())
            .collect(),
        throughput_seconds: trace.throughput() / MICROS_PER_SECOND,
        throughput_eligible: !trace.incomplete,
        incomplete: trace.incomplete,
        attributes: log
            .attribute_names
            .iter()
            .enumerate()
            .filter(|(i, _)| !step_level.contains(i))
            .filter_map(|(i, name)| {
                trace
                    .attribute(i)
                    .and_then(|v| log.attribute_values.label(v))
                    .map(|value| (name.clone(), value.to_owned()))
            })
            .collect(),
        total_cost_microunits: total_cost,
        source_rows: trace.events.iter().map(|e| e.source_row).collect(),
        gap_is_waiting,
        timestamp_reading: reading.to_owned(),
        events,
    }
}

/// Renders an instant in ISO form.
///
/// Deliberately without an offset: the value means whatever the recorded
/// timezone reading says it means, and appending `Z` would assert UTC even
/// when the user chose to read timestamps as written. The reading is reported
/// alongside, in [`CaseDetail::timestamp_reading`].
fn render(at: Micros) -> String {
    chrono::DateTime::from_timestamp_micros(at)
        .map(|d| d.naive_utc().format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::dict::{CaseId, Dictionary};
    use pm_core::log::{Event, NO_ATTRIBUTE};
    use std::collections::BTreeMap;

    const HOUR: Micros = 3_600_000_000;

    fn sample() -> EventLog {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let mut resources = Dictionary::new();
        let mut attribute_values = Dictionary::new();

        let c1 = CaseId(cases.intern("ORD-1"));
        let c2 = CaseId(cases.intern("ORD-2"));
        let receive = ActivityId(activities.intern("Receive"));
        let check = ActivityId(activities.intern("Check"));
        let ship = ActivityId(activities.intern("Ship"));
        let alice = resources.intern("alice");
        let bob = resources.intern("bob");
        let emea = attribute_values.intern("EMEA");

        let mut e1 = Event::new(receive, 0, 2);
        e1.resource = alice;
        e1.end_at = HOUR;
        let mut e2 = Event::new(check, 3 * HOUR, 3);
        e2.resource = alice;
        let mut e3 = Event::new(ship, 10 * HOUR, 4);
        e3.resource = bob;

        let mut costs = BTreeMap::new();
        costs.insert((c1, 2u32), 1_500_000i64);
        costs.insert((c1, 4u32), 2_500_000i64);

        EventLog {
            cases,
            activities,
            resources,
            attribute_names: vec!["region".to_owned()],
            attribute_values,
            costs,
            step_values: Default::default(),
            traces: vec![
                CaseTrace {
                    case: c1,
                    events: vec![e1, e2, e3],
                    incomplete: false,
                    attributes: vec![emea],
                },
                CaseTrace {
                    case: c2,
                    events: vec![Event::new(receive, HOUR, 5), Event::new(ship, 4 * HOUR, 6)],
                    incomplete: true,
                    attributes: vec![NO_ATTRIBUTE],
                },
            ],
        }
    }

    #[test]
    fn returns_a_case_in_full() {
        let d = inspect_case(&sample(), "ORD-1").expect("case exists");
        assert_eq!(d.case, "ORD-1");
        assert_eq!(d.variant, vec!["Receive", "Check", "Ship"]);
        assert_eq!(d.throughput_seconds, 10 * 3600);
        assert!(!d.incomplete);
    }

    #[test]
    fn an_unknown_case_is_not_an_error() {
        assert!(inspect_case(&sample(), "NOPE").is_none());
    }

    /// Evidence has to point at a line the user can open.
    #[test]
    fn carries_source_rows() {
        let d = inspect_case(&sample(), "ORD-1").unwrap();
        assert_eq!(d.source_rows, vec![2, 3, 4]);
        assert_eq!(d.events[1].source_row, 3);
    }

    /// Only the first event of ORD-1 has an end timestamp, so the gaps here
    /// are not pure waiting and must not claim to be.
    #[test]
    fn reports_time_between_events() {
        let d = inspect_case(&sample(), "ORD-1").unwrap();
        assert_eq!(d.events[0].since_previous_seconds, None, "the first event");
        // Because a later transition lacks an end timestamp, the entire
        // column consistently falls back to start-to-start elapsed time.
        assert_eq!(d.events[1].since_previous_seconds, Some(3 * 3600));
        assert_eq!(d.events[2].since_previous_seconds, Some(7 * 3600));
        assert!(!d.gap_is_waiting, "not every preceding event has an end");
    }

    /// With end timestamps throughout, the gap excludes processing time.
    #[test]
    fn measures_real_waiting_when_every_event_has_an_end() {
        let mut l = sample();
        l.traces[0].events[1].end_at = 4 * HOUR;
        l.traces[0].events[2].end_at = 11 * HOUR;
        let d = inspect_case(&l, "ORD-1").unwrap();
        assert!(d.gap_is_waiting);
        // Check ends at 4h, Ship starts at 10h.
        assert_eq!(d.events[2].since_previous_seconds, Some(6 * 3600));
    }

    /// Overlapping activities are bad data, not negative waiting.
    #[test]
    fn overlapping_activities_yield_zero_not_a_negative_gap() {
        let mut l = sample();
        l.traces[0].events[0].end_at = 5 * HOUR;
        l.traces[0].events[1].end_at = 7 * HOUR;
        let d = inspect_case(&l, "ORD-1").unwrap();
        assert!(d.gap_is_waiting);
        assert_eq!(d.events[1].since_previous_seconds, Some(0));
    }

    #[test]
    fn incomplete_cases_never_claim_complete_waiting_measurement() {
        let mut l = sample();
        l.traces[1].events[0].end_at = 2 * HOUR;
        let d = inspect_case(&l, "ORD-2").unwrap();
        assert!(!d.gap_is_waiting);
        assert_eq!(d.events[1].since_previous_seconds, Some(3 * 3600));
    }

    /// A displayed instant means different things under different readings, so
    /// the reading travels with it.
    #[test]
    fn states_how_timestamps_should_be_read() {
        let d = inspect_case_read_as(&sample(), "ORD-1", "UTC").unwrap();
        assert_eq!(d.timestamp_reading, "UTC");
    }

    #[test]
    fn reports_processing_time_where_an_end_timestamp_exists() {
        let d = inspect_case(&sample(), "ORD-1").unwrap();
        assert_eq!(d.events[0].processing_seconds, Some(3600));
        assert_eq!(d.events[1].processing_seconds, None);
    }

    #[test]
    fn reports_resources_costs_and_attributes() {
        let d = inspect_case(&sample(), "ORD-1").unwrap();
        assert_eq!(d.events[0].resource.as_deref(), Some("alice"));
        assert_eq!(d.events[2].resource.as_deref(), Some("bob"));
        assert_eq!(d.events[0].cost_microunits, Some(1_500_000));
        assert_eq!(d.total_cost_microunits, Some(4_000_000i128));
        assert_eq!(d.attributes, vec![("region".to_owned(), "EMEA".to_owned())]);
    }

    /// A case excluded from duration averages must say so when opened,
    /// otherwise its throughput reads as a fact.
    #[test]
    fn flags_an_incomplete_case() {
        let d = inspect_case(&sample(), "ORD-2").unwrap();
        assert!(d.incomplete);
        assert!(!d.throughput_eligible);
    }

    #[test]
    fn complete_cases_are_throughput_eligible() {
        let d = inspect_case(&sample(), "ORD-1").unwrap();
        assert!(d.throughput_eligible);
    }

    #[test]
    fn a_case_without_attributes_reports_none() {
        let d = inspect_case(&sample(), "ORD-2").unwrap();
        assert!(d.attributes.is_empty());
        assert_eq!(d.total_cost_microunits, None);
    }

    /// Case totals accumulate in i128, matching the whole-log total, so a
    /// long or expensive case cannot overflow where the aggregate would not.
    #[test]
    fn case_cost_totals_do_not_overflow() {
        let mut l = sample();
        let c1 = l.traces[0].case;
        for row in [2u32, 3, 4] {
            l.costs.insert((c1, row), i64::MAX);
        }
        let d = inspect_case(&l, "ORD-1").unwrap();
        assert_eq!(d.total_cost_microunits, Some(3 * i128::from(i64::MAX)));
    }

    #[test]
    fn finds_cases_behind_a_transition() {
        let l = sample();
        let e = Evidence::Transition {
            from: "Receive".to_owned(),
            to: "Check".to_owned(),
        };
        assert_eq!(find_cases(&l, &e, 10), vec!["ORD-1"]);
        assert_eq!(count_cases(&l, &e), 1);
    }

    #[test]
    fn finds_cases_behind_an_activity() {
        let l = sample();
        let e = Evidence::Activity("Ship".to_owned());
        assert_eq!(find_cases(&l, &e, 10), vec!["ORD-1", "ORD-2"]);
    }

    #[test]
    fn finds_cases_behind_a_variant() {
        let l = sample();
        let e = Evidence::Variant(vec!["Receive".to_owned(), "Ship".to_owned()]);
        assert_eq!(find_cases(&l, &e, 10), vec!["ORD-2"]);
    }

    #[test]
    fn finds_cases_behind_a_handover() {
        let l = sample();
        let e = Evidence::Handover {
            from: "alice".to_owned(),
            to: "bob".to_owned(),
        };
        assert_eq!(find_cases(&l, &e, 10), vec!["ORD-1"]);
    }

    /// A name that does not occur has no witnesses — not every case.
    #[test]
    fn an_unknown_name_matches_no_cases() {
        let l = sample();
        assert!(find_cases(&l, &Evidence::Activity("Nope".to_owned()), 10).is_empty());
        assert!(
            find_cases(
                &l,
                &Evidence::Transition {
                    from: "Receive".to_owned(),
                    to: "Nope".to_owned()
                },
                10
            )
            .is_empty()
        );
    }

    /// Results are bounded: a transition on a large log has too many witnesses
    /// to return them all.
    #[test]
    fn limits_how_many_cases_are_returned() {
        let l = sample();
        let e = Evidence::Activity("Ship".to_owned());
        assert_eq!(find_cases(&l, &e, 1).len(), 1);
        assert_eq!(count_cases(&l, &e), 2, "the count is not truncated");
    }

    #[test]
    fn is_deterministic() {
        let l = sample();
        assert_eq!(inspect_case(&l, "ORD-1"), inspect_case(&l, "ORD-1"));
    }
}
