//! Core types for process mining.
//!
//! This crate knows nothing about DuckDB, HTTP, or the CLI. It operates on
//! compact dictionary-encoded identifiers and ordered sequences, which is what
//! keeps the algorithms testable in isolation.
//!
//! # Determinism
//!
//! Every structure here is deterministic. Identifiers are assigned in
//! first-seen order, and no output depends on hash-map iteration order.
//! Reprocessing identical input must produce byte-identical results.

pub mod alignment;
pub mod alpha;
pub mod automation;
pub mod declare;
pub mod dfg;
pub mod dict;
pub mod filter;
pub mod formal;
pub mod handover;
pub mod heuristics;
pub mod inductive;
pub mod log;
pub mod number;
pub mod ocel;
pub mod ocel_bottlenecks;
pub mod ocel_discovery;
pub mod ocel_filter;
pub mod ocel_graph;
pub mod ocel_performance;
pub mod ocel_projection;
pub mod ocel_replay;
pub mod ocel_synchronization;
pub mod ocel_variants;
pub mod replay;
pub mod rework;
pub mod steps;
pub mod trace;
pub mod variants;
pub mod window;

pub use alignment::{AlignmentError, AlignmentMove, CaseAlignment, LogAlignment, align_log};
pub use alpha::{AlphaError, AlphaModel, discover_alpha};
pub use automation::{
    Automation, BreakPoint, Opportunity, OpportunityContext, RankingBasis, ResourceClassification,
    ResourceKind, ResourceProfile, profile_resources, suggest,
};
pub use dfg::{Dfg, Edge, Node};
pub use dict::{ActivityId, CaseId, Dictionary};
pub use filter::{Filter, Filtered};
pub use formal::{ModelError, PetriNet, PetriTransition, PlaceId, ProcessTree, TransitionId};
pub use handover::{Handover, Organization, ResourceActivity};
pub use heuristics::{
    AndBinding, DependencyKind, HeuristicArc, HeuristicsError, HeuristicsMinerConfig,
    HeuristicsNet, discover_heuristics,
};
pub use inductive::{
    DiscoveryError, InductiveMinerConfig, InductiveModel, discover_inductive,
    discover_inductive_from_variants,
};
pub use log::{CaseTrace, Event, EventLog, Micros, NO_ATTRIBUTE, ResourceId, StepValue};
pub use number::{
    NumberField, NumberRange, case_number, main_number_field, number_fields, parse_number,
};
pub use ocel::{
    AttributeDefinition, AttributeNameId, AttributeType, AttributeValue, AttributeValueId,
    EventAttribute, EventObjectRelation, EventType, EventTypeId, Object, ObjectAttribute, ObjectId,
    ObjectObjectRelation, ObjectType, ObjectTypeId, OcelBuildError, OcelBuilder, OcelEvent,
    OcelEventId, OcelLog, QualifierId, UndeclaredLinks,
};
pub use ocel_bottlenecks::{
    OcelBottleneckRanking, OcelLifecycleBottleneck, OcelSynchronizationBottleneck,
    rank_bottlenecks, rank_ocel_bottlenecks,
};
pub use ocel_discovery::{
    OcelArcEndpoint, OcelDiscoveryConfig, OcelDiscoveryError, OcelDiscoveryFold,
    OcelObjectTypeDiscovery, OcelPetriNet, OcelPlace, OcelPlaceId, OcelTransition,
    OcelTransitionId, discover_ocel_petri_net,
};
pub use ocel_filter::{OcelCohort, OcelFilter, OcelFilterError, OcelView};
pub use ocel_graph::{
    OcelActivityNode, OcelDirectlyFollowsEdge, OcelDirectlyFollowsEvidence, OcelGraphFold,
    OcelGraphs, OcelInteractionEdge, OcelParticipationEdge, OcelRelationshipEdge, TimelineStep,
    for_each_timeline,
};
pub use ocel_performance::{
    OcelTimeBetween, OcelTimeBetweenEvidence, OcelTimeBetweenFold, measure_ocel_progress,
    measure_ocel_time_between, nearest_rank,
};
pub use ocel_projection::{
    OcelConvergenceEvidence, OcelDivergenceEvidence, OcelProjectedCase, OcelProjectedEvent,
    OcelProjection, OcelProjectionFold, OcelProjectionLayer, project_ocel,
};
pub use ocel_replay::{
    OcelAlignmentConformance, OcelAlignmentMove, OcelConformanceFold, OcelObjectAlignment,
    OcelObjectReplay, OcelReplayDeviation, OcelReplayError, OcelTokenReplay, align_ocel,
    replay_ocel_tokens,
};
pub use ocel_synchronization::{
    OcelParticipant, OcelSynchronization, OcelSynchronizationEvidence, OcelSynchronizationFold,
    OcelSynchronizationReadinessEvidence, analyze_ocel_synchronization,
};
pub use ocel_variants::{
    OcelLifecycle, OcelLifecycleEvent, OcelVariant, OcelVariantFold, OcelVariantLayer,
    discover_ocel_variants, inspect_ocel_lifecycle,
};
pub use replay::{CaseReplay, ReplayDeviation, ReplayError, TokenReplay, replay_tokens};
pub use rework::{ActivityRework, LoopStats, Rework};
pub use trace::Trace;
pub use variants::{Variant, Variants};
pub use window::{CaseWindow, ObservationWindow, WindowSummary};

pub mod segment;
