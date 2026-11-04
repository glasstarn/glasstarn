//! Profiling, validation, and normalization of event logs.
//!
//! # Report, don't guess
//!
//! Real event logs contain ambiguities that no algorithm can resolve
//! correctly: is `05/01/2026` January or May? Does a blank timestamp mean the
//! step did not happen, or that data is missing? Is a repeated row at the same
//! timestamp an extraction artifact or genuine rework?
//!
//! Each of these has a plausible default, and each default is wrong often
//! enough to corrupt every downstream number while producing output that looks
//! entirely reasonable. So this layer **surfaces the decision with the
//! evidence and the counts**, and applies the user's answer. It does not
//! choose silently.

pub mod bpmn;
pub mod finding;
pub mod normalize;
pub mod ocel_json;
pub mod ocel_quality;
pub mod ocel_read;
pub mod ocel_sqlite;
pub mod ocel_xml;
pub mod parquet;
pub mod prepare;
pub mod profile;
pub mod read;
pub mod row_cache;
pub mod rules;
pub mod timestamp;
pub mod validate;
pub mod wide;
pub mod xes;

pub use bpmn::{BpmnError, BpmnModel, import_bpmn};
pub use finding::{Decision, Finding, Kind, Policy, PolicyError, Report, Severity};
pub use normalize::{
    Applied, NormalizeError, Normalized, Normalizer, needs_mixed_formats, normalize,
    normalize_with, prepare,
};
pub use ocel_json::{OcelJsonError, parse_ocel_json, read_ocel_json};
pub use ocel_quality::{
    OcelQualityFinding, OcelQualityKind, OcelQualityReport, OcelQualitySeverity,
    OcelQualitySummary, assess_ocel_quality,
};
pub use ocel_read::{OcelReadError, is_ocel, read_ocel};
pub use ocel_sqlite::{
    DiskList, DiskLog, DiskView, ObjectTable, OcelSqliteError, read_ocel_sqlite,
    read_ocel_sqlite_skeleton,
};
pub use ocel_xml::{OcelXmlError, parse_ocel_xml, read_ocel_xml};
pub use parquet::{ParquetError, is_parquet, read_parquet, stream_parquet};
pub use prepare::{
    ActivityColumn, ActivitySource, ActivityValue, EventRule, Field, InputInventory,
    InputReference, LookupRule, PreparationError, PreparationOutcome, PreparationRecipe,
    PreparationReport, inventory_csv, prepare_csv,
};
pub use profile::{Column, ColumnKind, PROFILE_ROWS, Profile, Role};
pub use read::{
    ReadError, Row, Schema, Table, read_any, read_csv, sample_column, sample_rows, stream_any,
    stream_csv,
};
pub use rules::{Rules, RulesError};
pub use timestamp::{Ambiguity, Detection, Format};
pub use validate::{Mapping, Validator, validate};
pub use wide::{EventColumn, WideSpec, unpivot_all};
pub use xes::{is_xes, read_xes, stream_xes};

/// Reviewed connected-process preparation contract.
pub mod handout;
pub mod pack;
pub mod prepare_ocel;
pub mod reshape;
pub mod text_export;
pub mod xlsx;

/// Whether `GLASSTARN_FORCE_DISK` asks for the disk path whatever the
/// size, for measuring and testing it on small data.
pub fn force_disk() -> bool {
    std::env::var_os("GLASSTARN_FORCE_DISK").is_some_and(|v| !v.is_empty() && v != "0")
}
