//! The analysis entry point every front end calls.

use pm_core::dfg::Dfg;
use pm_core::filter::Filter;
use pm_core::handover::Organization;
use pm_core::log::{EventLog, Micros};
use pm_core::rework::Rework as CoreRework;
use pm_core::variants::Variants;
use pm_ingest::finding::{Policy, Report, Severity};
use pm_ingest::normalize::{Normalizer, prepare};
use pm_ingest::timestamp;
use pm_ingest::validate::Mapping;
use pm_ingest::validate::Validator;
use pm_ingest::wide;
use pm_ingest::{Rules, Table, normalize_with, validate};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::rc::Rc;

/// Why an analysis could not be attempted.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error(transparent)]
    Read(#[from] pm_ingest::ReadError),
    #[error(transparent)]
    Normalize(#[from] pm_ingest::NormalizeError),
    #[error("could not propose a mapping; columns are: {columns}")]
    NoMapping { columns: String },
}

/// The result of asking for an analysis.
///
/// Blocking is a first-class outcome, not an error: the data was fine, the
/// tool simply will not guess on the user's behalf.
#[derive(Debug)]
pub enum Outcome {
    /// Decisions must be answered first.
    ///
    /// Boxed so both variants stay a similar size; a `Report` carries every
    /// finding and would otherwise make the common `Complete` case expensive
    /// to move.
    Blocked(Box<Blocked>),
    /// A complete analysis.
    Complete(Box<Analysis>),
}

/// Why an analysis stopped, and what must be answered to continue.
#[derive(Debug)]
pub struct Blocked {
    pub report: Report,
    pub mapping: Mapping,
}

/// A choice the user may make, which did not block the analysis.
///
/// Reported so a non-blocking finding is still actionable — otherwise we would
/// point out a problem and offer nothing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Offer {
    /// The decision identifier, as used on the command line.
    pub flag: String,
    pub question: String,
    /// Option id and its label.
    pub options: Vec<(String, String)>,
}

/// One activity in the process map.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Node {
    pub activity: String,
    pub frequency: u64,
    pub case_frequency: u64,
    pub starts: u64,
    pub ends: u64,
    /// Cases evidencing this activity.
    pub example_cases: Vec<String>,
}

/// One transition in the process map.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// Times this transition occurred, including cases ineligible for timing.
    pub frequency: u64,
    pub case_frequency: u64,
    /// Observations included in the duration statistics.
    pub duration_eligible_occurrences: u64,
    pub mean_seconds: i64,
    pub total_seconds: i64,
    pub min_seconds: i64,
    pub max_seconds: i64,
    /// Share of all directly-follows observations in the current view.
    pub frequency_share_percent: f64,
    /// Share of all measured transition time in the current view.
    pub performance_share_percent: f64,
    /// Deterministic position in the frequency overlay, most frequent first.
    pub frequency_rank: usize,
    /// Position in the performance overlay, or `None` when no duration was
    /// eligible for measurement.
    pub performance_rank: Option<usize>,
    /// Cases evidencing this transition.
    pub example_cases: Vec<String>,
    /// Observations whose two steps carry the same recorded time, so the
    /// order between them comes from the file, not the clock.
    #[serde(default)]
    pub same_time_occurrences: u64,
}

/// A transition ranked by how much time it accounts for.
///
/// The duration is **waiting time** only when end timestamps were mapped;
/// otherwise it is start-to-start elapsed time, which includes however long
/// the earlier activity itself took. [`Analysis::measures_true_waiting`] says
/// which.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bottleneck {
    /// One-based rank by total duration, with activity labels breaking ties.
    pub rank: usize,
    pub from: String,
    pub to: String,
    /// Total waiting time across duration-eligible occurrences.
    pub total_seconds: i64,
    pub mean_seconds: i64,
    /// All times the transition occurred, including incomplete cases.
    pub total_occurrences: u64,
    /// Occurrences used by the duration statistics above.
    pub duration_eligible_occurrences: u64,
    /// Share of all transition time in the process, as a percentage.
    pub share_percent: f64,
    /// Cases evidencing this transition.
    pub example_cases: Vec<String>,
}

/// Rework statistics for one activity.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReworkActivitySummary {
    pub activity: String,
    /// Occurrences after the activity's first occurrence in each case.
    pub repetitions: u64,
    pub immediate_repetitions: u64,
    pub returns_after_intervening: u64,
    pub cases_affected: u64,
    pub duration_eligible_occurrences: u64,
    /// Mean start-to-start interval since the previous occurrence.
    pub mean_repeat_interval_seconds: i64,
    pub total_repeat_interval_seconds: i64,
    pub example_cases: Vec<String>,
}

/// One loop path observed inside actual cases.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopSummary {
    /// Includes the repeated anchor at both ends, e.g. `A → B → A`.
    pub sequence: Vec<String>,
    pub occurrences: u64,
    pub cases_affected: u64,
    pub duration_eligible_occurrences: u64,
    pub mean_duration_seconds: i64,
    pub total_duration_seconds: i64,
    pub example_cases: Vec<String>,
}

/// First-class rework result for the current filtered view.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ReworkSummary {
    pub cases_affected: u64,
    pub case_percent: f64,
    /// Sum of every repeated activity occurrence. A multi-step loop can repeat
    /// more than one activity, so this may exceed `loop_traversals`.
    pub repeated_activity_occurrences: u64,
    pub immediate_repetitions: u64,
    pub returns_after_intervening: u64,
    pub loop_traversals: u64,
    /// Candidates omitted from path detail by bounded-memory safety limits.
    /// Activity-level repetition counts remain complete.
    pub omitted_loop_candidates: u64,
    pub activities: Vec<ReworkActivitySummary>,
    pub loops: Vec<LoopSummary>,
}

/// One distinct path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VariantSummary {
    pub sequence: Vec<String>,
    pub cases: usize,
    pub percent: f64,
    /// Cases used by the mean throughput calculation.
    pub duration_eligible_cases: usize,
    pub mean_throughput_seconds: i64,
    /// A few case identifiers, so any figure can be traced to real cases.
    pub example_cases: Vec<String>,
}

/// Work passing between two people, teams, or systems.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HandoverSummary {
    pub from: String,
    pub to: String,
    pub occurrences: u64,
    pub cases: u64,
    /// Observations used by the duration statistics below.
    pub duration_eligible_occurrences: u64,
    pub mean_seconds: i64,
    pub total_seconds: i64,
    /// Share of all handover waiting time, as a percentage.
    pub share_percent: f64,
    /// Cases evidencing this handover.
    pub example_cases: Vec<String>,
}

/// The organizational view, present only when a resource column was mapped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrganizationSummary {
    pub resources: usize,
    /// Share of consecutive event pairs that changed hands.
    pub fragmentation_percent: f64,
    pub handovers: Vec<HandoverSummary>,
}

/// How much of the data the analysis actually covers.
///
/// Always reported, because a view of 12% of the process must say so — silent
/// narrowing is how a finding about a fraction gets presented as a finding
/// about the whole.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct View {
    pub cases_shown: usize,
    pub cases_total: usize,
    pub events_total: usize,
    pub percent_shown: f64,
    /// Whether a filter removed anything.
    pub narrowed: bool,
}

/// Headline numbers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    pub events: usize,
    pub cases: usize,
    pub activities: usize,
    pub variants: usize,
    pub single_event_cases: usize,
    pub unique_variants: usize,
    /// Share of cases covered by the top five variants.
    pub top_five_coverage_percent: f64,
    /// Variants needed to account for 80% of cases.
    pub variants_for_80_percent: usize,
    pub mean_throughput_seconds: i64,
    pub median_throughput_seconds: i64,
    /// 90th percentile completion time (nearest rank): nine in ten
    /// duration-eligible cases finished within it. Absent from older caches.
    #[serde(default)]
    pub p90_throughput_seconds: i64,
    /// Cases used by the throughput statistics above.
    pub duration_eligible_cases: usize,
    /// Cases excluded from duration statistics because they were marked as
    /// having missing data.
    pub incomplete_cases: usize,
    /// Sum of mapped event costs in millionths of the source currency unit.
    /// `None` means no numeric costs were present.
    pub total_cost_microunits: Option<i128>,
}

/// A complete analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Analysis {
    pub source: String,
    pub case_column: String,
    pub activity_column: String,
    pub timestamp_column: String,
    /// The timestamp format that was applied.
    pub timestamp_format: Option<String>,
    /// Columns carried through as case attributes.
    pub case_attributes: Vec<String>,
    /// The complete mapping that was used, so a saved project reproduces this
    /// analysis rather than an approximation of it.
    pub mapping: Mapping,
    /// Optional columns that were mapped, as (field, column). Shown so the
    /// user can see why an analysis appeared — or notice one is missing.
    pub optional_columns: Vec<(String, String)>,
    /// Set when the source was a wide table that had to be unpivoted, naming
    /// the columns that became activities. Shown so the user can confirm or
    /// rename them.
    pub unpivoted_from: Option<Vec<String>>,
    pub summary: Summary,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub bottlenecks: Vec<Bottleneck>,
    pub rework: ReworkSummary,
    pub variants: Vec<VariantSummary>,
    /// How much of the log this analysis covers.
    pub view: View,
    /// Whether transition durations exclude processing time.
    ///
    /// True only when an end-timestamp column was mapped. Otherwise durations
    /// are start-to-start elapsed time and must not be called waiting.
    pub measures_true_waiting: bool,
    /// Events and cases removed by preparation rules.
    pub excluded_events: usize,
    pub excluded_cases: usize,
    /// Advisory findings. Nothing here blocked the analysis.
    pub notes: Vec<String>,
    /// Extent of recorded timestamps in the active view, without correction.
    #[serde(default)]
    pub recorded_window: Option<(Micros, Micros)>,
    /// Choices the user may make, offered but not required.
    pub offers: Vec<Offer>,
    /// Who passes work to whom. Absent unless a resource column was mapped.
    pub organization: Option<OrganizationSummary>,
    /// Completion-time distribution and slowest cases. Absent from older
    /// caches.
    #[serde(default)]
    pub timing: crate::timing::TimingSummary,
    /// Where the machine stops and people start.
    ///
    /// Present whenever the log has resources or deviates from a single path,
    /// because either makes the question answerable.
    pub targeting: Option<crate::targeting::AutomationReport>,
    /// Where the shown cases sit against the extract's observation window.
    /// Absent when the project has no window.
    #[serde(default)]
    pub window: Option<pm_core::WindowSummary>,
    /// Case attributes that hold numbers, such as amounts. Absent from older
    /// caches.
    #[serde(default)]
    pub number_fields: Vec<pm_core::NumberField>,
    /// The calculated facts worth reading first, ranked. Absent from older
    /// caches.
    #[serde(default)]
    pub key_findings: Vec<crate::findings::KeyFinding>,
    /// Field values that go with the slow items. Absent from older caches.
    #[serde(default)]
    pub drivers: crate::drivers::Drivers,
    /// For the hand-offs where time goes, the field value that makes each
    /// slow. Absent from older caches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_drivers: Vec<crate::patterns::StepDriver>,
    /// Usual steps that some items skip. Absent from older caches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_steps: Vec<crate::patterns::SkippedStep>,
    /// Everyone who recorded a step, alphabetically, for the "handled by"
    /// filter. Empty when no person column was mapped.
    #[serde(default)]
    pub people: Vec<String>,
    /// Fields whose value changes between an item's steps (a status): their
    /// filters match items that ever had a value, or whose latest value is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub step_level_fields: Vec<String>,
    /// A first read of the kind of process from its step names, when clear.
    #[serde(default)]
    pub process: Option<crate::process::ProcessProfile>,
}

const MICROS_PER_SECOND: Micros = 1_000_000;

/// Preserve every finding produced by the bounded finding rules.
const KEY_FINDINGS: usize = usize::MAX;

/// The command-line name for a decision.
fn flag_for(kind: &pm_ingest::Kind) -> &'static str {
    use pm_ingest::Kind;
    match kind {
        Kind::BlankTimestamps => "blank_timestamps",
        Kind::DuplicateRows => "duplicate_rows",
        Kind::AmbiguousDateOrder => "ambiguous_date_order",
        Kind::MixedZoneAndNaive => "mixed_zone",
        Kind::NearDuplicateLabels => "near_duplicate_labels",
        _ => "unknown",
    }
}

/// Analyses a CSV or Parquet file, chosen by extension.
pub fn analyze(
    path: impl AsRef<Path>,
    mapping: Option<Mapping>,
    policy: &Policy,
) -> Result<Outcome, QueryError> {
    analyze_rules(path, mapping, policy, Rules::new())
}

/// Analyses a CSV file under explicit preparation rules.
pub fn analyze_rules(
    path: impl AsRef<Path>,
    mapping: Option<Mapping>,
    policy: &Policy,
    rules: Rules,
) -> Result<Outcome, QueryError> {
    analyze_filtered(path, mapping, policy, rules, Filter::new())
}

/// A normalized log together with the interpretation needed to display it.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedLog {
    pub log: pm_core::log::EventLog,
    /// Human-readable description of the timezone treatment actually applied.
    pub timestamp_reading: &'static str,
}

/// Validates a source and returns a structured, serializable quality report.
///
/// This deliberately stops before normalization: unresolved decisions are the
/// result, not an error, so a UI can show what must be answered.
pub fn assess_quality(
    path: impl AsRef<Path>,
    mapping: Option<Mapping>,
    policy: &Policy,
) -> Result<crate::quality::DataQualityReport, QueryError> {
    let path = path.as_ref();
    let source = path.display().to_string();
    let (plan, report) = validate_plan(path, mapping)?;
    Ok(crate::quality::DataQualityReport::from_validation(
        source,
        plan.mapping,
        &report,
        policy,
    ))
}

/// Produces the normalized log for a file, for drill-down.
///
/// Runs the same two streaming passes as [`analyze_filtered`], so a case
/// opened here is the same case the analysis counted.
pub fn load_log(
    path: impl AsRef<Path>,
    mapping: Option<Mapping>,
    policy: &Policy,
    rules: Rules,
) -> Result<Result<LoadedLog, Box<Blocked>>, QueryError> {
    let path = path.as_ref();
    let (plan, report) = validate_plan(path, mapping)?;
    if !policy.resolves(&report) {
        return Ok(Err(Box::new(Blocked {
            report,
            mapping: plan.mapping,
        })));
    }

    let format = prepare(&plan.mapping, &report, policy, &plan.sample_stamps)?;
    let timestamp_reading = timestamp_reading(policy, &format);
    let mut normalizer = Normalizer::new(&plan.mapping, format, policy)
        .allowing_mixed_formats(pm_ingest::needs_mixed_formats(&plan.sample_stamps))
        .with_rules(rules);
    // The same merge pass analysis runs. Without it, drill-down would resolve
    // labels differently and show evidence that does not match the figures it
    // is supposed to support.
    if policy.get(&pm_ingest::Kind::NearDuplicateLabels) == Some("merge") {
        normalizer = normalizer.with_label_merges(label_merges(path, &plan)?);
    }
    for row in stream(path, &plan)? {
        normalizer.observe(&row?);
    }
    Ok(Ok(LoadedLog {
        log: normalizer.finish().log,
        timestamp_reading,
    }))
}

/// How the timestamps in a log should be read, for display alongside them.
pub fn timestamp_reading(policy: &Policy, format: &pm_ingest::timestamp::Format) -> &'static str {
    match policy.get(&pm_ingest::Kind::MixedZoneAndNaive) {
        Some("utc") => "converted to UTC",
        Some("as_written") => "wall-clock, exactly as written",
        _ if format.has_explicit_zone() => "converted to UTC using the recorded offset",
        _ => "exactly as recorded, with no timezone applied",
    }
}

/// Analyses a file under preparation rules and a view filter.
///
/// **This streams.** The source is read twice — once to validate, once to
/// normalize — and never held in memory. Peak memory tracks the encoded log,
/// not the file, which is what lets a log larger than memory be analysed at
/// all. The benchmark harness measures this same path.
pub fn analyze_filtered(
    path: impl AsRef<Path>,
    mapping: Option<Mapping>,
    policy: &Policy,
    rules: Rules,
    filter: Filter,
) -> Result<Outcome, QueryError> {
    let path = path.as_ref();
    let source = path.display().to_string();

    let (plan, report) = validate_plan(path, mapping)?;

    if !policy.resolves(&report) {
        return Ok(Outcome::Blocked(Box::new(Blocked {
            report,
            mapping: plan.mapping,
        })));
    }

    // Pass two: normalize.
    let rules_for_report = rules.clone();
    let format = prepare(&plan.mapping, &report, policy, &plan.sample_stamps)?;
    let mut normalizer = Normalizer::new(&plan.mapping, format, policy)
        .allowing_mixed_formats(pm_ingest::needs_mixed_formats(&plan.sample_stamps))
        .with_rules(rules);
    if policy.get(&pm_ingest::Kind::NearDuplicateLabels) == Some("merge") {
        normalizer = normalizer.with_label_merges(label_merges(path, &plan)?);
    }
    for row in stream(path, &plan)? {
        normalizer.observe(&row?);
    }
    let normalized = normalizer.finish();

    Ok(assemble(
        &source,
        &plan,
        &report,
        &normalized,
        &filter,
        &rules_for_report,
    ))
}

/// How many rows to read before streaming, for profiling and detection.
const SAMPLE_ROWS: usize = 500;

/// Builds the interpretation plan and performs the shared streaming
/// validation pass used by quality reporting, analysis, and drill-down.
fn validate_plan(path: &Path, mapping: Option<Mapping>) -> Result<(Plan, Report), QueryError> {
    // A sample is enough to propose a mapping, detect wide format, and choose
    // a timestamp format. Everything after this streams.
    let (schema, sample) = pm_ingest::sample_rows(path, SAMPLE_ROWS)?;
    let plan = Plan::build(&schema, &sample, mapping)?;
    let mut validator = Validator::new(&plan.mapping, timestamp::detect(&plan.sample_stamps));
    for row in stream(path, &plan)? {
        validator.observe(&row?);
    }
    let report = validator.finish();
    Ok((plan, report))
}

/// What the sample established: how to read the file, and how to map it.
struct Plan {
    mapping: Mapping,
    /// Set when the source is wide and must be unpivoted row by row.
    wide: Option<wide::WideSpec>,
    /// Timestamp values from the sample, for format detection.
    sample_stamps: Vec<String>,
    /// Columns that became activities, when unpivoted.
    unpivoted_from: Option<Vec<String>>,
    case_attributes: Vec<String>,
}

impl Plan {
    fn build(
        schema: &Rc<pm_ingest::Schema>,
        sample: &[pm_ingest::Row],
        given: Option<Mapping>,
    ) -> Result<Plan, QueryError> {
        // A long-format mapping if one can be proposed; otherwise treat the
        // table as wide, which is the shape most operational exports have.
        let (mapping, wide, unpivoted_from) =
            match given.or_else(|| crate::propose_mapping(schema.headers())) {
                Some(m) => (m, None, None),
                None => match wide::detect(schema, sample) {
                    Some(spec) => {
                        let columns = spec
                            .events
                            .iter()
                            .map(|e| format!("{} → {}", e.column, e.activity))
                            .collect();
                        let mut mapping = spec.output_mapping();
                        mapping.case_attributes = spec.attributes.clone();
                        (mapping, Some(spec), Some(columns))
                    }
                    None => {
                        return Err(QueryError::NoMapping {
                            columns: schema.headers().join(", "),
                        });
                    }
                },
            };

        // Timestamp values are taken after any unpivot, so detection sees the
        // values that will actually be parsed.
        let sample_stamps = match &wide {
            Some(spec) => {
                let out_schema = spec.output_schema();
                sample
                    .iter()
                    .flat_map(|r| wide::unpivot(r, spec, &out_schema))
                    .take(SAMPLE_ROWS)
                    .map(|r| r.get(&mapping.timestamp).to_owned())
                    .collect()
            }
            None => sample
                .iter()
                .map(|r| r.get(&mapping.timestamp).to_owned())
                .collect(),
        };

        Ok(Plan {
            case_attributes: mapping.case_attributes.clone(),
            mapping,
            wide,
            sample_stamps,
            unpivoted_from,
        })
    }
}

/// Streams the file, unpivoting on the way through when the source is wide.
fn stream<'a>(
    path: &Path,
    plan: &'a Plan,
) -> Result<Box<dyn Iterator<Item = Result<pm_ingest::Row, QueryError>> + 'a>, QueryError> {
    let (_, rows) = pm_ingest::stream_any(path)?;
    match &plan.wide {
        None => Ok(Box::new(rows.map(|r| r.map_err(QueryError::from)))),
        Some(spec) => {
            let out_schema = spec.output_schema();
            Ok(Box::new(rows.flat_map(move |r| {
                match r {
                    Ok(row) => wide::unpivot(&row, spec, &out_schema)
                        .into_iter()
                        .map(Ok)
                        .collect::<Vec<_>>(),
                    Err(e) => vec![Err(QueryError::from(e))],
                }
            })))
        }
    }
}

/// Counts activity labels in a streaming pass, for the near-duplicate merge.
fn label_merges(
    path: &Path,
    plan: &Plan,
) -> Result<std::collections::BTreeMap<String, String>, QueryError> {
    let mut counts: std::collections::HashMap<String, usize> = Default::default();
    for row in stream(path, plan)? {
        *counts
            .entry(row?.get(&plan.mapping.activity).to_owned())
            .or_insert(0) += 1;
    }
    Ok(pm_ingest::validate::merge_map(&counts))
}

/// Analyses an already-read table.
///
/// Kept for small in-memory tables and tests. The product path is
/// [`analyze_filtered`], which streams.
pub fn analyze_table_filtered(
    table: &Table,
    source: &str,
    mapping: Option<Mapping>,
    policy: &Policy,
    rules: Rules,
    filter: Filter,
) -> Result<Outcome, QueryError> {
    let plan = Plan::build(&table.schema, &table.rows, mapping)?;

    let rows: Vec<pm_ingest::Row> = match &plan.wide {
        None => table.rows.clone(),
        Some(spec) => wide::unpivot_all(&table.rows, spec),
    };

    let report = validate(&rows, &plan.mapping);
    if !policy.resolves(&report) {
        return Ok(Outcome::Blocked(Box::new(Blocked {
            report,
            mapping: plan.mapping,
        })));
    }

    let rules_for_report = rules.clone();
    let normalized = normalize_with(&rows, &plan.mapping, &report, policy, rules)?;
    Ok(assemble(
        source,
        &plan,
        &report,
        &normalized,
        &filter,
        &rules_for_report,
    ))
}

/// Turns a normalized log into the reported analysis.
///
/// Shared by the streaming and in-memory paths so they cannot drift apart and
/// report different numbers for the same file.
fn assemble(
    source: &str,
    plan: &Plan,
    report: &Report,
    normalized: &pm_ingest::Normalized,
    filter: &Filter,
    rules_used: &Rules,
) -> Outcome {
    let filtered = filter.apply(&normalized.log);
    let view = View {
        cases_shown: filtered.log.case_count(),
        cases_total: filtered.cases_before,
        events_total: filtered.events_before,
        percent_shown: filtered.cases_kept_percent(),
        narrowed: filtered.narrowed(),
    };
    let log = &filtered.log;
    let dfg = Dfg::build(log);
    let variants = Variants::build(log);
    let rework = summarize_rework(log);
    let mapping = &plan.mapping;

    let (nodes, edges) = map_elements(log, &dfg);

    // Derive the bottleneck result from the same ranked public edges used by
    // the performance overlay. This makes it impossible for the two surfaces
    // to disagree about ties or eligibility.
    let mut bottlenecks: Vec<Bottleneck> = edges
        .iter()
        .filter_map(|edge| {
            edge.performance_rank.map(|rank| Bottleneck {
                rank,
                from: edge.from.clone(),
                to: edge.to.clone(),
                total_seconds: edge.total_seconds,
                mean_seconds: edge.mean_seconds,
                total_occurrences: edge.frequency,
                duration_eligible_occurrences: edge.duration_eligible_occurrences,
                share_percent: edge.performance_share_percent,
                example_cases: edge.example_cases.clone(),
            })
        })
        .collect();
    bottlenecks.sort_by_key(|bottleneck| bottleneck.rank);
    bottlenecks.truncate(10);

    let variant_list = variants
        .variants
        .iter()
        .take(20)
        .map(|v| VariantSummary {
            sequence: v
                .sequence
                .iter()
                .map(|a| log.activity_label(*a).to_owned())
                .collect(),
            cases: v.case_count(),
            percent: if variants.case_count > 0 {
                v.case_count() as f64 / variants.case_count as f64 * 100.0
            } else {
                0.0
            },
            duration_eligible_cases: v.duration_cases,
            mean_throughput_seconds: v.mean_throughput() / MICROS_PER_SECOND,
            example_cases: v
                .cases
                .iter()
                .take(3)
                .map(|c| log.case_label(*c).to_owned())
                .collect(),
        })
        .collect();

    let mut notes: Vec<String> = report
        .at_least(Severity::Info)
        .iter()
        .map(|f| format!("{}: {}", f.kind.title(), f.detail))
        .collect();
    notes.extend(order_notes(
        normalized.applied.format.as_deref(),
        &dfg,
        log.event_count().saturating_sub(log.case_count()),
    ));

    let offers = report
        .findings
        .iter()
        .filter(|f| f.offers_choice() && !f.is_blocking())
        .map(|f| {
            let d = f.decision.as_ref().expect("offers_choice");
            Offer {
                flag: flag_for(&f.kind).to_owned(),
                question: d.question.clone(),
                options: d
                    .options
                    .iter()
                    .map(|o| (o.id.to_owned(), o.label.clone()))
                    .collect(),
            }
        })
        .collect();

    let organization = log.has_resources().then(|| {
        let org = Organization::build(log);
        let total = org.total_handover_time();
        OrganizationSummary {
            resources: org.resource_count(),
            fragmentation_percent: org.fragmentation_percent(),
            handovers: org
                .slowest_handovers()
                .into_iter()
                .take(10)
                .map(|((from, to), h)| HandoverSummary {
                    from: log.resource_label(from).to_owned(),
                    to: log.resource_label(to).to_owned(),
                    occurrences: h.frequency,
                    cases: h.case_frequency,
                    duration_eligible_occurrences: h.duration_frequency,
                    mean_seconds: h.mean_duration() / MICROS_PER_SECOND,
                    total_seconds: h.total_duration / MICROS_PER_SECOND,
                    share_percent: if total > 0 {
                        h.total_duration as f64 / total as f64 * 100.0
                    } else {
                        0.0
                    },
                    example_cases: h
                        .examples
                        .iter()
                        .take(3)
                        .map(|case| log.case_label(*case).to_owned())
                        .collect(),
                })
                .collect(),
        }
    });

    let mut analysis = Box::new(Analysis {
        recorded_window: log
            .traces
            .iter()
            .flat_map(|trace| trace.events.iter().map(|event| event.at))
            .min()
            .zip(
                log.traces
                    .iter()
                    .flat_map(|trace| trace.events.iter().map(|event| event.at))
                    .max(),
            ),
        source: source.to_owned(),
        case_column: mapping.case_id.clone(),
        activity_column: mapping.activity.clone(),
        timestamp_column: mapping.timestamp.clone(),
        case_attributes: plan.case_attributes.clone(),
        mapping: mapping.clone(),
        optional_columns: [
            ("event order", &mapping.event_order),
            ("end timestamp", &mapping.end_timestamp),
            ("resource", &mapping.resource),
            ("lifecycle", &mapping.lifecycle),
            ("cost", &mapping.cost),
        ]
        .into_iter()
        .filter_map(|(field, column)| column.clone().map(|c| (field.to_owned(), c)))
        .collect(),
        timestamp_format: normalized.applied.format.clone(),
        unpivoted_from: plan.unpivoted_from.clone(),
        excluded_events: normalized.applied.excluded_events,
        excluded_cases: normalized.applied.excluded_cases,
        view,
        measures_true_waiting: dfg.measures_true_waiting,
        summary: summarize(log, &variants),
        timing: crate::timing::summarize_timing(log, &variants),
        nodes,
        edges,
        bottlenecks,
        rework,
        variants: variant_list,
        notes,
        offers,
        organization,
        targeting: {
            let report = crate::targeting::build_automation_report(log, rules_used, source);
            // Nothing useful to say about a log with one path and no resources.
            (!mapping.object_history && (report.resources_mapped || !report.divergences.is_empty()))
                .then_some(report)
        },
        window: rules_used
            .observation_window
            .as_ref()
            .map(|window| window.summarize(log)),
        number_fields: pm_core::number_fields(log),
        people: {
            let mut people: Vec<String> = log
                .resources
                .labels()
                .into_iter()
                .map(str::to_owned)
                .collect();
            people.sort();
            people
        },
        step_level_fields: log
            .step_level_attributes()
            .into_iter()
            .filter_map(|index| log.attribute_names.get(index).cloned())
            .collect(),
        key_findings: Vec::new(),
        drivers: if mapping.object_history {
            crate::drivers::Drivers::default()
        } else {
            crate::drivers::item_drivers(log, 6)
        },
        step_drivers: Vec::new(),
        skipped_steps: Vec::new(),
        process: None,
    });
    if !mapping.object_history {
        analysis.step_drivers =
            crate::patterns::step_drivers(log, &crate::findings::edges_worth_explaining(&analysis));
        analysis.skipped_steps = crate::patterns::skipped_steps(log, 3);
    }
    analysis.process = (!mapping.object_history)
        .then(|| {
            crate::process::recognise(
                analysis
                    .nodes
                    .iter()
                    .map(|node| node.activity.as_str())
                    .chain([analysis.case_column.as_str()]),
            )
        })
        .flatten();
    analysis.key_findings = crate::findings::key_findings(&analysis, KEY_FINDINGS);
    crate::findings::attach_values(&mut analysis.key_findings, log, &analysis.number_fields);
    Outcome::Complete(analysis)
}

/// Says when step order rests on file order rather than recorded times.
///
/// Day-only timestamps cannot order two steps on the same day, and identical
/// full timestamps cannot either. The engine still needs one sequence, so it
/// falls back to the file row; these notes keep that from reading as evidence.
fn order_notes(format: Option<&str>, dfg: &Dfg, transitions: usize) -> Vec<String> {
    let mut notes = Vec::new();
    let day_only = format.is_some_and(|id| {
        pm_ingest::timestamp::all()
            .iter()
            .any(|f| f.id == id && f.shape == pm_ingest::timestamp::Shape::DateOnly)
    });
    if day_only {
        notes.push(
            "Day-only times: timestamps record the date but not the time. Steps on the same \
             day are placed in file order, which is not evidence of which came first."
                .to_owned(),
        );
    }
    let same_time: u64 = dfg.edges.values().map(|edge| edge.same_time).sum();
    if same_time > 0 && transitions > 0 {
        notes.push(format!(
            "Same recorded time: {same_time} of {transitions} consecutive steps share their \
             timestamp ({:.1}%). Their order follows the file, not the clock.",
            same_time as f64 / transitions as f64 * 100.0
        ));
    }
    notes
}

/// The public process-map elements for a log: every activity and every
/// directly-follows connection, with the overlay ranks every front end uses.
pub(crate) fn map_elements(log: &EventLog, dfg: &Dfg) -> (Vec<Node>, Vec<Edge>) {
    let nodes = dfg
        .nodes
        .iter()
        .map(|(a, n)| Node {
            activity: log.activity_label(*a).to_owned(),
            frequency: n.frequency,
            case_frequency: n.case_frequency,
            starts: n.starts,
            ends: n.ends,
            example_cases: dfg.example_cases_for_activity(log, *a, 3),
        })
        .collect();
    let edges = ranked_overlay_edges(
        dfg.edges
            .iter()
            .map(|((from, to), e)| {
                (
                    Edge {
                        from: log.activity_label(*from).to_owned(),
                        to: log.activity_label(*to).to_owned(),
                        frequency: e.frequency,
                        case_frequency: e.case_frequency,
                        duration_eligible_occurrences: e.duration_frequency,
                        mean_seconds: e.mean_duration() / MICROS_PER_SECOND,
                        total_seconds: e.total_duration / MICROS_PER_SECOND,
                        min_seconds: if e.duration_frequency == 0 {
                            0
                        } else {
                            e.min_duration / MICROS_PER_SECOND
                        },
                        max_seconds: if e.duration_frequency == 0 {
                            0
                        } else {
                            e.max_duration / MICROS_PER_SECOND
                        },
                        frequency_share_percent: 0.0,
                        performance_share_percent: 0.0,
                        frequency_rank: 0,
                        performance_rank: None,
                        example_cases: dfg.example_cases(log, (*from, *to), 3),
                        same_time_occurrences: e.same_time,
                    },
                    e.total_duration,
                )
            })
            .collect(),
    );
    (nodes, edges)
}

fn summarize_rework(log: &EventLog) -> ReworkSummary {
    let rework = CoreRework::build(log);
    let mut activities: Vec<ReworkActivitySummary> = rework
        .activities
        .iter()
        .map(|(activity, stats)| ReworkActivitySummary {
            activity: log.activity_label(*activity).to_owned(),
            repetitions: stats.repetitions,
            immediate_repetitions: stats.immediate_repetitions,
            returns_after_intervening: stats.returns_after_intervening,
            cases_affected: stats.case_frequency,
            duration_eligible_occurrences: stats.duration_frequency,
            mean_repeat_interval_seconds: stats.mean_repeat_interval() / MICROS_PER_SECOND,
            total_repeat_interval_seconds: stats.total_repeat_interval / MICROS_PER_SECOND,
            example_cases: stats
                .examples
                .iter()
                .take(3)
                .map(|case| log.case_label(*case).to_owned())
                .collect(),
        })
        .collect();
    activities.sort_by(|left, right| {
        right
            .repetitions
            .cmp(&left.repetitions)
            .then_with(|| left.activity.cmp(&right.activity))
    });

    let mut loops: Vec<LoopSummary> = rework
        .loops
        .iter()
        .map(|(sequence, stats)| LoopSummary {
            sequence: sequence
                .iter()
                .map(|activity| log.activity_label(*activity).to_owned())
                .collect(),
            occurrences: stats.occurrences,
            cases_affected: stats.case_frequency,
            duration_eligible_occurrences: stats.duration_frequency,
            mean_duration_seconds: stats.mean_duration() / MICROS_PER_SECOND,
            total_duration_seconds: stats.total_duration / MICROS_PER_SECOND,
            example_cases: stats
                .examples
                .iter()
                .take(3)
                .map(|case| log.case_label(*case).to_owned())
                .collect(),
        })
        .collect();
    loops.sort_by(|left, right| {
        right
            .occurrences
            .cmp(&left.occurrences)
            .then_with(|| left.sequence.cmp(&right.sequence))
    });

    ReworkSummary {
        cases_affected: rework.cases_with_rework,
        case_percent: if log.case_count() > 0 {
            rework.cases_with_rework as f64 / log.case_count() as f64 * 100.0
        } else {
            0.0
        },
        repeated_activity_occurrences: rework.repeated_activity_occurrences(),
        immediate_repetitions: rework.immediate_repetitions(),
        returns_after_intervening: rework.returns_after_intervening(),
        loop_traversals: rework.loop_traversals(),
        omitted_loop_candidates: rework.omitted_loop_candidates,
        activities,
        loops,
    }
}

/// Adds the two process-map overlay orderings without duplicating the graph.
///
/// Edges remain in their canonical label order; ranks tell every front end how
/// to order and emphasize them for either view. This keeps large JSON results
/// compact and makes the CLI, browser, and MCP agree exactly.
fn ranked_overlay_edges(mut edges: Vec<(Edge, Micros)>) -> Vec<Edge> {
    let frequency_total: u64 = edges.iter().map(|(edge, _)| edge.frequency).sum();
    let performance_total: Micros = edges.iter().map(|(_, micros)| *micros).sum();

    for (edge, duration_micros) in &mut edges {
        edge.frequency_share_percent = if frequency_total > 0 {
            edge.frequency as f64 / frequency_total as f64 * 100.0
        } else {
            0.0
        };
        edge.performance_share_percent = if performance_total > 0 {
            *duration_micros as f64 / performance_total as f64 * 100.0
        } else {
            0.0
        };
    }

    let mut frequency_order: Vec<usize> = (0..edges.len()).collect();
    frequency_order.sort_by(|left, right| {
        edges[*right]
            .0
            .frequency
            .cmp(&edges[*left].0.frequency)
            .then_with(|| edges[*left].0.from.cmp(&edges[*right].0.from))
            .then_with(|| edges[*left].0.to.cmp(&edges[*right].0.to))
    });
    for (rank, index) in frequency_order.into_iter().enumerate() {
        edges[index].0.frequency_rank = rank + 1;
    }

    let mut performance_order: Vec<usize> = (0..edges.len())
        .filter(|index| edges[*index].0.duration_eligible_occurrences > 0)
        .collect();
    performance_order.sort_by(|left, right| {
        edges[*right]
            .1
            .cmp(&edges[*left].1)
            .then_with(|| edges[*left].0.from.cmp(&edges[*right].0.from))
            .then_with(|| edges[*left].0.to.cmp(&edges[*right].0.to))
    });
    for (rank, index) in performance_order.into_iter().enumerate() {
        edges[index].0.performance_rank = Some(rank + 1);
    }

    edges.into_iter().map(|(edge, _)| edge).collect()
}

fn summarize(log: &EventLog, variants: &Variants) -> Summary {
    // Cases the user marked as having missing data are excluded from duration
    // statistics: their recorded span understates reality, so including them
    // would drag every average down by an unknown amount. They still appear in
    // the map, the variants, and the counts — only the durations they cannot
    // support are withheld.
    let mut throughputs: Vec<Micros> = log
        .traces
        .iter()
        .filter(|t| !t.incomplete)
        .map(|t| t.throughput())
        .collect();
    throughputs.sort_unstable();
    let mean = if throughputs.is_empty() {
        0
    } else {
        throughputs.iter().sum::<Micros>() / throughputs.len() as i64
    };
    let median = if throughputs.is_empty() {
        0
    } else {
        throughputs[throughputs.len() / 2]
    };
    let p90 = if throughputs.is_empty() {
        0
    } else {
        throughputs[(throughputs.len() * 9).div_ceil(10) - 1]
    };

    Summary {
        events: log.event_count(),
        cases: log.case_count(),
        activities: log.activity_count(),
        variants: variants.len(),
        single_event_cases: log.single_event_cases(),
        unique_variants: variants.unique_variants(),
        top_five_coverage_percent: variants.coverage(5),
        variants_for_80_percent: variants.variants_needed_for(80.0),
        mean_throughput_seconds: mean / MICROS_PER_SECOND,
        median_throughput_seconds: median / MICROS_PER_SECOND,
        p90_throughput_seconds: p90 / MICROS_PER_SECOND,
        duration_eligible_cases: throughputs.len(),
        incomplete_cases: log.traces.iter().filter(|t| t.incomplete).count(),
        total_cost_microunits: log
            .has_costs()
            .then(|| log.costs.values().map(|cost| i128::from(*cost)).sum()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_ingest::Kind;
    use std::io::Write;

    fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("pm_query_{name}.csv"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    fn write_xes(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("pm_query_{name}.xes"));
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// The canonical sequential log, whose expected values were derived by
    /// hand in `testdata/canonical/README.md`.
    const SEQUENTIAL: &str = "case_id,activity,timestamp\n\
        ORDER-1,Receive Order,2026-03-02 09:00:00\n\
        ORDER-1,Check Credit,2026-03-02 11:00:00\n\
        ORDER-1,Approve Order,2026-03-03 09:00:00\n\
        ORDER-1,Ship Order,2026-03-03 15:00:00\n\
        ORDER-2,Receive Order,2026-03-02 10:00:00\n\
        ORDER-2,Check Credit,2026-03-02 14:00:00\n\
        ORDER-2,Approve Order,2026-03-04 09:00:00\n\
        ORDER-2,Ship Order,2026-03-04 17:00:00\n\
        ORDER-3,Receive Order,2026-03-03 08:00:00\n\
        ORDER-3,Check Credit,2026-03-03 09:00:00\n\
        ORDER-3,Approve Order,2026-03-03 16:00:00\n\
        ORDER-3,Ship Order,2026-03-04 10:00:00\n";

    fn complete(path: &std::path::Path, policy: &Policy) -> Analysis {
        match analyze(path, None, policy).unwrap() {
            Outcome::Complete(a) => *a,
            Outcome::Blocked(b) => panic!("unexpectedly blocked: {:?}", b.report),
        }
    }

    #[test]
    fn timestamp_reading_uses_the_applied_format_when_no_decision_was_needed() {
        let offset = timestamp::all()
            .iter()
            .find(|format| format.id == "iso_t_offset_frac")
            .unwrap();
        let naive = timestamp::all()
            .iter()
            .find(|format| format.id == "iso_space_seconds")
            .unwrap();
        assert_eq!(
            timestamp_reading(&Policy::new(), offset),
            "converted to UTC using the recorded offset"
        );
        assert_eq!(
            timestamp_reading(&Policy::new(), naive),
            "exactly as recorded, with no timezone applied"
        );
    }

    #[test]
    fn quality_report_is_available_before_and_after_a_decision() {
        let path = write_temp(
            "quality",
            "case_id,activity,timestamp\nC1,Receive,2026-03-02 09:00:00\nC1,Review,\n",
        );
        let open = assess_quality(&path, None, &Policy::new()).unwrap();
        assert!(!open.ready_for_analysis);
        assert_eq!(open.summary.open_decisions, 1);
        assert_eq!(open.findings[0].affected_rows, 1);

        let answered = assess_quality(
            &path,
            None,
            &Policy::new().answer(Kind::BlankTimestamps, "missing_data"),
        )
        .unwrap();
        assert!(answered.ready_for_analysis);
        assert_eq!(answered.summary.open_decisions, 0);
        assert_eq!(
            answered.findings[0]
                .decision
                .as_ref()
                .and_then(|decision| decision.answer.as_deref()),
            Some("missing_data")
        );
    }

    #[test]
    fn xes_flows_through_quality_and_analysis_without_a_manual_mapping() {
        let path = write_xes(
            "end_to_end",
            r#"<log xes.version="1.0">
  <trace>
    <string key="concept:name" value="ORDER-1"/>
    <string key="region" value="EMEA"/>
    <event>
      <string key="concept:name" value="Receive"/>
      <date key="time:timestamp" value="2026-03-02T09:00:00.000+00:00"/>
      <string key="org:resource" value="alice"/>
      <float key="cost:total" value="12.50"/>
    </event>
    <event>
      <string key="concept:name" value="Ship"/>
      <date key="time:timestamp" value="2026-03-02T11:00:00.000+00:00"/>
      <string key="org:resource" value="bob"/>
      <float key="cost:total" value="2.25"/>
    </event>
  </trace>
</log>"#,
        );

        let quality = assess_quality(&path, None, &Policy::new()).unwrap();
        assert!(quality.ready_for_analysis);
        assert_eq!(quality.mapping.resource.as_deref(), Some("resource"));
        assert_eq!(quality.mapping.cost.as_deref(), Some("cost"));
        assert_eq!(quality.mapping.case_attributes, vec!["region"]);

        let analysis = complete(&path, &Policy::new());
        assert_eq!(analysis.summary.events, 2);
        assert_eq!(analysis.summary.cases, 1);
        assert_eq!(analysis.summary.total_cost_microunits, Some(14_750_000));
        assert_eq!(analysis.case_attributes, vec!["region"]);
        assert_eq!(analysis.organization.as_ref().unwrap().resources, 2);
        assert_eq!(analysis.edges[0].from, "Receive");
        assert_eq!(analysis.edges[0].to, "Ship");
    }

    fn canonical(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/canonical")
            .join(name)
    }

    #[test]
    fn self_loop_fixture_reports_first_class_rework() {
        let analysis = complete(&canonical("02-self-loop.csv"), &Policy::new());
        assert_eq!(analysis.rework.cases_affected, 2);
        assert_eq!(analysis.rework.case_percent, 2.0 / 3.0 * 100.0);
        assert_eq!(analysis.rework.repeated_activity_occurrences, 3);
        assert_eq!(analysis.rework.immediate_repetitions, 3);
        assert_eq!(analysis.rework.returns_after_intervening, 0);
        assert_eq!(analysis.rework.loop_traversals, 3);
        assert_eq!(analysis.rework.activities[0].activity, "Investigate");
        assert_eq!(analysis.rework.activities[0].cases_affected, 2);
        assert_eq!(
            analysis.rework.loops[0].sequence,
            ["Investigate", "Investigate"]
        );
        assert_eq!(analysis.rework.loops[0].occurrences, 3);
    }

    #[test]
    fn long_loop_fixture_reports_observed_paths_not_only_self_loops() {
        let analysis = complete(&canonical("03-long-loop.csv"), &Policy::new());
        assert_eq!(analysis.rework.cases_affected, 2);
        assert_eq!(analysis.rework.immediate_repetitions, 0);
        assert_eq!(analysis.rework.returns_after_intervening, 4);
        assert_eq!(analysis.rework.loop_traversals, 3);
        assert_eq!(analysis.rework.loops.len(), 1);
        assert_eq!(
            analysis.rework.loops[0].sequence,
            ["Review", "Request Documents", "Review"]
        );
        assert_eq!(analysis.rework.loops[0].occurrences, 3);
        assert_eq!(analysis.rework.loops[0].cases_affected, 2);
    }

    #[test]
    fn parallel_fixture_does_not_invent_rework_from_a_dfg_cycle() {
        let analysis = complete(&canonical("04-parallel.csv"), &Policy::new());
        assert_eq!(analysis.rework.cases_affected, 0);
        assert_eq!(analysis.rework.loop_traversals, 0);
        assert!(analysis.rework.activities.is_empty());
        assert!(analysis.rework.loops.is_empty());
    }

    #[test]
    fn incomplete_rework_counts_shape_but_excludes_its_duration() {
        let path = write_temp(
            "incomplete_rework",
            "case_id,activity,timestamp\n\
             C1,A,2026-03-02 09:00:00\nC1,Missing,\nC1,A,2026-03-02 11:00:00\n",
        );
        let analysis = complete(
            &path,
            &Policy::new().answer(Kind::BlankTimestamps, "missing_data"),
        );
        assert_eq!(analysis.rework.loop_traversals, 1);
        assert_eq!(analysis.rework.loops[0].occurrences, 1);
        assert_eq!(analysis.rework.loops[0].duration_eligible_occurrences, 0);
        assert_eq!(analysis.rework.loops[0].mean_duration_seconds, 0);
        assert_eq!(
            analysis.rework.activities[0].duration_eligible_occurrences,
            0
        );
    }

    /// Every number here is from the hand-derived canonical expectations.
    #[test]
    fn reproduces_the_hand_derived_sequential_numbers() {
        let p = write_temp("sequential", SEQUENTIAL);
        let a = complete(&p, &Policy::new());

        assert_eq!(a.summary.events, 12);
        assert_eq!(a.summary.cases, 3);
        assert_eq!(a.summary.activities, 4);
        assert_eq!(a.summary.variants, 1);
        assert_eq!(a.edges.len(), 3);

        // Throughput: 30h, 55h, 26h — mean 37h, median 30h.
        assert_eq!(a.summary.mean_throughput_seconds, 37 * 3600);
        assert_eq!(a.summary.median_throughput_seconds, 30 * 3600);
        // Nearest-rank P90 of three cases is the slowest: 55h.
        assert_eq!(a.summary.p90_throughput_seconds, 55 * 3600);

        // The bottleneck is the wait before Approve Order: mean 24h.
        let top = &a.bottlenecks[0];
        assert_eq!(top.rank, 1);
        assert_eq!(top.from, "Check Credit");
        assert_eq!(top.to, "Approve Order");
        assert_eq!(top.mean_seconds, 24 * 3600);
        assert_eq!(top.total_seconds, 72 * 3600);
        assert_eq!(top.total_occurrences, 3);
        assert_eq!(top.duration_eligible_occurrences, 3);
    }

    #[test]
    fn frequency_and_performance_overlays_rank_independently() {
        let path = write_temp(
            "overlays",
            "case_id,activity,timestamp\n\
             C1,A,2026-03-02 09:00:00\nC1,B,2026-03-02 10:00:00\n\
             C2,A,2026-03-02 09:00:00\nC2,B,2026-03-02 10:00:00\n\
             C3,A,2026-03-02 09:00:00\nC3,B,2026-03-02 10:00:00\n\
             C4,A,2026-03-02 09:00:00\nC4,C,2026-03-02 19:00:00\n",
        );
        let analysis = complete(&path, &Policy::new());
        let frequent = analysis.edges.iter().find(|edge| edge.to == "B").unwrap();
        let slow = analysis.edges.iter().find(|edge| edge.to == "C").unwrap();

        assert_eq!(frequent.frequency_rank, 1);
        assert_eq!(slow.frequency_rank, 2);
        assert_eq!(slow.performance_rank, Some(1));
        assert_eq!(frequent.performance_rank, Some(2));
        assert_eq!(frequent.frequency_share_percent, 75.0);
        assert_eq!(slow.performance_share_percent, 10.0 / 13.0 * 100.0);
        assert_eq!(frequent.duration_eligible_occurrences, 3);
        assert_eq!(slow.duration_eligible_occurrences, 1);
        assert_eq!(analysis.bottlenecks[0].rank, 1);
        assert_eq!(analysis.bottlenecks[0].to, slow.to);
    }

    #[test]
    fn bottleneck_ranks_share_the_public_label_tie_break() {
        let path = write_temp(
            "bottleneck_tie",
            "case_id,activity,timestamp\n\
             C1,Z,2026-03-02 09:00:00\nC1,B,2026-03-02 10:00:00\n\
             C2,A,2026-03-02 09:00:00\nC2,C,2026-03-02 10:00:00\n",
        );
        let analysis = complete(&path, &Policy::new());

        assert_eq!(analysis.bottlenecks.len(), 2);
        assert_eq!(analysis.bottlenecks[0].rank, 1);
        assert_eq!(analysis.bottlenecks[0].from, "A");
        assert_eq!(analysis.bottlenecks[1].rank, 2);
        assert_eq!(analysis.bottlenecks[1].from, "Z");
        for bottleneck in &analysis.bottlenecks {
            let overlay = analysis
                .edges
                .iter()
                .find(|edge| edge.from == bottleneck.from && edge.to == bottleneck.to)
                .unwrap();
            assert_eq!(overlay.performance_rank, Some(bottleneck.rank));
        }
    }

    #[test]
    fn incomplete_cases_shape_frequency_but_not_performance_overlays() {
        let path = write_temp(
            "overlay_eligibility",
            "case_id,activity,timestamp\n\
             C1,A,2026-03-02 09:00:00\nC1,B,\nC1,C,2026-03-02 11:00:00\n",
        );
        let analysis = complete(
            &path,
            &Policy::new().answer(Kind::BlankTimestamps, "missing_data"),
        );
        let edge = &analysis.edges[0];

        assert_eq!(edge.frequency, 1);
        assert_eq!(edge.frequency_rank, 1);
        assert_eq!(edge.frequency_share_percent, 100.0);
        assert_eq!(edge.duration_eligible_occurrences, 0);
        assert_eq!(edge.performance_rank, None);
        assert_eq!(edge.performance_share_percent, 0.0);
    }

    #[test]
    fn duration_eligibility_denominators_reconcile_across_results() {
        let path = write_temp(
            "eligibility_denominators",
            "case_id,activity,timestamp,resource\n\
             C1,A,2026-03-02 09:00:00,alice\n\
             C1,B,2026-03-02 10:00:00,bob\n\
             C2,A,2026-03-02 09:00:00,alice\n\
             C2,Missing,,alice\n\
             C2,B,2026-03-02 11:00:00,bob\n",
        );
        let analysis = complete(
            &path,
            &Policy::new().answer(Kind::BlankTimestamps, "missing_data"),
        );

        assert_eq!(analysis.summary.cases, 2);
        assert_eq!(analysis.summary.duration_eligible_cases, 1);
        assert_eq!(analysis.summary.incomplete_cases, 1);

        let variant = &analysis.variants[0];
        assert_eq!(variant.cases, 2);
        assert_eq!(variant.duration_eligible_cases, 1);

        let edge = &analysis.edges[0];
        assert_eq!(edge.frequency, 2);
        assert_eq!(edge.duration_eligible_occurrences, 1);

        let bottleneck = &analysis.bottlenecks[0];
        assert_eq!(bottleneck.total_occurrences, 2);
        assert_eq!(bottleneck.duration_eligible_occurrences, 1);

        let handover = &analysis.organization.as_ref().unwrap().handovers[0];
        assert_eq!(handover.occurrences, 2);
        assert_eq!(handover.duration_eligible_occurrences, 1);
    }

    #[test]
    fn performance_overlays_keep_subsecond_precision() {
        let path = write_temp(
            "overlay_subseconds",
            "case_id,activity,timestamp\n\
             C1,A,2026-03-02 09:00:00.000000\nC1,B,2026-03-02 09:00:00.900000\n\
             C2,A,2026-03-02 09:00:00.000000\nC2,C,2026-03-02 09:00:00.100000\n",
        );
        let analysis = complete(&path, &Policy::new());
        let slower = analysis.edges.iter().find(|edge| edge.to == "B").unwrap();
        let faster = analysis.edges.iter().find(|edge| edge.to == "C").unwrap();

        assert_eq!(
            slower.total_seconds, 0,
            "public durations use whole seconds"
        );
        assert_eq!(faster.total_seconds, 0);
        assert_eq!(slower.performance_rank, Some(1));
        assert_eq!(faster.performance_rank, Some(2));
        assert_eq!(slower.performance_share_percent, 90.0);
        assert_eq!(faster.performance_share_percent, 10.0);
    }

    #[test]
    fn proposes_a_mapping_without_being_told() {
        let p = write_temp("propose", SEQUENTIAL);
        let a = complete(&p, &Policy::new());
        assert_eq!(a.case_column, "case_id");
        assert_eq!(a.activity_column, "activity");
        assert_eq!(a.timestamp_column, "timestamp");
        assert_eq!(a.timestamp_format.as_deref(), Some("iso_space_seconds"));
    }

    /// Blocking is an outcome, not an error.
    #[test]
    fn blocks_rather_than_guessing() {
        let p = write_temp(
            "blocked",
            "case_id,activity,timestamp\nC1,A,2026-03-02 09:00:00\nC1,B,\n",
        );
        match analyze(&p, None, &Policy::new()).unwrap() {
            Outcome::Blocked(b) => assert!(b.report.has_blocking()),
            Outcome::Complete(_) => panic!("should have blocked on blank timestamps"),
        }
    }

    #[test]
    fn completes_once_the_policy_answers_the_question() {
        let p = write_temp(
            "answered",
            "case_id,activity,timestamp\nC1,A,2026-03-02 09:00:00\nC1,B,\nC1,C,2026-03-02 11:00:00\n",
        );
        let a = complete(
            &p,
            &Policy::new().answer(Kind::BlankTimestamps, "did_not_happen"),
        );
        assert_eq!(a.summary.events, 2);
    }

    /// Variant percentages must sum to 100 and case counts must reconcile.
    #[test]
    fn variant_shares_reconcile() {
        let p = write_temp(
            "variants",
            "case_id,activity,timestamp\n\
             A1,Submit,2026-07-01 09:00:00\nA1,Assess,2026-07-01 14:00:00\nA1,Accept,2026-07-02 09:00:00\n\
             A2,Submit,2026-07-01 10:00:00\nA2,Assess,2026-07-02 11:00:00\nA2,Reject,2026-07-02 15:00:00\n\
             A3,Submit,2026-07-02 08:00:00\nA3,Assess,2026-07-02 13:00:00\nA3,Accept,2026-07-03 09:00:00\n",
        );
        let a = complete(&p, &Policy::new());
        assert_eq!(a.summary.variants, 2);
        let cases: usize = a.variants.iter().map(|v| v.cases).sum();
        assert_eq!(cases, a.summary.cases);
        let pct: f64 = a.variants.iter().map(|v| v.percent).sum();
        assert!((pct - 100.0).abs() < 1e-9, "shares sum to {pct}");
    }

    /// Concurrency must be visible as two variants, and the DFG must show the
    /// cycle that no case executed.
    #[test]
    fn surfaces_concurrency_honestly() {
        let p = write_temp(
            "parallel",
            "case_id,activity,timestamp\n\
             O1,Receive,2026-06-01 09:00:00\nO1,Pick,2026-06-01 10:00:00\nO1,Print,2026-06-01 11:00:00\n\
             O2,Receive,2026-06-01 09:30:00\nO2,Print,2026-06-01 10:30:00\nO2,Pick,2026-06-01 12:00:00\n",
        );
        let a = complete(&p, &Policy::new());
        assert_eq!(a.summary.variants, 2);
        assert!(a.edges.iter().any(|e| e.from == "Pick" && e.to == "Print"));
        assert!(a.edges.iter().any(|e| e.from == "Print" && e.to == "Pick"));
    }

    #[test]
    fn bottleneck_shares_do_not_exceed_one_hundred() {
        let p = write_temp("shares", SEQUENTIAL);
        let a = complete(&p, &Policy::new());
        let total: f64 = a.bottlenecks.iter().map(|b| b.share_percent).sum();
        assert!(total <= 100.0 + 1e-9, "shares total {total}");
    }

    #[test]
    fn carries_evidence_case_identifiers() {
        let p = write_temp("evidence", SEQUENTIAL);
        let a = complete(&p, &Policy::new());
        assert!(a.variants[0].example_cases.contains(&"ORDER-1".to_owned()));
        assert!(a.edges.iter().all(|edge| !edge.example_cases.is_empty()));
        assert!(
            a.bottlenecks
                .iter()
                .all(|finding| !finding.example_cases.is_empty())
        );
    }

    #[test]
    fn reports_mapped_event_costs_in_the_summary() {
        let p = write_temp(
            "costs",
            "case_id,activity,timestamp,amount\n\
             C1,A,2026-03-02 09:00:00,12.50\n\
             C1,B,2026-03-02 10:00:00,0.125\n",
        );
        let mapping = Mapping::new("case_id", "activity", "timestamp").with_cost("amount");
        let Outcome::Complete(a) = analyze(&p, Some(mapping), &Policy::new()).unwrap() else {
            panic!("cost log should complete")
        };
        assert_eq!(a.summary.total_cost_microunits, Some(12_625_000));
    }

    /// Preparation rules must reach the analysis through the shared layer, so
    /// the CLI, browser, and MCP all get them.
    #[test]
    fn applies_preparation_rules() {
        let p = write_temp("rules", SEQUENTIAL);
        let outcome = analyze_rules(
            &p,
            None,
            &Policy::new(),
            Rules::new().exclude_activity("Check Credit"),
        )
        .unwrap();
        let Outcome::Complete(a) = outcome else {
            panic!("should complete")
        };
        assert_eq!(a.summary.events, 9);
        assert_eq!(a.excluded_events, 3);
        assert!(!a.nodes.iter().any(|n| n.activity == "Check Credit"));
    }

    /// Mapping a resource column must produce the organizational view without
    /// the user asking for it separately.
    #[test]
    fn produces_the_organizational_view_when_a_resource_is_mapped() {
        let p = write_temp(
            "handover",
            "case_id,activity,timestamp,resource\n\
             C1,Receive,2026-03-02 09:00:00,alice\n\
             C1,Check,2026-03-02 11:00:00,alice\n\
             C1,Approve,2026-03-03 11:00:00,bob\n\
             C2,Receive,2026-03-02 10:00:00,alice\n\
             C2,Check,2026-03-02 12:00:00,alice\n\
             C2,Approve,2026-03-04 12:00:00,bob\n",
        );
        let a = complete(&p, &Policy::new());
        assert_eq!(
            a.optional_columns,
            vec![("resource".to_owned(), "resource".to_owned())]
        );
        let org = a.organization.expect("resource column was mapped");
        assert_eq!(org.resources, 2);
        assert_eq!(org.handovers.len(), 1);
        assert_eq!(org.handovers[0].from, "alice");
        assert_eq!(org.handovers[0].to, "bob");
        assert_eq!(org.handovers[0].example_cases, vec!["C1", "C2"]);
        // 24h and 48h waits: 36h mean.
        assert_eq!(org.handovers[0].mean_seconds, 36 * 3600);
        // Four pairs, two change hands.
        assert!((org.fragmentation_percent - 50.0).abs() < 1e-9);
    }

    #[test]
    fn omits_the_organizational_view_without_a_resource_column() {
        let p = write_temp("noresource", SEQUENTIAL);
        assert!(complete(&p, &Policy::new()).organization.is_none());
    }

    /// A filter must reach the analysis through the shared layer, and the
    /// result must say how much it covers.
    #[test]
    fn applies_a_filter_and_reports_coverage() {
        let p = write_temp("filtered", SEQUENTIAL);
        let outcome = analyze_filtered(
            &p,
            None,
            &Policy::new(),
            Rules::new(),
            Filter::new().with_activity("Ship Order").min_duration(0),
        )
        .unwrap();
        let Outcome::Complete(a) = outcome else {
            panic!("should complete")
        };
        assert_eq!(a.view.cases_total, 3);
        assert_eq!(a.view.cases_shown, 3);
        assert_eq!(a.view.percent_shown, 100.0);
    }

    #[test]
    fn a_narrowing_filter_is_reported_as_such() {
        let p = write_temp("narrowed", SEQUENTIAL);
        let outcome = analyze_filtered(
            &p,
            None,
            &Policy::new(),
            Rules::new(),
            Filter::new().min_duration(40 * 3600 * 1_000_000),
        )
        .unwrap();
        let Outcome::Complete(a) = outcome else {
            panic!("should complete")
        };
        assert!(a.view.narrowed);
        assert_eq!(a.view.cases_shown, 1, "only ORDER-2 runs 55 hours");
        assert!((a.view.percent_shown - 33.333).abs() < 0.01);
    }

    #[test]
    fn an_unfiltered_analysis_covers_everything() {
        let p = write_temp("unfiltered", SEQUENTIAL);
        let a = complete(&p, &Policy::new());
        assert!(!a.view.narrowed);
        assert_eq!(a.view.percent_shown, 100.0);
    }

    #[test]
    fn is_deterministic() {
        let p = write_temp("determinism", SEQUENTIAL);
        assert_eq!(complete(&p, &Policy::new()), complete(&p, &Policy::new()));
    }

    #[test]
    fn reports_a_missing_file_as_an_error() {
        assert!(analyze("/nonexistent/file.csv", None, &Policy::new()).is_err());
    }

    /// A wide table must just work — it is the shape most real exports have.
    #[test]
    fn unpivots_a_wide_table_automatically() {
        let p = write_temp(
            "wide",
            "ticket_id,priority,created_at,assigned_at,resolved_at\n\
             T-1,High,2026-03-01 09:00:00,2026-03-01 11:00:00,2026-03-04 16:00:00\n\
             T-2,Low,2026-03-01 10:00:00,2026-03-02 09:00:00,\n",
        );
        let a = complete(&p, &Policy::new());
        assert_eq!(a.summary.events, 5, "blank cells produce no event");
        assert_eq!(a.summary.cases, 2);
        let columns = a.unpivoted_from.expect("should report the unpivot");
        assert!(columns.iter().any(|c| c.contains("created_at → Created")));
    }

    /// A table that is neither long nor wide must say so clearly rather than
    /// producing nonsense.
    #[test]
    fn rejects_a_table_that_is_neither_long_nor_wide() {
        let p = write_temp("neither", "col_a,col_b\nfoo,bar\nbaz,qux\n");
        assert!(matches!(
            analyze(&p, None, &Policy::new()),
            Err(QueryError::NoMapping { .. })
        ));
    }
}
