//! Label-resolved object-centric analysis shared by every front end.

use pm_core::{
    OcelAlignmentMove, OcelArcEndpoint, OcelDiscoveryConfig, OcelDiscoveryError, OcelFilter,
    OcelGraphs, OcelLog, OcelObjectTypeDiscovery, OcelPetriNet, OcelPlace, OcelPlaceId,
    OcelReplayDeviation, OcelReplayError, OcelTransition, OcelTransitionId, OcelView,
    align_ocel as align_core_ocel, analyze_ocel_synchronization as analyze_core_synchronization,
    discover_ocel_petri_net as discover_core_ocel_model, discover_ocel_variants,
    inspect_ocel_lifecycle, measure_ocel_time_between as measure_core_time_between,
    project_ocel as project_core_ocel, rank_ocel_bottlenecks as rank_core_bottlenecks,
    replay_ocel_tokens,
};
use pm_ingest::{OcelQualityReport, assess_ocel_quality, read_ocel};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum OcelQueryError {
    #[error(transparent)]
    Ingest(#[from] pm_ingest::OcelReadError),
    #[error(transparent)]
    Filter(#[from] pm_core::OcelFilterError),
    #[error("object '{0}' was not found")]
    UnknownObject(String),
    #[error("object '{0}' is outside the active object-centric view")]
    ObjectOutsideView(String),
    #[error("activity '{0}' was not found")]
    UnknownActivity(String),
    #[error("object type '{0}' was not found")]
    UnknownObjectType(String),
    #[error("object-centric performance is blocked by data-quality errors")]
    QualityBlocked,
    #[error("object-centric discovery thresholds must be between 0 and 1000 thousandths")]
    InvalidDiscoveryThreshold,
    #[error(
        "object '{object}' has several related events at timestamp {at_micros}; discovery will not invent an order within a simultaneous layer"
    )]
    SimultaneousObjectEvents { object: String, at_micros: i64 },
    #[error("object-centric model discovery failed: {0}")]
    Discovery(String),
    #[error("could not read the object-centric model: {0}")]
    ModelIo(#[source] std::io::Error),
    #[error("the object-centric model is not valid JSON: {0}")]
    ModelJson(#[source] serde_json::Error),
    #[error("object-centric conformance failed: {0}")]
    Conformance(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelCatalog {
    pub event_types: Vec<String>,
    pub object_types: Vec<String>,
    pub event_object_qualifiers: Vec<String>,
    pub object_object_qualifiers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelScope {
    pub events_shown: usize,
    pub events_total: usize,
    pub objects_shown: usize,
    pub objects_total: usize,
    pub event_object_relations_shown: usize,
    pub event_object_relations_total: usize,
    pub object_object_relations_shown: usize,
    pub object_object_relations_total: usize,
    pub narrowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelActivity {
    pub activity: String,
    pub events: u64,
    pub evidence_events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelDfgEvidence {
    pub object: String,
    pub source_event: String,
    pub target_event: String,
    pub source_at_micros: i64,
    pub target_at_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelDirectlyFollows {
    pub object_type: String,
    pub source: String,
    pub target: String,
    pub occurrences: u64,
    pub objects: u64,
    pub total_elapsed_micros: i64,
    pub mean_elapsed_micros: i64,
    pub min_elapsed_micros: i64,
    pub max_elapsed_micros: i64,
    pub evidence: Vec<LabelledOcelDfgEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelParticipation {
    pub activity: String,
    pub object_type: String,
    pub qualifier: String,
    pub occurrences: u64,
    pub evidence_events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelInteraction {
    pub left_object_type: String,
    pub right_object_type: String,
    pub events: u64,
    pub object_pairs: u64,
    pub evidence_events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelRelationshipEvidence {
    pub source_object: String,
    pub target_object: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelRelationship {
    pub source_object_type: String,
    pub target_object_type: String,
    pub qualifier: String,
    pub occurrences: u64,
    pub evidence: Vec<LabelledOcelRelationshipEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelGraphs {
    pub activities: Vec<LabelledOcelActivity>,
    pub directly_follows: Vec<LabelledOcelDirectlyFollows>,
    pub participation: Vec<LabelledOcelParticipation>,
    pub interactions: Vec<LabelledOcelInteraction>,
    pub relationships: Vec<LabelledOcelRelationship>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelVariantLayer {
    /// Simultaneous activities; array order is canonical, not temporal. A
    /// layer of very many (an item in hundreds of steps on one day) lists the
    /// first [`VARIANT_LAYER_ACTIVITIES_SHOWN`] and counts the rest.
    pub activities: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelVariant {
    pub object_type: String,
    pub rank: usize,
    /// The route's steps; a very long route (an item that takes part in
    /// most steps, such as a vendor) keeps its first steps and its last,
    /// and says how many were left out between them.
    pub layers: Vec<LabelledOcelVariantLayer>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub layers_omitted: usize,
    pub objects: u64,
    pub share_millionths: u64,
    pub duration_eligible_objects: u64,
    pub mean_duration_micros: Option<i64>,
    pub evidence_objects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelLifecycleEvent {
    pub event: String,
    pub activity: String,
    pub at_micros: i64,
    pub time_layer: usize,
    pub qualifiers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelLifecycle {
    pub source: String,
    pub object: String,
    pub object_type: String,
    pub duration_micros: Option<i64>,
    pub events: Vec<LabelledOcelLifecycleEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelTimeBetweenEvidence {
    pub object: String,
    pub source_event: String,
    pub target_event: String,
    pub source_at_micros: i64,
    pub target_at_micros: i64,
    pub elapsed_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelTimeBetween {
    pub object_type: String,
    pub observations: u64,
    pub objects: u64,
    pub mean_elapsed_micros: i64,
    pub median_elapsed_micros: i64,
    pub p90_elapsed_micros: i64,
    pub min_elapsed_micros: i64,
    pub max_elapsed_micros: i64,
    pub source_objects: u64,
    pub evidence: Vec<LabelledOcelTimeBetweenEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelTimeBetweenReport {
    pub source: String,
    pub filter: OcelFilter,
    pub from_activity: String,
    pub to_activity: String,
    pub measurements: Vec<LabelledOcelTimeBetween>,
    pub semantics: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelSynchronizationReadiness {
    pub object: String,
    pub object_type: String,
    pub prior_at_micros: i64,
    pub prior_events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelSynchronizationEvidence {
    pub event: String,
    pub at_micros: i64,
    pub participating_objects: u64,
    pub historically_observed_objects: u64,
    pub first_seen_objects: u64,
    pub arrival_spread_micros: i64,
    pub elapsed_after_last_prior_micros: i64,
    pub elapsed_from_first_prior_micros: i64,
    pub readiness: Vec<LabelledOcelSynchronizationReadiness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelSynchronization {
    pub activity: String,
    pub object_types: Vec<String>,
    pub multi_object_events: u64,
    pub eligible_events: u64,
    pub insufficient_history_events: u64,
    pub historically_observed_object_participations: u64,
    pub first_seen_object_participations: u64,
    pub total_arrival_spread_micros: i64,
    pub mean_arrival_spread_micros: Option<i64>,
    pub min_arrival_spread_micros: Option<i64>,
    pub max_arrival_spread_micros: Option<i64>,
    pub total_elapsed_after_last_prior_micros: i64,
    pub mean_elapsed_after_last_prior_micros: Option<i64>,
    pub min_elapsed_after_last_prior_micros: Option<i64>,
    pub max_elapsed_after_last_prior_micros: Option<i64>,
    pub total_elapsed_from_first_prior_micros: i64,
    pub mean_elapsed_from_first_prior_micros: Option<i64>,
    pub min_elapsed_from_first_prior_micros: Option<i64>,
    pub max_elapsed_from_first_prior_micros: Option<i64>,
    pub evidence: Vec<LabelledOcelSynchronizationEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelSynchronizationReport {
    pub source: String,
    pub filter: OcelFilter,
    pub activities: Vec<LabelledOcelSynchronization>,
    pub semantics: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelLifecycleBottleneck {
    pub rank: usize,
    pub object_type: String,
    pub source: String,
    pub target: String,
    pub occurrences: u64,
    pub objects: u64,
    pub total_elapsed_micros: i64,
    pub mean_elapsed_micros: i64,
    pub share_millionths: u64,
    pub evidence: Vec<LabelledOcelDfgEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelSynchronizationBottleneck {
    pub rank: usize,
    pub activity: String,
    pub object_types: Vec<String>,
    pub multi_object_events: u64,
    pub eligible_events: u64,
    pub insufficient_history_events: u64,
    pub total_arrival_spread_micros: i64,
    pub mean_arrival_spread_micros: i64,
    pub share_millionths: u64,
    pub evidence: Vec<LabelledOcelSynchronizationEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelBottleneckReport {
    pub source: String,
    pub filter: OcelFilter,
    pub lifecycle_candidates: u64,
    pub lifecycle_positive_candidates: u64,
    pub total_lifecycle_elapsed_micros: i64,
    pub lifecycle: Vec<LabelledOcelLifecycleBottleneck>,
    pub synchronization_eligible_activities: u64,
    pub synchronization_positive_activities: u64,
    pub total_arrival_spread_micros: i64,
    pub synchronization: Vec<LabelledOcelSynchronizationBottleneck>,
    pub semantics: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelProjectedEvent {
    pub source_event: String,
    pub activity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelProjectionLayer {
    pub at_micros: i64,
    /// Simultaneous events; array order is canonical, not temporal.
    pub events: Vec<LabelledOcelProjectedEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelProjectedCase {
    pub case_object: String,
    pub events_available: u64,
    pub events_returned: usize,
    pub truncated: bool,
    pub layers: Vec<LabelledOcelProjectionLayer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelConvergenceEvidence {
    pub source_event: String,
    pub activity: String,
    pub projected_case_count: u64,
    pub additional_occurrences: u64,
    pub evidence_cases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelDivergenceEvidence {
    pub projected_case: String,
    pub activity: String,
    pub related_object_type: String,
    pub projected_occurrences: u64,
    pub distinct_related_objects: u64,
    pub evidence_events: Vec<String>,
    pub evidence_objects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelProjectionReport {
    pub source: String,
    pub filter: OcelFilter,
    pub case_object_type: String,
    pub source_events: u64,
    pub projected_cases: u64,
    pub included_unique_events: u64,
    pub excluded_events: u64,
    pub projected_event_occurrences: u64,
    pub convergence_events: u64,
    pub convergence_additional_occurrences: u64,
    pub divergence_candidates: u64,
    pub cases_available: u64,
    pub cases_returned: usize,
    pub event_limit_per_case: usize,
    pub cases: Vec<LabelledOcelProjectedCase>,
    pub convergence_evidence: Vec<LabelledOcelConvergenceEvidence>,
    pub divergence_evidence: Vec<LabelledOcelDivergenceEvidence>,
    pub semantics: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelledOcelModelPlace {
    pub id: u32,
    pub object_type: String,
    pub initial: bool,
    pub final_place: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelledOcelModelArc {
    pub place: u32,
    /// Variable arcs represent observed one-to-many or many-to-one
    /// event/object participation for this activity and object type.
    pub variable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelledOcelModelTransition {
    pub id: u32,
    /// `None` denotes an unobservable routing transition.
    pub activity: Option<String>,
    pub inputs: Vec<LabelledOcelModelArc>,
    pub outputs: Vec<LabelledOcelModelArc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelledOcelModelObjectType {
    pub object_type: String,
    pub objects: u64,
    pub objects_with_events: u64,
    pub lifecycle_event_occurrences: u64,
    pub filtered_edges: usize,
    pub used_flower_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelModelReport {
    pub source: String,
    pub filter: OcelFilter,
    pub places: Vec<LabelledOcelModelPlace>,
    pub transitions: Vec<LabelledOcelModelTransition>,
    pub object_types: Vec<LabelledOcelModelObjectType>,
    pub variable_arcs: u64,
    pub fixed_arcs: u64,
    pub noise_threshold_milli: u16,
    pub variable_arc_threshold_milli: u16,
    pub semantics: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelConformanceObject {
    pub object: String,
    pub object_type: String,
    pub events: u64,
    pub fitness_millionths: u64,
    pub consumed_tokens: u64,
    pub produced_tokens: u64,
    pub missing_tokens: u64,
    pub remaining_tokens: u64,
    pub unmapped_events: u64,
    pub silent_firings: u64,
    pub lifecycle_fit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabelledOcelConformanceDeviation {
    ActivityNotInModel {
        event: String,
        activity: String,
        object: String,
    },
    MissingTokens {
        event: String,
        activity: String,
        object: String,
        count: u64,
    },
    FinalTokensMissing {
        object: String,
        count: u64,
    },
    TokensRemaining {
        object: String,
        count: u64,
    },
    MissingObjectParticipation {
        event: String,
        activity: Option<String>,
        object_type: String,
        variable: bool,
    },
    FixedArcMultiplicity {
        event: String,
        activity: Option<String>,
        object_type: String,
        observed_objects: u64,
    },
    UnexpectedObjectParticipation {
        event: String,
        activity: Option<String>,
        object_type: String,
        observed_objects: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelConformanceReport {
    pub source: String,
    /// The file or caller-supplied identity of the explicit baseline model.
    pub model_source: String,
    /// The source recorded when the baseline model was discovered.
    pub model_training_source: String,
    pub filter: OcelFilter,
    pub fitness_millionths: u64,
    pub lifecycle_fit_objects: u64,
    pub replayed_objects: u64,
    pub fit_events: u64,
    pub replayed_events: u64,
    pub out_of_scope_events: u64,
    pub consumed_tokens: u64,
    pub produced_tokens: u64,
    pub missing_tokens: u64,
    pub remaining_tokens: u64,
    pub unmapped_event_occurrences: u64,
    pub binding_violations: u64,
    pub objects_available: usize,
    pub objects_returned: usize,
    pub objects_truncated: bool,
    pub deviations_available: usize,
    pub deviations_returned: usize,
    pub deviations_truncated: bool,
    pub objects: Vec<LabelledOcelConformanceObject>,
    pub deviations: Vec<LabelledOcelConformanceDeviation>,
    pub semantics: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabelledOcelAlignmentDeviation {
    LogOnly { event: String, activity: String },
    VisibleModel { transition: u32, activity: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelledOcelAlignmentObject {
    pub object: String,
    pub object_type: String,
    pub events: u64,
    pub cost: u64,
    pub denominator: u64,
    pub fitness_millionths: u64,
    pub synchronous_moves: u64,
    pub invisible_moves: u64,
    pub log_moves: u64,
    pub visible_model_moves: u64,
    pub lifecycle_fit: bool,
    pub deviations_available: usize,
    pub deviations_returned: usize,
    pub deviations_truncated: bool,
    pub deviations: Vec<LabelledOcelAlignmentDeviation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelAlignmentConformanceReport {
    pub source: String,
    pub model_source: String,
    pub model_training_source: String,
    pub filter: OcelFilter,
    pub total_cost: u64,
    pub denominator: u64,
    pub fitness_millionths: u64,
    pub lifecycle_fit_objects: u64,
    pub aligned_objects: u64,
    pub fit_events: u64,
    pub aligned_events: u64,
    pub out_of_scope_events: u64,
    pub binding_violations: u64,
    pub objects_available: usize,
    pub objects_returned: usize,
    pub objects_truncated: bool,
    pub binding_deviations_available: usize,
    pub binding_deviations_returned: usize,
    pub binding_deviations_truncated: bool,
    pub objects: Vec<LabelledOcelAlignmentObject>,
    pub binding_deviations: Vec<LabelledOcelConformanceDeviation>,
    pub semantics: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OcelAnalysis {
    pub source: String,
    pub ready_for_analysis: bool,
    pub filter: OcelFilter,
    pub catalog: OcelCatalog,
    pub scope: OcelScope,
    pub quality: OcelQualityReport,
    /// Absent when blocking quality findings would make the aggregates unsafe.
    pub graphs: Option<LabelledOcelGraphs>,
    /// Object-type-specific lifecycle variants, absent under blocking quality:
    /// the most common [`VARIANTS_PER_TYPE`] of each item type.
    pub variants: Option<Vec<LabelledOcelVariant>>,
    /// Rarer variants left out of `variants`, all item types together. Every
    /// item is still counted in the graphs; following an item type gives its
    /// full route list.
    pub variants_omitted: usize,
}

/// Route steps listed for one variant: the first ones and the last.
pub const VARIANT_LAYERS_SHOWN: usize = 200;
/// Simultaneous activities listed for one route step.
pub const VARIANT_LAYER_ACTIVITIES_SHOWN: usize = 20;

fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// How many variants of one item type an analysis keeps. A large export can
/// hold tens of thousands of rare routes; listing them all made the saved
/// analysis larger than a project file may be.
pub const VARIANTS_PER_TYPE: usize = 100;

pub fn analyze_ocel(
    path: impl AsRef<Path>,
    filter: &OcelFilter,
) -> Result<OcelAnalysis, OcelQueryError> {
    let path = path.as_ref();
    if crate::follow::from_disk(path) {
        return analyze_ocel_disk(path, filter);
    }
    let log = read_for_overview(path)?;
    analyze_ocel_log(&path.display().to_string(), &log, filter)
}

/// The names a connected log uses: activities and item types in declared
/// order, and the qualifiers of each kind of link, sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcelNames {
    pub activities: Vec<String>,
    pub object_types: Vec<String>,
    pub event_object_qualifiers: Vec<String>,
    pub object_object_qualifiers: Vec<String>,
}

/// The names a saved log uses, read from disk for a large log.
pub fn ocel_names(path: impl AsRef<Path>) -> Result<OcelNames, OcelQueryError> {
    names(path.as_ref(), crate::follow::from_disk(path.as_ref()))
}

pub(crate) fn names(path: &Path, disk: bool) -> Result<OcelNames, OcelQueryError> {
    if disk {
        let log = open_disk(path)?;
        let qualifiers = |ids: &BTreeSet<u32>| {
            ids.iter()
                .map(|id| log.qualifiers[*id as usize].clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        return Ok(OcelNames {
            activities: log.event_types.clone(),
            object_types: log.object_types.clone(),
            event_object_qualifiers: qualifiers(log.event_object_qualifiers()),
            object_object_qualifiers: qualifiers(log.object_object_qualifiers()),
        });
    }
    let log = read_for_overview(path)?;
    let qualifiers = |ids: Vec<pm_core::QualifierId>| {
        ids.into_iter()
            .filter_map(|id| log.qualifier_label(id).map(str::to_owned))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    Ok(OcelNames {
        activities: log.event_types.iter().map(|t| t.name.clone()).collect(),
        object_types: log.object_types.iter().map(|t| t.name.clone()).collect(),
        event_object_qualifiers: qualifiers(
            log.event_object_relations
                .iter()
                .map(|r| r.qualifier)
                .collect(),
        ),
        object_object_qualifiers: qualifiers(
            log.object_object_relations
                .iter()
                .map(|r| r.qualifier)
                .collect(),
        ),
    })
}

/// The first item identifier, in item order, that `matches` accepts.
pub fn find_ocel_object(
    path: impl AsRef<Path>,
    matches: impl FnMut(&str) -> bool,
) -> Result<Option<String>, OcelQueryError> {
    find_object(
        path.as_ref(),
        crate::follow::from_disk(path.as_ref()),
        matches,
    )
}

pub(crate) fn find_object(
    path: &Path,
    disk: bool,
    mut matches: impl FnMut(&str) -> bool,
) -> Result<Option<String>, OcelQueryError> {
    if disk {
        return open_disk(path)?
            .find_object_label(matches)
            .map_err(disk_failed);
    }
    let log = read_for_overview(path)?;
    Ok(log
        .object_ids
        .iter()
        .find(|id| matches(id))
        .map(str::to_owned))
}

/// Opens a saved log for reading in pieces from disk.
pub(crate) fn open_disk(path: &Path) -> Result<pm_ingest::DiskLog, OcelQueryError> {
    pm_ingest::DiskLog::open_cached(path, &std::env::temp_dir().join("glasstarn-disk"))
        .map_err(|e| OcelQueryError::Ingest(pm_ingest::OcelReadError::from(e)))
}

pub(crate) fn disk_failed(error: pm_ingest::OcelSqliteError) -> OcelQueryError {
    OcelQueryError::Ingest(pm_ingest::OcelReadError::from(error))
}

/// The overview of a SQLite log read in pieces from disk: the same result as
/// [`analyze_ocel_log`] on the loaded log, with memory that does not grow
/// with the log.
pub fn analyze_ocel_disk(path: &Path, filter: &OcelFilter) -> Result<OcelAnalysis, OcelQueryError> {
    let started = std::time::Instant::now();
    let lap = |step: &str| {
        if std::env::var_os("PM_DISK_TIMING").is_some() {
            eprintln!("{:>8.2}s {step}", started.elapsed().as_secs_f64());
        }
    };
    let log = open_disk(path)?;
    lap("opened");
    let quality = log.quality().map_err(disk_failed)?;
    lap("quality");
    let view = log.view(filter)?;
    let shown = log.view_counts(&view).map_err(disk_failed)?;
    let totals = [
        log.events,
        log.objects,
        log.event_object_relations,
        log.object_object_relations,
    ];
    let scope = OcelScope {
        events_shown: shown[0],
        events_total: totals[0],
        objects_shown: shown[1],
        objects_total: totals[1],
        event_object_relations_shown: shown[2],
        event_object_relations_total: totals[2],
        object_object_relations_shown: shown[3],
        object_object_relations_total: totals[3],
        narrowed: shown[0] != totals[0]
            || shown[1] != totals[1]
            || shown[2] != totals[2]
            || shown[3] != totals[3],
    };
    let (graphs, variants) = if quality.ready_for_analysis {
        let mut fold = pm_core::OcelGraphFold::default();
        let mut variants = pm_core::OcelVariantFold::default();
        log.for_each_event(&view, |event, event_type, object_types| {
            fold.event(event, event_type);
            fold.event_objects(event, object_types);
            Ok(())
        })
        .map_err(disk_failed)?;
        lap("events");
        log.for_each_timeline(&view, |object, object_type, timeline| {
            fold.lifecycle(object, object_type, timeline);
            variants.object(object, object_type, timeline);
            Ok(())
        })
        .map_err(disk_failed)?;
        lap("timelines");
        log.for_each_participation(&view, |event, event_type, object_type, qualifier| {
            fold.participation(event, event_type, object_type, qualifier);
        })
        .map_err(disk_failed)?;
        lap("participation");
        log.for_each_relationship(&view, |index, source, target, qualifier| {
            fold.relationship(index, source, target, qualifier);
        })
        .map_err(disk_failed)?;
        (Some(fold.finish()), Some(variants.finish()))
    } else {
        (None, None)
    };
    let mut variants_omitted = 0;
    let variants =
        variants.map(|variants| keep_common(label_variants(&log, variants), &mut variants_omitted));
    let catalog = OcelCatalog {
        event_types: log.event_types.clone(),
        object_types: log.object_types.clone(),
        event_object_qualifiers: log
            .event_object_qualifiers()
            .iter()
            .map(|id| log.qualifiers[*id as usize].clone())
            .collect(),
        object_object_qualifiers: log
            .object_object_qualifiers()
            .iter()
            .map(|id| log.qualifiers[*id as usize].clone())
            .collect(),
    };
    Ok(OcelAnalysis {
        source: path.display().to_string(),
        ready_for_analysis: quality.ready_for_analysis,
        filter: filter.clone(),
        catalog,
        scope,
        quality,
        graphs: graphs.map(|graphs| label_graphs(&log, graphs)),
        variants,
        variants_omitted,
    })
}

/// The most common [`VARIANTS_PER_TYPE`] variants of each item type, counting
/// the rest.
fn keep_common(
    variants: Vec<LabelledOcelVariant>,
    omitted: &mut usize,
) -> Vec<LabelledOcelVariant> {
    let mut kept = BTreeMap::<String, usize>::new();
    variants
        .into_iter()
        .filter(|variant| {
            let count = kept.entry(variant.object_type.clone()).or_default();
            *count += 1;
            let keep = *count <= VARIANTS_PER_TYPE;
            if !keep {
                *omitted += 1;
            }
            keep
        })
        .collect()
}

/// The overview never uses recorded attribute values, so a SQLite log is
/// read without them (except where the data checks need them); the results
/// are the same and a large log needs far less memory.
fn read_for_overview(path: &Path) -> Result<OcelLog, OcelQueryError> {
    let sqlite = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("sqlite") || e.eq_ignore_ascii_case("sqlite3"));
    if sqlite {
        return pm_ingest::read_ocel_sqlite_skeleton(path)
            .map_err(|e| OcelQueryError::Ingest(pm_ingest::OcelReadError::from(e)));
    }
    Ok(read_ocel(path)?)
}

pub fn analyze_ocel_log(
    source: &str,
    log: &OcelLog,
    filter: &OcelFilter,
) -> Result<OcelAnalysis, OcelQueryError> {
    let quality = assess_ocel_quality(log);
    let view = OcelView::apply(log, filter)?;
    let scope = OcelScope {
        events_shown: view.events.len(),
        events_total: log.events.len(),
        objects_shown: view.objects.len(),
        objects_total: log.objects.len(),
        event_object_relations_shown: view.event_object_relations.len(),
        event_object_relations_total: log.event_object_relations.len(),
        object_object_relations_shown: view.object_object_relations.len(),
        object_object_relations_total: log.object_object_relations.len(),
        narrowed: view.events.len() != log.events.len()
            || view.objects.len() != log.objects.len()
            || view.event_object_relations.len() != log.event_object_relations.len()
            || view.object_object_relations.len() != log.object_object_relations.len(),
    };
    let graphs = quality
        .ready_for_analysis
        .then(|| label_graphs(log, OcelGraphs::discover(log, &view)));
    let mut variants_omitted = 0;
    let variants = quality.ready_for_analysis.then(|| {
        keep_common(
            label_variants(log, discover_ocel_variants(log, &view)),
            &mut variants_omitted,
        )
    });
    Ok(OcelAnalysis {
        source: source.to_owned(),
        ready_for_analysis: quality.ready_for_analysis,
        filter: filter.clone(),
        catalog: catalog(log),
        scope,
        quality,
        graphs,
        variants,
        variants_omitted,
    })
}

pub fn inspect_ocel_object(
    path: impl AsRef<Path>,
    filter: &OcelFilter,
    object: &str,
) -> Result<LabelledOcelLifecycle, OcelQueryError> {
    let path = path.as_ref();
    if crate::follow::from_disk(path) {
        return inspect_ocel_object_disk(path, filter, object);
    }
    let log = read_ocel(path)?;
    inspect_ocel_object_log(&path.display().to_string(), &log, filter, object)
}

/// [`inspect_ocel_object_log`] read from disk.
pub fn inspect_ocel_object_disk(
    path: &Path,
    filter: &OcelFilter,
    object: &str,
) -> Result<LabelledOcelLifecycle, OcelQueryError> {
    let log = open_disk(path)?;
    let (object_id, kind) = log
        .find_object(object)
        .map_err(disk_failed)?
        .ok_or_else(|| OcelQueryError::UnknownObject(object.to_owned()))?;
    let view = log.view(filter)?;
    if !log.in_view(&view, kind) {
        return Err(OcelQueryError::ObjectOutsideView(object.to_owned()));
    }
    let found = log.object_events(&view, object_id).map_err(disk_failed)?;
    let mut previous = None;
    let mut layer = 0usize;
    let events = found
        .into_iter()
        .map(|(event, activity, at, qualifiers)| {
            if previous.is_some_and(|before| before != at) {
                layer += 1;
            }
            previous = Some(at);
            LabelledOcelLifecycleEvent {
                event: log.event_name(event),
                activity: log.type_name(activity),
                at_micros: at,
                time_layer: layer,
                qualifiers: qualifiers
                    .into_iter()
                    .map(|value| log.qualifier_name(value))
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    Ok(LabelledOcelLifecycle {
        source: path.display().to_string(),
        object: object.to_owned(),
        object_type: log.object_type_name(kind),
        duration_micros: events
            .first()
            .zip(events.last())
            .map(|(first, last)| last.at_micros.saturating_sub(first.at_micros)),
        events,
    })
}

/// Fails when the data checks found errors.
fn ready_on_disk(log: &pm_ingest::DiskLog) -> Result<(), OcelQueryError> {
    if log.quality().map_err(disk_failed)?.ready_for_analysis {
        Ok(())
    } else {
        Err(OcelQueryError::QualityBlocked)
    }
}

fn disk_event_type(
    log: &pm_ingest::DiskLog,
    name: &str,
) -> Result<pm_core::EventTypeId, OcelQueryError> {
    log.event_types
        .iter()
        .position(|candidate| candidate == name)
        .map(|index| pm_core::EventTypeId(index as u32))
        .ok_or_else(|| OcelQueryError::UnknownActivity(name.to_owned()))
}

/// Elapsed times kept in memory until there are many, then on disk.
struct Elapsed<'a> {
    log: &'a pm_ingest::DiskLog,
    memory: BTreeMap<pm_core::ObjectTypeId, Vec<i64>>,
    held: usize,
    spill_after: usize,
    disk: Option<pm_ingest::DiskList<'a>>,
    failed: Option<pm_ingest::OcelSqliteError>,
}

impl<'a> Elapsed<'a> {
    fn new(log: &'a pm_ingest::DiskLog, spill_after: usize) -> Self {
        Self {
            log,
            memory: BTreeMap::new(),
            held: 0,
            spill_after,
            disk: None,
            failed: None,
        }
    }

    fn push(&mut self, kind: pm_core::ObjectTypeId, value: i64) {
        if self.failed.is_some() {
            return;
        }
        let result = self.try_push(kind, value);
        if let Err(error) = result {
            self.failed = Some(error);
        }
    }

    fn try_push(
        &mut self,
        kind: pm_core::ObjectTypeId,
        value: i64,
    ) -> Result<(), pm_ingest::OcelSqliteError> {
        if let Some(disk) = &self.disk {
            return disk.push(i64::from(kind.0), value);
        }
        self.memory.entry(kind).or_default().push(value);
        self.held += 1;
        if self.held > self.spill_after {
            let disk = self.log.new_list()?;
            for (kind, values) in std::mem::take(&mut self.memory) {
                for value in values {
                    disk.push(i64::from(kind.0), value)?;
                }
            }
            self.disk = Some(disk);
        }
        Ok(())
    }

    fn rank(
        &mut self,
        kind: pm_core::ObjectTypeId,
        numerator: usize,
        denominator: usize,
    ) -> Result<i64, pm_ingest::OcelSqliteError> {
        if let Some(disk) = &mut self.disk {
            return disk.rank(i64::from(kind.0), numerator, denominator);
        }
        let Some(values) = self.memory.get_mut(&kind) else {
            return Ok(0);
        };
        if values.is_empty() {
            return Ok(0);
        }
        values.sort_unstable();
        Ok(values[pm_core::nearest_rank(values.len(), numerator, denominator)])
    }
}

/// Elapsed times held in memory before the rest go to disk.
pub(crate) const ELAPSED_IN_MEMORY: usize = 4_000_000;

/// Time between each pair of activities, in one pass over the timelines.
/// With `unreached`, each pair's items that never reached the target are
/// kept on disk under their type, in the list at the pair's position.
pub(crate) fn disk_time_between<'a>(
    log: &'a pm_ingest::DiskLog,
    view: &pm_ingest::DiskView,
    pairs: &[(pm_core::EventTypeId, pm_core::EventTypeId)],
    spill_after: usize,
    unreached: Option<&[pm_ingest::DiskList<'a>]>,
    include_unreached: bool,
) -> Result<Vec<Vec<pm_core::OcelTimeBetween>>, OcelQueryError> {
    let activities = pairs
        .iter()
        .flat_map(|(from, to)| [from.0, to.0])
        .collect::<BTreeSet<_>>();
    let mut folds = pairs
        .iter()
        .map(|(from, to)| pm_core::OcelTimeBetweenFold::new(*from, *to))
        .collect::<Vec<_>>();
    let mut elapsed = pairs
        .iter()
        .map(|_| Elapsed::new(log, spill_after))
        .collect::<Vec<_>>();
    let mut failed: Option<pm_ingest::OcelSqliteError> = None;
    log.for_each_timeline_of(view, Some(&activities), |object, kind, timeline| {
        for (index, fold) in folds.iter_mut().enumerate() {
            let store = &mut elapsed[index];
            fold.object(
                object,
                kind,
                timeline,
                |kind, value| store.push(kind, value),
                |kind, object| {
                    if let Some(lists) = unreached
                        && let Err(error) =
                            lists[index].push(i64::from(kind.0), i64::from(object.0))
                    {
                        failed.get_or_insert(error);
                    }
                },
            );
        }
        Ok(())
    })
    .map_err(disk_failed)?;
    if let Some(error) = failed.or_else(|| elapsed.iter_mut().find_map(|e| e.failed.take())) {
        return Err(disk_failed(error));
    }
    let mut measured = Vec::new();
    for (fold, mut store) in folds.into_iter().zip(elapsed) {
        let mut failed = None;
        let rank_value = |kind, numerator, denominator| {
            store
                .rank(kind, numerator, denominator)
                .unwrap_or_else(|error| {
                    failed.get_or_insert(error);
                    0
                })
        };
        let result = if include_unreached {
            fold.finish_with_unreached(rank_value)
        } else {
            fold.finish(rank_value)
        };
        if let Some(error) = failed {
            return Err(disk_failed(error));
        }
        measured.push(result);
    }
    Ok(measured)
}

/// [`measure_ocel_time_between_log`] read from disk.
pub fn measure_ocel_time_between_disk(
    path: &Path,
    filter: &OcelFilter,
    from_activity: &str,
    to_activity: &str,
) -> Result<OcelTimeBetweenReport, OcelQueryError> {
    measure_time_between_disk(path, filter, from_activity, to_activity, ELAPSED_IN_MEMORY)
}

pub(crate) fn measure_time_between_disk(
    path: &Path,
    filter: &OcelFilter,
    from_activity: &str,
    to_activity: &str,
    spill_after: usize,
) -> Result<OcelTimeBetweenReport, OcelQueryError> {
    let log = open_disk(path)?;
    ready_on_disk(&log)?;
    let from = disk_event_type(&log, from_activity)?;
    let to = disk_event_type(&log, to_activity)?;
    let view = log.view(filter)?;
    let measured = disk_time_between(&log, &view, &[(from, to)], spill_after, None, false)?
        .pop()
        .unwrap_or_default();
    Ok(OcelTimeBetweenReport {
        source: path.display().to_string(),
        filter: filter.clone(),
        from_activity: from_activity.to_owned(),
        to_activity: to_activity.to_owned(),
        measurements: label_time_between(&log, measured),
        semantics: TIME_BETWEEN_SEMANTICS.to_owned(),
    })
}

/// Synchronization read from disk.
fn disk_synchronization(
    log: &pm_ingest::DiskLog,
    view: &pm_ingest::DiskView,
) -> Result<Vec<pm_core::OcelSynchronization>, OcelQueryError> {
    let mut fold = pm_core::OcelSynchronizationFold::default();
    let mut failed = None;
    log.for_each_shared_event(view, |event, kind, at, objects| {
        fold.event(event, kind, at, objects, |object, prior| {
            log.events_at(view, object, prior, 5)
                .unwrap_or_else(|error| {
                    failed.get_or_insert(error);
                    Vec::new()
                })
        });
        Ok(())
    })
    .map_err(disk_failed)?;
    if let Some(error) = failed {
        return Err(disk_failed(error));
    }
    Ok(fold.finish())
}

/// [`analyze_ocel_synchronization_log`] read from disk.
pub fn analyze_ocel_synchronization_disk(
    path: &Path,
    filter: &OcelFilter,
) -> Result<OcelSynchronizationReport, OcelQueryError> {
    let log = open_disk(path)?;
    ready_on_disk(&log)?;
    let view = log.view(filter)?;
    let activities = label_synchronization(&log, disk_synchronization(&log, &view)?);
    Ok(OcelSynchronizationReport {
        source: path.display().to_string(),
        filter: filter.clone(),
        activities,
        semantics: SYNCHRONIZATION_SEMANTICS.to_owned(),
    })
}

/// [`rank_ocel_bottlenecks_log`] read from disk.
pub fn rank_ocel_bottlenecks_disk(
    path: &Path,
    filter: &OcelFilter,
) -> Result<OcelBottleneckReport, OcelQueryError> {
    let log = open_disk(path)?;
    ready_on_disk(&log)?;
    let view = log.view(filter)?;
    let mut fold = pm_core::OcelGraphFold::default();
    log.for_each_timeline(&view, |object, kind, timeline| {
        fold.lifecycle(object, kind, timeline);
        Ok(())
    })
    .map_err(disk_failed)?;
    let ranking = pm_core::rank_bottlenecks(
        fold.finish().directly_follows,
        disk_synchronization(&log, &view)?,
    );
    let (lifecycle, synchronization) = label_bottlenecks(&log, &ranking);
    Ok(bottleneck_report(
        &path.display().to_string(),
        filter,
        &ranking,
        lifecycle,
        synchronization,
    ))
}

pub fn inspect_ocel_object_log(
    source: &str,
    log: &OcelLog,
    filter: &OcelFilter,
    object: &str,
) -> Result<LabelledOcelLifecycle, OcelQueryError> {
    let object_id = log
        .object_ids
        .get(object)
        .map(pm_core::ObjectId)
        .ok_or_else(|| OcelQueryError::UnknownObject(object.to_owned()))?;
    let view = OcelView::apply(log, filter)?;
    let lifecycle = inspect_ocel_lifecycle(log, &view, object_id)
        .ok_or_else(|| OcelQueryError::ObjectOutsideView(object.to_owned()))?;
    Ok(LabelledOcelLifecycle {
        source: source.to_owned(),
        object: object.to_owned(),
        object_type: object_type(log, lifecycle.object_type),
        duration_micros: lifecycle.duration_micros,
        events: lifecycle
            .events
            .into_iter()
            .map(|event| LabelledOcelLifecycleEvent {
                event: event_id(log, event.event),
                activity: event_type(log, event.event_type),
                at_micros: event.at,
                time_layer: event.time_layer,
                qualifiers: event
                    .qualifiers
                    .into_iter()
                    .map(|value| qualifier(log, value))
                    .collect(),
            })
            .collect(),
    })
}

pub fn measure_ocel_time_between(
    path: impl AsRef<Path>,
    filter: &OcelFilter,
    from_activity: &str,
    to_activity: &str,
) -> Result<OcelTimeBetweenReport, OcelQueryError> {
    let path = path.as_ref();
    if crate::follow::from_disk(path) {
        return measure_ocel_time_between_disk(path, filter, from_activity, to_activity);
    }
    let log = read_ocel(path)?;
    measure_ocel_time_between_log(
        &path.display().to_string(),
        &log,
        filter,
        from_activity,
        to_activity,
    )
}

pub fn measure_ocel_time_between_log(
    source: &str,
    log: &OcelLog,
    filter: &OcelFilter,
    from_activity: &str,
    to_activity: &str,
) -> Result<OcelTimeBetweenReport, OcelQueryError> {
    if !assess_ocel_quality(log).ready_for_analysis {
        return Err(OcelQueryError::QualityBlocked);
    }
    let from = resolve_event_type(log, from_activity)?;
    let to = resolve_event_type(log, to_activity)?;
    let view = OcelView::apply(log, filter)?;
    let measurements = label_time_between(log, measure_core_time_between(log, &view, from, to));
    Ok(OcelTimeBetweenReport {
        source: source.to_owned(),
        filter: filter.clone(),
        from_activity: from_activity.to_owned(),
        to_activity: to_activity.to_owned(),
        measurements,
        semantics: TIME_BETWEEN_SEMANTICS.to_owned(),
    })
}

pub fn analyze_ocel_synchronization(
    path: impl AsRef<Path>,
    filter: &OcelFilter,
) -> Result<OcelSynchronizationReport, OcelQueryError> {
    let path = path.as_ref();
    if crate::follow::from_disk(path) {
        return analyze_ocel_synchronization_disk(path, filter);
    }
    let log = read_ocel(path)?;
    analyze_ocel_synchronization_log(&path.display().to_string(), &log, filter)
}

pub fn analyze_ocel_synchronization_log(
    source: &str,
    log: &OcelLog,
    filter: &OcelFilter,
) -> Result<OcelSynchronizationReport, OcelQueryError> {
    if !assess_ocel_quality(log).ready_for_analysis {
        return Err(OcelQueryError::QualityBlocked);
    }
    let view = OcelView::apply(log, filter)?;
    let activities = label_synchronization(log, analyze_core_synchronization(log, &view));
    Ok(OcelSynchronizationReport {
        source: source.to_owned(),
        filter: filter.clone(),
        activities,
        semantics: SYNCHRONIZATION_SEMANTICS.to_owned(),
    })
}

pub fn rank_ocel_bottlenecks(
    path: impl AsRef<Path>,
    filter: &OcelFilter,
) -> Result<OcelBottleneckReport, OcelQueryError> {
    let path = path.as_ref();
    if crate::follow::from_disk(path) {
        return rank_ocel_bottlenecks_disk(path, filter);
    }
    let log = read_ocel(path)?;
    rank_ocel_bottlenecks_log(&path.display().to_string(), &log, filter)
}

pub fn rank_ocel_bottlenecks_log(
    source: &str,
    log: &OcelLog,
    filter: &OcelFilter,
) -> Result<OcelBottleneckReport, OcelQueryError> {
    if !assess_ocel_quality(log).ready_for_analysis {
        return Err(OcelQueryError::QualityBlocked);
    }
    let view = OcelView::apply(log, filter)?;
    let ranking = rank_core_bottlenecks(log, &view);
    let (lifecycle, synchronization) = label_bottlenecks(log, &ranking);
    Ok(bottleneck_report(
        source,
        filter,
        &ranking,
        lifecycle,
        synchronization,
    ))
}

fn bottleneck_report(
    source: &str,
    filter: &OcelFilter,
    ranking: &pm_core::OcelBottleneckRanking,
    lifecycle: Vec<LabelledOcelLifecycleBottleneck>,
    synchronization: Vec<LabelledOcelSynchronizationBottleneck>,
) -> OcelBottleneckReport {
    OcelBottleneckReport {
        source: source.to_owned(),
        filter: filter.clone(),
        lifecycle_candidates: ranking.lifecycle_candidates,
        lifecycle_positive_candidates: ranking.lifecycle_positive_candidates,
        total_lifecycle_elapsed_micros: ranking.total_lifecycle_elapsed_micros,
        lifecycle,
        synchronization_eligible_activities: ranking.synchronization_eligible_activities,
        synchronization_positive_activities: ranking.synchronization_positive_activities,
        total_arrival_spread_micros: ranking.total_arrival_spread_micros,
        synchronization,
        semantics: BOTTLENECK_SEMANTICS.to_owned(),
    }
}

pub fn project_ocel_cases(
    path: impl AsRef<Path>,
    filter: &OcelFilter,
    case_object_type: &str,
    case_limit: usize,
    event_limit_per_case: usize,
) -> Result<OcelProjectionReport, OcelQueryError> {
    let path = path.as_ref();
    if crate::follow::from_disk(path) {
        return project_ocel_cases_disk(
            path,
            filter,
            case_object_type,
            case_limit,
            event_limit_per_case,
        );
    }
    let log = read_ocel(path)?;
    project_ocel_cases_log(
        &path.display().to_string(),
        &log,
        filter,
        case_object_type,
        case_limit,
        event_limit_per_case,
    )
}

pub fn project_ocel_cases_log(
    source: &str,
    log: &OcelLog,
    filter: &OcelFilter,
    case_object_type: &str,
    case_limit: usize,
    event_limit_per_case: usize,
) -> Result<OcelProjectionReport, OcelQueryError> {
    if !assess_ocel_quality(log).ready_for_analysis {
        return Err(OcelQueryError::QualityBlocked);
    }
    let case_type = resolve_object_type(log, case_object_type)?;
    let view = OcelView::apply(log, filter)?;
    let projection = project_core_ocel(log, &view, case_type);
    Ok(label_projection(
        log,
        source,
        filter,
        case_object_type,
        projection,
        case_limit,
        event_limit_per_case,
    ))
}

fn label_projection(
    log: &dyn Labels,
    source: &str,
    filter: &OcelFilter,
    case_object_type: &str,
    projection: pm_core::OcelProjection,
    case_limit: usize,
    event_limit_per_case: usize,
) -> OcelProjectionReport {
    let cases_available = projection.projected_cases;
    let cases = projection
        .cases
        .into_iter()
        .take(case_limit)
        .map(|case| {
            let events_available = case
                .layers
                .iter()
                .map(|layer| u64::try_from(layer.events.len()).unwrap_or(u64::MAX))
                .fold(0u64, u64::saturating_add);
            let mut remaining = event_limit_per_case;
            let mut layers = Vec::new();
            for layer in case.layers {
                if remaining == 0 {
                    break;
                }
                let take = remaining.min(layer.events.len());
                remaining -= take;
                layers.push(LabelledOcelProjectionLayer {
                    at_micros: layer.at,
                    events: layer
                        .events
                        .into_iter()
                        .take(take)
                        .map(|event| LabelledOcelProjectedEvent {
                            source_event: log.event_name(event.source_event),
                            activity: log.type_name(event.event_type),
                        })
                        .collect(),
                });
            }
            let events_returned = layers.iter().map(|layer| layer.events.len()).sum();
            LabelledOcelProjectedCase {
                case_object: log.object_name(case.case_object),
                events_available,
                events_returned,
                truncated: u64::try_from(events_returned).unwrap_or(u64::MAX) < events_available,
                layers,
            }
        })
        .collect::<Vec<_>>();
    let convergence_evidence = projection
        .convergence_evidence
        .into_iter()
        .map(|evidence| LabelledOcelConvergenceEvidence {
            source_event: log.event_name(evidence.source_event),
            activity: log.type_name(log.event_type_of(evidence.source_event)),
            projected_case_count: evidence.projected_case_count,
            additional_occurrences: evidence.additional_occurrences,
            evidence_cases: evidence
                .evidence_cases
                .into_iter()
                .map(|case| log.object_name(case))
                .collect(),
        })
        .collect();
    let divergence_evidence = projection
        .divergence_evidence
        .into_iter()
        .map(|evidence| LabelledOcelDivergenceEvidence {
            projected_case: log.object_name(evidence.projected_case),
            activity: log.type_name(evidence.event_type),
            related_object_type: log.object_type_name(evidence.related_object_type),
            projected_occurrences: evidence.projected_occurrences,
            distinct_related_objects: evidence.distinct_related_objects,
            evidence_events: evidence
                .evidence_events
                .into_iter()
                .map(|event| log.event_name(event))
                .collect(),
            evidence_objects: evidence
                .evidence_objects
                .into_iter()
                .map(|object| log.object_name(object))
                .collect(),
        })
        .collect();
    OcelProjectionReport {
        source: source.to_owned(),
        filter: filter.clone(),
        case_object_type: case_object_type.to_owned(),
        source_events: projection.source_events,
        projected_cases: projection.projected_cases,
        included_unique_events: projection.included_unique_events,
        excluded_events: projection.excluded_events,
        projected_event_occurrences: projection.projected_event_occurrences,
        convergence_events: projection.convergence_events,
        convergence_additional_occurrences: projection.convergence_additional_occurrences,
        divergence_candidates: projection.divergence_candidates,
        cases_available,
        cases_returned: cases.len(),
        event_limit_per_case,
        cases,
        convergence_evidence,
        divergence_evidence,
        semantics: PROJECTION_SEMANTICS.to_owned(),
    }
}

/// [`project_ocel_cases_log`] read from disk; only the cases returned are
/// kept in memory.
pub fn project_ocel_cases_disk(
    path: &Path,
    filter: &OcelFilter,
    case_object_type: &str,
    case_limit: usize,
    event_limit_per_case: usize,
) -> Result<OcelProjectionReport, OcelQueryError> {
    let log = open_disk(path)?;
    ready_on_disk(&log)?;
    let case_type = log
        .object_types
        .iter()
        .position(|candidate| candidate == case_object_type)
        .map(|index| pm_core::ObjectTypeId(index as u32))
        .ok_or_else(|| OcelQueryError::UnknownObjectType(case_object_type.to_owned()))?;
    let view = log.view(filter)?;
    let mut fold = pm_core::OcelProjectionFold::new(case_type, Some(case_limit));
    fold.source_events(log.view_counts(&view).map_err(disk_failed)?[0] as u64);
    log.for_each_event_items(&view, case_type, |event, cases| fold.event(event, cases))
        .map_err(disk_failed)?;
    let mut failed = None;
    log.for_each_timeline(&view.only_type(case_type), |object, _, timeline| {
        fold.case(object, timeline, |event| {
            log.other_items(&view, event, case_type)
                .unwrap_or_else(|error| {
                    failed.get_or_insert(error);
                    Default::default()
                })
        });
        Ok(())
    })
    .map_err(disk_failed)?;
    if let Some(error) = failed {
        return Err(disk_failed(error));
    }
    Ok(label_projection(
        &log,
        &path.display().to_string(),
        filter,
        case_object_type,
        fold.finish(),
        case_limit,
        event_limit_per_case,
    ))
}

pub fn discover_ocel_model(
    path: impl AsRef<Path>,
    filter: &OcelFilter,
    config: OcelDiscoveryConfig,
) -> Result<OcelModelReport, OcelQueryError> {
    let path = path.as_ref();
    if crate::follow::from_disk(path) {
        return discover_ocel_model_disk(path, filter, config);
    }
    let log = read_ocel(path)?;
    discover_ocel_model_log(&path.display().to_string(), &log, filter, config)
}

pub fn discover_ocel_model_log(
    source: &str,
    log: &OcelLog,
    filter: &OcelFilter,
    config: OcelDiscoveryConfig,
) -> Result<OcelModelReport, OcelQueryError> {
    if !assess_ocel_quality(log).ready_for_analysis {
        return Err(OcelQueryError::QualityBlocked);
    }
    let view = OcelView::apply(log, filter)?;
    let model = discover_core_ocel_model(log, &view, config)
        .map_err(|error| label_discovery_error(log, error))?;
    Ok(label_ocel_model(source, log, filter, model))
}

/// [`discover_ocel_model_log`] read from disk: each item type's variants
/// with their counts are all the miner needs.
pub fn discover_ocel_model_disk(
    path: &Path,
    filter: &OcelFilter,
    config: OcelDiscoveryConfig,
) -> Result<OcelModelReport, OcelQueryError> {
    let log = open_disk(path)?;
    ready_on_disk(&log)?;
    let view = log.view(filter)?;
    let mut fold = pm_core::OcelDiscoveryFold::default();
    log.for_each_event(&view, |_, event_type, object_types| {
        fold.event(event_type, object_types);
        Ok(())
    })
    .map_err(disk_failed)?;
    log.for_each_timeline(&view, |object, object_type, timeline| {
        fold.object(object, object_type, timeline);
        Ok(())
    })
    .map_err(disk_failed)?;
    let model = fold
        .finish(config)
        .map_err(|error| label_discovery_error(&log, error))?;
    Ok(label_ocel_model(
        &path.display().to_string(),
        &log,
        filter,
        model,
    ))
}

pub fn check_ocel_conformance(
    path: impl AsRef<Path>,
    model_path: impl AsRef<Path>,
    filter: &OcelFilter,
    detail_limit: usize,
) -> Result<OcelConformanceReport, OcelQueryError> {
    let path = path.as_ref();
    let model_path = model_path.as_ref();
    if crate::follow::from_disk(path) {
        return check_ocel_conformance_disk(path, model_path, filter, detail_limit);
    }
    let log = read_ocel(path)?;
    let model = read_model(model_path)?;
    check_ocel_conformance_log(
        &path.display().to_string(),
        &log,
        filter,
        &model_path.display().to_string(),
        &model,
        detail_limit,
    )
}

/// The first `limit` items in an order, kept as they arrive.
struct Kept<K: Ord, T> {
    limit: usize,
    items: Vec<(K, T)>,
}

impl<K: Ord, T> Kept<K, T> {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            items: Vec::new(),
        }
    }

    /// Whether an item with this key would be kept.
    fn wants(&self, key: &K) -> bool {
        self.items.len() < self.limit || self.items.last().is_some_and(|(worst, _)| key < worst)
    }

    fn offer(&mut self, key: K, item: T) {
        if !self.wants(&key) {
            return;
        }
        let at = self.items.partition_point(|(kept, _)| *kept <= key);
        self.items.insert(at, (key, item));
        self.items.truncate(self.limit);
    }

    fn into_items(self) -> Vec<T> {
        self.items.into_iter().map(|(_, item)| item).collect()
    }
}

/// Reads a saved model for conformance.
fn read_model(model_path: &Path) -> Result<OcelModelReport, OcelQueryError> {
    let bytes = std::fs::read(model_path).map_err(OcelQueryError::ModelIo)?;
    serde_json::from_slice::<OcelModelReport>(&bytes).map_err(OcelQueryError::ModelJson)
}

/// [`check_ocel_conformance_log`] read from disk; only the items and
/// deviations returned are kept in memory.
pub fn check_ocel_conformance_disk(
    path: &Path,
    model_path: &Path,
    filter: &OcelFilter,
    detail_limit: usize,
) -> Result<OcelConformanceReport, OcelQueryError> {
    let log = open_disk(path)?;
    let model = read_model(model_path)?;
    ready_on_disk(&log)?;
    let view = log.view(filter)?;
    let typed_model = resolve_ocel_model(&log, &model)?;
    let mut fold = pm_core::OcelConformanceFold::new(
        &log.event_types,
        log.object_types.len(),
        &typed_model,
        &log.view_object_types(&view).map_err(disk_failed)?,
    )
    .map_err(|error| label_replay_error(&log, error))?;
    let deviating = log.new_list().map_err(disk_failed)?;
    let mut failed = None;
    let mut deviations = Vec::new();
    let mut deviations_available = 0usize;
    let mut found = |deviation: pm_core::OcelReplayDeviation| {
        if let Some(event) = replay_deviation_event(&deviation)
            && let Err(error) = deviating.push(0, i64::from(event.0))
        {
            failed.get_or_insert(error);
        }
        deviations_available += 1;
        if deviations.len() < detail_limit {
            deviations.push(deviation);
        }
    };
    log.for_each_event(&view, |event, event_type, object_types| {
        fold.event(event, event_type, object_types, &mut found);
        Ok(())
    })
    .map_err(disk_failed)?;
    let mut kept = Kept::new(detail_limit);
    log.for_each_timeline(&view, |object, object_type, timeline| {
        if let Some((replayed, object_deviations)) =
            fold.replay_object(object, object_type, timeline)
        {
            for deviation in object_deviations {
                found(deviation);
            }
            let labelled = label_conformance_object(&log, &replayed);
            let key = ConformanceKey(labelled.clone());
            kept.offer(key, labelled);
        }
        Ok(())
    })
    .map_err(disk_failed)?;
    if let Some(error) = failed {
        return Err(disk_failed(error));
    }
    fold.check()
        .map_err(|error| label_replay_error(&log, error))?;
    let replay = fold.replay_totals(
        Vec::new(),
        Vec::new(),
        deviating.distinct().map_err(disk_failed)?,
    );
    let objects = kept.into_items();
    let objects_available = replay.replayed_objects as usize;
    let deviations = deviations
        .into_iter()
        .map(|deviation| label_replay_deviation(&log, &typed_model, deviation))
        .collect::<Vec<_>>();
    Ok(OcelConformanceReport {
        source: path.display().to_string(),
        model_source: model_path.display().to_string(),
        model_training_source: model.source.clone(),
        filter: filter.clone(),
        fitness_millionths: replay.fitness_millionths,
        lifecycle_fit_objects: replay.lifecycle_fit_objects,
        replayed_objects: replay.replayed_objects,
        fit_events: replay.fit_events,
        replayed_events: replay.replayed_events,
        out_of_scope_events: replay.out_of_scope_events,
        consumed_tokens: replay.consumed_tokens,
        produced_tokens: replay.produced_tokens,
        missing_tokens: replay.missing_tokens,
        remaining_tokens: replay.remaining_tokens,
        unmapped_event_occurrences: replay.unmapped_event_occurrences,
        binding_violations: replay.binding_violations,
        objects_available,
        objects_returned: objects.len(),
        objects_truncated: objects.len() < objects_available,
        deviations_available,
        deviations_returned: deviations.len(),
        deviations_truncated: deviations.len() < deviations_available,
        objects,
        deviations,
        semantics: CONFORMANCE_SEMANTICS.to_owned(),
    })
}

/// Orders conformance details as the report lists them.
#[derive(PartialEq, Eq)]
struct ConformanceKey(LabelledOcelConformanceObject);

impl PartialOrd for ConformanceKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ConformanceKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        conformance_order(&self.0, &other.0)
    }
}

/// The event a deviation concerns, if any.
fn replay_deviation_event(
    deviation: &pm_core::OcelReplayDeviation,
) -> Option<pm_core::OcelEventId> {
    use pm_core::OcelReplayDeviation as D;
    match deviation {
        D::ActivityNotInModel { event, .. }
        | D::MissingTokens { event, .. }
        | D::MissingObjectParticipation { event, .. }
        | D::FixedArcMultiplicity { event, .. }
        | D::UnexpectedObjectParticipation { event, .. } => Some(*event),
        D::FinalTokensMissing { .. } | D::TokensRemaining { .. } => None,
    }
}

/// [`align_ocel_conformance_log`] read from disk; only the items and
/// deviations returned are kept in memory.
pub fn align_ocel_conformance_disk(
    path: &Path,
    model_path: &Path,
    filter: &OcelFilter,
    detail_limit: usize,
) -> Result<OcelAlignmentConformanceReport, OcelQueryError> {
    let log = open_disk(path)?;
    let model = read_model(model_path)?;
    ready_on_disk(&log)?;
    let view = log.view(filter)?;
    let typed_model = resolve_ocel_model(&log, &model)?;
    let mut fold = pm_core::OcelConformanceFold::new(
        &log.event_types,
        log.object_types.len(),
        &typed_model,
        &log.view_object_types(&view).map_err(disk_failed)?,
    )
    .map_err(|error| label_replay_error(&log, error))?;
    let deviating = log.new_list().map_err(disk_failed)?;
    let mut failed = None;
    let mut binding = Vec::new();
    let mut binding_available = 0usize;
    log.for_each_event(&view, |event, event_type, object_types| {
        fold.event(event, event_type, object_types, |deviation| {
            if let Some(event) = replay_deviation_event(&deviation)
                && let Err(error) = deviating.push(0, i64::from(event.0))
            {
                failed.get_or_insert(error);
            }
            binding_available += 1;
            if binding.len() < detail_limit {
                binding.push(deviation);
            }
        });
        Ok(())
    })
    .map_err(disk_failed)?;
    let mut kept = Kept::new(detail_limit);
    log.for_each_timeline(&view, |object, object_type, timeline| {
        if let Some(aligned) = fold.align_object(object, object_type, timeline) {
            for movement in &aligned.moves {
                if let pm_core::OcelAlignmentMove::LogOnly { event, .. } = movement
                    && let Err(error) = deviating.push(0, i64::from(event.0))
                {
                    failed.get_or_insert(error);
                }
            }
            let key = (
                std::cmp::Reverse(aligned.cost),
                aligned.fitness_millionths,
                log.object_type_name(aligned.object_type),
                log.object_name(aligned.object),
            );
            if kept.wants(&key) {
                kept.offer(key, label_alignment_object(&log, aligned, detail_limit));
            }
        }
        Ok(())
    })
    .map_err(disk_failed)?;
    if let Some(error) = failed {
        return Err(disk_failed(error));
    }
    fold.check()
        .map_err(|error| label_replay_error(&log, error))?;
    let alignment = fold.alignment_totals(
        Vec::new(),
        Vec::new(),
        deviating.distinct().map_err(disk_failed)?,
    );
    let objects = kept.into_items();
    let objects_available = alignment.aligned_objects as usize;
    let binding_deviations = binding
        .into_iter()
        .map(|deviation| label_replay_deviation(&log, &typed_model, deviation))
        .collect::<Vec<_>>();
    Ok(OcelAlignmentConformanceReport {
        source: path.display().to_string(),
        model_source: model_path.display().to_string(),
        model_training_source: model.source.clone(),
        filter: filter.clone(),
        total_cost: alignment.total_cost,
        denominator: alignment.denominator,
        fitness_millionths: alignment.fitness_millionths,
        lifecycle_fit_objects: alignment.lifecycle_fit_objects,
        aligned_objects: alignment.aligned_objects,
        fit_events: alignment.fit_events,
        aligned_events: alignment.aligned_events,
        out_of_scope_events: alignment.out_of_scope_events,
        binding_violations: alignment.binding_violations,
        objects_available,
        objects_returned: objects.len(),
        objects_truncated: objects.len() < objects_available,
        binding_deviations_available: binding_available,
        binding_deviations_returned: binding_deviations.len(),
        binding_deviations_truncated: binding_deviations.len() < binding_available,
        objects,
        binding_deviations,
        semantics: ALIGNMENT_SEMANTICS.to_owned(),
    })
}

pub fn check_ocel_conformance_log(
    source: &str,
    log: &OcelLog,
    filter: &OcelFilter,
    model_source: &str,
    model: &OcelModelReport,
    detail_limit: usize,
) -> Result<OcelConformanceReport, OcelQueryError> {
    if !assess_ocel_quality(log).ready_for_analysis {
        return Err(OcelQueryError::QualityBlocked);
    }
    let view = OcelView::apply(log, filter)?;
    let typed_model = resolve_ocel_model(log, model)?;
    let replay = replay_ocel_tokens(log, &view, &typed_model)
        .map_err(|error| label_replay_error(log, error))?;

    let objects_available = replay.objects.len();
    let mut objects = replay
        .objects
        .iter()
        .map(|item| label_conformance_object(log, item))
        .collect::<Vec<_>>();
    objects.sort_by(conformance_order);
    objects.truncate(detail_limit);

    let deviations_available = replay.deviations.len();
    let deviations = replay
        .deviations
        .into_iter()
        .take(detail_limit)
        .map(|deviation| label_replay_deviation(log, &typed_model, deviation))
        .collect::<Vec<_>>();

    Ok(OcelConformanceReport {
        source: source.to_owned(),
        model_source: model_source.to_owned(),
        model_training_source: model.source.clone(),
        filter: filter.clone(),
        fitness_millionths: replay.fitness_millionths,
        lifecycle_fit_objects: replay.lifecycle_fit_objects,
        replayed_objects: replay.replayed_objects,
        fit_events: replay.fit_events,
        replayed_events: replay.replayed_events,
        out_of_scope_events: replay.out_of_scope_events,
        consumed_tokens: replay.consumed_tokens,
        produced_tokens: replay.produced_tokens,
        missing_tokens: replay.missing_tokens,
        remaining_tokens: replay.remaining_tokens,
        unmapped_event_occurrences: replay.unmapped_event_occurrences,
        binding_violations: replay.binding_violations,
        objects_available,
        objects_returned: objects.len(),
        objects_truncated: objects.len() < objects_available,
        deviations_available,
        deviations_returned: deviations.len(),
        deviations_truncated: deviations.len() < deviations_available,
        objects,
        deviations,
        semantics: CONFORMANCE_SEMANTICS.to_owned(),
    })
}

pub fn align_ocel_conformance(
    path: impl AsRef<Path>,
    model_path: impl AsRef<Path>,
    filter: &OcelFilter,
    detail_limit: usize,
) -> Result<OcelAlignmentConformanceReport, OcelQueryError> {
    let path = path.as_ref();
    let model_path = model_path.as_ref();
    if crate::follow::from_disk(path) {
        return align_ocel_conformance_disk(path, model_path, filter, detail_limit);
    }
    let log = read_ocel(path)?;
    let model = read_model(model_path)?;
    align_ocel_conformance_log(
        &path.display().to_string(),
        &log,
        filter,
        &model_path.display().to_string(),
        &model,
        detail_limit,
    )
}

pub fn align_ocel_conformance_log(
    source: &str,
    log: &OcelLog,
    filter: &OcelFilter,
    model_source: &str,
    model: &OcelModelReport,
    detail_limit: usize,
) -> Result<OcelAlignmentConformanceReport, OcelQueryError> {
    if !assess_ocel_quality(log).ready_for_analysis {
        return Err(OcelQueryError::QualityBlocked);
    }
    let view = OcelView::apply(log, filter)?;
    let typed_model = resolve_ocel_model(log, model)?;
    let alignment = align_core_ocel(log, &view, &typed_model)
        .map_err(|error| label_replay_error(log, error))?;

    let objects_available = alignment.objects.len();
    let mut objects = alignment
        .objects
        .into_iter()
        .map(|object| label_alignment_object(log, object, detail_limit))
        .collect::<Vec<_>>();
    objects.sort_by(alignment_order);
    objects.truncate(detail_limit);

    let binding_deviations_available = alignment.binding_deviations.len();
    let binding_deviations = alignment
        .binding_deviations
        .into_iter()
        .take(detail_limit)
        .map(|deviation| label_replay_deviation(log, &typed_model, deviation))
        .collect::<Vec<_>>();

    Ok(OcelAlignmentConformanceReport {
        source: source.to_owned(),
        model_source: model_source.to_owned(),
        model_training_source: model.source.clone(),
        filter: filter.clone(),
        total_cost: alignment.total_cost,
        denominator: alignment.denominator,
        fitness_millionths: alignment.fitness_millionths,
        lifecycle_fit_objects: alignment.lifecycle_fit_objects,
        aligned_objects: alignment.aligned_objects,
        fit_events: alignment.fit_events,
        aligned_events: alignment.aligned_events,
        out_of_scope_events: alignment.out_of_scope_events,
        binding_violations: alignment.binding_violations,
        objects_available,
        objects_returned: objects.len(),
        objects_truncated: objects.len() < objects_available,
        binding_deviations_available,
        binding_deviations_returned: binding_deviations.len(),
        binding_deviations_truncated: binding_deviations.len() < binding_deviations_available,
        objects,
        binding_deviations,
        semantics: ALIGNMENT_SEMANTICS.to_owned(),
    })
}

fn label_alignment_deviation(
    log: &dyn Labels,
    movement: OcelAlignmentMove,
) -> Option<LabelledOcelAlignmentDeviation> {
    match movement {
        OcelAlignmentMove::LogOnly {
            event,
            event_type: activity,
        } => Some(LabelledOcelAlignmentDeviation::LogOnly {
            event: log.event_name(event),
            activity: log.type_name(activity),
        }),
        OcelAlignmentMove::VisibleModel {
            transition,
            event_type: activity,
        } => Some(LabelledOcelAlignmentDeviation::VisibleModel {
            transition: transition.0,
            activity: log.type_name(activity),
        }),
        OcelAlignmentMove::Synchronous { .. } | OcelAlignmentMove::InvisibleModel { .. } => None,
    }
}

fn resolve_ocel_model(
    log: &dyn Labels,
    model: &OcelModelReport,
) -> Result<OcelPetriNet, OcelQueryError> {
    Ok(OcelPetriNet {
        places: model
            .places
            .iter()
            .map(|place| {
                Ok(OcelPlace {
                    id: OcelPlaceId(place.id),
                    object_type: resolve_type_name(log, &place.object_type)?,
                    initial: place.initial,
                    final_place: place.final_place,
                })
            })
            .collect::<Result<_, OcelQueryError>>()?,
        transitions: model
            .transitions
            .iter()
            .map(|transition| {
                Ok(OcelTransition {
                    id: OcelTransitionId(transition.id),
                    event_type: transition
                        .activity
                        .as_deref()
                        .map(|activity| resolve_activity_name(log, activity))
                        .transpose()?,
                    inputs: transition
                        .inputs
                        .iter()
                        .map(|arc| OcelArcEndpoint {
                            place: OcelPlaceId(arc.place),
                            variable: arc.variable,
                        })
                        .collect(),
                    outputs: transition
                        .outputs
                        .iter()
                        .map(|arc| OcelArcEndpoint {
                            place: OcelPlaceId(arc.place),
                            variable: arc.variable,
                        })
                        .collect(),
                })
            })
            .collect::<Result<_, OcelQueryError>>()?,
        object_types: model
            .object_types
            .iter()
            .map(|stats| {
                Ok(OcelObjectTypeDiscovery {
                    object_type: resolve_type_name(log, &stats.object_type)?,
                    objects: stats.objects,
                    objects_with_events: stats.objects_with_events,
                    lifecycle_event_occurrences: stats.lifecycle_event_occurrences,
                    filtered_edges: stats.filtered_edges,
                    used_flower_fallback: stats.used_flower_fallback,
                })
            })
            .collect::<Result<_, OcelQueryError>>()?,
        variable_arcs: model.variable_arcs,
        fixed_arcs: model.fixed_arcs,
        noise_threshold_milli: model.noise_threshold_milli,
        variable_arc_threshold_milli: model.variable_arc_threshold_milli,
    })
}

fn label_replay_error(log: &dyn Labels, error: OcelReplayError) -> OcelQueryError {
    match error {
        OcelReplayError::SimultaneousEvents { object, at } => {
            OcelQueryError::SimultaneousObjectEvents {
                object: log.object_name(object),
                at_micros: at,
            }
        }
        error => OcelQueryError::Conformance(error.to_string()),
    }
}

fn label_replay_deviation(
    log: &dyn Labels,
    model: &OcelPetriNet,
    deviation: OcelReplayDeviation,
) -> LabelledOcelConformanceDeviation {
    let transition_activity = |transition: OcelTransitionId| {
        model
            .transitions
            .get(transition.0 as usize)
            .and_then(|transition| transition.event_type)
            .map(|activity| log.type_name(activity))
    };
    match deviation {
        OcelReplayDeviation::ActivityNotInModel {
            event,
            event_type: activity,
            object,
        } => LabelledOcelConformanceDeviation::ActivityNotInModel {
            event: log.event_name(event),
            activity: log.type_name(activity),
            object: log.object_name(object),
        },
        OcelReplayDeviation::MissingTokens {
            event,
            event_type: activity,
            object,
            count,
        } => LabelledOcelConformanceDeviation::MissingTokens {
            event: log.event_name(event),
            activity: log.type_name(activity),
            object: log.object_name(object),
            count,
        },
        OcelReplayDeviation::FinalTokensMissing { object, count } => {
            LabelledOcelConformanceDeviation::FinalTokensMissing {
                object: log.object_name(object),
                count,
            }
        }
        OcelReplayDeviation::TokensRemaining { object, count } => {
            LabelledOcelConformanceDeviation::TokensRemaining {
                object: log.object_name(object),
                count,
            }
        }
        OcelReplayDeviation::MissingObjectParticipation {
            event,
            transition,
            object_type: kind,
            variable,
        } => LabelledOcelConformanceDeviation::MissingObjectParticipation {
            event: log.event_name(event),
            activity: transition_activity(transition),
            object_type: log.object_type_name(kind),
            variable,
        },
        OcelReplayDeviation::FixedArcMultiplicity {
            event,
            transition,
            object_type: kind,
            observed_objects,
        } => LabelledOcelConformanceDeviation::FixedArcMultiplicity {
            event: log.event_name(event),
            activity: transition_activity(transition),
            object_type: log.object_type_name(kind),
            observed_objects,
        },
        OcelReplayDeviation::UnexpectedObjectParticipation {
            event,
            transition,
            object_type: kind,
            observed_objects,
        } => LabelledOcelConformanceDeviation::UnexpectedObjectParticipation {
            event: log.event_name(event),
            activity: transition_activity(transition),
            object_type: log.object_type_name(kind),
            observed_objects,
        },
    }
}

fn label_ocel_model(
    source: &str,
    log: &dyn Labels,
    filter: &OcelFilter,
    model: OcelPetriNet,
) -> OcelModelReport {
    OcelModelReport {
        source: source.to_owned(),
        filter: filter.clone(),
        places: model
            .places
            .into_iter()
            .map(|place| LabelledOcelModelPlace {
                id: place.id.0,
                object_type: log.object_type_name(place.object_type),
                initial: place.initial,
                final_place: place.final_place,
            })
            .collect(),
        transitions: model
            .transitions
            .into_iter()
            .map(|transition| LabelledOcelModelTransition {
                id: transition.id.0,
                activity: transition
                    .event_type
                    .map(|event_type_id| log.type_name(event_type_id)),
                inputs: transition
                    .inputs
                    .into_iter()
                    .map(|arc| LabelledOcelModelArc {
                        place: arc.place.0,
                        variable: arc.variable,
                    })
                    .collect(),
                outputs: transition
                    .outputs
                    .into_iter()
                    .map(|arc| LabelledOcelModelArc {
                        place: arc.place.0,
                        variable: arc.variable,
                    })
                    .collect(),
            })
            .collect(),
        object_types: model
            .object_types
            .into_iter()
            .map(|stats| LabelledOcelModelObjectType {
                object_type: log.object_type_name(stats.object_type),
                objects: stats.objects,
                objects_with_events: stats.objects_with_events,
                lifecycle_event_occurrences: stats.lifecycle_event_occurrences,
                filtered_edges: stats.filtered_edges,
                used_flower_fallback: stats.used_flower_fallback,
            })
            .collect(),
        variable_arcs: model.variable_arcs,
        fixed_arcs: model.fixed_arcs,
        noise_threshold_milli: model.noise_threshold_milli,
        variable_arc_threshold_milli: model.variable_arc_threshold_milli,
        semantics: "This native object-centric Petri net is discovered by mining a sound lifecycle model for each retained object type, keeping places typed, and merging equally named visible activities across those models. Unlabelled transitions are silent routing steps. A variable arc means the activity sometimes related to a number of objects of that type other than exactly one, according to the disclosed threshold. Filters and edge removal change the discovered model. Simultaneous lifecycle events are never ordered arbitrarily. The model describes observed control-flow structure; it does not prove causation, business rules, or conformance.".to_owned(),
    }
}

fn label_discovery_error(log: &dyn Labels, error: OcelDiscoveryError) -> OcelQueryError {
    match error {
        OcelDiscoveryError::InvalidThreshold => OcelQueryError::InvalidDiscoveryThreshold,
        OcelDiscoveryError::SimultaneousEvents { object, at } => {
            OcelQueryError::SimultaneousObjectEvents {
                object: log.object_name(object),
                at_micros: at,
            }
        }
        OcelDiscoveryError::Discovery(error) => OcelQueryError::Discovery(error.to_string()),
    }
}

fn catalog(log: &OcelLog) -> OcelCatalog {
    let qualifier_labels = |ids: BTreeSet<_>| {
        ids.into_iter()
            .filter_map(|id| log.qualifier_label(id).map(str::to_owned))
            .collect()
    };
    OcelCatalog {
        event_types: log
            .event_types
            .iter()
            .map(|event_type| event_type.name.clone())
            .collect(),
        object_types: log
            .object_types
            .iter()
            .map(|object_type| object_type.name.clone())
            .collect(),
        event_object_qualifiers: qualifier_labels(
            log.event_object_relations
                .iter()
                .map(|relation| relation.qualifier)
                .collect(),
        ),
        object_object_qualifiers: qualifier_labels(
            log.object_object_relations
                .iter()
                .map(|relation| relation.qualifier)
                .collect(),
        ),
    }
}

fn label_graphs(log: &dyn Labels, graphs: OcelGraphs) -> LabelledOcelGraphs {
    LabelledOcelGraphs {
        activities: graphs
            .activities
            .into_iter()
            .map(|node| LabelledOcelActivity {
                activity: log.type_name(node.event_type),
                events: node.events,
                evidence_events: node
                    .evidence_events
                    .into_iter()
                    .map(|event| log.event_name(event))
                    .collect(),
            })
            .collect(),
        directly_follows: graphs
            .directly_follows
            .into_iter()
            .map(|edge| LabelledOcelDirectlyFollows {
                object_type: log.object_type_name(edge.object_type),
                source: log.type_name(edge.source),
                target: log.type_name(edge.target),
                occurrences: edge.occurrences,
                objects: edge.objects,
                total_elapsed_micros: edge.total_elapsed_micros,
                mean_elapsed_micros: edge.mean_elapsed_micros,
                min_elapsed_micros: edge.min_elapsed_micros,
                max_elapsed_micros: edge.max_elapsed_micros,
                evidence: edge
                    .evidence
                    .into_iter()
                    .map(|evidence| LabelledOcelDfgEvidence {
                        object: log.object_name(evidence.object),
                        source_event: log.event_name(evidence.source_event),
                        target_event: log.event_name(evidence.target_event),
                        source_at_micros: log.event_time(evidence.source_event),
                        target_at_micros: log.event_time(evidence.target_event),
                    })
                    .collect(),
            })
            .collect(),
        participation: graphs
            .participation
            .into_iter()
            .map(|edge| LabelledOcelParticipation {
                activity: log.type_name(edge.event_type),
                object_type: log.object_type_name(edge.object_type),
                qualifier: log.qualifier_name(edge.qualifier),
                occurrences: edge.occurrences,
                evidence_events: edge
                    .evidence_events
                    .into_iter()
                    .map(|event| log.event_name(event))
                    .collect(),
            })
            .collect(),
        interactions: graphs
            .interactions
            .into_iter()
            .map(|edge| LabelledOcelInteraction {
                left_object_type: log.object_type_name(edge.left),
                right_object_type: log.object_type_name(edge.right),
                events: edge.events,
                object_pairs: edge.object_pairs,
                evidence_events: edge
                    .evidence_events
                    .into_iter()
                    .map(|event| log.event_name(event))
                    .collect(),
            })
            .collect(),
        relationships: graphs
            .relationships
            .into_iter()
            .map(|edge| LabelledOcelRelationship {
                source_object_type: log.object_type_name(edge.source),
                target_object_type: log.object_type_name(edge.target),
                qualifier: log.qualifier_name(edge.qualifier),
                occurrences: edge.occurrences,
                evidence: edge
                    .evidence_relations
                    .into_iter()
                    .map(|index| {
                        let (source, target) = log.relationship_ends(index);
                        LabelledOcelRelationshipEvidence {
                            source_object: log.object_name(source),
                            target_object: log.object_name(target),
                        }
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn label_variants(
    log: &dyn Labels,
    variants: Vec<pm_core::OcelVariant>,
) -> Vec<LabelledOcelVariant> {
    variants
        .into_iter()
        .map(|variant| LabelledOcelVariant {
            object_type: log.object_type_name(variant.object_type),
            rank: variant.rank,
            layers_omitted: variant.layers.len().saturating_sub(VARIANT_LAYERS_SHOWN),
            layers: {
                let total = variant.layers.len();
                variant
                    .layers
                    .into_iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        total <= VARIANT_LAYERS_SHOWN
                            || *index < VARIANT_LAYERS_SHOWN - 1
                            || *index == total - 1
                    })
                    .map(|(_, layer)| LabelledOcelVariantLayer {
                        omitted: layer
                            .activities
                            .len()
                            .saturating_sub(VARIANT_LAYER_ACTIVITIES_SHOWN),
                        activities: layer
                            .activities
                            .into_iter()
                            .take(VARIANT_LAYER_ACTIVITIES_SHOWN)
                            .map(|activity| log.type_name(activity))
                            .collect(),
                    })
                    .collect()
            },
            objects: variant.objects,
            share_millionths: variant.share_millionths,
            duration_eligible_objects: variant.duration_eligible_objects,
            mean_duration_micros: variant.mean_duration_micros,
            evidence_objects: variant
                .evidence_objects
                .into_iter()
                .map(|object| log.object_name(object))
                .collect(),
        })
        .collect()
}

fn label_synchronization(
    log: &dyn Labels,
    activities: Vec<pm_core::OcelSynchronization>,
) -> Vec<LabelledOcelSynchronization> {
    activities
        .into_iter()
        .map(|activity| LabelledOcelSynchronization {
            activity: log.type_name(activity.activity),
            object_types: activity
                .object_types
                .into_iter()
                .map(|value| log.object_type_name(value))
                .collect(),
            multi_object_events: activity.multi_object_events,
            eligible_events: activity.eligible_events,
            insufficient_history_events: activity.insufficient_history_events,
            historically_observed_object_participations: activity
                .historically_observed_object_participations,
            first_seen_object_participations: activity.first_seen_object_participations,
            total_arrival_spread_micros: activity.total_arrival_spread_micros,
            mean_arrival_spread_micros: activity.mean_arrival_spread_micros,
            min_arrival_spread_micros: activity.min_arrival_spread_micros,
            max_arrival_spread_micros: activity.max_arrival_spread_micros,
            total_elapsed_after_last_prior_micros: activity.total_elapsed_after_last_prior_micros,
            mean_elapsed_after_last_prior_micros: activity.mean_elapsed_after_last_prior_micros,
            min_elapsed_after_last_prior_micros: activity.min_elapsed_after_last_prior_micros,
            max_elapsed_after_last_prior_micros: activity.max_elapsed_after_last_prior_micros,
            total_elapsed_from_first_prior_micros: activity.total_elapsed_from_first_prior_micros,
            mean_elapsed_from_first_prior_micros: activity.mean_elapsed_from_first_prior_micros,
            min_elapsed_from_first_prior_micros: activity.min_elapsed_from_first_prior_micros,
            max_elapsed_from_first_prior_micros: activity.max_elapsed_from_first_prior_micros,
            evidence: activity
                .evidence
                .into_iter()
                .map(|evidence| LabelledOcelSynchronizationEvidence {
                    event: log.event_name(evidence.event),
                    at_micros: log.event_time(evidence.event),
                    participating_objects: evidence.participating_objects,
                    historically_observed_objects: evidence.historically_observed_objects,
                    first_seen_objects: evidence.first_seen_objects,
                    arrival_spread_micros: evidence.arrival_spread_micros,
                    elapsed_after_last_prior_micros: evidence.elapsed_after_last_prior_micros,
                    elapsed_from_first_prior_micros: evidence.elapsed_from_first_prior_micros,
                    readiness: evidence
                        .readiness
                        .into_iter()
                        .map(|readiness| LabelledOcelSynchronizationReadiness {
                            object: log.object_name(readiness.object),
                            object_type: log.object_type_name(readiness.object_type),
                            prior_at_micros: readiness.prior_at,
                            prior_events: readiness
                                .prior_events
                                .into_iter()
                                .map(|event| log.event_name(event))
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

fn label_time_between(
    log: &dyn Labels,
    measurements: Vec<pm_core::OcelTimeBetween>,
) -> Vec<LabelledOcelTimeBetween> {
    measurements
        .into_iter()
        .map(|measurement| LabelledOcelTimeBetween {
            object_type: log.object_type_name(measurement.object_type),
            observations: measurement.observations,
            objects: measurement.objects,
            mean_elapsed_micros: measurement.mean_elapsed_micros,
            median_elapsed_micros: measurement.median_elapsed_micros,
            p90_elapsed_micros: measurement.p90_elapsed_micros,
            min_elapsed_micros: measurement.min_elapsed_micros,
            max_elapsed_micros: measurement.max_elapsed_micros,
            source_objects: measurement.source_objects,
            evidence: measurement
                .evidence
                .into_iter()
                .map(|evidence| LabelledOcelTimeBetweenEvidence {
                    object: log.object_name(evidence.object),
                    source_event: log.event_name(evidence.source_event),
                    target_event: log.event_name(evidence.target_event),
                    source_at_micros: log.event_time(evidence.source_event),
                    target_at_micros: log.event_time(evidence.target_event),
                    elapsed_micros: evidence.elapsed_micros,
                })
                .collect(),
        })
        .collect()
}

type LabelledBottlenecks = (
    Vec<LabelledOcelLifecycleBottleneck>,
    Vec<LabelledOcelSynchronizationBottleneck>,
);

fn label_bottlenecks(
    log: &dyn Labels,
    ranking: &pm_core::OcelBottleneckRanking,
) -> LabelledBottlenecks {
    let lifecycle = ranking
        .lifecycle
        .iter()
        .cloned()
        .map(|item| LabelledOcelLifecycleBottleneck {
            rank: item.rank,
            object_type: log.object_type_name(item.object_type),
            source: log.type_name(item.source),
            target: log.type_name(item.target),
            occurrences: item.occurrences,
            objects: item.objects,
            total_elapsed_micros: item.total_elapsed_micros,
            mean_elapsed_micros: item.mean_elapsed_micros,
            share_millionths: item.share_millionths,
            evidence: item
                .evidence
                .into_iter()
                .map(|evidence| LabelledOcelDfgEvidence {
                    object: log.object_name(evidence.object),
                    source_event: log.event_name(evidence.source_event),
                    target_event: log.event_name(evidence.target_event),
                    source_at_micros: log.event_time(evidence.source_event),
                    target_at_micros: log.event_time(evidence.target_event),
                })
                .collect(),
        })
        .collect();
    let synchronization = ranking
        .synchronization
        .iter()
        .cloned()
        .map(|item| LabelledOcelSynchronizationBottleneck {
            rank: item.rank,
            activity: log.type_name(item.activity),
            object_types: item
                .object_types
                .into_iter()
                .map(|value| log.object_type_name(value))
                .collect(),
            multi_object_events: item.multi_object_events,
            eligible_events: item.eligible_events,
            insufficient_history_events: item.insufficient_history_events,
            total_arrival_spread_micros: item.total_arrival_spread_micros,
            mean_arrival_spread_micros: item.mean_arrival_spread_micros,
            share_millionths: item.share_millionths,
            evidence: item
                .evidence
                .into_iter()
                .map(|evidence| LabelledOcelSynchronizationEvidence {
                    event: log.event_name(evidence.event),
                    at_micros: log.event_time(evidence.event),
                    participating_objects: evidence.participating_objects,
                    historically_observed_objects: evidence.historically_observed_objects,
                    first_seen_objects: evidence.first_seen_objects,
                    arrival_spread_micros: evidence.arrival_spread_micros,
                    elapsed_after_last_prior_micros: evidence.elapsed_after_last_prior_micros,
                    elapsed_from_first_prior_micros: evidence.elapsed_from_first_prior_micros,
                    readiness: evidence
                        .readiness
                        .into_iter()
                        .map(|readiness| LabelledOcelSynchronizationReadiness {
                            object: log.object_name(readiness.object),
                            object_type: log.object_type_name(readiness.object_type),
                            prior_at_micros: readiness.prior_at,
                            prior_events: readiness
                                .prior_events
                                .into_iter()
                                .map(|event| log.event_name(event))
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    (lifecycle, synchronization)
}

const PROJECTION_SEMANTICS: &str = "This is an explicit projection: each retained object of the selected type becomes one case, and only directly related events enter that case. Source identifiers remain unchanged and simultaneous events remain an unordered timestamp layer. Convergence counts source events replicated into several projected cases. Divergence candidates are repeated activities within one projected case that involve distinct objects of another type; they indicate distortion risk, not proof of an artificial loop. Counts describe the complete active view, while projected case and event detail is bounded by cases_returned and event_limit_per_case. A truncated case may show only part of its final simultaneous layer.";
const CONFORMANCE_SEMANTICS: &str = "Token replay compares this active OCEL view with the explicitly supplied typed object-centric Petri-net baseline. Lifecycle control-flow is replayed per object without flattening, while shared-event participation and fixed-versus-variable multiplicity are checked separately. Global token fitness includes binding violations; lifecycle_fit describes only an individual object's control-flow replay. Out-of-scope events do not touch an active modeled object type. Totals cover the complete active view even when object and deviation detail is bounded. Fitness is diagnostic evidence, not proof of regulatory compliance, business-rule compliance, causation, or correctness of the baseline model.";
const ALIGNMENT_SEMANTICS: &str = "Optimal A* alignment compares each active object lifecycle with the explicitly supplied typed object-centric Petri-net baseline without choosing or flattening to one case notion. A log-only move is an observed event occurrence that the model skips; a visible-model move is expected model behavior absent from that object's lifecycle. Zero-cost synchronous and silent routing moves are counted but omitted from bounded deviation detail. Alignment fitness and total cost cover lifecycle control flow only; shared-event participation and fixed-versus-variable multiplicity violations are reported separately and do not alter optimal lifecycle paths. Out-of-scope events do not touch an active modeled object type. This is diagnostic evidence, not proof of regulatory compliance, business-rule compliance, causation, or correctness of the baseline model.";
const TIME_BETWEEN_SEMANTICS: &str = "Each target occurrence is measured from the most recent strictly earlier source layer on the same object. Results are grouped by the common object type. Simultaneous events are not ordered; elapsed time is not waiting time.";
const SYNCHRONIZATION_SEMANTICS: &str = "For each event linked to at least two distinct objects, each object's most recent strictly earlier event layer is its observable prior point. Objects first seen at the event are disclosed and excluded from readiness timing. Events with at least two historically observed objects decompose elapsed time into the spread between first and last prior points plus elapsed time from the last prior point to the event. These observations do not establish waiting, availability, causation, or execution duration.";
const BOTTLENECK_SEMANTICS: &str = "Lifecycle bottlenecks rank positive total start-to-start elapsed time across native object lifecycle transitions. Synchronization bottlenecks separately rank positive total spread between participating objects' strictly earlier lifecycle points. Zero-duration candidates remain in the disclosed denominators but are not labelled bottlenecks. The two rankings are not combined, and neither establishes waiting or causation.";

fn label_conformance_object(
    log: &dyn Labels,
    item: &pm_core::OcelObjectReplay,
) -> LabelledOcelConformanceObject {
    let item = item.clone();
    LabelledOcelConformanceObject {
        object: log.object_name(item.object),
        object_type: log.object_type_name(item.object_type),
        events: item.events,
        fitness_millionths: item.fitness_millionths,
        consumed_tokens: item.consumed_tokens,
        produced_tokens: item.produced_tokens,
        missing_tokens: item.missing_tokens,
        remaining_tokens: item.remaining_tokens,
        unmapped_events: item.unmapped_events,
        silent_firings: item.silent_firings,
        lifecycle_fit: item.lifecycle_fit,
    }
}

/// Least fit first, then lowest fitness, then type and item.
fn conformance_order(
    left: &LabelledOcelConformanceObject,
    right: &LabelledOcelConformanceObject,
) -> std::cmp::Ordering {
    left.lifecycle_fit
        .cmp(&right.lifecycle_fit)
        .then_with(|| left.fitness_millionths.cmp(&right.fitness_millionths))
        .then_with(|| left.object_type.cmp(&right.object_type))
        .then_with(|| left.object.cmp(&right.object))
}

fn label_alignment_object(
    log: &dyn Labels,
    object: pm_core::OcelObjectAlignment,
    detail_limit: usize,
) -> LabelledOcelAlignmentObject {
    let deviations = object
        .moves
        .into_iter()
        .filter_map(|movement| label_alignment_deviation(log, movement))
        .collect::<Vec<_>>();
    let deviations_available = deviations.len();
    let deviations = deviations
        .into_iter()
        .take(detail_limit)
        .collect::<Vec<_>>();
    LabelledOcelAlignmentObject {
        object: log.object_name(object.object),
        object_type: log.object_type_name(object.object_type),
        events: object.events,
        cost: object.cost,
        denominator: object.denominator,
        fitness_millionths: object.fitness_millionths,
        synchronous_moves: object.synchronous_moves,
        invisible_moves: object.invisible_moves,
        log_moves: object.log_moves,
        visible_model_moves: object.visible_model_moves,
        lifecycle_fit: object.fit,
        deviations_available,
        deviations_returned: deviations.len(),
        deviations_truncated: deviations.len() < deviations_available,
        deviations,
    }
}

/// Highest cost first, then lowest fitness, then type and item.
fn alignment_order(
    left: &LabelledOcelAlignmentObject,
    right: &LabelledOcelAlignmentObject,
) -> std::cmp::Ordering {
    std::cmp::Reverse(left.cost)
        .cmp(&std::cmp::Reverse(right.cost))
        .then_with(|| left.fitness_millionths.cmp(&right.fitness_millionths))
        .then_with(|| left.object_type.cmp(&right.object_type))
        .then_with(|| left.object.cmp(&right.object))
}

/// Names for results: a loaded log's, or a log on disk's.
trait Labels {
    fn type_name(&self, id: pm_core::EventTypeId) -> String;
    fn object_type_name(&self, id: pm_core::ObjectTypeId) -> String;
    fn qualifier_name(&self, id: pm_core::QualifierId) -> String;
    fn event_name(&self, id: pm_core::OcelEventId) -> String;
    fn event_time(&self, id: pm_core::OcelEventId) -> i64;
    fn event_type_of(&self, id: pm_core::OcelEventId) -> pm_core::EventTypeId;
    fn object_name(&self, id: pm_core::ObjectId) -> String;
    fn relationship_ends(&self, index: usize) -> (pm_core::ObjectId, pm_core::ObjectId);
    fn activity_names(&self) -> Vec<String>;
    fn object_type_names(&self) -> Vec<String>;
}

fn resolve_type_name(
    log: &dyn Labels,
    name: &str,
) -> Result<pm_core::ObjectTypeId, OcelQueryError> {
    log.object_type_names()
        .iter()
        .position(|candidate| candidate == name)
        .and_then(|index| u32::try_from(index).ok())
        .map(pm_core::ObjectTypeId)
        .ok_or_else(|| OcelQueryError::UnknownObjectType(name.to_owned()))
}

fn resolve_activity_name(
    log: &dyn Labels,
    name: &str,
) -> Result<pm_core::EventTypeId, OcelQueryError> {
    log.activity_names()
        .iter()
        .position(|candidate| candidate == name)
        .and_then(|index| u32::try_from(index).ok())
        .map(pm_core::EventTypeId)
        .ok_or_else(|| OcelQueryError::UnknownActivity(name.to_owned()))
}

impl Labels for OcelLog {
    fn type_name(&self, id: pm_core::EventTypeId) -> String {
        event_type(self, id)
    }
    fn object_type_name(&self, id: pm_core::ObjectTypeId) -> String {
        object_type(self, id)
    }
    fn qualifier_name(&self, id: pm_core::QualifierId) -> String {
        qualifier(self, id)
    }
    fn event_name(&self, id: pm_core::OcelEventId) -> String {
        event_id(self, id)
    }
    fn event_time(&self, id: pm_core::OcelEventId) -> i64 {
        self.events[id.0 as usize].at
    }
    fn event_type_of(&self, id: pm_core::OcelEventId) -> pm_core::EventTypeId {
        self.events[id.0 as usize].event_type
    }
    fn object_name(&self, id: pm_core::ObjectId) -> String {
        object_id(self, id)
    }
    fn relationship_ends(&self, index: usize) -> (pm_core::ObjectId, pm_core::ObjectId) {
        let relation = self.object_object_relations[index];
        (relation.source, relation.target)
    }
    fn activity_names(&self) -> Vec<String> {
        self.event_types.iter().map(|t| t.name.clone()).collect()
    }
    fn object_type_names(&self) -> Vec<String> {
        self.object_types.iter().map(|t| t.name.clone()).collect()
    }
}

impl Labels for pm_ingest::DiskLog {
    fn type_name(&self, id: pm_core::EventTypeId) -> String {
        self.event_types
            .get(id.0 as usize)
            .cloned()
            .unwrap_or_else(|| "<unknown event type>".to_owned())
    }
    fn object_type_name(&self, id: pm_core::ObjectTypeId) -> String {
        self.object_types
            .get(id.0 as usize)
            .cloned()
            .unwrap_or_else(|| "<unknown object type>".to_owned())
    }
    fn qualifier_name(&self, id: pm_core::QualifierId) -> String {
        self.qualifiers
            .get(id.0 as usize)
            .cloned()
            .unwrap_or_else(|| "<unknown qualifier>".to_owned())
    }
    fn event_name(&self, id: pm_core::OcelEventId) -> String {
        self.event_label(id)
            .unwrap_or_else(|_| "<unknown event>".to_owned())
    }
    fn event_time(&self, id: pm_core::OcelEventId) -> i64 {
        self.event_at(id).unwrap_or_default()
    }
    fn event_type_of(&self, id: pm_core::OcelEventId) -> pm_core::EventTypeId {
        self.event_kind(id)
            .unwrap_or(pm_core::EventTypeId(u32::MAX))
    }
    fn object_name(&self, id: pm_core::ObjectId) -> String {
        self.object_label(id)
            .unwrap_or_else(|_| "<unknown object>".to_owned())
    }
    fn activity_names(&self) -> Vec<String> {
        self.event_types.clone()
    }
    fn object_type_names(&self) -> Vec<String> {
        self.object_types.clone()
    }
    fn relationship_ends(&self, index: usize) -> (pm_core::ObjectId, pm_core::ObjectId) {
        self.relationship(index)
            .unwrap_or((pm_core::ObjectId(u32::MAX), pm_core::ObjectId(u32::MAX)))
    }
}

fn event_type(log: &OcelLog, id: pm_core::EventTypeId) -> String {
    log.event_type(id)
        .map(|event_type| event_type.name.clone())
        .unwrap_or_else(|| "<unknown event type>".to_owned())
}

fn resolve_event_type(log: &OcelLog, name: &str) -> Result<pm_core::EventTypeId, OcelQueryError> {
    log.event_types
        .iter()
        .position(|event_type| event_type.name == name)
        .and_then(|index| u32::try_from(index).ok())
        .map(pm_core::EventTypeId)
        .ok_or_else(|| OcelQueryError::UnknownActivity(name.to_owned()))
}

fn resolve_object_type(log: &OcelLog, name: &str) -> Result<pm_core::ObjectTypeId, OcelQueryError> {
    log.object_types
        .iter()
        .position(|object_type| object_type.name == name)
        .and_then(|index| u32::try_from(index).ok())
        .map(pm_core::ObjectTypeId)
        .ok_or_else(|| OcelQueryError::UnknownObjectType(name.to_owned()))
}

fn object_type(log: &OcelLog, id: pm_core::ObjectTypeId) -> String {
    log.object_type(id)
        .map(|object_type| object_type.name.clone())
        .unwrap_or_else(|| "<unknown object type>".to_owned())
}

fn event_id(log: &OcelLog, id: pm_core::OcelEventId) -> String {
    log.event_label(id).unwrap_or("<unknown event>").to_owned()
}

fn object_id(log: &OcelLog, id: pm_core::ObjectId) -> String {
    log.object_label(id)
        .unwrap_or("<unknown object>")
        .to_owned()
}

fn qualifier(log: &OcelLog, id: pm_core::QualifierId) -> String {
    log.qualifier_label(id)
        .unwrap_or("<unknown qualifier>")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../../testdata/ocel/order-management.jsonocel");
    const DEVIATION_FIXTURE: &[u8] =
        include_bytes!("../../../testdata/ocel/order-management-missing-package.jsonocel");

    #[test]
    fn canonical_log_becomes_a_labelled_self_contained_analysis() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let analysis = analyze_ocel_log("fixture", &log, &OcelFilter::default()).unwrap();
        assert!(analysis.ready_for_analysis);
        assert_eq!(analysis.catalog.object_types, ["order", "item", "package"]);
        assert_eq!(analysis.scope.events_shown, 3);
        assert_eq!(
            analysis.graphs.as_ref().unwrap().activities[0].activity,
            "place order"
        );
        assert_eq!(analysis.graphs.as_ref().unwrap().activities[0].events, 1);
    }

    #[test]
    fn canonical_json_xml_and_sqlite_produce_the_same_labelled_analysis() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/ocel");
        let mut json = analyze_ocel(
            root.join("order-management.jsonocel"),
            &OcelFilter::default(),
        )
        .unwrap();
        let mut xml = analyze_ocel(
            root.join("order-management.xmlocel"),
            &OcelFilter::default(),
        )
        .unwrap();
        let mut sqlite =
            analyze_ocel(root.join("order-management.sqlite"), &OcelFilter::default()).unwrap();
        json.source.clear();
        xml.source.clear();
        sqlite.source.clear();
        assert_eq!(xml, json);
        assert_eq!(sqlite, json);
    }

    #[test]
    fn filtering_is_disclosed_and_resolved_by_labels() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let analysis = analyze_ocel_log(
            "fixture",
            &log,
            &OcelFilter {
                object_types: vec!["package".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(analysis.scope.narrowed);
        assert_eq!(analysis.scope.objects_shown, 1);
        assert_eq!(analysis.scope.events_shown, 2);
        let edge = &analysis.graphs.unwrap().directly_follows[0];
        assert_eq!(edge.object_type, "package");
        assert_eq!(edge.source, "create package");
        assert_eq!(edge.target, "deliver package");
        assert_eq!(edge.mean_elapsed_micros, 86_400_000_000);
        assert_eq!(edge.evidence[0].object, "package-1");
    }

    #[test]
    fn blocking_quality_never_emits_unsafe_graphs() {
        let mut log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        log.event_object_relations
            .push(log.event_object_relations[0]);
        let analysis = analyze_ocel_log("fixture", &log, &OcelFilter::default()).unwrap();
        assert!(!analysis.ready_for_analysis);
        assert!(analysis.graphs.is_none());
        assert!(analysis.variants.is_none());
        assert_eq!(analysis.quality.summary.errors, 1);
    }

    #[test]
    fn variants_are_object_type_specific_and_evidence_backed() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let analysis = analyze_ocel_log("fixture", &log, &OcelFilter::default()).unwrap();
        let variants = analysis.variants.unwrap();
        assert_eq!(variants.len(), 3);
        let items = variants
            .iter()
            .find(|variant| variant.object_type == "item")
            .unwrap();
        assert_eq!(items.objects, 2);
        assert_eq!(items.share_millionths, 1_000_000);
        assert_eq!(items.layers[0].activities, ["place order"]);
        assert_eq!(items.layers[1].activities, ["create package"]);
        assert_eq!(items.evidence_objects, ["item-1", "item-2"]);
    }

    #[test]
    fn object_lifecycle_returns_original_timeline_and_rejects_filtered_objects() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let lifecycle =
            inspect_ocel_object_log("fixture", &log, &OcelFilter::default(), "package-1").unwrap();
        assert_eq!(lifecycle.object_type, "package");
        assert_eq!(lifecycle.events.len(), 2);
        assert_eq!(lifecycle.events[0].event, "e-pack");
        assert_eq!(lifecycle.events[1].activity, "deliver package");
        let error = inspect_ocel_object_log(
            "fixture",
            &log,
            &OcelFilter {
                object_types: vec!["item".into()],
                ..Default::default()
            },
            "package-1",
        )
        .unwrap_err();
        assert!(matches!(error, OcelQueryError::ObjectOutsideView(_)));
    }

    #[test]
    fn time_between_activities_is_grouped_by_the_common_object_type() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let report = measure_ocel_time_between_log(
            "fixture",
            &log,
            &OcelFilter::default(),
            "place order",
            "create package",
        )
        .unwrap();
        assert_eq!(report.measurements.len(), 1);
        let measurement = &report.measurements[0];
        assert_eq!(measurement.object_type, "item");
        assert_eq!(measurement.observations, 2);
        assert_eq!(measurement.objects, 2);
        assert_eq!(measurement.mean_elapsed_micros, 86_400_000_000);
        assert_eq!(measurement.evidence[0].object, "item-1");
    }

    #[test]
    fn time_between_rejects_unknown_activities_and_blocking_quality() {
        let mut log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let error = measure_ocel_time_between_log(
            "fixture",
            &log,
            &OcelFilter::default(),
            "missing",
            "create package",
        )
        .unwrap_err();
        assert!(matches!(error, OcelQueryError::UnknownActivity(_)));
        log.event_object_relations
            .push(log.event_object_relations[0]);
        let error = measure_ocel_time_between_log(
            "fixture",
            &log,
            &OcelFilter::default(),
            "place order",
            "create package",
        )
        .unwrap_err();
        assert!(matches!(error, OcelQueryError::QualityBlocked));
    }

    #[test]
    fn synchronization_decomposes_canonical_multi_object_events() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let report =
            analyze_ocel_synchronization_log("fixture", &log, &OcelFilter::default()).unwrap();
        assert_eq!(report.activities.len(), 2);
        let place = report
            .activities
            .iter()
            .find(|activity| activity.activity == "place order")
            .unwrap();
        assert_eq!(place.multi_object_events, 1);
        assert_eq!(place.insufficient_history_events, 1);
        assert_eq!(place.mean_arrival_spread_micros, None);

        let package = report
            .activities
            .iter()
            .find(|activity| activity.activity == "create package")
            .unwrap();
        assert_eq!(package.object_types, ["item", "package"]);
        assert_eq!(package.eligible_events, 1);
        assert_eq!(package.historically_observed_object_participations, 2);
        assert_eq!(package.first_seen_object_participations, 1);
        assert_eq!(package.mean_arrival_spread_micros, Some(0));
        assert_eq!(
            package.mean_elapsed_after_last_prior_micros,
            Some(86_400_000_000)
        );
        assert_eq!(
            package.mean_elapsed_from_first_prior_micros,
            Some(86_400_000_000)
        );
        assert_eq!(package.evidence[0].event, "e-pack");
        assert_eq!(package.evidence[0].first_seen_objects, 1);
        assert_eq!(package.evidence[0].readiness[0].object, "item-1");
        assert_eq!(package.evidence[0].readiness[0].prior_events, ["e-place"]);
    }

    #[test]
    fn synchronization_respects_filters_and_blocks_unsafe_quality() {
        let mut log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let report = analyze_ocel_synchronization_log(
            "fixture",
            &log,
            &OcelFilter {
                object_types: vec!["package".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.activities.is_empty());

        log.event_object_relations
            .push(log.event_object_relations[0]);
        let error =
            analyze_ocel_synchronization_log("fixture", &log, &OcelFilter::default()).unwrap_err();
        assert!(matches!(error, OcelQueryError::QualityBlocked));
    }

    #[test]
    fn object_centric_bottlenecks_rank_exact_elapsed_burden_with_denominators() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let report = rank_ocel_bottlenecks_log("fixture", &log, &OcelFilter::default()).unwrap();
        assert_eq!(report.lifecycle_candidates, 2);
        assert_eq!(report.lifecycle_positive_candidates, 2);
        assert_eq!(report.total_lifecycle_elapsed_micros, 259_200_000_000);
        assert_eq!(report.lifecycle[0].rank, 1);
        assert_eq!(report.lifecycle[0].object_type, "item");
        assert_eq!(report.lifecycle[0].source, "place order");
        assert_eq!(report.lifecycle[0].target, "create package");
        assert_eq!(report.lifecycle[0].total_elapsed_micros, 172_800_000_000);
        assert_eq!(report.lifecycle[0].share_millionths, 666_666);
        assert_eq!(report.lifecycle[0].evidence[0].object, "item-1");
        assert_eq!(report.synchronization_eligible_activities, 1);
        assert_eq!(report.synchronization_positive_activities, 0);
        assert_eq!(report.total_arrival_spread_micros, 0);
        assert!(report.synchronization.is_empty());
    }

    #[test]
    fn object_centric_bottleneck_ranking_respects_view_and_quality() {
        let mut log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let report = rank_ocel_bottlenecks_log(
            "fixture",
            &log,
            &OcelFilter {
                object_types: vec!["package".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.lifecycle.len(), 1);
        assert_eq!(report.lifecycle[0].object_type, "package");
        assert_eq!(report.lifecycle[0].share_millionths, 1_000_000);

        log.event_object_relations
            .push(log.event_object_relations[0]);
        let error = rank_ocel_bottlenecks_log("fixture", &log, &OcelFilter::default()).unwrap_err();
        assert!(matches!(error, OcelQueryError::QualityBlocked));
    }

    #[test]
    fn projection_is_labelled_bounded_and_discloses_occurrence_inflation() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let report =
            project_ocel_cases_log("fixture", &log, &OcelFilter::default(), "item", 1, 1).unwrap();
        assert_eq!(report.case_object_type, "item");
        assert_eq!(report.source_events, 3);
        assert_eq!(report.projected_cases, 2);
        assert_eq!(report.included_unique_events, 2);
        assert_eq!(report.excluded_events, 1);
        assert_eq!(report.projected_event_occurrences, 4);
        assert_eq!(report.convergence_events, 2);
        assert_eq!(report.convergence_additional_occurrences, 2);
        assert_eq!(report.divergence_candidates, 0);
        assert_eq!(report.cases_available, 2);
        assert_eq!(report.cases_returned, 1);
        assert_eq!(report.event_limit_per_case, 1);
        assert_eq!(report.cases[0].case_object, "item-1");
        assert_eq!(report.cases[0].events_available, 2);
        assert_eq!(report.cases[0].events_returned, 1);
        assert!(report.cases[0].truncated);
        assert_eq!(report.cases[0].layers[0].events[0].source_event, "e-place");
        assert_eq!(report.convergence_evidence[0].source_event, "e-place");
        assert_eq!(
            report.convergence_evidence[0].evidence_cases,
            ["item-1", "item-2"]
        );
    }

    #[test]
    fn projection_rejects_unknown_types_and_blocking_quality() {
        let mut log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let error =
            project_ocel_cases_log("fixture", &log, &OcelFilter::default(), "missing", 5, 100)
                .unwrap_err();
        assert!(matches!(error, OcelQueryError::UnknownObjectType(_)));

        log.event_object_relations
            .push(log.event_object_relations[0]);
        let error = project_ocel_cases_log("fixture", &log, &OcelFilter::default(), "item", 5, 100)
            .unwrap_err();
        assert!(matches!(error, OcelQueryError::QualityBlocked));
    }

    #[test]
    fn object_centric_model_is_labelled_typed_and_self_contained() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let report = discover_ocel_model_log(
            "fixture",
            &log,
            &OcelFilter::default(),
            OcelDiscoveryConfig::default(),
        )
        .unwrap();

        assert_eq!(report.object_types.len(), 3);
        assert_eq!(
            report
                .object_types
                .iter()
                .map(|stats| stats.object_type.as_str())
                .collect::<Vec<_>>(),
            ["order", "item", "package"]
        );
        assert!(
            report
                .places
                .iter()
                .all(|place| ["order", "item", "package"].contains(&place.object_type.as_str()))
        );
        assert_eq!(
            report
                .transitions
                .iter()
                .filter(|transition| transition.activity.as_deref() == Some("create package"))
                .count(),
            1
        );
        let pack = report
            .transitions
            .iter()
            .find(|transition| transition.activity.as_deref() == Some("create package"))
            .unwrap();
        let touched_types = pack
            .inputs
            .iter()
            .chain(&pack.outputs)
            .map(|arc| report.places[arc.place as usize].object_type.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(touched_types, BTreeSet::from(["item", "package"]));
        assert!(pack.inputs.iter().chain(&pack.outputs).any(|arc| {
            report.places[arc.place as usize].object_type == "item" && arc.variable
        }));
        assert!(pack.inputs.iter().chain(&pack.outputs).any(|arc| {
            report.places[arc.place as usize].object_type == "package" && !arc.variable
        }));
        assert!(report.variable_arcs > 0);
        assert!(report.fixed_arcs > 0);
        assert!(report.semantics.contains("does not prove causation"));
    }

    #[test]
    fn object_centric_model_respects_filters_and_quality() {
        let mut log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let report = discover_ocel_model_log(
            "fixture",
            &log,
            &OcelFilter {
                object_types: vec!["package".into()],
                ..Default::default()
            },
            OcelDiscoveryConfig::default(),
        )
        .unwrap();
        assert_eq!(report.object_types.len(), 1);
        assert_eq!(report.object_types[0].object_type, "package");
        assert!(
            report
                .places
                .iter()
                .all(|place| place.object_type == "package")
        );
        assert!(
            report
                .transitions
                .iter()
                .all(|transition| { transition.activity.as_deref() != Some("place order") })
        );

        log.event_object_relations
            .push(log.event_object_relations[0]);
        let error = discover_ocel_model_log(
            "fixture",
            &log,
            &OcelFilter::default(),
            OcelDiscoveryConfig::default(),
        )
        .unwrap_err();
        assert!(matches!(error, OcelQueryError::QualityBlocked));
    }

    #[test]
    fn object_centric_model_names_the_object_when_time_is_ambiguous() {
        let mut log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        log.events[1].at = log.events[0].at;
        let error = discover_ocel_model_log(
            "fixture",
            &log,
            &OcelFilter::default(),
            OcelDiscoveryConfig::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            OcelQueryError::SimultaneousObjectEvents {
                ref object,
                at_micros: 0
            } if object == "item-1"
        ));
    }

    #[test]
    fn object_centric_conformance_uses_a_portable_explicit_baseline() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let baseline = discover_ocel_model_log(
            "training-log",
            &log,
            &OcelFilter::default(),
            OcelDiscoveryConfig::default(),
        )
        .unwrap();
        let baseline_json = serde_json::to_vec(&baseline).unwrap();
        let reloaded: OcelModelReport = serde_json::from_slice(&baseline_json).unwrap();

        let report = check_ocel_conformance_log(
            "tested-log",
            &log,
            &OcelFilter::default(),
            "baseline.ocelmodel.json",
            &reloaded,
            100,
        )
        .unwrap();

        assert_eq!(report.model_source, "baseline.ocelmodel.json");
        assert_eq!(report.model_training_source, "training-log");
        assert_eq!(report.replayed_objects, 4);
        assert_eq!(report.lifecycle_fit_objects, 4);
        assert_eq!(report.replayed_events, 3);
        assert_eq!(report.fit_events, 3);
        assert_eq!(report.fitness_millionths, 1_000_000);
        assert_eq!(report.deviations_available, 0);
        assert!(report.semantics.contains("explicitly supplied"));
        assert!(
            report
                .semantics
                .contains("not proof of regulatory compliance")
        );
    }

    #[test]
    fn object_centric_alignment_is_optimal_labelled_and_bounded() {
        let training = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let observed = pm_ingest::parse_ocel_json(DEVIATION_FIXTURE).unwrap();
        let baseline = discover_ocel_model_log(
            "training-log",
            &training,
            &OcelFilter::default(),
            OcelDiscoveryConfig::default(),
        )
        .unwrap();
        let report = align_ocel_conformance_log(
            "observed-log",
            &observed,
            &OcelFilter::default(),
            "baseline.ocelmodel.json",
            &baseline,
            1,
        )
        .unwrap();

        assert_eq!(report.aligned_objects, 4);
        assert_eq!(report.lifecycle_fit_objects, 3);
        assert_eq!(report.total_cost, 1);
        assert_eq!(report.binding_violations, 1);
        assert_eq!(report.objects_available, 4);
        assert_eq!(report.objects_returned, 1);
        assert!(report.objects_truncated);
        let package = &report.objects[0];
        assert_eq!(package.object, "package-1");
        assert_eq!(package.cost, 1);
        assert_eq!(package.visible_model_moves, 1);
        assert_eq!(package.deviations_available, 1);
        assert!(matches!(
            package.deviations[0],
            LabelledOcelAlignmentDeviation::VisibleModel {
                ref activity,
                ..
            } if activity == "create package"
        ));
        assert_eq!(report.binding_deviations_available, 1);
        assert!(report.semantics.contains("Optimal A* alignment"));
        assert!(report.semantics.contains("reported separately"));
    }

    #[test]
    fn object_centric_conformance_keeps_complete_totals_when_evidence_is_bounded() {
        let mut log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let baseline = discover_ocel_model_log(
            "training-log",
            &log,
            &OcelFilter::default(),
            OcelDiscoveryConfig::default(),
        )
        .unwrap();
        // Remove package participation from "create package" while retaining
        // the event and package lifecycle. The explicit baseline must catch
        // both shared binding and lifecycle consequences.
        log.event_object_relations.remove(3);

        let report = check_ocel_conformance_log(
            "tested-log",
            &log,
            &OcelFilter::default(),
            "baseline",
            &baseline,
            1,
        )
        .unwrap();

        assert!(report.fitness_millionths < 1_000_000);
        assert!(report.binding_violations > 0);
        assert!(report.deviations_available > report.deviations_returned);
        assert_eq!(report.deviations_returned, 1);
        assert!(report.deviations_truncated);
        assert_eq!(report.objects_available, 4);
        assert_eq!(report.objects_returned, 1);
        assert!(report.objects_truncated);
        assert!(matches!(
            report.deviations[0],
            LabelledOcelConformanceDeviation::MissingObjectParticipation {
                ref event,
                ref object_type,
                ..
            } if event == "e-pack" && object_type == "package"
        ));
    }

    #[test]
    fn object_centric_conformance_respects_active_object_type_filters() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let baseline = discover_ocel_model_log(
            "training-log",
            &log,
            &OcelFilter::default(),
            OcelDiscoveryConfig::default(),
        )
        .unwrap();
        let filter = OcelFilter {
            object_types: vec!["package".to_owned()],
            ..Default::default()
        };

        let report =
            check_ocel_conformance_log("tested-log", &log, &filter, "baseline", &baseline, 10)
                .unwrap();

        assert_eq!(report.replayed_objects, 1);
        assert_eq!(report.replayed_events, 2);
        assert_eq!(report.out_of_scope_events, 0);
        assert_eq!(report.fitness_millionths, 1_000_000);
        assert!(report.deviations.is_empty());
    }

    #[test]
    fn object_centric_conformance_rejects_schema_mismatch_and_bad_models() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let mut baseline = discover_ocel_model_log(
            "training-log",
            &log,
            &OcelFilter::default(),
            OcelDiscoveryConfig::default(),
        )
        .unwrap();
        baseline.places[0].object_type = "unknown type".to_owned();
        let error = check_ocel_conformance_log(
            "tested-log",
            &log,
            &OcelFilter::default(),
            "baseline",
            &baseline,
            10,
        )
        .unwrap_err();
        assert!(matches!(error, OcelQueryError::UnknownObjectType(_)));

        let mut baseline = discover_ocel_model_log(
            "training-log",
            &log,
            &OcelFilter::default(),
            OcelDiscoveryConfig::default(),
        )
        .unwrap();
        baseline.transitions[0].id = 99;
        let error = check_ocel_conformance_log(
            "tested-log",
            &log,
            &OcelFilter::default(),
            "baseline",
            &baseline,
            10,
        )
        .unwrap_err();
        assert!(matches!(error, OcelQueryError::Conformance(_)));
    }

    #[test]
    fn json_is_deterministic_and_contains_no_internal_identifier_objects() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let first = serde_json::to_string(
            &analyze_ocel_log("fixture", &log, &OcelFilter::default()).unwrap(),
        )
        .unwrap();
        let second = serde_json::to_string(
            &analyze_ocel_log("fixture", &log, &OcelFilter::default()).unwrap(),
        )
        .unwrap();
        assert_eq!(first, second);
        assert!(first.contains("package-1"));
        assert!(!first.contains("event_type\":0"));
        assert!(!first.contains("object_type\":0"));
    }

    /// A large export keeps its most common routes per item type, not
    /// every rare one, and says how many it left out.
    #[test]
    fn keeps_the_most_common_variants_of_each_item_type() {
        let mut builder = pm_core::OcelBuilder::new();
        builder.declare_object_type("order", Vec::new()).unwrap();
        for step in 0..=VARIANTS_PER_TYPE {
            builder
                .declare_event_type(&format!("step {step}"), Vec::new())
                .unwrap();
        }
        // Each order follows its own route: one more variant than is kept.
        for order in 0..=VARIANTS_PER_TYPE {
            let id = format!("o{order}");
            builder.add_object(&id, "order").unwrap();
            let event = format!("e{order}");
            builder
                .add_event(&event, &format!("step {order}"), 0)
                .unwrap();
            builder.relate_event_object(&event, &id, "o").unwrap();
        }
        let analysis = analyze_ocel_log("test", &builder.build(), &OcelFilter::default()).unwrap();
        assert_eq!(analysis.variants.unwrap().len(), VARIANTS_PER_TYPE);
        assert_eq!(analysis.variants_omitted, 1);
    }

    /// Reading a SQLite log without attribute values gives the same overview
    /// as reading everything, on the official sample (whose items carry
    /// attribute histories) and on a prepared SAP log.
    #[test]
    fn the_overview_is_the_same_without_attribute_values() {
        let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.sqlite");
        let full =
            analyze_ocel_log("x", &read_ocel(&sample).unwrap(), &OcelFilter::default()).unwrap();
        let lean = analyze_ocel_log(
            "x",
            &pm_ingest::read_ocel_sqlite_skeleton(&sample).unwrap(),
            &OcelFilter::default(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&lean).unwrap(),
            serde_json::to_value(&full).unwrap()
        );
    }
}
