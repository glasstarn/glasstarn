//! Normalization: raw rows plus an answered policy become an event log.
//!
//! This is the gate. Normalization **refuses to run** while any decision the
//! validation report raised is unanswered, so there is no code path from
//! ambiguous data to analysis results. "Never guess" is enforced here rather
//! than merely intended.

use crate::finding::{Kind, Policy, PolicyError, Report};
use crate::read::Row;
use crate::rules::Rules;
use crate::timestamp::{self, Format};
use crate::validate::{Mapping, label_counts, merge_map};
use pm_core::dict::{ActivityId, CaseId, Dictionary};
use pm_core::log::{
    COST_SCALE, CaseTrace, Event, EventLog, Micros, NO_ATTRIBUTE, NO_END, NO_RESOURCE,
};
use std::collections::BTreeMap;

/// Why normalization could not run or could not finish.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NormalizeError {
    #[error("cannot normalize until every decision is answered: {0:?}")]
    UnresolvedDecisions(Vec<PolicyError>),
    #[error("no timestamp format could be determined for column '{column}'")]
    NoFormat { column: String },
    #[error("column '{column}' is not present in the data")]
    MissingColumn { column: String },
}

/// What normalization did, beyond producing the log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// The timestamp format used.
    pub format: Option<String>,
    /// Rows dropped because they had no timestamp.
    pub dropped_blank: usize,
    /// Rows dropped as duplicates.
    pub dropped_duplicate: usize,
    /// Rows dropped because the timestamp could not be read.
    pub dropped_unparseable: usize,
    /// Cases flagged incomplete because data was missing.
    pub incomplete_cases: usize,
    /// Events reordered because file order disagreed with time order.
    pub reordered_events: usize,
    /// Distinct spellings folded into a canonical label, when the user asked
    /// for near-duplicate labels to be merged.
    pub merged_labels: usize,
    /// Events dropped because their activity was excluded.
    pub excluded_events: usize,
    /// Cases dropped by an exclusion, size, or completeness rule.
    pub excluded_cases: usize,
}

/// A normalized log together with a record of what was applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Normalized {
    pub log: EventLog,
    pub applied: Applied,
}

/// Ordering key for one event, applied in field order.
///
/// This encodes the documented ordering rule directly, so the priority is
/// visible rather than buried in a comparison function: an explicit order
/// column wins, then the timestamp, then lifecycle rank, then the original
/// file row — which guarantees a total order, so tied events never reorder
/// between runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SortKey {
    order: i64,
    at: Micros,
    lifecycle: u8,
    source_row: u32,
}

/// An event and the rest of its sort key: the explicit order and lifecycle
/// rank; the time and source row are the event's own.
struct Keyed {
    order: i64,
    lifecycle: u8,
    event: Event,
}

impl Keyed {
    fn key(&self) -> SortKey {
        SortKey {
            order: self.order,
            at: self.event.at,
            lifecycle: self.lifecycle,
            source_row: self.event.source_row,
        }
    }
}

/// Accumulates a normalized log across a stream of rows.
///
/// Holds only the encoded log, so peak memory tracks the *result* rather than
/// the source file. An early benchmark measured 603 MB for 833k events when
/// the whole file was materialized first; streaming removes that entirely.
pub struct Normalizer<'a> {
    mapping: &'a Mapping,
    format: Format,
    /// Set when no single format explains the column, so each value is read
    /// with whichever recognised format fits it.
    mixed_formats: bool,
    /// Day/month order selected by the user for ambiguous numeric dates.
    date_order: Option<timestamp::Order>,
    /// Whether offset-bearing timestamps are normalized to UTC. Otherwise
    /// their wall-clock spelling is preserved (`as_written`).
    normalize_offsets_to_utc: bool,
    drop_duplicates: bool,
    blanks_mean_missing: bool,

    applied: Applied,
    /// Renames, exclusions, and case rules stated by the user.
    rules: Rules,
    cases: Dictionary,
    activities: Dictionary,
    resources: Dictionary,
    attribute_values: Dictionary,
    /// First non-blank attribute value seen per case, `width` values per case
    /// in case order, in one flat list.
    case_attributes: Vec<u32>,
    /// Events of each case (indexed by case) with the parts of their sort key
    /// the event itself does not hold. The keys are dropped once ordering is
    /// done, so they never inflate the stored log.
    grouped: Vec<Vec<Keyed>>,
    /// Costs by (case, source row), recorded as events are read.
    costs: BTreeMap<(CaseId, u32), i64>,
    /// Attribute values a row gave that differ from its case's first value,
    /// by (case, source row): the step-level part of a column that changes.
    step_values: Vec<pm_core::StepValue>,
    /// Distinct step values seen per attribute, up to [`STEP_VALUE_LIMIT`].
    step_distinct: Vec<std::collections::HashSet<u32>>,
    /// Attributes past the limit: free text, kept once per case only.
    free_text: Vec<bool>,
    /// Keys already seen in cases large enough to need a set.
    large_cases:
        std::collections::HashMap<u32, std::collections::HashSet<(ActivityId, Micros, i64, u8)>>,
    incomplete: BTreeMap<u32, bool>,
}

impl<'a> Normalizer<'a> {
    /// Creates a normalizer using an already-chosen timestamp format.
    pub fn new(mapping: &'a Mapping, format: Format, policy: &Policy) -> Self {
        Self {
            mapping,
            applied: Applied {
                format: Some(format.id.to_string()),
                ..Default::default()
            },
            format,
            mixed_formats: false,
            date_order: match policy.get(&Kind::AmbiguousDateOrder) {
                Some("day_first") => Some(timestamp::Order::DayFirst),
                Some("month_first") => Some(timestamp::Order::MonthFirst),
                _ => None,
            },
            normalize_offsets_to_utc: policy.get(&Kind::MixedZoneAndNaive) != Some("as_written"),
            drop_duplicates: policy.get(&Kind::DuplicateRows) != Some("keep_as_rework"),
            blanks_mean_missing: policy.get(&Kind::BlankTimestamps) == Some("missing_data"),
            rules: Rules::new(),
            cases: Dictionary::new(),
            activities: Dictionary::new(),
            resources: Dictionary::new(),
            attribute_values: Dictionary::new(),
            case_attributes: Vec::new(),
            grouped: Vec::new(),
            costs: BTreeMap::new(),
            step_values: Vec::new(),
            step_distinct: Vec::new(),
            free_text: Vec::new(),
            large_cases: Default::default(),
            incomplete: BTreeMap::new(),
        }
    }

    /// Reads each value with whichever recognised format fits it.
    ///
    /// Used when the column genuinely mixes layouts.
    pub fn allowing_mixed_formats(mut self, mixed: bool) -> Self {
        self.mixed_formats = mixed;
        self
    }

    /// Applies user-stated preparation rules.
    pub fn with_rules(mut self, rules: Rules) -> Self {
        self.applied.merged_labels = rules.rename.len();
        self.rules = rules;
        self
    }

    /// Folds near-duplicate labels onto a canonical spelling.
    ///
    /// Applied only when the user has chosen to merge them; otherwise labels
    /// are used exactly as recorded.
    pub fn with_label_merges(self, merges: BTreeMap<String, String>) -> Self {
        let rules = self.rules.clone().with_automatic_merges(merges);
        self.with_rules(rules)
    }

    /// Observes one row.
    pub fn observe(&mut self, row: &Row) {
        let raw_case = row.get(&self.mapping.case_id);
        if self.rules.excludes_case(raw_case) {
            self.applied.excluded_events += 1;
            return;
        }

        let raw = row.get(&self.mapping.timestamp);
        let case = self.cases.intern(raw_case);
        let differing = self.record_attributes(case, row);

        if raw.trim().is_empty() {
            self.applied.dropped_blank += 1;
            if self.blanks_mean_missing {
                self.incomplete.insert(case, true);
            }
            // The case still exists; it simply has one fewer event. If it ends
            // up with no events at all it is dropped in `finish`, with a count.
            self.case_entry(case);
            return;
        }

        let Some(at) = self.parse_stamp(raw) else {
            self.applied.dropped_unparseable += 1;
            self.incomplete.insert(case, true);
            self.case_entry(case);
            return;
        };
        let raw_activity = row.get(&self.mapping.activity);
        let Some(label) = self.rules.resolve_activity(raw_activity) else {
            self.applied.excluded_events += 1;
            // The case survives; excluding one activity simply joins its
            // neighbours directly, which is the intended effect.
            self.case_entry(case);
            return;
        };
        let activity = self.activities.intern(label);

        // Lifecycle and explicit order distinguish events that legitimately
        // share a timestamp. Without them in the key, `Review/start` and
        // `Review/complete` would be collapsed into one event.
        let order = self.read_order(row);
        let lifecycle = self.read_lifecycle_rank(row);

        // Identical rows can only repeat within one case, so they are found
        // among that case's events rather than in a set over the whole file.
        if self.drop_duplicates && self.repeats(case, ActivityId(activity), at, order, lifecycle) {
            self.applied.dropped_duplicate += 1;
            return;
        }

        let source_row = u32::try_from(row.source_row).unwrap_or(u32::MAX);
        let event = Event {
            activity: ActivityId(activity),
            at,
            end_at: self.read_end(row, at),
            resource: self.read_resource(row),
            source_row,
        };

        // The documented ordering, in priority order: an explicit order
        // column, then the timestamp, then lifecycle rank so `start` precedes
        // `complete`, then the original file row as a total-order guarantee.
        if let Some(cost) = self.read_cost(row) {
            self.costs.insert((CaseId(case), source_row), cost);
        }
        self.step_values
            .extend(
                differing
                    .into_iter()
                    .map(|(attribute, value)| pm_core::StepValue {
                        case: CaseId(case),
                        row: source_row,
                        attribute,
                        value,
                    }),
            );
        self.case_entry(case).push(Keyed {
            order,
            lifecycle,
            event,
        });
    }

    /// Whether the case already has an event with this activity, time,
    /// order and lifecycle; records it otherwise. Small cases are scanned;
    /// a case that grows large gets its own set.
    fn repeats(
        &mut self,
        case: u32,
        activity: ActivityId,
        at: Micros,
        order: i64,
        lifecycle: u8,
    ) -> bool {
        const SCAN: usize = 64;
        let key = (activity, at, order, lifecycle);
        if let Some(seen) = self.large_cases.get_mut(&case) {
            return !seen.insert(key);
        }
        let events = self.case_entry(case);
        let found = events
            .iter()
            .any(|k| (k.event.activity, k.event.at, k.order, k.lifecycle) == key);
        if !found && events.len() + 1 >= SCAN {
            let mut seen = events
                .iter()
                .map(|k| (k.event.activity, k.event.at, k.order, k.lifecycle))
                .collect::<std::collections::HashSet<_>>();
            seen.insert(key);
            self.large_cases.insert(case, seen);
        }
        found
    }

    /// The events of `case`, creating its (possibly empty) entry.
    fn case_entry(&mut self, case: u32) -> &mut Vec<Keyed> {
        let at = case as usize;
        if self.grouped.len() <= at {
            self.grouped.resize_with(at + 1, Vec::new);
        }
        &mut self.grouped[at]
    }

    /// Records case attributes the first time each case is seen.
    ///
    /// The first non-blank value wins. A case attribute is by definition a
    /// property of the case, so a later differing value is a data-quality
    /// matter rather than a reason to change what was recorded.
    /// Records the case's first non-blank value for each attribute, and
    /// returns the `(attribute, value)` pairs where this row's value differs
    /// from it: those are kept on the step.
    fn record_attributes(&mut self, case: u32, row: &Row) -> Vec<(u32, u32)> {
        let mut differing = Vec::new();
        if self.mapping.case_attributes.is_empty() {
            return differing;
        }
        let width = self.mapping.case_attributes.len();
        let start = case as usize * width;
        if self.case_attributes.len() < start + width {
            self.case_attributes.resize(start + width, NO_ATTRIBUTE);
        }
        for (i, column) in self.mapping.case_attributes.iter().enumerate() {
            let raw = row.get(column).trim();
            if raw.is_empty() {
                continue;
            }
            let first = self.case_attributes[start + i];
            if first == NO_ATTRIBUTE {
                self.case_attributes[start + i] = self.attribute_values.intern(raw);
                continue;
            }
            // A value differing only in capitals or spaces is the same value
            // spelled differently, not a change; validation reports it.
            let same = self
                .attribute_values
                .label(first)
                .is_some_and(|first| same_spelling(first, raw));
            if same || self.free_text.get(i).copied().unwrap_or(false) {
                continue;
            }
            if self.step_distinct.len() < width {
                self.step_distinct.resize_with(width, Default::default);
                self.free_text.resize(width, false);
            }
            let value = self.attribute_values.intern(raw);
            self.step_distinct[i].insert(value);
            if self.step_distinct[i].len() > STEP_VALUE_LIMIT {
                // Free text (comments, descriptions): too varied to keep on
                // every step. The case keeps its first value, as before.
                self.free_text[i] = true;
                self.step_distinct[i] = Default::default();
                continue;
            }
            differing.push((i as u32, value));
        }
        differing
    }

    /// Parses a timestamp, trying the chosen format first and falling back to
    /// any recognised format.
    ///
    /// A column that mixes layouts is real — a system changed its export
    /// format mid-history, or two extracts were concatenated. Refusing the
    /// whole file is unhelpful when every individual value is unambiguous on
    /// its own; what matters is that no value is *guessed*. The fallback is
    /// only enabled when detection found no single explaining format.
    fn parse_stamp(&self, raw: &str) -> Option<Micros> {
        if let Some(parsed) = self.parse_with(&self.format, raw) {
            return Some(parsed.and_utc().timestamp_micros());
        }
        if !self.mixed_formats {
            return None;
        }
        timestamp::all()
            .iter()
            .filter(|f| match self.date_order {
                Some(timestamp::Order::DayFirst) => !f.is_month_first(),
                Some(timestamp::Order::MonthFirst) => !f.is_day_first(),
                _ => true,
            })
            .find_map(|f| self.parse_with(f, raw))
            .map(|p| p.and_utc().timestamp_micros())
    }

    fn parse_with(&self, format: &Format, raw: &str) -> Option<chrono::NaiveDateTime> {
        if self.normalize_offsets_to_utc {
            format.parse(raw)
        } else {
            format.parse_wall_time(raw)
        }
    }

    /// Reads the end timestamp, ignoring one that precedes the start.
    fn read_end(&self, row: &Row, start: Micros) -> Micros {
        let Some(column) = &self.mapping.end_timestamp else {
            return NO_END;
        };
        let raw = row.get(column);
        if raw.trim().is_empty() {
            return NO_END;
        }
        match self.parse_stamp(raw) {
            // An end before its start is bad data, not a negative duration.
            // Dropping it keeps every reported duration non-negative.
            Some(end) => {
                if end < start {
                    NO_END
                } else {
                    end
                }
            }
            None => NO_END,
        }
    }

    fn read_resource(&mut self, row: &Row) -> u32 {
        let Some(column) = &self.mapping.resource else {
            return NO_RESOURCE;
        };
        let raw = row.get(column).trim();
        if raw.is_empty() {
            return NO_RESOURCE;
        }
        self.resources.intern(raw)
    }

    fn read_cost(&self, row: &Row) -> Option<i64> {
        let column = self.mapping.cost.as_ref()?;
        parse_cost(row.get(column))
    }

    /// An explicit order value, when an order column is mapped.
    fn read_order(&self, row: &Row) -> i64 {
        self.mapping
            .event_order
            .as_ref()
            .map(|c| row.get(c).trim().parse::<i64>().unwrap_or(i64::MAX))
            .unwrap_or(0)
    }

    /// Rank for a lifecycle value: `start` sorts before `complete`.
    ///
    /// Unknown values sort between them rather than being reordered
    /// arbitrarily.
    fn read_lifecycle_rank(&self, row: &Row) -> u8 {
        let Some(column) = &self.mapping.lifecycle else {
            return 1;
        };
        match row.get(column).trim().to_lowercase().as_str() {
            "start" | "started" | "begin" | "assign" | "schedule" => 0,
            "complete" | "completed" | "end" | "finish" | "finished" => 2,
            _ => 1,
        }
    }

    /// Produces the normalized log.
    pub fn finish(mut self) -> Normalized {
        let mut traces = Vec::with_capacity(self.grouped.len());
        let mut costs = std::mem::take(&mut self.costs);
        let mut excluded = std::collections::BTreeSet::new();
        let width = self.mapping.case_attributes.len();
        for (case, mut events) in std::mem::take(&mut self.grouped).into_iter().enumerate() {
            let case = case as u32;
            let is_incomplete = self.incomplete.get(&case).copied().unwrap_or(false);
            if !self.rules.keeps_case(events.len(), is_incomplete) {
                // Either every event was dropped — emitting an empty case
                // would break the "a case always has an event" invariant — or
                // a size or completeness rule excluded it.
                excluded.insert(CaseId(case));
                if !events.is_empty() {
                    self.applied.excluded_cases += 1;
                    self.applied.excluded_events += events.len();
                    for keyed in &events {
                        costs.remove(&(CaseId(case), keyed.event.source_row));
                    }
                }
                continue;
            }

            let before: Vec<u32> = events.iter().map(|k| k.event.source_row).collect();
            events.sort_by_key(Keyed::key);
            let events: Vec<Event> = events.into_iter().map(|keyed| keyed.event).collect();
            self.applied.reordered_events += before
                .iter()
                .zip(events.iter())
                .filter(|(a, b)| **a != b.source_row)
                .count();

            if is_incomplete {
                self.applied.incomplete_cases += 1;
            }
            traces.push(CaseTrace {
                case: CaseId(case),
                events,
                incomplete: is_incomplete,
                attributes: self
                    .case_attributes
                    .get(case as usize * width..(case as usize + 1) * width)
                    .map(<[u32]>::to_vec)
                    .unwrap_or_default(),
            });
        }

        Normalized {
            log: EventLog {
                cases: self.cases,
                activities: self.activities,
                resources: self.resources,
                attribute_names: self.mapping.case_attributes.clone(),
                attribute_values: self.attribute_values,
                costs,
                step_values: {
                    let free_text = std::mem::take(&mut self.free_text);
                    let mut step_values = std::mem::take(&mut self.step_values);
                    step_values.retain(|v| {
                        !excluded.contains(&v.case)
                            && !free_text
                                .get(v.attribute as usize)
                                .copied()
                                .unwrap_or(false)
                    });
                    // Sorted once, so every later lookup is a binary search.
                    step_values.sort_unstable();
                    step_values.shrink_to_fit();
                    step_values
                },
                traces,
            },
            applied: self.applied,
        }
    }
}

/// A column with more distinct changing values than this is free text
/// (comments, descriptions): each case keeps its first value only.
pub const STEP_VALUE_LIMIT: usize = 1_000;

/// Whether two values differ only in capitals or surrounding spaces.
pub fn same_spelling(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    a == b || a.to_lowercase() == b.to_lowercase()
}

/// Parses a decimal cost exactly into millionths of a currency unit.
fn parse_cost(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (negative, unsigned) = raw
        .strip_prefix('-')
        .map_or((false, raw), |rest| (true, rest));
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 6
    {
        return None;
    }
    let whole = whole.parse::<i64>().ok()?.checked_mul(COST_SCALE)?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>().ok()? * 10_i64.pow((6 - fraction.len()) as u32)
    };
    let value = whole.checked_add(fraction)?;
    Some(if negative { -value } else { value })
}

/// Checks a policy fully resolves a report, and picks the timestamp format.
///
/// This is the gate: without a complete policy there is no path to a
/// normalized log.
pub fn prepare(
    mapping: &Mapping,
    report: &Report,
    policy: &Policy,
    sample: &[String],
) -> Result<Format, NormalizeError> {
    let problems = policy.validate_against(report);
    if !problems.is_empty() {
        return Err(NormalizeError::UnresolvedDecisions(problems));
    }
    choose_format(mapping, policy, sample)
}

/// Normalizes rows already held in memory.
pub fn normalize(
    rows: &[Row],
    mapping: &Mapping,
    report: &Report,
    policy: &Policy,
) -> Result<Normalized, NormalizeError> {
    normalize_with(rows, mapping, report, policy, Rules::new())
}

/// Normalizes rows under explicit preparation rules.
pub fn normalize_with(
    rows: &[Row],
    mapping: &Mapping,
    report: &Report,
    policy: &Policy,
    rules: Rules,
) -> Result<Normalized, NormalizeError> {
    let sample: Vec<String> = rows
        .iter()
        .take(1_000)
        .map(|r| r.get(&mapping.timestamp).to_owned())
        .collect();
    let format = prepare(mapping, report, policy, &sample)?;
    let mut normalizer = Normalizer::new(mapping, format, policy)
        .allowing_mixed_formats(needs_mixed_formats(&sample))
        .with_rules(rules);
    if policy.get(&Kind::NearDuplicateLabels) == Some("merge") {
        normalizer = normalizer.with_label_merges(merge_map(&label_counts(rows, mapping)));
    }
    for row in rows {
        normalizer.observe(row);
    }
    Ok(normalizer.finish())
}

/// Picks the timestamp format, honouring an answered date-order decision.
fn choose_format(
    mapping: &Mapping,
    policy: &Policy,
    sample: &[String],
) -> Result<Format, NormalizeError> {
    let detection = timestamp::detect(sample);

    if let Some(ambiguity) = &detection.ambiguity {
        return match policy.get(&Kind::AmbiguousDateOrder) {
            Some("day_first") => Ok(ambiguity.day_first.clone()),
            Some("month_first") => Ok(ambiguity.month_first.clone()),
            // Unreachable in practice: the policy gate above already requires
            // an answer. Kept explicit rather than unwrapping into a panic.
            _ => Err(NormalizeError::NoFormat {
                column: mapping.timestamp.clone(),
            }),
        };
    }

    if let Some(single) = detection.candidates.first() {
        return Ok(single.clone());
    }

    // No single format explains the column, but several together might. The
    // most frequently applicable one leads; the rest are tried per value.
    if !detection.mixed.is_empty() {
        return Ok(detection.mixed[0].clone());
    }

    Err(NormalizeError::NoFormat {
        column: mapping.timestamp.clone(),
    })
}

/// Whether a column needs per-value format selection.
pub fn needs_mixed_formats(sample: &[String]) -> bool {
    let detection = timestamp::detect(sample);
    detection.candidates.is_empty() && !detection.mixed.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::validate;

    fn rows(data: &[(&str, &str, &str)]) -> Vec<Row> {
        let schema = std::rc::Rc::new(crate::read::Schema::new(vec![
            "case_id".to_owned(),
            "activity".to_owned(),
            "timestamp".to_owned(),
        ]));
        data.iter()
            .enumerate()
            .map(|(i, (c, a, t))| {
                Row::new(
                    std::rc::Rc::clone(&schema),
                    vec![(*c).to_owned(), (*a).to_owned(), (*t).to_owned()],
                    i + 2,
                )
            })
            .collect()
    }

    fn mapping() -> Mapping {
        Mapping::new("case_id", "activity", "timestamp")
    }

    fn run(data: &[(&str, &str, &str)], policy: Policy) -> Result<Normalized, NormalizeError> {
        let rows = rows(data);
        let report = validate(&rows, &mapping());
        normalize(&rows, &mapping(), &report, &policy)
    }

    const CLEAN: &[(&str, &str, &str)] = &[
        ("C1", "Receive", "2026-03-02 09:00:00"),
        ("C1", "Check", "2026-03-02 11:00:00"),
        ("C1", "Ship", "2026-03-03 15:00:00"),
        ("C2", "Receive", "2026-03-02 10:00:00"),
        ("C2", "Check", "2026-03-02 14:00:00"),
        ("C2", "Ship", "2026-03-04 17:00:00"),
    ];

    #[test]
    fn normalizes_a_clean_log() {
        let n = run(CLEAN, Policy::new()).expect("clean data needs no policy");
        assert_eq!(n.log.case_count(), 2);
        assert_eq!(n.log.event_count(), 6);
        assert_eq!(n.log.activity_count(), 3);
    }

    /// The central guarantee: unanswered decisions block normalization
    /// entirely. There is no path from ambiguous data to results.
    #[test]
    fn refuses_to_normalize_with_unanswered_decisions() {
        let err = run(
            &[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C1", "B", ""),
                ("C1", "C", "2026-03-02 11:00:00"),
            ],
            Policy::new(),
        )
        .expect_err("blank timestamps must block");
        assert!(matches!(err, NormalizeError::UnresolvedDecisions(_)));
    }

    #[test]
    fn proceeds_once_the_decision_is_answered() {
        let n = run(
            &[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C1", "B", ""),
                ("C1", "C", "2026-03-02 11:00:00"),
            ],
            Policy::new().answer(Kind::BlankTimestamps, "did_not_happen"),
        )
        .expect("answered policy proceeds");
        assert_eq!(n.log.event_count(), 2);
        assert_eq!(n.applied.dropped_blank, 1);
        assert!(!n.log.traces[0].incomplete);
    }

    /// The two blank-timestamp answers must actually behave differently, or
    /// asking the question was pointless.
    #[test]
    fn missing_data_flags_the_case_incomplete() {
        let n = run(
            &[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C1", "B", ""),
                ("C1", "C", "2026-03-02 11:00:00"),
            ],
            Policy::new().answer(Kind::BlankTimestamps, "missing_data"),
        )
        .unwrap();
        assert!(n.log.traces[0].incomplete);
        assert_eq!(n.applied.incomplete_cases, 1);
    }

    const DUPES: &[(&str, &str, &str)] = &[
        ("C1", "Create", "2026-02-02 09:00:00"),
        ("C1", "Approve", "2026-02-02 14:00:00"),
        ("C1", "Approve", "2026-02-02 14:00:00"),
        ("C1", "Send", "2026-02-03 09:00:00"),
    ];

    #[test]
    fn deduplicates_when_told_to() {
        let n = run(
            DUPES,
            Policy::new().answer(Kind::DuplicateRows, "deduplicate"),
        )
        .unwrap();
        assert_eq!(n.log.event_count(), 3);
        assert_eq!(n.applied.dropped_duplicate, 1);
    }

    #[test]
    fn keeps_duplicates_when_told_to() {
        let n = run(
            DUPES,
            Policy::new().answer(Kind::DuplicateRows, "keep_as_rework"),
        )
        .unwrap();
        assert_eq!(n.log.event_count(), 4);
        assert_eq!(n.applied.dropped_duplicate, 0);
        assert_eq!(n.log.traces[0].trace().self_loops(), 1);
    }

    /// Sorting by timestamp must survive scrambled file order.
    #[test]
    fn orders_events_by_timestamp_not_file_order() {
        let n = run(
            &[
                ("C1", "Pay", "2026-11-05 16:00:00"),
                ("C1", "Receive", "2026-11-02 09:00:00"),
                ("C1", "Approve", "2026-11-03 11:00:00"),
            ],
            Policy::new(),
        )
        .unwrap();
        let labels: Vec<&str> = n.log.traces[0]
            .events
            .iter()
            .map(|e| n.log.activity_label(e.activity))
            .collect();
        assert_eq!(labels, vec!["Receive", "Approve", "Pay"]);
        assert!(n.applied.reordered_events > 0);
    }

    /// Tied timestamps fall through to original file order, deterministically.
    #[test]
    fn breaks_timestamp_ties_by_file_order() {
        let n = run(
            &[
                ("C1", "Receive", "2026-08-03 09:00:00"),
                ("C1", "Validate", "2026-08-03 09:00:00"),
                ("C1", "Post", "2026-08-03 09:00:00"),
            ],
            Policy::new(),
        )
        .unwrap();
        let labels: Vec<&str> = n.log.traces[0]
            .events
            .iter()
            .map(|e| n.log.activity_label(e.activity))
            .collect();
        assert_eq!(labels, vec!["Receive", "Validate", "Post"]);
        assert_eq!(n.applied.reordered_events, 0);
    }

    #[test]
    fn applies_the_chosen_date_order() {
        let data = &[
            ("C1", "A", "05/01/2026 09:00"),
            ("C1", "B", "06/01/2026 10:00"),
        ];
        let day = run(
            data,
            Policy::new().answer(Kind::AmbiguousDateOrder, "day_first"),
        )
        .unwrap();
        let month = run(
            data,
            Policy::new().answer(Kind::AmbiguousDateOrder, "month_first"),
        )
        .unwrap();
        assert_ne!(
            day.log.traces[0].events[0].at, month.log.traces[0].events[0].at,
            "the two readings must produce different instants"
        );
        assert_eq!(
            day.applied.format.as_deref(),
            Some("slash_day_first_minutes")
        );
        assert_eq!(
            month.applied.format.as_deref(),
            Some("slash_month_first_minutes")
        );
    }

    #[test]
    fn mixed_layouts_still_apply_the_chosen_date_order() {
        let data = &[
            ("C1", "ISO", "2026-01-01 09:00:00"),
            ("C1", "Slash", "05/01/2026 10:00"),
        ];
        let day = run(
            data,
            Policy::new().answer(Kind::AmbiguousDateOrder, "day_first"),
        )
        .unwrap();
        let month = run(
            data,
            Policy::new().answer(Kind::AmbiguousDateOrder, "month_first"),
        )
        .unwrap();
        assert_ne!(
            day.log.traces[0].events[1].at,
            month.log.traces[0].events[1].at
        );
    }

    #[test]
    fn timezone_policy_changes_the_normalized_order() {
        let data = &[
            ("C1", "Offset", "2026-03-02T09:00:00+02:00"),
            ("C1", "Naive", "2026-03-02 08:00:00"),
        ];
        let utc = run(data, Policy::new().answer(Kind::MixedZoneAndNaive, "utc")).unwrap();
        let written = run(
            data,
            Policy::new().answer(Kind::MixedZoneAndNaive, "as_written"),
        )
        .unwrap();
        assert_eq!(labels(&utc, 0), vec!["Offset", "Naive"]);
        assert_eq!(labels(&written, 0), vec!["Naive", "Offset"]);
    }

    #[test]
    fn keeps_single_event_cases() {
        let n = run(
            &[
                ("C1", "Created", "2026-10-01 09:00:00"),
                ("C2", "Created", "2026-10-01 10:00:00"),
                ("C2", "Qualified", "2026-10-01 15:00:00"),
            ],
            Policy::new(),
        )
        .unwrap();
        assert_eq!(n.log.case_count(), 2);
        assert_eq!(n.log.single_event_cases(), 1);
    }

    /// A case whose every event was dropped must not appear as an empty case.
    #[test]
    fn drops_cases_left_with_no_events() {
        let n = run(
            &[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C2", "A", ""),
                ("C2", "B", ""),
            ],
            Policy::new().answer(Kind::BlankTimestamps, "did_not_happen"),
        )
        .unwrap();
        assert_eq!(n.log.case_count(), 1);
        assert!(n.log.traces.iter().all(|t| !t.is_empty()));
    }

    /// The default preserves the data exactly as recorded.
    #[test]
    fn keeps_near_duplicate_labels_distinct_by_default() {
        let n = run(
            &[
                ("C1", "Approve", "2026-04-06 09:00:00"),
                ("C1", "approve", "2026-04-06 11:00:00"),
                ("C1", "Approve ", "2026-04-06 14:00:00"),
                ("C1", "Publish", "2026-04-06 16:00:00"),
            ],
            Policy::new(),
        )
        .unwrap();
        assert_eq!(n.log.activity_count(), 4, "labels must not merge silently");
        assert_eq!(n.applied.merged_labels, 0);
    }

    /// And merging happens only when the user says so.
    #[test]
    fn merges_near_duplicate_labels_when_the_user_confirms() {
        let n = run(
            &[
                ("C1", "Approve", "2026-04-06 09:00:00"),
                ("C1", "approve", "2026-04-06 11:00:00"),
                ("C1", "Approve ", "2026-04-06 14:00:00"),
                ("C1", "Publish", "2026-04-06 16:00:00"),
            ],
            Policy::new().answer(Kind::NearDuplicateLabels, "merge"),
        )
        .unwrap();
        assert_eq!(n.log.activity_count(), 2, "Approve variants become one");
        assert_eq!(n.applied.merged_labels, 3);
    }

    /// The surviving spelling is the commonest one, not an arbitrary pick.
    #[test]
    fn merged_label_uses_the_commonest_spelling() {
        let n = run(
            &[
                ("C1", "approve", "2026-04-06 09:00:00"),
                ("C2", "approve", "2026-04-06 10:00:00"),
                ("C3", "approve", "2026-04-06 11:00:00"),
                ("C4", "Approve", "2026-04-06 12:00:00"),
            ],
            Policy::new().answer(Kind::NearDuplicateLabels, "merge"),
        )
        .unwrap();
        assert_eq!(n.log.activities.labels(), ["approve"]);
    }

    /// Merging must not touch genuinely different activities.
    #[test]
    fn merging_leaves_distinct_activities_alone() {
        let n = run(
            &[
                ("C1", "Approve", "2026-04-06 09:00:00"),
                ("C1", "Reject", "2026-04-06 11:00:00"),
            ],
            Policy::new().answer(Kind::NearDuplicateLabels, "merge"),
        )
        .unwrap();
        assert_eq!(n.log.activity_count(), 2);
        assert_eq!(n.applied.merged_labels, 0);
    }

    fn run_with(data: &[(&str, &str, &str)], rules: Rules) -> Result<Normalized, NormalizeError> {
        let rows = rows(data);
        let report = validate(&rows, &mapping());
        normalize_with(&rows, &mapping(), &report, &Policy::new(), rules)
    }

    const CODED: &[(&str, &str, &str)] = &[
        ("C1", "STAT_01", "2026-05-01 09:00:00"),
        ("C1", "SYS_PING", "2026-05-01 09:30:00"),
        ("C1", "STAT_03", "2026-05-01 11:00:00"),
        ("C1", "Approved", "2026-05-02 09:00:00"),
        ("C2", "STAT_01", "2026-05-01 10:00:00"),
        ("C2", "STAT_03", "2026-05-01 14:00:00"),
        ("C2", "APPROVAL_GRANTED", "2026-05-03 09:00:00"),
    ];

    #[test]
    fn renames_coded_activities_to_readable_names() {
        let n = run_with(
            CODED,
            Rules::new()
                .rename("STAT_01", "Receive")
                .rename("STAT_03", "Credit Check"),
        )
        .unwrap();
        assert!(n.log.activities.labels().contains(&"Credit Check"));
        assert!(!n.log.activities.labels().contains(&"STAT_03"));
    }

    #[test]
    fn merges_spellings_no_automatic_rule_could_catch() {
        let n = run_with(
            CODED,
            Rules::new().merge("Approve", &["Approved", "APPROVAL_GRANTED"]),
        )
        .unwrap();
        let labels = n.log.activities.labels();
        assert!(labels.contains(&"Approve"));
        assert!(!labels.contains(&"Approved"));
        assert!(!labels.contains(&"APPROVAL_GRANTED"));
    }

    /// Excluding an activity must join its neighbours directly, not split the
    /// case in two.
    #[test]
    fn excluding_an_activity_joins_its_neighbours() {
        let n = run_with(CODED, Rules::new().exclude_activity("SYS_PING")).unwrap();
        assert_eq!(n.applied.excluded_events, 1);
        assert_eq!(n.log.case_count(), 2, "the case survives");
        let c1: Vec<&str> = n.log.traces[0]
            .events
            .iter()
            .map(|e| n.log.activity_label(e.activity))
            .collect();
        assert_eq!(c1, vec!["STAT_01", "STAT_03", "Approved"]);
    }

    #[test]
    fn excludes_a_named_case() {
        let n = run_with(CODED, Rules::new().exclude_case("C2")).unwrap();
        assert_eq!(n.log.case_count(), 1);
        assert_eq!(n.log.case_label(n.log.traces[0].case), "C1");
    }

    #[test]
    fn drops_cases_below_the_minimum_length() {
        let n = run_with(
            &[
                ("C1", "A", "2026-05-01 09:00:00"),
                ("C1", "B", "2026-05-01 10:00:00"),
                ("C1", "C", "2026-05-01 11:00:00"),
                ("C2", "A", "2026-05-01 09:00:00"),
            ],
            Rules::new().min_events_per_case(2),
        )
        .unwrap();
        assert_eq!(n.log.case_count(), 1);
        assert_eq!(n.applied.excluded_cases, 1);
        assert_eq!(n.applied.excluded_events, 1);
    }

    /// A case whose every activity is excluded must disappear rather than
    /// appear empty.
    #[test]
    fn a_case_stripped_of_every_activity_disappears() {
        let n = run_with(
            &[
                ("C1", "Noise", "2026-05-01 09:00:00"),
                ("C2", "Real", "2026-05-01 10:00:00"),
            ],
            Rules::new().exclude_activity("Noise"),
        )
        .unwrap();
        assert_eq!(n.log.case_count(), 1);
        assert!(n.log.traces.iter().all(|t| !t.is_empty()));
    }

    #[test]
    fn rules_are_deterministic() {
        let rules = || Rules::new().merge("Approve", &["Approved", "APPROVAL_GRANTED"]);
        assert_eq!(
            run_with(CODED, rules()).unwrap(),
            run_with(CODED, rules()).unwrap()
        );
    }

    #[test]
    fn empty_rules_match_plain_normalization() {
        let rows = rows(CODED);
        let report = validate(&rows, &mapping());
        let plain = normalize(&rows, &mapping(), &report, &Policy::new()).unwrap();
        let ruled = run_with(CODED, Rules::new()).unwrap();
        assert_eq!(plain.log, ruled.log);
    }

    fn rows_with(headers: &[&str], data: &[&[&str]]) -> Vec<Row> {
        let schema = std::rc::Rc::new(crate::read::Schema::new(
            headers.iter().map(|h| (*h).to_owned()).collect(),
        ));
        data.iter()
            .enumerate()
            .map(|(i, values)| {
                Row::new(
                    std::rc::Rc::clone(&schema),
                    values.iter().map(|v| (*v).to_owned()).collect(),
                    i + 2,
                )
            })
            .collect()
    }

    fn normalize_rows(rows: &[Row], mapping: &Mapping) -> Normalized {
        let report = validate(rows, mapping);
        normalize(rows, mapping, &report, &Policy::new()).unwrap()
    }

    fn labels(n: &Normalized, case: usize) -> Vec<&str> {
        n.log.traces[case]
            .events
            .iter()
            .map(|e| n.log.activity_label(e.activity))
            .collect()
    }

    /// An explicit order column beats the timestamp — a system that records
    /// one usually knows better than its own clock resolution.
    #[test]
    fn an_explicit_order_column_is_authoritative() {
        let rows = rows_with(
            &["case_id", "activity", "timestamp", "seq"],
            &[
                &["C1", "Third", "2026-03-02 09:00:00", "3"],
                &["C1", "First", "2026-03-02 09:00:00", "1"],
                &["C1", "Second", "2026-03-02 09:00:00", "2"],
            ],
        );
        let mapping = Mapping::new("case_id", "activity", "timestamp").with_event_order("seq");
        let n = normalize_rows(&rows, &mapping);
        assert_eq!(labels(&n, 0), vec!["First", "Second", "Third"]);
    }

    /// Without it, tied timestamps fall through to file order — which here is
    /// the wrong answer, and exactly why an order column matters.
    #[test]
    fn without_an_order_column_ties_fall_back_to_file_order() {
        let rows = rows_with(
            &["case_id", "activity", "timestamp", "seq"],
            &[
                &["C1", "Third", "2026-03-02 09:00:00", "3"],
                &["C1", "First", "2026-03-02 09:00:00", "1"],
                &["C1", "Second", "2026-03-02 09:00:00", "2"],
            ],
        );
        let mapping = Mapping::new("case_id", "activity", "timestamp");
        let n = normalize_rows(&rows, &mapping);
        assert_eq!(labels(&n, 0), vec!["Third", "First", "Second"]);
    }

    /// Lifecycle breaks ties so a start precedes its own completion.
    #[test]
    fn lifecycle_orders_start_before_complete() {
        let rows = rows_with(
            &["case_id", "activity", "timestamp", "lifecycle"],
            &[
                &["C1", "Review", "2026-03-02 09:00:00", "complete"],
                &["C1", "Review", "2026-03-02 09:00:00", "start"],
            ],
        );
        let mapping = Mapping::new("case_id", "activity", "timestamp").with_lifecycle("lifecycle");
        let n = normalize_rows(&rows, &mapping);
        assert_eq!(n.log.traces[0].events[0].source_row, 3, "the start row");
    }

    #[test]
    fn captures_resources_when_mapped() {
        let rows = rows_with(
            &["case_id", "activity", "timestamp", "who"],
            &[
                &["C1", "Review", "2026-03-02 09:00:00", "alice"],
                &["C1", "Approve", "2026-03-02 11:00:00", "bob"],
            ],
        );
        let mapping = Mapping::new("case_id", "activity", "timestamp").with_resource("who");
        let n = normalize_rows(&rows, &mapping);
        assert!(n.log.has_resources());
        let first = n.log.traces[0].events[0].resource().unwrap();
        assert_eq!(n.log.resource_label(first), "alice");
    }

    #[test]
    fn captures_decimal_costs_without_growing_every_event() {
        let rows = rows_with(
            &["case_id", "activity", "timestamp", "amount"],
            &[
                &["C1", "Review", "2026-03-02 09:00:00", "12.50"],
                &["C1", "Approve", "2026-03-02 11:00:00", "0.125"],
            ],
        );
        let mapping = Mapping::new("case_id", "activity", "timestamp").with_cost("amount");
        let n = normalize_rows(&rows, &mapping);
        let trace = &n.log.traces[0];
        assert_eq!(
            n.log.event_cost(trace.case, &trace.events[0]),
            Some(12_500_000)
        );
        assert_eq!(
            n.log.event_cost(trace.case, &trace.events[1]),
            Some(125_000)
        );
    }

    #[test]
    fn captures_end_timestamps_when_mapped() {
        let rows = rows_with(
            &["case_id", "activity", "start", "finish"],
            &[&["C1", "Review", "2026-03-02 09:00:00", "2026-03-02 11:00:00"]],
        );
        let mapping = Mapping::new("case_id", "activity", "start").with_end_timestamp("finish");
        let n = normalize_rows(&rows, &mapping);
        assert!(n.log.has_end_timestamps());
        assert_eq!(
            n.log.traces[0].events[0].processing_time(),
            Some(2 * 3_600_000_000)
        );
    }

    /// An end before its start is bad data, not a negative duration.
    #[test]
    fn ignores_an_end_timestamp_that_precedes_its_start() {
        let rows = rows_with(
            &["case_id", "activity", "start", "finish"],
            &[&["C1", "Review", "2026-03-02 11:00:00", "2026-03-02 09:00:00"]],
        );
        let mapping = Mapping::new("case_id", "activity", "start").with_end_timestamp("finish");
        let n = normalize_rows(&rows, &mapping);
        assert_eq!(n.log.traces[0].events[0].processing_time(), None);
    }

    #[test]
    fn a_blank_optional_value_is_simply_absent() {
        let rows = rows_with(
            &["case_id", "activity", "timestamp", "who"],
            &[
                &["C1", "Review", "2026-03-02 09:00:00", ""],
                &["C1", "Approve", "2026-03-02 11:00:00", "bob"],
            ],
        );
        let mapping = Mapping::new("case_id", "activity", "timestamp").with_resource("who");
        let n = normalize_rows(&rows, &mapping);
        assert_eq!(n.log.traces[0].events[0].resource(), None);
        assert!(n.log.traces[0].events[1].resource().is_some());
    }

    #[test]
    fn records_what_it_applied() {
        let n = run(CLEAN, Policy::new()).unwrap();
        assert_eq!(n.applied.format.as_deref(), Some("iso_space_seconds"));
    }

    #[test]
    fn is_deterministic() {
        let a = run(CLEAN, Policy::new()).unwrap();
        let b = run(CLEAN, Policy::new()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn preserves_evidence_row_anchors() {
        let n = run(CLEAN, Policy::new()).unwrap();
        assert_eq!(n.log.traces[0].events[0].source_row, 2);
    }
}
