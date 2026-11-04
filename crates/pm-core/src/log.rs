//! The normalized event log — the input every analysis works from.
//!
//! By the time a log reaches this type, all interpretation has already
//! happened: labels are dictionary-encoded, events are ordered, and every
//! ambiguity has been resolved by an explicit, recorded decision. Nothing
//! downstream guesses.

use crate::dict::{ActivityId, CaseId, Dictionary};
use crate::trace::Trace;
use std::collections::BTreeMap;

/// A dictionary-encoded resource — the person, team, or system that performed
/// an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId(pub u32);

/// Microseconds since the Unix epoch.
///
/// Integer arithmetic throughout: durations must be exact and reproducible,
/// and floating point would make summation order matter.
pub type Micros = i64;

/// Marks an absent end timestamp.
///
/// A sentinel rather than `Option<Micros>` because an `Option` would add eight
/// bytes to every event. At ten million events that is 80 MB for a field most
/// logs do not have. The accessors below keep the sentinel out of calling code.
pub const NO_END: Micros = Micros::MIN;

/// Marks an absent resource, for the same reason.
pub const NO_RESOURCE: u32 = u32::MAX;

/// Marks a case that had no value for an attribute.
pub const NO_ATTRIBUTE: u32 = u32::MAX;

/// Fixed-point cost scale. One stored unit is one millionth of the source
/// currency unit, preserving ordinary decimal exports without floating-point
/// summation drift.
pub const COST_SCALE: i64 = 1_000_000;

/// One event in a normalized log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub activity: ActivityId,
    /// When it happened. With an end timestamp mapped, when it *started*.
    pub at: Micros,
    /// When it finished, or [`NO_END`]. Read through [`Event::end`].
    pub end_at: Micros,
    /// Who performed it, or [`NO_RESOURCE`]. Read through [`Event::resource`].
    pub resource: u32,
    /// 1-based row in the source file, matching what a spreadsheet shows.
    /// This is both the final ordering tie-breaker and the evidence anchor.
    pub source_row: u32,
}

impl Event {
    /// A minimal event with no end timestamp and no resource.
    pub fn new(activity: ActivityId, at: Micros, source_row: u32) -> Self {
        Self {
            activity,
            at,
            end_at: NO_END,
            resource: NO_RESOURCE,
            source_row,
        }
    }

    /// When the event finished, if an end timestamp was mapped.
    pub fn end(&self) -> Option<Micros> {
        (self.end_at != NO_END).then_some(self.end_at)
    }

    /// Who performed the event, if a resource was mapped.
    pub fn resource(&self) -> Option<ResourceId> {
        (self.resource != NO_RESOURCE).then_some(ResourceId(self.resource))
    }

    /// How long the activity itself took, if an end timestamp was mapped.
    ///
    /// This is *processing* time. The gap before the next event starting is
    /// *waiting* time, and that is usually where process loss actually is.
    pub fn processing_time(&self) -> Option<Micros> {
        self.end().map(|end| end - self.at)
    }
}

/// One step's value for a column that changes between steps, where it
/// differs from the case's own value. Sixteen bytes; see
/// [`EventLog::step_values`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepValue {
    pub case: CaseId,
    /// The step's source row, as in [`Event::source_row`].
    pub row: u32,
    /// Position in [`EventLog::attribute_names`].
    pub attribute: u32,
    /// Interned value.
    pub value: u32,
}

/// One case: its events, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseTrace {
    pub case: CaseId,
    /// Ordered events. Always at least one.
    pub events: Vec<Event>,
    /// Set when the log had missing data for this case, so duration
    /// statistics would understate it.
    pub incomplete: bool,
    /// Interned attribute values, positionally matching
    /// [`EventLog::attribute_names`]. [`NO_ATTRIBUTE`] means the case had no
    /// value.
    ///
    /// Stored **per case, not per event**: a real export has dozens of columns
    /// like region, customer, and priority, and those are properties of the
    /// case. Holding them once per case rather than once per event is the
    /// difference between a few megabytes and a few hundred.
    pub attributes: Vec<u32>,
}

impl CaseTrace {
    /// The interned value of the attribute at `index`, if the case had one.
    pub fn attribute(&self, index: usize) -> Option<u32> {
        self.attributes
            .get(index)
            .copied()
            .filter(|v| *v != NO_ATTRIBUTE)
    }

    /// The activity sequence, without timings.
    pub fn trace(&self) -> Trace {
        Trace::new(self.case, self.events.iter().map(|e| e.activity).collect())
    }

    /// First event timestamp.
    pub fn start(&self) -> Micros {
        self.events.first().map_or(0, |e| e.at)
    }

    /// Last event timestamp.
    pub fn end(&self) -> Micros {
        self.events.last().map_or(0, |e| e.at)
    }

    /// Elapsed time from first to last event.
    ///
    /// Zero for a single-event case, which is correct rather than an error:
    /// nothing elapsed because nothing followed.
    pub fn throughput(&self) -> Micros {
        self.end() - self.start()
    }

    /// Number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the case has no events. A normalized case never does.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// A complete normalized event log.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventLog {
    /// Case label dictionary.
    pub cases: Dictionary,
    /// Activity label dictionary.
    pub activities: Dictionary,
    /// Resource label dictionary. Empty when no resource column was mapped.
    pub resources: Dictionary,
    /// Names of the case attributes carried through, in a fixed order.
    ///
    /// A file may have any number of columns beyond the three required ones.
    /// Those become case attributes, available for filtering and cohort
    /// comparison rather than being discarded.
    pub attribute_names: Vec<String>,
    /// Shared dictionary of attribute values.
    pub attribute_values: Dictionary,
    /// Event costs keyed by `(case, source row)`. Empty when no cost column was
    /// mapped. A sparse sidecar keeps the common no-cost event representation
    /// compact.
    pub costs: BTreeMap<(CaseId, u32), i64>,
    /// Step-level attribute values keyed by `(case, source row)`: each entry
    /// lists `(attribute index, interned value)` where that step's row gave a
    /// different value than the case's own (its first non-blank one).
    ///
    /// A column that is the same on every row of a case (vendor, region)
    /// lives once per case in [`CaseTrace::attributes`]. A column that changes
    /// between steps (status, owner) keeps its other values here, so nothing
    /// is lost and the common case stays compact. Empty for most logs.
    ///
    /// Held as one flat list sorted by case, row and attribute: 16 bytes per
    /// change, read by binary search.
    pub step_values: Vec<StepValue>,
    /// Cases, in first-seen order. Each has ordered events.
    pub traces: Vec<CaseTrace>,
}

impl EventLog {
    /// Total events across all cases.
    pub fn event_count(&self) -> usize {
        self.traces.iter().map(CaseTrace::len).sum()
    }

    /// Number of cases.
    pub fn case_count(&self) -> usize {
        self.traces.len()
    }

    /// Number of distinct activities.
    pub fn activity_count(&self) -> usize {
        self.activities.len()
    }

    /// The display label for an activity.
    pub fn activity_label(&self, id: ActivityId) -> &str {
        self.activities.label(id.0).unwrap_or("<unknown>")
    }

    /// The display label for a case.
    pub fn case_label(&self, id: CaseId) -> &str {
        self.cases.label(id.0).unwrap_or("<unknown>")
    }

    /// The display label for a resource.
    pub fn resource_label(&self, id: ResourceId) -> &str {
        self.resources.label(id.0).unwrap_or("<unknown>")
    }

    /// Whether a resource column was mapped.
    pub fn has_resources(&self) -> bool {
        !self.resources.is_empty()
    }

    /// Fixed-point cost for an event, if one was recorded.
    pub fn event_cost(&self, case: CaseId, event: &Event) -> Option<i64> {
        self.costs.get(&(case, event.source_row)).copied()
    }

    /// Whether a cost column contributed any numeric values.
    pub fn has_costs(&self) -> bool {
        !self.costs.is_empty()
    }

    /// The position of a named attribute.
    pub fn attribute_index(&self, name: &str) -> Option<usize> {
        self.attribute_names.iter().position(|n| n == name)
    }

    /// The value of `name` for a case, as text.
    pub fn attribute_value(&self, trace: &CaseTrace, name: &str) -> Option<&str> {
        let index = self.attribute_index(name)?;
        let value = trace.attribute(index)?;
        self.attribute_values.label(value)
    }

    /// The changed values recorded for one case, in row order.
    fn case_step_values(&self, case: CaseId) -> &[StepValue] {
        let from = self.step_values.partition_point(|v| v.case < case);
        let to = self.step_values.partition_point(|v| v.case <= case);
        &self.step_values[from..to]
    }

    /// The value of the attribute at `index` on one step: the step's own
    /// value where it differed from the case's, otherwise the case's.
    pub fn step_attribute(&self, case: &CaseTrace, event: &Event, index: usize) -> Option<u32> {
        if !self.step_values.is_empty() {
            let key = (case.case, event.source_row, index as u32);
            if let Ok(at) = self
                .step_values
                .binary_search_by(|v| (v.case, v.row, v.attribute).cmp(&key))
            {
                return Some(self.step_values[at].value);
            }
        }
        case.attribute(index)
    }

    /// The attribute's value on the case's last step: where a status ended
    /// up, or an amount after its last revision. A blank cell on a step reads
    /// as the case's own value, as it does on every step.
    pub fn latest_value(&self, case: &CaseTrace, index: usize) -> Option<u32> {
        match case.events.last() {
            Some(last) if !self.step_values.is_empty() => self.step_attribute(case, last, index),
            _ => case.attribute(index),
        }
    }

    /// Every distinct value the attribute at `index` took on any step of a
    /// case, the case's own value first. One value for a column that never
    /// changes within the case.
    pub fn case_values(&self, case: &CaseTrace, index: usize) -> Vec<u32> {
        let mut values: Vec<u32> = case.attribute(index).into_iter().collect();
        for changed in self.case_step_values(case.case) {
            if changed.attribute as usize == index && !values.contains(&changed.value) {
                values.push(changed.value);
            }
        }
        values
    }

    /// Whether a case ever had `value` for the attribute at `index`, on any
    /// step. Filters on a step-level column read it this way: "items that
    /// were ever in status Blocked".
    pub fn case_had(&self, case: &CaseTrace, index: usize, value: u32) -> bool {
        case.attribute(index) == Some(value)
            || self
                .case_step_values(case.case)
                .iter()
                .any(|v| v.attribute as usize == index && v.value == value)
    }

    /// Positions of the attributes whose value changes between steps of at
    /// least one case.
    pub fn step_level_attributes(&self) -> std::collections::BTreeSet<usize> {
        self.step_values
            .iter()
            .map(|v| v.attribute as usize)
            .collect()
    }

    /// Whether the attribute at `index` changes between steps somewhere.
    pub fn is_step_level(&self, index: usize) -> bool {
        self.step_values
            .iter()
            .any(|v| v.attribute as usize == index)
    }

    /// Distinct values a case attribute takes, in sorted order.
    ///
    /// The basis of cohort comparison: split cases by region, priority, or
    /// customer and compare the resulting processes.
    pub fn attribute_options(&self, name: &str) -> Vec<&str> {
        let Some(index) = self.attribute_index(name) else {
            return Vec::new();
        };
        let mut seen: std::collections::BTreeSet<&str> = Default::default();
        for trace in &self.traces {
            if let Some(v) = trace.attribute(index)
                && let Some(label) = self.attribute_values.label(v)
            {
                seen.insert(label);
            }
        }
        // Values a step-level column took on later steps are options too.
        for changed in &self.step_values {
            if changed.attribute as usize == index
                && let Some(label) = self.attribute_values.label(changed.value)
            {
                seen.insert(label);
            }
        }
        seen.into_iter().collect()
    }

    /// Whether end timestamps were mapped, enabling processing-time analysis.
    pub fn has_end_timestamps(&self) -> bool {
        self.traces
            .iter()
            .any(|t| t.events.iter().any(|e| e.end().is_some()))
    }

    /// Cases with exactly one event.
    ///
    /// These contribute an activity occurrence but no transition, and have
    /// zero throughput. A common real shape, not an error.
    pub fn single_event_cases(&self) -> usize {
        self.traces.iter().filter(|t| t.len() == 1).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Micros = 3_600_000_000;

    fn log() -> EventLog {
        let mut activities = Dictionary::new();
        let mut cases = Dictionary::new();
        let a = ActivityId(activities.intern("Receive"));
        let b = ActivityId(activities.intern("Ship"));
        let c1 = CaseId(cases.intern("C1"));
        let c2 = CaseId(cases.intern("C2"));
        EventLog {
            cases,
            activities,
            resources: Dictionary::new(),
            attribute_names: Vec::new(),
            attribute_values: Dictionary::new(),
            costs: BTreeMap::new(),
            step_values: Vec::new(),
            traces: vec![
                CaseTrace {
                    case: c1,
                    events: vec![Event::new(a, 0, 2), Event::new(b, 3 * HOUR, 3)],
                    incomplete: false,
                    attributes: Vec::new(),
                },
                CaseTrace {
                    case: c2,
                    events: vec![Event::new(a, HOUR, 4)],
                    incomplete: false,
                    attributes: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn counts_events_cases_and_activities() {
        let l = log();
        assert_eq!(l.event_count(), 3);
        assert_eq!(l.case_count(), 2);
        assert_eq!(l.activity_count(), 2);
    }

    #[test]
    fn computes_throughput() {
        let l = log();
        assert_eq!(l.traces[0].throughput(), 3 * HOUR);
    }

    /// A single-event case has zero throughput, not an error and not a null.
    #[test]
    fn single_event_case_has_zero_throughput() {
        let l = log();
        assert_eq!(l.traces[1].throughput(), 0);
        assert_eq!(l.single_event_cases(), 1);
    }

    #[test]
    fn exposes_labels_for_display() {
        let l = log();
        assert_eq!(l.activity_label(l.traces[0].events[0].activity), "Receive");
        assert_eq!(l.case_label(l.traces[0].case), "C1");
    }

    #[test]
    fn absent_optional_fields_read_as_none() {
        let e = Event::new(ActivityId(0), 0, 2);
        assert_eq!(e.end(), None);
        assert_eq!(e.resource(), None);
        assert_eq!(e.processing_time(), None);
    }

    #[test]
    fn computes_processing_time_when_an_end_timestamp_exists() {
        let mut e = Event::new(ActivityId(0), 0, 2);
        e.end_at = 2 * HOUR;
        assert_eq!(e.processing_time(), Some(2 * HOUR));
        assert_eq!(e.end(), Some(2 * HOUR));
    }

    #[test]
    fn reports_whether_optional_columns_were_mapped() {
        let l = log();
        assert!(!l.has_resources());
        assert!(!l.has_end_timestamps());
    }

    /// The sentinels exist to keep events small; if this grows, ten-million
    /// event logs get materially more expensive.
    #[test]
    fn an_event_stays_compact() {
        assert!(
            std::mem::size_of::<Event>() <= 32,
            "Event grew to {} bytes",
            std::mem::size_of::<Event>()
        );
    }

    /// A changed step value costs 16 bytes; a status column on a large file
    /// can have one per row.
    #[test]
    fn a_step_value_stays_compact() {
        assert_eq!(std::mem::size_of::<StepValue>(), 16);
    }

    #[test]
    fn carries_case_attributes() {
        let mut l = log();
        let mut values = Dictionary::new();
        let eu = values.intern("EU");
        let us = values.intern("US");
        l.attribute_names = vec!["region".to_owned()];
        l.attribute_values = values;
        l.traces[0].attributes = vec![eu];
        l.traces[1].attributes = vec![us];

        assert_eq!(l.attribute_value(&l.traces[0], "region"), Some("EU"));
        assert_eq!(l.attribute_options("region"), vec!["EU", "US"]);
    }

    #[test]
    fn a_case_without_a_value_reports_none() {
        let mut l = log();
        l.attribute_names = vec!["region".to_owned()];
        l.traces[0].attributes = vec![NO_ATTRIBUTE];
        assert_eq!(l.traces[0].attribute(0), None);
        assert_eq!(l.attribute_value(&l.traces[0], "region"), None);
    }

    #[test]
    fn an_unknown_attribute_is_not_an_error() {
        let l = log();
        assert_eq!(l.attribute_value(&l.traces[0], "nope"), None);
        assert!(l.attribute_options("nope").is_empty());
    }

    #[test]
    fn produces_an_untimed_trace() {
        let l = log();
        assert_eq!(l.traces[0].trace().len(), 2);
        assert_eq!(l.traces[0].trace().directly_follows().count(), 1);
    }
}
