//! The shared query layer.
//!
//! Browser, CLI, and MCP server all call this and nothing below it. A
//! capability added here appears in all three; logic added *above* here would
//! exist in only one, and the three front ends would eventually disagree about
//! a number — which would destroy the credibility the evidence-linking is
//! meant to build.
//!
//! Every result type is serializable, because the CLI and the agent surface
//! both emit JSON and must emit the *same* JSON.

pub mod analysis;
pub mod bpmn_export;
pub mod brief;
pub mod budget;
pub mod cache;
pub mod comparison;
pub mod conformance;
pub mod discovery;
pub mod drilldown;
pub mod drivers;
pub mod exploration;
pub mod findings;
pub mod follow;
pub mod headline;
pub mod improvement;
pub mod inspection;
pub mod items;
pub mod journey;
pub mod linked_interval;
pub mod map_view;
pub mod ocel;
pub mod patterns;
pub mod process;
pub mod process_model;
pub mod propose;
pub mod quality;
pub mod targeting;
pub mod timing;
pub mod values;

pub use analysis::{
    Analysis, Blocked, Bottleneck, Edge, HandoverSummary, LoadedLog, LoopSummary, Node, Offer,
    OrganizationSummary, Outcome, QueryError, ReworkActivitySummary, ReworkSummary, Summary,
    VariantSummary, View, analyze, analyze_filtered, analyze_rules, analyze_table_filtered,
    assess_quality, load_log, timestamp_reading,
};
pub use bpmn_export::map_view_bpmn;
pub use brief::brief;
pub use cache::{AnalysisCache, CacheKey};
pub use comparison::{
    ActivityDifference, CohortComparison, CohortSummary, ComparisonError, PeriodComparison,
    PeriodSummary, compare_cohorts, compare_periods,
};
pub use conformance::{
    AlignedCase, AlignmentDeviation, AlignmentReport, ConformanceError, ConformanceReport,
    DeviatingCase, DeviationEvidence, ExpectedStep, FormalModelError, PortableFormalModel,
    align_conformance, check_conformance, expected_steps, has_silent_steps,
    parse_formal_model_json, validate_formal_model,
};
pub use discovery::{
    FormalDiscovery, HeuristicsDiscovery, ImportedFormalModel, LabelledAndBinding,
    LabelledHeuristicArc, LabelledPetriNet, LabelledProcessTree, LabelledTransition, NetDiscovery,
    discover_alpha_formal, discover_formal, discover_heuristics_formal, import_bpmn_model,
};
pub use drilldown::{
    CaseDetail, CaseEvent, Evidence, count_cases, find_cases, inspect_case, inspect_case_read_as,
};
pub use findings::{FindingKind, KeyFinding, Severity, journey_findings, key_findings};
pub use follow::{FollowSummary, OBJECT_HISTORY_SEMANTICS, follow_ocel, follow_ocel_log};
pub use headline::{Headline, HeadlineValue, headline};
pub use inspection::{ColumnInspection, LogInspection, RoleInspection, inspect_log};
pub use items::{ItemOrder, ItemPage, item_page};
pub use journey::{
    Journey, JourneyHandoff, JourneyStage, measure_journey_plan, ocel_journey, ocel_journey_disk,
    ocel_journey_log,
};
pub use map_view::{
    MapEdge, MapSelection, MapView, MapViewError, RankedActivity, RankedVariant, map_view,
};
pub use ocel::{
    LabelledOcelActivity, LabelledOcelAlignmentDeviation, LabelledOcelAlignmentObject,
    LabelledOcelConformanceDeviation, LabelledOcelConformanceObject,
    LabelledOcelConvergenceEvidence, LabelledOcelDfgEvidence, LabelledOcelDirectlyFollows,
    LabelledOcelDivergenceEvidence, LabelledOcelGraphs, LabelledOcelInteraction,
    LabelledOcelLifecycle, LabelledOcelLifecycleBottleneck, LabelledOcelLifecycleEvent,
    LabelledOcelModelArc, LabelledOcelModelObjectType, LabelledOcelModelPlace,
    LabelledOcelModelTransition, LabelledOcelParticipation, LabelledOcelProjectedCase,
    LabelledOcelProjectedEvent, LabelledOcelProjectionLayer, LabelledOcelRelationship,
    LabelledOcelRelationshipEvidence, LabelledOcelSynchronization,
    LabelledOcelSynchronizationBottleneck, LabelledOcelSynchronizationEvidence,
    LabelledOcelSynchronizationReadiness, LabelledOcelTimeBetween, LabelledOcelTimeBetweenEvidence,
    LabelledOcelVariant, LabelledOcelVariantLayer, OcelAlignmentConformanceReport, OcelAnalysis,
    OcelBottleneckReport, OcelCatalog, OcelConformanceReport, OcelModelReport,
    OcelProjectionReport, OcelQueryError, OcelScope, OcelSynchronizationReport,
    OcelTimeBetweenReport, align_ocel_conformance, align_ocel_conformance_log, analyze_ocel,
    analyze_ocel_disk, analyze_ocel_log, analyze_ocel_synchronization,
    analyze_ocel_synchronization_disk, analyze_ocel_synchronization_log, check_ocel_conformance,
    check_ocel_conformance_log, discover_ocel_model, discover_ocel_model_disk,
    discover_ocel_model_log, inspect_ocel_object, inspect_ocel_object_disk,
    inspect_ocel_object_log, measure_ocel_time_between, measure_ocel_time_between_disk,
    measure_ocel_time_between_log, project_ocel_cases, project_ocel_cases_disk,
    project_ocel_cases_log, rank_ocel_bottlenecks, rank_ocel_bottlenecks_disk,
    rank_ocel_bottlenecks_log,
};
pub use ocel::{OcelNames, find_ocel_object, ocel_names};
pub use ocel::{align_ocel_conformance_disk, check_ocel_conformance_disk};
pub use pm_core::filter::Filter;
pub use process::{ProcessProfile, ProcessType, recognise};
pub use propose::propose_mapping;
pub use quality::{
    DataQualityReport, QualityDecision, QualityFinding, QualityOption, QualitySeverity,
    QualitySummary,
};
pub use targeting::{
    AutomationReport, Divergence, ManualStep, ResourceReport, ResourceSummary, RoutingContext,
    automation_report, build_automation_report, build_resource_report, resource_report,
};
pub use timing::{DurationBin, SlowCase, TimingSummary, summarize_timing};
pub use values::{
    ValueError, ValueGroup, ValueGrouping, ValueStats, ValueSummary, summarize_values,
};
mod attribute_choices;
pub use attribute_choices::{AttributeChoice, AttributeChoices, attribute_choices};
