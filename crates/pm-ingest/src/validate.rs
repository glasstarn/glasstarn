//! Validation of a mapped event log.
//!
//! Produces a [`Report`] describing everything worth knowing about the data,
//! including the decisions that must be answered before analysis can proceed.
//!
//! Validation never modifies data and never resolves an ambiguity on the
//! user's behalf.
//!
//! # Memory
//!
//! [`Validator`] observes one row at a time and keeps only **aggregates** —
//! counts, interned identifiers, and a handful of examples. Nothing grows with
//! file size except state bounded by case count and activity cardinality, so a
//! log far larger than memory can still be validated.

use crate::finding::{
    Finding, Kind, Report, blank_timestamp_decision, date_order_decision, duplicate_rows_decision,
    label_merge_decision, timezone_policy_decision,
};
use crate::read::Row;
use crate::timestamp::{self, Detection};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Which columns hold which fields.
///
/// Three are required — case, activity, timestamp. The rest are optional and
/// each unlocks something: an explicit order column removes reliance on file
/// order for tied timestamps, an end timestamp separates processing from
/// waiting time, and a resource column enables handover-of-work analysis.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Mapping {
    /// A projection of an object's history, which may span several business
    /// transactions. Repetition and first-to-last time are descriptive here,
    /// not evidence of process rework or a document's completion cycle.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub object_history: bool,
    pub case_id: String,
    pub activity: String,
    pub timestamp: String,
    /// Explicit event order. **Authoritative when present** — it beats the
    /// timestamp, because a system that records one usually knows better than
    /// its own clock resolution.
    pub event_order: Option<String>,
    /// When the activity finished, enabling processing-vs-waiting time.
    pub end_timestamp: Option<String>,
    /// Who performed the event.
    pub resource: Option<String>,
    /// Lifecycle stage, typically `start` or `complete`. Used to order events
    /// that share a timestamp.
    pub lifecycle: Option<String>,
    /// Numeric cost.
    pub cost: Option<String>,
    /// Columns carried through as case attributes.
    ///
    /// A real export has many more columns than the three required ones —
    /// region, customer, priority, product. Those are properties of the case
    /// and are kept for filtering and cohort comparison rather than discarded.
    pub case_attributes: Vec<String>,
}

impl Mapping {
    /// The three required fields.
    pub fn new(case_id: &str, activity: &str, timestamp: &str) -> Self {
        Self {
            case_id: case_id.to_owned(),
            activity: activity.to_owned(),
            timestamp: timestamp.to_owned(),
            ..Default::default()
        }
    }

    /// Maps an explicit order column.
    pub fn with_event_order(mut self, column: &str) -> Self {
        self.event_order = Some(column.to_owned());
        self
    }

    /// Maps an end-timestamp column.
    pub fn with_end_timestamp(mut self, column: &str) -> Self {
        self.end_timestamp = Some(column.to_owned());
        self
    }

    /// Maps a resource column.
    pub fn with_resource(mut self, column: &str) -> Self {
        self.resource = Some(column.to_owned());
        self
    }

    /// Maps a lifecycle column.
    pub fn with_lifecycle(mut self, column: &str) -> Self {
        self.lifecycle = Some(column.to_owned());
        self
    }

    /// Maps a cost column.
    pub fn with_cost(mut self, column: &str) -> Self {
        self.cost = Some(column.to_owned());
        self
    }

    /// Carries `columns` through as case attributes.
    pub fn with_case_attributes(mut self, columns: Vec<String>) -> Self {
        self.case_attributes = columns;
        self
    }

    /// Every column this mapping assigns to a field.
    pub fn mapped_columns(&self) -> Vec<&str> {
        let mut out = vec![
            self.case_id.as_str(),
            self.activity.as_str(),
            self.timestamp.as_str(),
        ];
        for c in [
            &self.event_order,
            &self.end_timestamp,
            &self.resource,
            &self.lifecycle,
            &self.cost,
        ]
        .into_iter()
        .flatten()
        {
            out.push(c.as_str());
        }
        out
    }
}

/// How many values to sample when detecting the timestamp format.
pub const SAMPLE_SIZE: usize = 500;

/// Above this many distinct activities, a process map stops being readable.
const CARDINALITY_LIMIT: usize = 50;

/// How many example values to keep per finding.
const EXAMPLES: usize = 5;

/// Accumulates validation state across a stream of rows.
pub struct Validator<'a> {
    mapping: &'a Mapping,
    detection: Detection,

    cases: pm_core::Dictionary,
    /// Fallback interner for timestamps only used when no single format
    /// explains the column. When a format is known, timestamps become integer
    /// keys and no strings are held at all — which at 8M events is the
    /// difference between 1.4 GB and a few hundred megabytes.
    stamps: HashMap<String, u32>,
    /// Activity labels interned to stable identifiers. Must be a real
    /// interner: deriving an id from a position in a growing sorted set would
    /// renumber existing activities as new ones arrive, which silently
    /// corrupts duplicate detection.
    activities: HashMap<String, u32>,
    /// How often each label occurs, so a merge can display the commonest
    /// spelling rather than an arbitrary one.
    label_counts: HashMap<String, usize>,

    /// (activity, stamp, stage) of each case's rows, indexed by case: repeated
    /// rows and shared times can only occur within one case.
    case_keys: Vec<Vec<(u32, i64, u32)>>,
    /// Sets for the few cases too large to scan.
    large_cases: HashMap<u32, LargeCase>,
    case_sizes: Vec<u32>,
    last_seen: Vec<Option<i64>>,

    blank_rows: usize,
    blank_cases: BTreeSet<u32>,
    blank_examples: Vec<String>,

    unparseable_rows: usize,

    duplicate_rows: usize,
    duplicate_cases: BTreeSet<u32>,
    duplicate_examples: Vec<String>,

    tied_events: usize,
    tied_cases: BTreeSet<u32>,

    out_of_order: usize,
    blank_case_rows: usize,
    blank_activity_rows: usize,
    blank_examples_required: Vec<String>,
    /// First non-blank value observed for each case attribute.
    /// First value of each case attribute, `width` per case, as names.
    case_attribute_values: Vec<u32>,
    attribute_values: pm_core::Dictionary,
    conflicting_attributes: BTreeSet<(u32, usize)>,
    conflicting_examples: Vec<String>,
    /// Distinct changing values per attribute, up to the free-text limit.
    changing_values: HashMap<usize, HashSet<u32>>,
    free_text: BTreeSet<usize>,
    /// (case, attribute) pairs whose values differ only in spelling.
    respelled: BTreeSet<(u32, usize)>,
    respelled_examples: Vec<String>,
}

impl<'a> Validator<'a> {
    /// Creates a validator for `mapping`, using `detection` from a sample.
    pub fn new(mapping: &'a Mapping, detection: Detection) -> Self {
        Self {
            mapping,
            detection,
            cases: pm_core::Dictionary::new(),
            stamps: HashMap::new(),
            activities: HashMap::new(),
            label_counts: HashMap::new(),
            case_keys: Vec::new(),
            large_cases: HashMap::new(),
            case_sizes: Vec::new(),
            last_seen: Vec::new(),
            blank_rows: 0,
            blank_cases: BTreeSet::new(),
            blank_examples: Vec::new(),
            unparseable_rows: 0,
            duplicate_rows: 0,
            duplicate_cases: BTreeSet::new(),
            duplicate_examples: Vec::new(),
            tied_events: 0,
            tied_cases: BTreeSet::new(),
            out_of_order: 0,
            blank_case_rows: 0,
            blank_activity_rows: 0,
            blank_examples_required: Vec::new(),
            case_attribute_values: Vec::new(),
            attribute_values: pm_core::Dictionary::new(),
            conflicting_attributes: BTreeSet::new(),
            conflicting_examples: Vec::new(),
            changing_values: HashMap::new(),
            free_text: BTreeSet::new(),
            respelled: BTreeSet::new(),
            respelled_examples: Vec::new(),
        }
    }

    /// Observes one row.
    pub fn observe(&mut self, row: &Row) {
        let raw_case = row.get(&self.mapping.case_id);
        let raw_activity = row.get(&self.mapping.activity);
        let raw_stamp = row.get(&self.mapping.timestamp);

        // A blank case id or activity is not a legitimate value — it would be
        // interned as an activity called "" and appear in the process map.
        if raw_case.trim().is_empty() || raw_activity.trim().is_empty() {
            if raw_case.trim().is_empty() {
                self.blank_case_rows += 1;
            }
            if raw_activity.trim().is_empty() {
                self.blank_activity_rows += 1;
            }
            if self.blank_examples_required.len() < EXAMPLES {
                self.blank_examples_required.push(format!(
                    "row {}: case {:?}, activity {:?}",
                    row.source_row, raw_case, raw_activity
                ));
            }
        }

        let case = self.cases.intern(raw_case);
        if self.case_sizes.len() <= case as usize {
            self.case_sizes.resize(case as usize + 1, 0);
            self.last_seen.resize(case as usize + 1, None);
            self.case_keys.resize_with(case as usize + 1, Vec::new);
        }
        self.case_sizes[case as usize] += 1;
        self.observe_case_attributes(case, raw_case, row);

        let activity = intern(&mut self.activities, raw_activity);
        *self
            .label_counts
            .entry(raw_activity.to_owned())
            .or_insert(0) += 1;

        if raw_stamp.trim().is_empty() {
            self.blank_rows += 1;
            self.blank_cases.insert(case);
            if self.blank_examples.len() < EXAMPLES {
                self.blank_examples
                    .push(format!("row {}: {}", row.source_row, raw_activity));
            }
            return;
        }

        let (stamp, parsed) = self.stamp_key(raw_stamp);

        // Lifecycle distinguishes two stages of one activity instance. Without
        // it in the key, `Review/start` and `Review/complete` at the same
        // timestamp would be reported as duplicates, and their ordering as an
        // unresolved tie — neither of which is true.
        let stage = self.stage_key(row);

        let (repeated, tied) = self.seen_in_case(case, activity, stamp, stage);
        if repeated {
            self.duplicate_rows += 1;
            self.duplicate_cases.insert(case);
            if self.duplicate_examples.len() < EXAMPLES {
                self.duplicate_examples.push(format!(
                    "row {}: {} / {} / {}",
                    row.source_row, raw_case, raw_activity, raw_stamp
                ));
            }
        }

        if tied {
            self.tied_events += 1;
            self.tied_cases.insert(case);
        }

        if let Some(at) = parsed {
            if let Some(previous) = self.last_seen[case as usize]
                && at < previous
            {
                self.out_of_order += 1;
            }
            self.last_seen[case as usize] = Some(at);
        } else if !timestamp::all()
            .iter()
            .any(|f| f.parse(raw_stamp).is_some())
        {
            self.unparseable_rows += 1;
        }
    }

    /// Whether the case already has a row with this activity, stamp and stage
    /// (a repeated row) and with this stamp and stage (a shared time), and
    /// records this row. Small cases are scanned; large ones get sets.
    fn seen_in_case(&mut self, case: u32, activity: u32, stamp: i64, stage: u32) -> (bool, bool) {
        const SCAN: usize = 64;
        if let Some((rows, times)) = self.large_cases.get_mut(&case) {
            return (
                !rows.insert((activity, stamp, stage)),
                !times.insert((stamp, stage)),
            );
        }
        let keys = &mut self.case_keys[case as usize];
        let repeated = keys.contains(&(activity, stamp, stage));
        let tied = keys.iter().any(|(_, s, g)| (*s, *g) == (stamp, stage));
        keys.push((activity, stamp, stage));
        if keys.len() >= SCAN {
            let keys = std::mem::take(keys);
            let times = keys.iter().map(|(_, s, g)| (*s, *g)).collect();
            self.large_cases
                .insert(case, (keys.into_iter().collect(), times));
        }
        (repeated, tied)
    }

    fn observe_case_attributes(&mut self, case: u32, case_label: &str, row: &Row) {
        let width = self.mapping.case_attributes.len();
        let start = case as usize * width;
        if self.case_attribute_values.len() < start + width {
            self.case_attribute_values.resize(start + width, u32::MAX);
        }
        for (index, column) in self.mapping.case_attributes.iter().enumerate() {
            let value = row.get(column).trim();
            if value.is_empty() {
                continue;
            }
            let key = (case, index);
            let stored = self.case_attribute_values[start + index];
            if stored == u32::MAX {
                self.case_attribute_values[start + index] = self.attribute_values.intern(value);
                continue;
            }
            let first = self.attribute_values.label(stored).unwrap_or_default();
            if first == value {
                continue;
            }
            let example = format!("case {case_label:?}, column {column:?}: {first:?} / {value:?}");
            if crate::normalize::same_spelling(first, value) {
                if self.respelled.insert(key) && self.respelled_examples.len() < EXAMPLES {
                    self.respelled_examples.push(example);
                }
                continue;
            }
            if !self.free_text.contains(&index) {
                let id = self.attribute_values.intern(value);
                let seen = self.changing_values.entry(index).or_default();
                seen.insert(id);
                if seen.len() > crate::normalize::STEP_VALUE_LIMIT {
                    self.free_text.insert(index);
                    self.changing_values.remove(&index);
                }
            }
            if self.conflicting_attributes.insert(key) && self.conflicting_examples.len() < EXAMPLES
            {
                self.conflicting_examples.push(example);
            }
        }
    }

    /// An exact comparison key for a timestamp, plus its parsed instant.
    ///
    /// When a single format explains the column, the key is the instant
    /// itself, so nothing is stored per row. Otherwise the raw text is
    /// interned and the key is offset into a disjoint negative range, so an
    /// interner id can never collide with a real instant.
    fn stamp_key(&mut self, raw: &str) -> (i64, Option<i64>) {
        if let Some(format) = self.single_format()
            && let Some(parsed) = format.parse(raw)
        {
            let at = parsed.and_utc().timestamp_micros();
            return (at, Some(at));
        }
        let id = intern(&mut self.stamps, raw);
        (i64::MIN + i64::from(id), None)
    }

    /// A key distinguishing events that share a timestamp legitimately.
    ///
    /// Uses the explicit order column when mapped, otherwise the lifecycle
    /// value, otherwise nothing.
    fn stage_key(&mut self, row: &Row) -> u32 {
        if let Some(column) = &self.mapping.event_order {
            let raw = row.get(column).trim();
            if !raw.is_empty() {
                return intern(&mut self.stamps, &format!("\u{1}ord:{raw}"));
            }
        }
        if let Some(column) = &self.mapping.lifecycle {
            let raw = row.get(column).trim();
            if !raw.is_empty() {
                return intern(&mut self.stamps, &format!("\u{1}life:{raw}"));
            }
        }
        0
    }

    fn single_format(&self) -> Option<&timestamp::Format> {
        (self.detection.candidates.len() == 1).then(|| &self.detection.candidates[0])
    }

    /// Produces the report.
    pub fn finish(self) -> Report {
        let mut report = Report::default();
        let d = &self.detection;

        if let Some(a) = &d.ambiguity {
            report.push(
                Finding::decision(
                    Kind::AmbiguousDateOrder,
                    format!(
                        "Dates such as {} read validly as either {} or {}.",
                        a.example, a.day_first_meaning, a.month_first_meaning
                    ),
                    date_order_decision(&a.example, &a.day_first_meaning, &a.month_first_meaning),
                )
                .examples(vec![a.example.clone()]),
            );
        }

        if !d.mixed.is_empty() {
            report.push(
                Finding::warning(
                    Kind::MixedTimestampFormats,
                    format!(
                        "The timestamp column mixes at least {} different layouts; no single \
                         format explains every value.",
                        d.mixed.len()
                    ),
                )
                .examples(
                    d.mixed
                        .iter()
                        .take(EXAMPLES)
                        .map(|f| f.example.to_owned())
                        .collect(),
                ),
            );
        }

        if self.unparseable_rows > 0 {
            report.push(
                Finding::warning(
                    Kind::UnparseableTimestamps,
                    format!(
                        "{} rows have a timestamp no known format can read.",
                        self.unparseable_rows
                    ),
                )
                .counts(self.unparseable_rows, 0)
                .examples(d.unparseable.clone()),
            );
        }

        if self.blank_rows > 0 {
            report.push(
                Finding::decision(
                    Kind::BlankTimestamps,
                    format!(
                        "{} rows across {} cases have no timestamp.",
                        self.blank_rows,
                        self.blank_cases.len()
                    ),
                    blank_timestamp_decision(),
                )
                .counts(self.blank_rows, self.blank_cases.len())
                .examples(self.blank_examples.clone()),
            );
        }

        // Time zone handling deserves care about *when* to ask, not just what
        // to ask. A decision raised on every log trains the user to click
        // through it without reading, defeating the point of asking at all.
        // So this blocks only when the answer demonstrably changes the
        // numbers: values that declare an offset cannot be compared with
        // values that do not. Otherwise the recorded default of "as written"
        // is stated plainly and remains changeable.
        let has_naive = d
            .candidates
            .iter()
            .chain(d.mixed.iter())
            .any(|f| !f.has_explicit_zone());

        if d.mixes_zoned_and_naive() {
            report.push(Finding::decision(
                Kind::MixedZoneAndNaive,
                "Some timestamps declare a UTC offset and others do not, so they cannot be \
                 compared until you say how to read the ones without an offset."
                    .to_owned(),
                timezone_policy_decision(),
            ));
        } else if has_naive {
            report.push(Finding::info(
                Kind::TimezonePolicyUnset,
                "Timestamps carry no time zone. They are read exactly as written, which is \
                 correct for most system exports. Durations across a daylight-saving change will \
                 follow the clock rather than real elapsed time. This is recorded with the \
                 project and can be changed."
                    .to_owned(),
            ));
        }

        if self.duplicate_rows > 0 {
            report.push(
                Finding::decision(
                    Kind::DuplicateRows,
                    format!(
                        "{} rows are identical in case, activity, and timestamp, across {} cases.",
                        self.duplicate_rows,
                        self.duplicate_cases.len()
                    ),
                    duplicate_rows_decision(),
                )
                .counts(self.duplicate_rows, self.duplicate_cases.len())
                .examples(self.duplicate_examples.clone()),
            );
        }

        if !self.tied_cases.is_empty() {
            report.push(
                Finding::warning(
                    Kind::TimestampTies,
                    format!(
                        "{} cases contain events sharing a timestamp ({} events beyond the \
                         first). {}",
                        self.tied_cases.len(),
                        self.tied_events,
                        if self.mapping.event_order.is_some() {
                            "The mapped order column decides their order."
                        } else {
                            "Their order is decided by position in the file, because no \
                             explicit order column is mapped."
                        }
                    ),
                )
                .counts(self.tied_events, self.tied_cases.len())
                .examples(self.label_cases(&self.tied_cases)),
            );
        }

        let singles: Vec<u32> = self
            .case_sizes
            .iter()
            .enumerate()
            .filter(|(_, n)| **n == 1)
            .map(|(c, _)| c as u32)
            .collect();
        if !singles.is_empty() {
            let total = self.case_sizes.len().max(1);
            let pct = singles.len() as f64 / total as f64 * 100.0;
            let set: BTreeSet<u32> = singles.iter().copied().collect();
            report.push(
                Finding::info(
                    Kind::SingleEventCases,
                    format!(
                        "{} of {} cases ({pct:.0}%) contain exactly one event. These contribute \
                         an activity but no transition, and have no throughput time.",
                        singles.len(),
                        total
                    ),
                )
                .counts(singles.len(), singles.len())
                .examples(self.label_cases(&set)),
            );
        }

        if self.blank_case_rows > 0 || self.blank_activity_rows > 0 {
            let mut parts = Vec::new();
            if self.blank_case_rows > 0 {
                parts.push(format!("{} without a case id", self.blank_case_rows));
            }
            if self.blank_activity_rows > 0 {
                parts.push(format!("{} without an activity", self.blank_activity_rows));
            }
            report.push(
                Finding::warning(
                    Kind::BlankRequiredValues,
                    format!(
                        "{}. These are grouped under a blank name and will appear as such in \
                         the process map. They usually indicate the wrong column was mapped, or \
                         rows that should be excluded.",
                        parts.join(" and ")
                    ),
                )
                .counts(self.blank_case_rows + self.blank_activity_rows, 0)
                .examples(self.blank_examples_required.clone()),
            );
        }

        let column_names = |indexes: &mut dyn Iterator<Item = usize>| -> String {
            indexes
                .filter_map(|index| self.mapping.case_attributes.get(index))
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        };
        let changing: BTreeSet<(u32, usize)> = self
            .conflicting_attributes
            .iter()
            .filter(|(_, index)| !self.free_text.contains(index))
            .copied()
            .collect();
        if !changing.is_empty() {
            let cases: BTreeSet<u32> = changing.iter().map(|(case, _)| *case).collect();
            report.push(
                Finding::info(
                    Kind::ConflictingCaseAttributes,
                    format!(
                        "{} change between steps within {} items, so their values are kept \
                         on each step. A filter on them keeps items that ever had the value \
                         (or, if you choose, whose latest value it is), and an item can appear \
                         in more than one group when broken down or compared.",
                        column_names(&mut changing.iter().map(|(_, index)| *index)),
                        cases.len()
                    ),
                )
                .counts(changing.len(), cases.len())
                .examples(self.conflicting_examples.clone()),
            );
        }
        if !self.free_text.is_empty() {
            report.push(Finding::info(
                Kind::ConflictingCaseAttributes,
                format!(
                    "{} take more than {} different values across steps, like free text, \
                     so each item keeps only its first value for them.",
                    column_names(&mut self.free_text.iter().copied()),
                    crate::normalize::STEP_VALUE_LIMIT
                ),
            ));
        }
        if !self.respelled.is_empty() {
            let cases: BTreeSet<u32> = self.respelled.iter().map(|(case, _)| *case).collect();
            report.push(
                Finding::warning(
                    Kind::InconsistentFieldSpelling,
                    format!(
                        "{} have values spelled differently within {} items, only in capitals \
                         or spaces. Within an item they count as one value, but across items \
                         they show as separate values; clean them in the source for \
                         consistent groups.",
                        column_names(&mut self.respelled.iter().map(|(_, index)| *index)),
                        cases.len()
                    ),
                )
                .counts(self.respelled.len(), cases.len())
                .examples(self.respelled_examples.clone()),
            );
        }

        if self.out_of_order > 0 {
            report.push(
                Finding::info(
                    Kind::NonChronologicalFileOrder,
                    format!(
                        "{} rows appear in the file before an earlier event of the same case. \
                         Events are sorted by timestamp, so this does not affect results.",
                        self.out_of_order
                    ),
                )
                .counts(self.out_of_order, 0),
            );
        }

        self.report_labels(&mut report);
        report
    }

    /// Turns interned case ids back into a few readable identifiers.
    fn label_cases(&self, ids: &BTreeSet<u32>) -> Vec<String> {
        ids.iter()
            .take(EXAMPLES)
            .filter_map(|id| self.cases.label(*id).map(str::to_owned))
            .collect()
    }

    /// How often each activity label occurs.
    pub fn label_counts(&self) -> &HashMap<String, usize> {
        &self.label_counts
    }

    fn report_labels(&self, report: &mut Report) {
        let labels: BTreeSet<&str> = self.activities.keys().map(String::as_str).collect();
        let mut groups: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
        for label in &labels {
            groups
                .entry(label.trim().to_lowercase())
                .or_default()
                .insert(label);
        }
        let collisions: Vec<(&String, &BTreeSet<&str>)> =
            groups.iter().filter(|(_, set)| set.len() > 1).collect();

        if !collisions.is_empty() {
            let affected: usize = collisions.iter().map(|(_, set)| set.len()).sum();
            let word = if collisions.len() == 1 {
                "group"
            } else {
                "groups"
            };
            report.push(
                Finding::warning(
                    Kind::NearDuplicateLabels,
                    format!(
                        "{affected} activity labels in {} {word} differ only by capitalisation \
                         or surrounding spaces. They are treated as distinct activities unless \
                         you merge them.",
                        collisions.len()
                    ),
                )
                .examples(
                    collisions
                        .iter()
                        .take(EXAMPLES)
                        .map(|(_, set)| {
                            set.iter()
                                .map(|s| format!("{s:?}"))
                                .collect::<Vec<_>>()
                                .join(" / ")
                        })
                        .collect(),
                )
                .offering(label_merge_decision()),
            );
        }

        if labels.len() > CARDINALITY_LIMIT {
            report.push(Finding::warning(
                Kind::HighActivityCardinality,
                format!(
                    "{} distinct activities. A process map above roughly {CARDINALITY_LIMIT} is \
                     usually unreadable without filtering.",
                    labels.len()
                ),
            ));
        }
    }
}

/// A large case's (activity, stamp, stage) rows and (stamp, stage) times.
type LargeCase = (HashSet<(u32, i64, u32)>, HashSet<(i64, u32)>);

fn intern(map: &mut HashMap<String, u32>, value: &str) -> u32 {
    if let Some(id) = map.get(value) {
        return *id;
    }
    let id = map.len() as u32;
    map.insert(value.to_owned(), id);
    id
}

/// Validates rows already held in memory.
pub fn validate(rows: &[Row], mapping: &Mapping) -> Report {
    let sample: Vec<String> = rows
        .iter()
        .take(SAMPLE_SIZE)
        .map(|r| r.get(&mapping.timestamp).to_owned())
        .collect();
    let mut validator = Validator::new(mapping, timestamp::detect(&sample));
    for row in rows {
        validator.observe(row);
    }
    validator.finish()
}

/// Builds a raw-label to canonical-label map for labels that differ only by
/// case or surrounding whitespace.
///
/// The canonical spelling is whichever occurs most often; ties are broken
/// lexicographically so the result is deterministic. Only groups with more
/// than one spelling appear, so applying the map to a clean log is a no-op.
pub fn merge_map(counts: &HashMap<String, usize>) -> BTreeMap<String, String> {
    let mut groups: BTreeMap<String, Vec<(&String, usize)>> = BTreeMap::new();
    for (label, n) in counts {
        groups
            .entry(label.trim().to_lowercase())
            .or_default()
            .push((label, *n));
    }

    let mut map = BTreeMap::new();
    for (_, mut members) in groups {
        if members.len() < 2 {
            continue;
        }
        // Most frequent first; ties broken by spelling for determinism.
        members.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let canonical = members[0].0.clone();
        for (label, _) in members {
            map.insert(label.clone(), canonical.clone());
        }
    }
    map
}

/// Counts activity labels in rows already held in memory.
pub fn label_counts(rows: &[Row], mapping: &Mapping) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for row in rows {
        *counts
            .entry(row.get(&mapping.activity).to_owned())
            .or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Severity;
    use crate::read::{Row, Schema};
    use std::rc::Rc;

    fn rows(data: &[(&str, &str, &str)]) -> Vec<Row> {
        let schema = Rc::new(Schema::new(vec![
            "case_id".to_owned(),
            "activity".to_owned(),
            "timestamp".to_owned(),
        ]));
        data.iter()
            .enumerate()
            .map(|(i, (c, a, t))| {
                Row::new(
                    Rc::clone(&schema),
                    vec![(*c).to_owned(), (*a).to_owned(), (*t).to_owned()],
                    i + 2,
                )
            })
            .collect()
    }

    fn mapping() -> Mapping {
        Mapping::new("case_id", "activity", "timestamp")
    }

    fn kinds(report: &Report) -> BTreeSet<Kind> {
        report.findings.iter().map(|f| f.kind.clone()).collect()
    }

    /// A clean log must not block. If routine data raises a decision, users
    /// learn to dismiss decisions without reading them.
    #[test]
    fn clean_log_blocks_nothing() {
        let r = validate(
            &rows(&[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C1", "B", "2026-03-02 10:00:00"),
                ("C2", "A", "2026-03-02 09:30:00"),
                ("C2", "B", "2026-03-02 11:00:00"),
            ]),
            &mapping(),
        );
        assert!(!r.has_blocking(), "clean data must not block: {r:?}");
        assert_eq!(kinds(&r), BTreeSet::from([Kind::TimezonePolicyUnset]));
        assert_eq!(r.findings[0].severity, Severity::Info);
    }

    #[test]
    fn reports_conflicting_values_within_a_case_attribute() {
        let schema = Rc::new(Schema::new(vec![
            "case_id".to_owned(),
            "activity".to_owned(),
            "timestamp".to_owned(),
            "region".to_owned(),
        ]));
        let rows = vec![
            Row::new(
                Rc::clone(&schema),
                vec!["C1", "A", "2026-03-02 09:00:00", "US"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                2,
            ),
            Row::new(
                schema,
                vec!["C1", "B", "2026-03-02 10:00:00", "EU"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                3,
            ),
        ];
        let mapping = mapping().with_case_attributes(vec!["region".to_owned()]);
        let report = validate(&rows, &mapping);
        assert!(kinds(&report).contains(&Kind::ConflictingCaseAttributes));
    }

    #[test]
    fn mixed_zone_information_blocks() {
        let r = validate(
            &rows(&[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C1", "B", "2026-03-02T10:00:00Z"),
            ]),
            &mapping(),
        );
        assert!(r.has_blocking());
        assert!(kinds(&r).contains(&Kind::MixedZoneAndNaive));
    }

    fn rows_with(headers: &[&str], data: &[&[&str]]) -> Vec<Row> {
        let schema = Rc::new(Schema::new(
            headers.iter().map(|h| (*h).to_owned()).collect(),
        ));
        data.iter()
            .enumerate()
            .map(|(i, values)| {
                Row::new(
                    Rc::clone(&schema),
                    values.iter().map(|v| (*v).to_owned()).collect(),
                    i + 2,
                )
            })
            .collect()
    }

    /// Two lifecycle stages of one activity are not duplicates.
    #[test]
    fn lifecycle_stages_are_not_duplicates() {
        let rows = rows_with(
            &["case_id", "activity", "timestamp", "lifecycle"],
            &[
                &["C1", "Review", "2026-03-02 09:00:00", "start"],
                &["C1", "Review", "2026-03-02 09:00:00", "complete"],
            ],
        );
        let mapping = Mapping::new("case_id", "activity", "timestamp").with_lifecycle("lifecycle");
        let r = validate(&rows, &mapping);
        assert!(
            !kinds(&r).contains(&Kind::DuplicateRows),
            "start and complete are distinct stages: {r:?}"
        );
        assert!(
            !kinds(&r).contains(&Kind::TimestampTies),
            "lifecycle resolves their order, so it is not an unresolved tie"
        );
    }

    /// Without a lifecycle column they genuinely are indistinguishable.
    #[test]
    fn identical_rows_without_a_lifecycle_are_duplicates() {
        let rows = rows_with(
            &["case_id", "activity", "timestamp"],
            &[
                &["C1", "Review", "2026-03-02 09:00:00"],
                &["C1", "Review", "2026-03-02 09:00:00"],
            ],
        );
        let r = validate(&rows, &Mapping::new("case_id", "activity", "timestamp"));
        assert!(kinds(&r).contains(&Kind::DuplicateRows));
    }

    /// An explicit order column resolves ties, and the message says so.
    #[test]
    fn an_order_column_resolves_ties() {
        let rows = rows_with(
            &["case_id", "activity", "timestamp", "seq"],
            &[
                &["C1", "A", "2026-03-02 09:00:00", "1"],
                &["C1", "B", "2026-03-02 09:00:00", "2"],
            ],
        );
        let mapping = Mapping::new("case_id", "activity", "timestamp").with_event_order("seq");
        let r = validate(&rows, &mapping);
        assert!(!kinds(&r).contains(&Kind::TimestampTies));
    }

    #[test]
    fn detects_duplicate_rows_as_a_decision() {
        let r = validate(
            &rows(&[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C1", "B", "2026-03-02 10:00:00"),
            ]),
            &mapping(),
        );
        let f = r
            .findings
            .iter()
            .find(|f| f.kind == Kind::DuplicateRows)
            .expect("duplicates must be found");
        assert!(f.is_blocking());
        assert_eq!(f.affected_rows, 1);
    }

    #[test]
    fn detects_blank_timestamps_as_a_decision() {
        let r = validate(
            &rows(&[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C1", "B", ""),
                ("C2", "A", ""),
            ]),
            &mapping(),
        );
        let f = r
            .findings
            .iter()
            .find(|f| f.kind == Kind::BlankTimestamps)
            .expect("blanks must be found");
        assert_eq!(f.affected_rows, 2);
        assert_eq!(f.affected_cases, 2);
    }

    #[test]
    fn detects_timestamp_ties() {
        let r = validate(
            &rows(&[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C1", "B", "2026-03-02 09:00:00"),
                ("C1", "C", "2026-03-02 09:00:00"),
                ("C2", "A", "2026-03-02 09:00:00"),
                ("C2", "B", "2026-03-02 10:00:00"),
            ]),
            &mapping(),
        );
        let f = r
            .findings
            .iter()
            .find(|f| f.kind == Kind::TimestampTies)
            .expect("ties must be found");
        assert_eq!(f.affected_rows, 2);
        assert_eq!(f.affected_cases, 1);
        assert_eq!(f.examples, vec!["C1"]);
    }

    #[test]
    fn detects_single_event_cases() {
        let r = validate(
            &rows(&[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("C2", "A", "2026-03-02 09:30:00"),
                ("C2", "B", "2026-03-02 10:30:00"),
                ("C3", "A", "2026-03-02 11:00:00"),
            ]),
            &mapping(),
        );
        let f = r
            .findings
            .iter()
            .find(|f| f.kind == Kind::SingleEventCases)
            .expect("single-event cases must be reported");
        assert_eq!(f.affected_cases, 2);
        assert_eq!(f.examples, vec!["C1", "C3"]);
    }

    /// A blank case id or activity would otherwise be interned as a real
    /// value and appear in the process map as an unnamed activity.
    #[test]
    fn reports_blank_case_ids_and_activities() {
        let r = validate(
            &rows(&[
                ("C1", "A", "2026-03-02 09:00:00"),
                ("", "B", "2026-03-02 10:00:00"),
                ("C2", "", "2026-03-02 11:00:00"),
            ]),
            &mapping(),
        );
        let f = r
            .findings
            .iter()
            .find(|f| f.kind == Kind::BlankRequiredValues)
            .expect("blank required values must be reported");
        assert_eq!(f.affected_rows, 2);
    }

    #[test]
    fn a_clean_log_reports_no_blank_required_values() {
        let r = validate(&rows(&[("C1", "A", "2026-03-02 09:00:00")]), &mapping());
        assert!(!kinds(&r).contains(&Kind::BlankRequiredValues));
    }

    #[test]
    fn detects_non_chronological_file_order() {
        let r = validate(
            &rows(&[
                ("C1", "B", "2026-03-02 10:00:00"),
                ("C1", "A", "2026-03-02 09:00:00"),
            ]),
            &mapping(),
        );
        assert!(kinds(&r).contains(&Kind::NonChronologicalFileOrder));
    }

    #[test]
    fn detects_near_duplicate_labels() {
        let r = validate(
            &rows(&[
                ("C1", "Approve", "2026-03-02 09:00:00"),
                ("C1", "approve", "2026-03-02 10:00:00"),
                ("C1", "Approve ", "2026-03-02 11:00:00"),
            ]),
            &mapping(),
        );
        let f = r
            .findings
            .iter()
            .find(|f| f.kind == Kind::NearDuplicateLabels)
            .expect("near-duplicate labels must be reported");
        assert!(f.examples[0].contains("Approve"));
    }

    #[test]
    fn ambiguous_dates_block_analysis() {
        let r = validate(
            &rows(&[
                ("C1", "A", "05/01/2026 09:00"),
                ("C1", "B", "06/01/2026 10:00"),
            ]),
            &mapping(),
        );
        assert!(kinds(&r).contains(&Kind::AmbiguousDateOrder));
        assert!(r.has_blocking());
    }

    #[test]
    fn validation_is_deterministic() {
        let data = rows(&[
            ("C1", "A", "2026-03-02 09:00:00"),
            ("C1", "A", "2026-03-02 09:00:00"),
            ("C2", "B", ""),
        ]);
        assert_eq!(validate(&data, &mapping()), validate(&data, &mapping()));
    }

    #[test]
    fn validation_never_mutates_input() {
        let data = rows(&[("C1", "A", "2026-03-02 09:00:00")]);
        let before = data.clone();
        let _ = validate(&data, &mapping());
        assert_eq!(data, before);
    }

    /// Streaming and in-memory validation must agree exactly, or the fast path
    /// and the simple path would disagree about the data.
    #[test]
    fn streaming_matches_in_memory_validation() {
        let data = rows(&[
            ("C1", "A", "2026-03-02 09:00:00"),
            ("C1", "A", "2026-03-02 09:00:00"),
            ("C2", "B", ""),
            ("C3", "C", "2026-03-02 08:00:00"),
        ]);
        let eager = validate(&data, &mapping());

        let sample: Vec<String> = data
            .iter()
            .take(SAMPLE_SIZE)
            .map(|r| r.get("timestamp").to_owned())
            .collect();
        let m = mapping();
        let mut v = Validator::new(&m, timestamp::detect(&sample));
        for row in &data {
            v.observe(row);
        }
        assert_eq!(v.finish(), eager);
    }
}
