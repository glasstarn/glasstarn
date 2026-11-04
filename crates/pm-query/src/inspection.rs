//! Bounded, structured log inspection shared by every front end.

use crate::QueryError;
use pm_ingest::{Mapping, Profile, Role};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnInspection {
    pub name: String,
    pub index: usize,
    pub kind: String,
    pub sampled: usize,
    pub blank: usize,
    pub blank_percent: f64,
    pub distinct: usize,
    /// Empty when the caller selected the privacy-preserving inspection mode.
    pub examples: Vec<String>,
    pub suggested_roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoleInspection {
    pub role: String,
    pub required: bool,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogInspection {
    pub source: String,
    pub rows_sampled: usize,
    pub columns: Vec<ColumnInspection>,
    pub roles: Vec<RoleInspection>,
    pub unfillable_roles: Vec<String>,
    pub proposed_mapping: Option<Mapping>,
    pub examples_exposed: bool,
}

/// Profiles at most [`pm_ingest::PROFILE_ROWS`] rows.
///
/// `include_examples` is false for MCP until its redaction and exposure-preview
/// controls exist. The local CLI can show the bounded examples directly to its
/// user without crossing the AI privacy boundary.
pub fn inspect_log(
    path: impl AsRef<Path>,
    include_examples: bool,
) -> Result<LogInspection, QueryError> {
    let path = path.as_ref();
    let (schema, sample) = pm_ingest::sample_rows(path, pm_ingest::PROFILE_ROWS)?;
    let profile = Profile::build(&schema, &sample);
    let roles = all_roles()
        .into_iter()
        .map(|role| RoleInspection {
            role: role.label().to_owned(),
            required: role.required(),
            candidates: profile
                .candidates(role)
                .into_iter()
                .map(|column| column.name.clone())
                .collect(),
        })
        .collect();
    let columns = profile
        .columns
        .iter()
        .map(|column| ColumnInspection {
            name: column.name.clone(),
            index: column.index,
            kind: column.kind.describe(),
            sampled: column.sampled,
            blank: column.blank,
            blank_percent: column.blank_percent(),
            distinct: column.distinct,
            examples: if include_examples {
                column.examples.clone()
            } else {
                Vec::new()
            },
            suggested_roles: column
                .suggests
                .iter()
                .map(|role| role.label().to_owned())
                .collect(),
        })
        .collect();
    Ok(LogInspection {
        source: path.display().to_string(),
        rows_sampled: profile.rows_sampled,
        columns,
        roles,
        unfillable_roles: profile
            .unfillable_roles()
            .iter()
            .map(|role| role.label().to_owned())
            .collect(),
        proposed_mapping: crate::propose_mapping(schema.headers()),
        examples_exposed: include_examples,
    })
}

fn all_roles() -> [Role; 8] {
    [
        Role::CaseId,
        Role::Activity,
        Role::Timestamp,
        Role::EndTimestamp,
        Role::Resource,
        Role::EventOrder,
        Role::Lifecycle,
        Role::Cost,
    ]
}

/// Recorded history coverage, not proof of a source system's business meaning.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistoryReadiness {
    pub cases: usize,
    pub single_event_cases: usize,
    pub multiple_event_cases: usize,
    pub all_cases_have_one_event: bool,
    pub interpretation: &'static str,
}

pub fn history_readiness(log: &pm_core::EventLog) -> HistoryReadiness {
    let single = log
        .traces
        .iter()
        .filter(|trace| trace.events.len() == 1)
        .count();
    let multiple = log
        .traces
        .iter()
        .filter(|trace| trace.events.len() > 1)
        .count();
    let all_single = !log.traces.is_empty() && single == log.case_count();
    HistoryReadiness {
        cases: log.case_count(),
        single_event_cases: single,
        multiple_event_cases: multiple,
        all_cases_have_one_event: all_single,
        interpretation: if log.case_count() == 0 {
            "The active view has no cases; no history or process conclusion can be drawn."
        } else if all_single {
            "Every case has one recorded event. This may be a current-state snapshot or a genuinely single-step process. Confirm the source meaning; request change history or milestone timestamps before interpreting paths or cycle time."
        } else {
            "Multiple events exist in at least one case. This does not prove complete history or correct case and activity definitions. Confirm coverage and business meaning."
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static ID: AtomicU64 = AtomicU64::new(0);

    fn source() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "pm_inspection_{}_{}.csv",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(
            &path,
            "case_id,activity,timestamp,secret\nC1,Start,2026-01-01 10:00:00,private\n",
        )
        .unwrap();
        path
    }

    #[test]
    fn inspection_is_bounded_and_mapping_ready() {
        let result = inspect_log(source(), true).unwrap();
        assert_eq!(result.rows_sampled, 1);
        assert_eq!(result.proposed_mapping.unwrap().case_id, "case_id");
    }

    #[test]
    fn value_free_mode_does_not_leak_examples() {
        let result = inspect_log(source(), false).unwrap();
        assert!(!result.examples_exposed);
        assert!(
            result
                .columns
                .iter()
                .all(|column| column.examples.is_empty())
        );
        assert!(!serde_json::to_string(&result).unwrap().contains("private"));
    }
}
