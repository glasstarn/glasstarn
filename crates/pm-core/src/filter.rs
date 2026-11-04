//! Filtering — choosing which cases to look at.
//!
//! # Filters select cases; rules transform events
//!
//! This distinction matters and is easy to blur.
//!
//! - A **rule** changes what the log *is*: renaming `STAT_03` to `Credit
//!   Check`, dropping a system-noise activity. Rules are preparation, recorded
//!   with the project, and apply to every view of the data.
//! - A **filter** changes what you are *looking at*: only EMEA cases, only
//!   cases that were escalated, only last quarter. Filters are exploration.
//!   They are cheap, temporary, and never alter the underlying log.
//!
//! Because a filter selects whole cases, a filtered case keeps its complete
//! path. Showing a partial trace would invent transitions that never happened
//! — a case filtered to "only the steps I asked about" would appear to go
//! straight from Receive to Ship.
//!
//! # Every filter reports what it removed
//!
//! A view showing 12% of the data must say so. Silent narrowing is how someone
//! ends up presenting a finding about a fraction of their process believing it
//! describes all of it.

use crate::dict::ActivityId;
use crate::log::{CaseTrace, EventLog, Micros};
use crate::variants::Variants;
use std::collections::BTreeSet;

/// Which cases to include.
///
/// An empty filter includes everything. Conditions combine with **and**: a
/// case must satisfy every one that is set.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Case must contain **all** of these activities.
    pub with_activities: BTreeSet<String>,
    /// Case must contain **none** of these activities.
    pub without_activities: BTreeSet<String>,
    /// The first recorded event must have this activity.
    pub starts_with: Option<String>,
    /// The last recorded event must have this activity.
    pub ends_with: Option<String>,
    /// Every listed activity pair must occur consecutively in the case.
    pub directly_follows: BTreeSet<(String, String)>,
    /// Every listed activity pair must occur in order, with any steps between.
    pub eventually_follows: BTreeSet<(String, String)>,
    /// Contains the source but no target strictly later in recorded time.
    /// Unlike sequence order, a same-time pair does not prove a hand-off.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub not_reached: BTreeSet<(String, String)>,
    /// Case attribute must equal this value, as (attribute, value).
    pub attribute_equals: Vec<(String, String)>,
    /// Values within one field combine with OR; different fields combine with AND.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub attribute_in: std::collections::BTreeMap<String, BTreeSet<String>>,
    /// For a column that changes between steps: the item's latest value (at
    /// its last step with one) must be one of these. "Ever had" is
    /// `attribute_in`; this is "is now".
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub attribute_latest: std::collections::BTreeMap<String, BTreeSet<String>>,
    /// Fields whose value must be absent.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub attribute_missing: BTreeSet<String>,
    /// Someone in this set must have recorded at least one of the case's steps.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub handled_by: BTreeSet<String>,
    /// Case must record each of these activities more than once.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub repeats: BTreeSet<String>,
    /// Case must record some activity, any one, more than once.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub repeats_any: bool,
    /// Keep only cases whose whole step sequence is one of these variants,
    /// each given as its step names in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Vec<String>>,
    /// Keep only cases **not** following one of the N most common variants.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outside_top_variants: Option<usize>,
    /// The last recorded event must be none of these activities: items not
    /// (yet) at a finishing step.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub not_ending_with: BTreeSet<String>,
    /// Explicit recorded interval, including its occurrence and inclusive bounds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment: Option<crate::segment::Segment>,
    /// Case attribute must read as a number within these bounds.
    pub attribute_between: Vec<crate::number::NumberRange>,
    /// Case must start at or after this instant.
    pub started_after: Option<Micros>,
    /// Case must start at or before this instant.
    pub started_before: Option<Micros>,
    /// Case throughput must be at least this long.
    pub min_duration: Option<Micros>,
    /// Case throughput must be at most this long.
    pub max_duration: Option<Micros>,
    /// Keep only cases following one of the N most common variants.
    pub top_variants: Option<usize>,
    /// Case must have at least this many events.
    pub min_events: Option<usize>,
    /// Drop cases flagged incomplete.
    pub complete_only: bool,
    /// Keep only cases that ran their whole course inside this window: see
    /// [`crate::window`].
    pub inside_window: Option<crate::window::ObservationWindow>,
}

impl Filter {
    /// A filter that includes everything.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requires the case to contain `activity`.
    pub fn with_activity(mut self, activity: &str) -> Self {
        self.with_activities.insert(activity.to_owned());
        self
    }

    /// Requires the case not to contain `activity`.
    pub fn without_activity(mut self, activity: &str) -> Self {
        self.without_activities.insert(activity.to_owned());
        self
    }

    /// Requires the first recorded event to have `activity`.
    pub fn starts_with(mut self, activity: &str) -> Self {
        self.starts_with = Some(activity.to_owned());
        self
    }

    /// Requires the last recorded event to have `activity`.
    pub fn ends_with(mut self, activity: &str) -> Self {
        self.ends_with = Some(activity.to_owned());
        self
    }

    /// Requires `from` to be immediately followed by `to` in the normalized
    /// deterministic event sequence.
    pub fn directly_follows(mut self, from: &str, to: &str) -> Self {
        self.directly_follows
            .insert((from.to_owned(), to.to_owned()));
        self
    }

    /// Requires `from` to occur before a later `to`, allowing intervening
    /// events in the normalized deterministic event sequence.
    pub fn eventually_follows(mut self, from: &str, to: &str) -> Self {
        self.eventually_follows
            .insert((from.to_owned(), to.to_owned()));
        self
    }

    /// Requires a case attribute to equal a value.
    pub fn attribute(mut self, name: &str, value: &str) -> Self {
        self.attribute_equals
            .push((name.to_owned(), value.to_owned()));
        self
    }

    /// Requires a case attribute to read as a number within the inclusive
    /// bounds; either bound may be open.
    pub fn number_between(
        mut self,
        name: &str,
        at_least: Option<f64>,
        at_most: Option<f64>,
    ) -> Self {
        self.attribute_between.push(crate::number::NumberRange {
            attribute: name.to_owned(),
            at_least,
            at_most,
            ..Default::default()
        });
        self
    }

    /// Keeps only cases starting at or after `at`.
    pub fn started_after(mut self, at: Micros) -> Self {
        self.started_after = Some(at);
        self
    }

    /// Keeps only cases starting at or before `at`.
    pub fn started_before(mut self, at: Micros) -> Self {
        self.started_before = Some(at);
        self
    }

    /// Keeps only cases lasting at least `duration`.
    pub fn min_duration(mut self, duration: Micros) -> Self {
        self.min_duration = Some(duration);
        self
    }

    /// Keeps only cases lasting at most `duration`.
    pub fn max_duration(mut self, duration: Micros) -> Self {
        self.max_duration = Some(duration);
        self
    }

    /// Keeps only cases following one of the `n` most common paths.
    ///
    /// The most direct answer to an unreadable map: the tail of near-unique
    /// variants is usually what makes it illegible.
    pub fn top_variants(mut self, n: usize) -> Self {
        self.top_variants = Some(n);
        self
    }

    /// Requires at least `n` events.
    pub fn min_events(mut self, n: usize) -> Self {
        self.min_events = Some(n);
        self
    }

    /// Drops cases flagged incomplete.
    pub fn complete_only(mut self, only: bool) -> Self {
        self.complete_only = only;
        self
    }

    /// Keeps only cases that began, and when finishing steps are confirmed
    /// finished, inside `window`.
    pub fn inside_window(mut self, window: crate::window::ObservationWindow) -> Self {
        self.inside_window = Some(window);
        self
    }

    /// Whether this filter would change anything.
    pub fn is_empty(&self) -> bool {
        self.with_activities.is_empty()
            && self.without_activities.is_empty()
            && self.starts_with.is_none()
            && self.ends_with.is_none()
            && self.directly_follows.is_empty()
            && self.eventually_follows.is_empty()
            && self.not_reached.is_empty()
            && self.attribute_equals.is_empty()
            && self.attribute_in.is_empty()
            && self.attribute_missing.is_empty()
            && self.handled_by.is_empty()
            && self.repeats.is_empty()
            && !self.repeats_any
            && self.variants.is_empty()
            && self.attribute_latest.is_empty()
            && self.outside_top_variants.is_none()
            && self.not_ending_with.is_empty()
            && self.segment.is_none()
            && self.attribute_between.is_empty()
            && self.started_after.is_none()
            && self.started_before.is_none()
            && self.min_duration.is_none()
            && self.max_duration.is_none()
            && self.top_variants.is_none()
            && self.min_events.is_none()
            && !self.complete_only
            && self.inside_window.is_none()
    }

    /// Applies the filter, returning the narrowed log and what it removed.
    /// Applies the filter. With no conditions the log is borrowed, not
    /// copied: a copy would double the memory of a large log for nothing.
    pub fn apply<'a>(&self, log: &'a EventLog) -> Filtered<'a> {
        let log_ref = log;
        if self.is_empty() {
            return Filtered {
                log: std::borrow::Cow::Borrowed(log),
                cases_before: log.case_count(),
                events_before: log.event_count(),
            };
        }

        let ids = self.resolve_activities(log);
        let patterns = self.resolve_patterns(log);
        let attributes = self.resolve_attributes(log);
        let allowed_variants = self.resolve_top_variants(log);
        let excluded_variants = self.outside_top_variants.map(|n| {
            Variants::build(log)
                .variants
                .iter()
                .take(n)
                .map(|v| v.sequence.clone())
                .collect::<BTreeSet<_>>()
        });

        let traces: Vec<CaseTrace> = log
            .traces
            .iter()
            .filter(|t| {
                self.keeps(
                    log_ref,
                    t,
                    &ids,
                    &patterns,
                    &attributes,
                    allowed_variants.as_ref(),
                ) && self.keeps_more(log_ref, t, excluded_variants.as_ref())
            })
            .cloned()
            .collect();
        let kept_events: BTreeSet<_> = traces
            .iter()
            .flat_map(|trace| {
                trace
                    .events
                    .iter()
                    .map(move |event| (trace.case, event.source_row))
            })
            .collect();
        let costs = log
            .costs
            .iter()
            .filter(|(key, _)| kept_events.contains(key))
            .map(|(key, value)| (*key, *value))
            .collect();
        let step_values = log
            .step_values
            .iter()
            .filter(|v| kept_events.contains(&(v.case, v.row)))
            .copied()
            .collect();

        Filtered {
            cases_before: log.case_count(),
            events_before: log.event_count(),
            // Dictionaries are kept whole rather than rebuilt: identifiers stay
            // stable, so a case referenced in one view means the same thing in
            // another.
            log: std::borrow::Cow::Owned(EventLog {
                cases: log.cases.clone(),
                activities: log.activities.clone(),
                resources: log.resources.clone(),
                attribute_names: log.attribute_names.clone(),
                attribute_values: log.attribute_values.clone(),
                costs,
                step_values,
                traces,
            }),
        }
    }

    /// Resolves activity names to identifiers.
    ///
    /// A name that does not occur in the log resolves to nothing, which makes
    /// a `with_activity` condition match no case at all. That is the correct
    /// reading — no case contains an activity that does not exist — and the
    /// resulting empty view is reported rather than hidden.
    fn resolve_activities(
        &self,
        log: &EventLog,
    ) -> (Vec<Option<ActivityId>>, Vec<Option<ActivityId>>) {
        let lookup = |name: &String| log.activities.get(name).map(ActivityId);
        (
            self.with_activities.iter().map(lookup).collect(),
            self.without_activities.iter().map(lookup).collect(),
        )
    }

    fn resolve_attributes(&self, log: &EventLog) -> Vec<(Option<usize>, String)> {
        self.attribute_equals
            .iter()
            .map(|(name, value)| (log.attribute_index(name), value.clone()))
            .collect()
    }

    fn resolve_patterns(&self, log: &EventLog) -> ResolvedPatterns {
        let activity = |name: &str| log.activities.get(name).map(ActivityId);
        let pairs = |items: &BTreeSet<(String, String)>| {
            items
                .iter()
                .map(|(from, to)| (activity(from), activity(to)))
                .collect()
        };
        ResolvedPatterns {
            starts_with: self.starts_with.as_deref().and_then(activity),
            starts_with_unknown: self.starts_with.is_some()
                && self.starts_with.as_deref().and_then(activity).is_none(),
            ends_with: self.ends_with.as_deref().and_then(activity),
            ends_with_unknown: self.ends_with.is_some()
                && self.ends_with.as_deref().and_then(activity).is_none(),
            directly_follows: pairs(&self.directly_follows),
            eventually_follows: pairs(&self.eventually_follows),
            not_reached: pairs(&self.not_reached),
        }
    }

    /// The conditions added for findings: repeated steps, routes outside the
    /// most common ones, and items not at a finishing step.
    fn keeps_more(
        &self,
        log: &EventLog,
        trace: &CaseTrace,
        excluded_variants: Option<&BTreeSet<Vec<ActivityId>>>,
    ) -> bool {
        for name in &self.repeats {
            let Some(id) = log.activities.get(name) else {
                return false;
            };
            if trace
                .events
                .iter()
                .filter(|event| event.activity.0 == id)
                .count()
                < 2
            {
                return false;
            }
        }
        if !self.variants.is_empty()
            && !self.variants.iter().any(|variant| {
                variant.len() == trace.events.len()
                    && variant
                        .iter()
                        .zip(&trace.events)
                        .all(|(name, event)| log.activity_label(event.activity) == name)
            })
        {
            return false;
        }
        if self.repeats_any {
            let mut seen = BTreeSet::new();
            if trace.events.iter().all(|event| seen.insert(event.activity)) {
                return false;
            }
        }
        for (field, values) in &self.attribute_latest {
            let latest = log
                .attribute_index(field)
                .and_then(|index| log.latest_value(trace, index))
                .and_then(|id| log.attribute_values.label(id));
            if !latest.is_some_and(|value| values.contains(value)) {
                return false;
            }
        }
        if let Some(excluded) = excluded_variants {
            let sequence: Vec<ActivityId> = trace.events.iter().map(|e| e.activity).collect();
            if excluded.contains(&sequence) {
                return false;
            }
        }
        if !self.not_ending_with.is_empty()
            && trace.events.last().is_some_and(|event| {
                self.not_ending_with
                    .contains(log.activity_label(event.activity))
            })
        {
            return false;
        }
        true
    }

    fn resolve_top_variants(&self, log: &EventLog) -> Option<BTreeSet<Vec<ActivityId>>> {
        let n = self.top_variants?;
        let variants = Variants::build(log);
        Some(
            variants
                .variants
                .iter()
                .take(n)
                .map(|v| v.sequence.clone())
                .collect(),
        )
    }

    fn keeps(
        &self,
        log: &EventLog,
        trace: &CaseTrace,
        (with, without): &(Vec<Option<ActivityId>>, Vec<Option<ActivityId>>),
        patterns: &ResolvedPatterns,
        attributes: &[(Option<usize>, String)],
        allowed_variants: Option<&BTreeSet<Vec<ActivityId>>>,
    ) -> bool {
        if self.complete_only && trace.incomplete {
            return false;
        }
        if let Some(window) = &self.inside_window
            && !window.label(log, trace).complete()
        {
            return false;
        }
        if let Some(n) = self.min_events
            && trace.len() < n
        {
            return false;
        }

        let present: BTreeSet<ActivityId> = trace.events.iter().map(|e| e.activity).collect();
        for required in with {
            match required {
                Some(id) if present.contains(id) => {}
                // Includes the unknown-activity case: nothing satisfies it.
                _ => return false,
            }
        }
        for forbidden in without.iter().flatten() {
            if present.contains(forbidden) {
                return false;
            }
        }

        if patterns.starts_with_unknown
            || patterns.ends_with_unknown
            || patterns
                .directly_follows
                .iter()
                .chain(&patterns.eventually_follows)
                .any(|(from, to)| from.is_none() || to.is_none())
        {
            return false;
        }
        if let Some(activity) = patterns.starts_with
            && trace.events.first().map(|event| event.activity) != Some(activity)
        {
            return false;
        }
        if let Some(activity) = patterns.ends_with
            && trace.events.last().map(|event| event.activity) != Some(activity)
        {
            return false;
        }
        for (from, to) in &patterns.directly_follows {
            let (Some(from), Some(to)) = (from, to) else {
                return false;
            };
            if !trace
                .events
                .windows(2)
                .any(|pair| pair[0].activity == *from && pair[1].activity == *to)
            {
                return false;
            }
        }
        for (from, to) in &patterns.eventually_follows {
            let (Some(from), Some(to)) = (from, to) else {
                return false;
            };
            let mut saw_from = false;
            let mut matched = false;
            for event in &trace.events {
                if saw_from && event.activity == *to {
                    matched = true;
                    break;
                }
                saw_from |= event.activity == *from;
            }
            if !matched {
                return false;
            }
        }

        for (from, to) in &patterns.not_reached {
            let Some(first) = trace
                .events
                .iter()
                .filter(|event| Some(event.activity) == *from)
                .map(|event| event.at)
                .min()
            else {
                return false;
            };
            if trace
                .events
                .iter()
                .any(|event| Some(event.activity) == *to && event.at > first)
            {
                return false;
            }
        }
        if let Some(segment) = &self.segment
            && !segment
                .duration(log, trace)
                .is_some_and(|duration| segment.contains(duration))
        {
            return false;
        }
        // A column that changes between steps matches when the item ever
        // had one of the values, on any step.
        for (field, values) in &self.attribute_in {
            let had = log.attribute_index(field).is_some_and(|index| {
                log.case_values(trace, index).into_iter().any(|id| {
                    log.attribute_values
                        .label(id)
                        .is_some_and(|value| values.contains(value))
                })
            });
            if !had {
                return false;
            }
        }
        if !self.handled_by.is_empty() {
            let people: BTreeSet<u32> = self
                .handled_by
                .iter()
                .filter_map(|name| log.resources.get(name))
                .collect();
            if !trace.events.iter().any(|event| {
                event
                    .resource()
                    .is_some_and(|resource| people.contains(&resource.0))
            }) {
                return false;
            }
        }
        for field in &self.attribute_missing {
            if log
                .attribute_index(field)
                .and_then(|index| trace.attribute(index))
                .is_some()
            {
                return false;
            }
        }
        for range in &self.attribute_between {
            let number = log
                .attribute_index(&range.attribute)
                .and_then(|index| crate::number::case_number(log, trace, index));
            if !number.is_some_and(|number| range.contains(number)) {
                return false;
            }
        }

        for (index, wanted) in attributes {
            // An unknown attribute cannot be satisfied, and a case with no
            // value for it does not match a specific value.
            let Some(index) = index else { return false };
            let Some(interned) = log.attribute_values.get(wanted) else {
                return false;
            };
            if !log.case_had(trace, *index, interned) {
                return false;
            }
        }

        let start = trace.start();
        if let Some(after) = self.started_after
            && start < after
        {
            return false;
        }
        if let Some(before) = self.started_before
            && start > before
        {
            return false;
        }

        let duration = trace.throughput();
        if let Some(min) = self.min_duration
            && duration < min
        {
            return false;
        }
        if let Some(max) = self.max_duration
            && duration > max
        {
            return false;
        }

        if let Some(allowed) = allowed_variants {
            let sequence: Vec<ActivityId> = trace.events.iter().map(|e| e.activity).collect();
            if !allowed.contains(&sequence) {
                return false;
            }
        }

        true
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolvedPatterns {
    starts_with: Option<ActivityId>,
    starts_with_unknown: bool,
    ends_with: Option<ActivityId>,
    ends_with_unknown: bool,
    directly_follows: Vec<(Option<ActivityId>, Option<ActivityId>)>,
    eventually_follows: Vec<(Option<ActivityId>, Option<ActivityId>)>,
    not_reached: Vec<(Option<ActivityId>, Option<ActivityId>)>,
}

/// A filtered log, with what was removed.
#[derive(Debug, Clone, PartialEq)]
pub struct Filtered<'a> {
    pub log: std::borrow::Cow<'a, EventLog>,
    pub cases_before: usize,
    pub events_before: usize,
}

impl Filtered<'_> {
    /// Cases removed by the filter.
    pub fn cases_removed(&self) -> usize {
        self.cases_before.saturating_sub(self.log.case_count())
    }

    /// Events removed by the filter.
    pub fn events_removed(&self) -> usize {
        self.events_before.saturating_sub(self.log.event_count())
    }

    /// Share of cases kept, as a percentage.
    ///
    /// The number that must always be shown: a view of 12% of the data has to
    /// say so.
    pub fn cases_kept_percent(&self) -> f64 {
        if self.cases_before == 0 {
            100.0
        } else {
            self.log.case_count() as f64 / self.cases_before as f64 * 100.0
        }
    }

    /// Whether the filter removed anything.
    pub fn narrowed(&self) -> bool {
        self.cases_removed() > 0
    }

    /// Whether the filter removed everything.
    pub fn is_empty(&self) -> bool {
        self.log.case_count() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::{CaseId, Dictionary};
    use crate::log::{Event, NO_ATTRIBUTE};

    const HOUR: Micros = 3_600_000_000;

    type Step<'a> = (&'a str, i64);
    type Case<'a> = (&'a str, &'a [Step<'a>], &'a str);

    /// Builds a log from `(case, events, region)`.
    fn log(spec: &[Case<'_>]) -> EventLog {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let mut attribute_values = Dictionary::new();
        let mut traces = Vec::new();
        let mut row = 2u32;
        for (case, events, region) in spec {
            let case_id = CaseId(cases.intern(case));
            let mut list = Vec::new();
            for (activity, hour) in *events {
                list.push(Event::new(
                    ActivityId(activities.intern(activity)),
                    hour * HOUR,
                    row,
                ));
                row += 1;
            }
            let attributes = if region.is_empty() {
                vec![NO_ATTRIBUTE]
            } else {
                vec![attribute_values.intern(region)]
            };
            traces.push(CaseTrace {
                case: case_id,
                events: list,
                incomplete: false,
                attributes,
            });
        }
        EventLog {
            cases,
            activities,
            resources: Dictionary::new(),
            attribute_names: vec!["region".to_owned()],
            attribute_values,
            costs: Default::default(),
            step_values: Default::default(),
            traces,
        }
    }

    fn sample() -> EventLog {
        log(&[
            ("C1", &[("Receive", 0), ("Check", 2), ("Ship", 10)], "EMEA"),
            (
                "C2",
                &[("Receive", 1), ("Check", 3), ("Escalate", 20), ("Ship", 50)],
                "AMER",
            ),
            ("C3", &[("Receive", 2), ("Check", 4), ("Ship", 12)], "EMEA"),
            ("C4", &[("Receive", 3), ("Reject", 5)], "AMER"),
        ])
    }

    fn case_labels<'a>(f: &'a Filtered<'_>) -> Vec<&'a str> {
        f.log
            .traces
            .iter()
            .map(|t| f.log.case_label(t.case))
            .collect()
    }

    #[test]
    fn an_empty_filter_changes_nothing() {
        let l = sample();
        let f = Filter::new().apply(&l);
        assert_eq!(*f.log, l);
        assert!(!f.narrowed());
        assert_eq!(f.cases_kept_percent(), 100.0);
    }

    #[test]
    fn keeps_cases_containing_an_activity() {
        let sample_log = sample();
        let f = Filter::new().with_activity("Escalate").apply(&sample_log);
        assert_eq!(case_labels(&f), vec!["C2"]);
        assert_eq!(f.cases_removed(), 3);
    }

    #[test]
    fn inside_window_keeps_cases_that_began_and_finished_in_it() {
        let window = crate::window::ObservationWindow {
            start_micros: HOUR,
            end_micros: 60 * HOUR,
            end_activities: ["Ship".to_owned()].into(),
        };
        let sample_log = sample();
        let f = Filter::new().inside_window(window).apply(&sample_log);
        // C1 began before the window; C4 never reached a finishing step.
        assert_eq!(case_labels(&f), vec!["C2", "C3"]);
    }

    #[test]
    fn drops_cases_containing_an_activity() {
        let sample_log = sample();
        let f = Filter::new().without_activity("Reject").apply(&sample_log);
        assert_eq!(case_labels(&f), vec!["C1", "C2", "C3"]);
    }

    #[test]
    fn filters_by_start_and_end_activity_without_trimming_cases() {
        let sample_log = sample();
        let filtered = Filter::new()
            .starts_with("Receive")
            .ends_with("Ship")
            .apply(&sample_log);
        assert_eq!(case_labels(&filtered), vec!["C1", "C2", "C3"]);
        assert_eq!(filtered.log.event_count(), 10);
    }

    #[test]
    fn directly_follows_requires_a_consecutive_recorded_pair() {
        let sample_log = sample();
        let direct = Filter::new()
            .directly_follows("Check", "Ship")
            .apply(&sample_log);
        assert_eq!(case_labels(&direct), vec!["C1", "C3"]);

        let sample_log = sample();

        let not_direct = Filter::new()
            .directly_follows("Receive", "Ship")
            .apply(&sample_log);
        assert!(not_direct.is_empty());
    }

    #[test]
    fn eventually_follows_allows_intervening_events_but_requires_strict_order() {
        let sample_log = sample();
        let filtered = Filter::new()
            .eventually_follows("Check", "Ship")
            .apply(&sample_log);
        assert_eq!(case_labels(&filtered), vec!["C1", "C2", "C3"]);

        let sample_log = sample();

        let reverse = Filter::new()
            .eventually_follows("Ship", "Check")
            .apply(&sample_log);
        assert!(reverse.is_empty());
    }

    #[test]
    fn repeated_activity_eventually_follows_itself_only_with_two_occurrences() {
        let repeated = log(&[
            ("C1", &[("A", 0), ("B", 1), ("A", 2)], ""),
            ("C2", &[("A", 0), ("B", 1)], ""),
        ]);
        let filtered = Filter::new().eventually_follows("A", "A").apply(&repeated);
        assert_eq!(case_labels(&filtered), vec!["C1"]);
    }

    #[test]
    fn trace_conditions_compose_with_each_other_and_existing_filters() {
        let sample_log = sample();
        let filtered = Filter::new()
            .starts_with("Receive")
            .directly_follows("Receive", "Check")
            .eventually_follows("Check", "Escalate")
            .ends_with("Ship")
            .attribute("region", "AMER")
            .apply(&sample_log);
        assert_eq!(case_labels(&filtered), vec!["C2"]);
    }

    #[test]
    fn unknown_trace_activity_matches_no_case() {
        assert!(
            Filter::new()
                .starts_with("Unknown")
                .apply(&sample())
                .is_empty()
        );
        assert!(
            Filter::new()
                .directly_follows("Receive", "Unknown")
                .apply(&sample())
                .is_empty()
        );
    }

    /// A filtered case keeps its **whole** path. Trimming it would invent
    /// transitions that never occurred.
    #[test]
    fn a_kept_case_keeps_its_complete_path() {
        let l = sample();
        let f = Filter::new().with_activity("Escalate").apply(&l);
        assert_eq!(
            f.log.traces[0].len(),
            4,
            "the full trace, not just the match"
        );
    }

    #[test]
    fn filtering_keeps_only_costs_for_retained_events() {
        let mut l = sample();
        for trace in &l.traces {
            for event in &trace.events {
                l.costs.insert((trace.case, event.source_row), 1_000_000);
            }
        }
        let f = Filter::new().with_activity("Escalate").apply(&l);
        assert_eq!(f.log.case_count(), 1);
        assert_eq!(f.log.costs.len(), f.log.event_count());
        assert!(
            f.log
                .costs
                .keys()
                .all(|(case, _)| f.log.case_label(*case) == "C2")
        );
    }

    #[test]
    fn combines_conditions_with_and() {
        let sample_log = sample();
        let f = Filter::new()
            .without_activity("Escalate")
            .without_activity("Reject")
            .apply(&sample_log);
        assert_eq!(case_labels(&f), vec!["C1", "C3"]);
    }

    #[test]
    fn filters_by_duration() {
        let sample_log = sample();
        let f = Filter::new().min_duration(20 * HOUR).apply(&sample_log);
        assert_eq!(case_labels(&f), vec!["C2"], "only C2 runs 49 hours");
    }

    #[test]
    fn filters_by_start_time() {
        let sample_log = sample();
        let f = Filter::new().started_after(2 * HOUR).apply(&sample_log);
        assert_eq!(case_labels(&f), vec!["C3", "C4"]);
    }

    #[test]
    fn filters_by_case_length() {
        let sample_log = sample();
        let f = Filter::new().min_events(3).apply(&sample_log);
        assert_eq!(case_labels(&f), vec!["C1", "C2", "C3"]);
    }

    /// The direct answer to an unreadable map.
    #[test]
    fn keeps_only_the_most_common_paths() {
        let sample_log = sample();
        let f = Filter::new().top_variants(1).apply(&sample_log);
        // Receive → Check → Ship is followed by C1 and C3.
        assert_eq!(case_labels(&f), vec!["C1", "C3"]);
    }

    /// An activity that does not exist matches no case — and the empty result
    /// is reported rather than silently looking like "no matches by chance".
    #[test]
    fn an_unknown_activity_matches_nothing() {
        let sample_log = sample();
        let f = Filter::new()
            .with_activity("Nonexistent")
            .apply(&sample_log);
        assert!(f.is_empty());
        assert_eq!(f.cases_kept_percent(), 0.0);
        assert_eq!(f.cases_removed(), 4);
    }

    #[test]
    fn reports_how_much_was_removed() {
        let sample_log = sample();
        let f = Filter::new().with_activity("Escalate").apply(&sample_log);
        assert_eq!(f.cases_before, 4);
        assert_eq!(f.log.case_count(), 1);
        assert_eq!(f.cases_kept_percent(), 25.0);
        assert!(f.narrowed());
    }

    /// Identifiers must mean the same thing before and after filtering, so a
    /// case referenced in one view is the same case in another.
    #[test]
    fn identifiers_stay_stable_across_a_filter() {
        let l = sample();
        let f = Filter::new().with_activity("Escalate").apply(&l);
        let original = l
            .traces
            .iter()
            .find(|t| l.case_label(t.case) == "C2")
            .unwrap();
        assert_eq!(f.log.traces[0].case, original.case);
        assert_eq!(f.log.case_label(f.log.traces[0].case), "C2");
    }

    #[test]
    fn filters_by_case_attribute() {
        let sample_log = sample();
        let f = Filter::new().attribute("region", "EMEA").apply(&sample_log);
        assert_eq!(case_labels(&f), vec!["C1", "C3"]);
    }

    #[test]
    fn an_unknown_attribute_value_matches_nothing() {
        let sample_log = sample();
        let f = Filter::new().attribute("region", "APAC").apply(&sample_log);
        assert!(f.is_empty());
    }

    #[test]
    fn an_unknown_attribute_name_matches_nothing() {
        let sample_log = sample();
        let f = Filter::new().attribute("nope", "EMEA").apply(&sample_log);
        assert!(f.is_empty());
    }

    /// A case with no value for the attribute does not match a specific value.
    #[test]
    fn a_case_missing_the_attribute_does_not_match() {
        let l = log(&[("C1", &[("A", 0)], "EMEA"), ("C2", &[("A", 0)], "")]);
        let f = Filter::new().attribute("region", "EMEA").apply(&l);
        assert_eq!(case_labels(&f), vec!["C1"]);
    }

    /// The "region" column doubles as an amount here.
    #[test]
    fn filters_by_a_number_range_and_skips_missing_or_textual_values() {
        let l = log(&[
            ("C1", &[("A", 0)], "75,000"),
            ("C2", &[("A", 0)], "50000"),
            ("C3", &[("A", 0)], "49999.5"),
            ("C4", &[("A", 0)], ""),
            ("C5", &[("A", 0)], "n/a"),
        ]);
        let above = Filter::new()
            .number_between("region", Some(50_000.0), None)
            .apply(&l);
        assert_eq!(case_labels(&above), vec!["C1", "C2"]);
        let below = Filter::new()
            .number_between("region", None, Some(50_000.0))
            .apply(&l);
        assert_eq!(case_labels(&below), vec!["C2", "C3"]);
        let unknown = Filter::new()
            .number_between("nope", Some(0.0), None)
            .apply(&l);
        assert!(unknown.is_empty());
    }

    #[test]
    fn is_deterministic() {
        let l = sample();
        let f = || Filter::new().with_activity("Check").min_events(3).apply(&l);
        assert_eq!(f(), f());
    }

    #[test]
    fn filtering_an_empty_log_is_not_an_error() {
        let empty = EventLog::default();
        let f = Filter::new().with_activity("Anything").apply(&empty);
        assert!(f.is_empty());
        assert_eq!(f.cases_kept_percent(), 100.0);
    }

    #[test]
    fn handled_by_keeps_cases_someone_in_the_set_worked_on() {
        let mut log = sample();
        let dana = log.resources.intern("Dana");
        let sam = log.resources.intern("Sam");
        log.traces[0].events[1].resource = dana;
        log.traces[1].events[0].resource = sam;
        let mut filter = Filter::new();
        filter.handled_by.insert("Dana".to_owned());
        let kept = filter.apply(&log);
        assert_eq!(kept.log.case_count(), 1);
        assert_eq!(kept.log.case_label(kept.log.traces[0].case), "C1");
        filter.handled_by.insert("Sam".to_owned());
        assert_eq!(filter.apply(&log).log.case_count(), 2);
        assert!(!filter.is_empty());
    }

    #[test]
    fn finding_filters_keep_repeats_routes_off_the_main_ones_and_unfinished_items() {
        let log = sample();
        let mut repeats = Filter::new();
        repeats.repeats.insert("Check".to_owned());
        let before = log
            .traces
            .iter()
            .filter(|t| {
                t.events
                    .iter()
                    .filter(|e| log.activity_label(e.activity) == "Check")
                    .count()
                    > 1
            })
            .count();
        assert_eq!(repeats.apply(&log).log.case_count(), before);

        // Particular variants, by their steps.
        let mut chosen = Filter::new();
        let first: Vec<String> = log.traces[0]
            .events
            .iter()
            .map(|e| log.activity_label(e.activity).to_owned())
            .collect();
        chosen.variants.push(first.clone());
        let same = log
            .traces
            .iter()
            .filter(|t| {
                t.events
                    .iter()
                    .map(|e| log.activity_label(e.activity))
                    .eq(first.iter().map(String::as_str))
            })
            .count();
        assert_eq!(chosen.apply(&log).log.case_count(), same);

        // "Repeats a step": any step, not one in particular.
        let mut any = Filter::new();
        any.repeats_any = true;
        let repeating = log
            .traces
            .iter()
            .filter(|t| {
                let mut seen = BTreeSet::new();
                !t.events.iter().all(|e| seen.insert(e.activity))
            })
            .count();
        assert!(repeating >= before);
        assert_eq!(any.apply(&log).log.case_count(), repeating);

        let mut off = Filter::new();
        off.outside_top_variants = Some(1);
        let top = Filter::new().top_variants(1).apply(&log).log.case_count();
        assert_eq!(off.apply(&log).log.case_count() + top, log.case_count());

        let mut open = Filter::new();
        open.not_ending_with.insert("Ship".to_owned());
        let ending_ship = log
            .traces
            .iter()
            .filter(|t| {
                t.events
                    .last()
                    .is_some_and(|e| log.activity_label(e.activity) == "Ship")
            })
            .count();
        assert_eq!(
            open.apply(&log).log.case_count(),
            log.case_count() - ending_ship
        );
        assert!(!open.is_empty());
    }
}
