//! Local, read-only Model Context Protocol server.
//!
//! The server deliberately exposes named, bounded tools instead of arbitrary
//! file access or raw event-log queries. Project identifiers are paths relative
//! to the directory selected when the server starts; callers cannot escape
//! that directory with absolute paths or `..` components.

pub mod exposure;

pub use exposure::{ExposurePolicy, ExposurePreview, FieldPreview, Mode, ValuePreview};
use pm_project::{Project, SourceState};
use pm_query::{Analysis, Outcome};
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MODERN_PROTOCOL: &str = "2026-07-28";
pub const LEGACY_PROTOCOL: &str = "2025-11-25";
pub const DEFAULT_LIVE_PORT: u16 = 43_127;
const LEGACY_PROTOCOLS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", LEGACY_PROTOCOL];
const MAX_CASE_MODEL_BYTES: u64 = 10 * 1024 * 1024;
const MAX_OCEL_MODEL_BYTES: u64 = 100 * 1024 * 1024;
const MAX_DELEGATED_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// The exact browser-owned analysis state exposed when live delegation is
/// explicitly enabled. Paths remain local and are never returned through MCP.
#[derive(Debug, Clone, Default)]
pub struct LiveWorkspace {
    pub case_project: Option<LiveProject>,
    pub case_model: Option<LiveProcessModel>,
    pub ocel_log: Option<LiveOcelLog>,
    pub ocel_model: Option<LiveOcelModel>,
}

#[derive(Debug, Clone)]
pub struct LiveProject {
    pub project: Project,
    pub source_name: String,
}

#[derive(Debug, Clone)]
pub struct LiveProcessModel {
    pub path: PathBuf,
    pub source_name: String,
}

#[derive(Debug, Clone)]
pub struct LiveOcelLog {
    pub path: PathBuf,
    pub source_name: String,
    pub filter: pm_core::OcelFilter,
}

#[derive(Debug, Clone)]
pub struct LiveOcelModel {
    pub path: PathBuf,
    pub source_name: String,
}

#[derive(Debug, Clone)]
enum Workspace {
    Directory(PathBuf),
    Live(Box<LiveWorkspace>),
}

#[derive(Debug, Clone)]
struct ProjectRecord {
    key: String,
    source_name: String,
    project: Project,
    source_state: SourceState,
}

#[derive(Debug, Clone)]
struct OcelRecord {
    key: String,
    source_name: String,
    path: PathBuf,
    filter: pm_core::OcelFilter,
}

#[derive(Debug, Clone)]
struct OcelModelRecord {
    key: String,
    source_name: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct ProcessModelRecord {
    key: String,
    source_name: String,
    path: PathBuf,
}

/// A server scoped to one user-selected project directory.
#[derive(Debug, Clone)]
pub struct Server {
    workspace: Workspace,
    exposure: ExposurePolicy,
}

impl Server {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root
            .as_ref()
            .canonicalize()
            .map_err(|error| format!("could not open project directory: {error}"))?;
        if !root.is_dir() {
            return Err("the MCP project root is not a directory".to_owned());
        }
        Ok(Self {
            workspace: Workspace::Directory(root),
            exposure: ExposurePolicy::safe(),
        })
    }

    pub fn with_exposure_policy(
        root: impl AsRef<Path>,
        exposure: ExposurePolicy,
    ) -> Result<Self, String> {
        let mut server = Self::new(root)?;
        exposure.validate()?;
        server.exposure = exposure;
        Ok(server)
    }

    /// Creates the same bounded MCP surface over the exact state owned by a
    /// running browser application.
    pub fn with_live_workspace(
        workspace: LiveWorkspace,
        exposure: ExposurePolicy,
    ) -> Result<Self, String> {
        exposure.validate()?;
        Ok(Self {
            workspace: Workspace::Live(Box::new(workspace)),
            exposure,
        })
    }

    /// Handles one JSON-RPC message. Notifications intentionally have no
    /// response; requests always receive either a result or an error.
    pub fn handle(&self, request: Value) -> Option<Value> {
        let id = request.get("id").cloned()?;
        let method = request.get("method").and_then(Value::as_str);
        if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(rpc_error(id, -32600, "invalid JSON-RPC request"));
        }

        let result = match method {
            Some("server/discover") => Ok(discover()),
            // Legacy clients still in active use negotiate through initialize.
            Some("initialize") => Ok(legacy_initialize(request.get("params"))),
            Some("ping") => Ok(json!({})),
            Some("tools/list") => Ok(json!({
                "tools": tools().into_iter().map(with_detail_parameter).collect::<Vec<_>>(),
                "ttlMs": 60_000,
                "cacheScope": "global"
            })),
            Some("tools/call") => self.call_tool(request.get("params")),
            Some(_) | None => return Some(rpc_error(id, -32601, "method not found")),
        };

        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": with_server_meta(result) }),
            Err(message) => rpc_error(id, -32602, &message),
        })
    }

    /// Runs a tool and holds its answer to [`pm_query::budget::AGENT_BUDGET`]
    /// unless the agent passed `detail: "full"`.
    fn call_tool(&self, params: Option<&Value>) -> Result<Value, String> {
        let params = params.ok_or("tools/call requires params")?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or("tools/call requires a tool name")?;
        let mut arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        let full = match arguments
            .as_object_mut()
            .and_then(|map| map.remove("detail"))
        {
            None => false,
            Some(Value::String(detail)) if detail == "full" => true,
            Some(Value::String(detail)) if detail == "summary" => false,
            Some(_) => return Err(format!("{name}: detail must be \"summary\" or \"full\"")),
        };
        let result = self.dispatch_tool(name, &arguments)?;
        if full || !budgeted(name) {
            return Ok(result);
        }
        Ok(fit_tool_result(result, name))
    }

    fn dispatch_tool(&self, name: &str, arguments: &Value) -> Result<Value, String> {
        match name {
            "list_projects" => Ok(tool_result(self.list_projects())),
            "list_process_models" => Ok(tool_result(self.list_process_models())),
            "list_ocel_logs" => Ok(tool_result(self.list_ocel_logs())),
            "list_ocel_models" => Ok(tool_result(self.list_ocel_models())),
            "get_exposure_policy" => Ok(tool_result(Ok(json!({
                "policy": self.exposure.public_summary(),
                "privacy_note": "This reports modes only. The secret alias key and original values are never returned through MCP."
            })))),
            "get_claim_contract" => Ok(tool_result(Ok(claim_contract()))),
            "inspect_log" => {
                let project = arguments.get("project").and_then(Value::as_str).ok_or(
                    "inspect_log requires the project identifier returned by list_projects",
                )?;
                Ok(tool_result(self.inspect_log(project)))
            }
            "analyze_ocel" => Ok(tool_result(self.analyze_ocel(arguments))),
            "inspect_ocel_object" => Ok(tool_result(self.inspect_ocel_object(arguments))),
            "measure_ocel_time_between" => {
                Ok(tool_result(self.measure_ocel_time_between(arguments)))
            }
            "analyze_ocel_synchronization" => {
                Ok(tool_result(self.analyze_ocel_synchronization(arguments)))
            }
            "rank_ocel_bottlenecks" => Ok(tool_result(self.rank_ocel_bottlenecks(arguments))),
            "project_ocel_cases" => Ok(tool_result(self.project_ocel_cases(arguments))),
            "follow_ocel_object_type" => Ok(tool_result(self.follow_ocel_object_type(arguments))),
            "discover_ocel_model" => Ok(tool_result(self.discover_ocel_model(arguments))),
            "check_ocel_conformance" => Ok(tool_result(self.check_ocel_conformance(arguments))),
            "align_ocel_conformance" => Ok(tool_result(self.align_ocel_conformance(arguments))),
            "discover_process_model" => Ok(tool_result(self.discover_process_model(arguments))),
            "check_process_conformance" => {
                Ok(tool_result(self.check_process_conformance(arguments)))
            }
            "align_process_conformance" => {
                Ok(tool_result(self.align_process_conformance(arguments)))
            }
            "validate_mapping" => {
                let project = required_project(arguments, "validate_mapping")?;
                Ok(tool_result(self.validate_mapping(project)))
            }
            "run_analysis" => {
                let project = required_project(arguments, "run_analysis")?;
                Ok(tool_result(self.run_analysis(project)))
            }
            "get_process_summary" => {
                let project = required_project(arguments, "get_process_summary")?;
                Ok(tool_result(self.get_process_summary(project)))
            }
            "get_variants" => {
                let project = required_project(arguments, "get_variants")?;
                let limit = bounded_limit(arguments, 10, 20)?;
                Ok(tool_result(self.get_variants(project, limit)))
            }
            "get_slowest_cases" => {
                let project = required_project(arguments, "get_slowest_cases")?;
                let limit = bounded_limit(arguments, 10, 50)?;
                Ok(tool_result(self.get_slowest_cases(project, limit)))
            }
            "get_process_map" => {
                let project = required_project(arguments, "get_process_map")?;
                let selection = map_selection(arguments)?;
                Ok(tool_result(self.get_process_map(project, &selection)))
            }
            "find_bottlenecks" => {
                let project = required_project(arguments, "find_bottlenecks")?;
                let limit = bounded_limit(arguments, 10, 10)?;
                Ok(tool_result(self.find_bottlenecks(project, limit)))
            }
            "find_drivers" => {
                let project = required_project(arguments, "find_drivers")?;
                let limit = bounded_limit(arguments, 6, 6)?;
                Ok(tool_result(self.find_drivers(project, limit)))
            }
            "find_rework" => {
                let project = required_project(arguments, "find_rework")?;
                let activity_limit = bounded_named_limit(arguments, "activity_limit", 10, 20)?;
                let loop_limit = bounded_named_limit(arguments, "loop_limit", 10, 20)?;
                Ok(tool_result(self.find_rework(
                    project,
                    activity_limit,
                    loop_limit,
                )))
            }
            "get_automation_targeting" => {
                let project = required_project(arguments, "get_automation_targeting")?;
                let limit = bounded_limit(arguments, 10, 20)?;
                Ok(tool_result(self.get_automation_targeting(project, limit)))
            }
            "get_activity_context" => {
                let project = required_project(arguments, "get_activity_context")?;
                let activity = arguments.get("activity").and_then(Value::as_str).ok_or(
                    "get_activity_context requires an activity identifier returned by an MCP tool",
                )?;
                let limit = bounded_limit(arguments, 10, 20)?;
                Ok(tool_result(
                    self.get_activity_context(project, activity, limit),
                ))
            }
            "explore_linked_interval" => Ok(tool_result(self.explore_linked_interval(arguments))),
            "explore_process" => Ok(tool_result(self.explore_process(arguments))),
            "compare_cohorts" => {
                let project = required_project(arguments, "compare_cohorts")?;
                let attribute = required_string(arguments, "attribute", "compare_cohorts")?;
                let left = required_string(arguments, "left", "compare_cohorts")?;
                let right = required_string(arguments, "right", "compare_cohorts")?;
                let limit = bounded_limit(arguments, 10, 20)?;
                Ok(tool_result(
                    self.compare_cohorts(project, attribute, left, right, limit),
                ))
            }
            "get_attribute_choices" => {
                let project = required_project(arguments, "get_attribute_choices")?;
                let attribute = required_string(arguments, "attribute", "get_attribute_choices")?;
                Ok(tool_result(self.get_attribute_choices(project, attribute)))
            }
            "summarize_values" => {
                let project = required_project(arguments, "summarize_values")?;
                let limit = bounded_limit(arguments, 10, 20)?;
                Ok(tool_result(
                    self.summarize_values(project, arguments, limit),
                ))
            }
            "compare_periods" => {
                let project = required_project(arguments, "compare_periods")?;
                let baseline_start = required_i64(arguments, "baseline_start", "compare_periods")?;
                let baseline_end = required_i64(arguments, "baseline_end", "compare_periods")?;
                let comparison_start =
                    required_i64(arguments, "comparison_start", "compare_periods")?;
                let comparison_end = required_i64(arguments, "comparison_end", "compare_periods")?;
                let limit = bounded_limit(arguments, 10, 20)?;
                Ok(tool_result(self.compare_periods(
                    project,
                    baseline_start,
                    baseline_end,
                    comparison_start,
                    comparison_end,
                    limit,
                )))
            }
            "get_evidence" => {
                let project = required_project(arguments, "get_evidence")?;
                let limit = bounded_limit(arguments, 20, 100)?;
                Ok(tool_result(self.get_evidence(project, arguments, limit)))
            }
            "inspect_case" => {
                let project = required_project(arguments, "inspect_case")?;
                let case = arguments
                    .get("case")
                    .and_then(Value::as_str)
                    .ok_or("inspect_case requires a case identifier returned by an MCP tool")?;
                Ok(tool_result(self.inspect_case(project, case)))
            }
            _ => Err(format!("unknown tool '{name}'")),
        }
    }

    fn list_projects(&self) -> Result<Value, String> {
        let mut projects = Vec::new();
        for record in self.project_records()? {
            let project = record.project;
            let state = record.source_state;
            projects.push(json!({
                "project": self.project_handle(&record.key),
                "name": self.exposure.transform(self.exposure.project_names, "project_name", &project.name),
                "source_file": self.exposure.transform(
                    self.exposure.source_names,
                    "source",
                    &record.source_name
                ),
                "source_state": source_state(&state),
                "reproducible": state.reproduces(),
                "engine_changed": project.engine_changed()
            }));
        }
        projects.sort_by(|a, b| {
            a.get("project")
                .and_then(Value::as_str)
                .cmp(&b.get("project").and_then(Value::as_str))
        });
        Ok(json!({
            "workspace": self.workspace_kind(),
            "projects": projects,
            "count": projects.len()
        }))
    }

    fn list_ocel_logs(&self) -> Result<Value, String> {
        let logs = self
            .ocel_records()?
            .into_iter()
            .map(|record| {
                json!({
                    "log": self.ocel_handle(&record.key),
                    "source_file": self.exposure.transform(
                        self.exposure.source_names,
                        "source",
                        &record.source_name
                    )
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "workspace": self.workspace_kind(),
            "logs": logs,
            "count": logs.len()
        }))
    }

    fn list_process_models(&self) -> Result<Value, String> {
        let models = self
            .process_model_records()?
            .into_iter()
            .map(|record| {
                json!({
                    "model": self.process_model_handle(&record.key),
                    "source_file": self.exposure.transform(
                        self.exposure.source_names,
                        "source",
                        &record.source_name
                    )
                })
            })
            .collect::<Vec<_>>();
        let availability_note = match (&self.workspace, models.is_empty()) {
            (Workspace::Live(_), true) => {
                "The running browser has no selected case-centric baseline. Choose one in the browser before requesting live conformance."
            }
            (Workspace::Live(_), false) => {
                "The selected browser baseline is available through an opaque handle and remains local."
            }
            (Workspace::Directory(_), _) => {
                "Only explicit local *.model.json baselines are listed. A model discovered from the same analysis log is descriptive, not independent compliance evidence."
            }
        };
        Ok(json!({
            "workspace": self.workspace_kind(),
            "models": models,
            "count": models.len(),
            "availability_note": availability_note
        }))
    }

    fn list_ocel_models(&self) -> Result<Value, String> {
        let models = self
            .ocel_model_records()?
            .into_iter()
            .map(|record| {
                json!({
                    "model": self.ocel_model_handle(&record.key),
                    "source_file": self.exposure.transform(
                        self.exposure.source_names,
                        "source",
                        &record.source_name
                    )
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "workspace": self.workspace_kind(),
            "models": models,
            "count": models.len()
        }))
    }

    fn analyze_ocel(&self, arguments: &Value) -> Result<Value, String> {
        let handle = arguments
            .get("log")
            .and_then(Value::as_str)
            .ok_or("analyze_ocel requires a log identifier returned by list_ocel_logs")?;
        let record = self.resolve_ocel(handle)?;
        let log = pm_query::ocel_names(&record.path).map_err(|error| error.to_string())?;
        let filter = self.ocel_filter(arguments, &log, &record.filter)?;
        let limit = bounded_limit(arguments, 100, 200)?;
        let mut analysis =
            pm_query::analyze_ocel(&record.path, &filter).map_err(|error| error.to_string())?;
        analysis.source = handle.to_owned();
        Ok(expose_ocel_analysis(
            handle,
            &record.source_name,
            &analysis,
            &self.exposure,
            limit,
        ))
    }

    fn inspect_ocel_object(&self, arguments: &Value) -> Result<Value, String> {
        let handle = arguments
            .get("log")
            .and_then(Value::as_str)
            .ok_or("inspect_ocel_object requires a log identifier returned by list_ocel_logs")?;
        let exposed_object = arguments
            .get("object")
            .and_then(Value::as_str)
            .ok_or("inspect_ocel_object requires an object identifier returned by analyze_ocel")?;
        let record = self.resolve_ocel(handle)?;
        let log = pm_query::ocel_names(&record.path).map_err(|error| error.to_string())?;
        let object = resolve_exposed_object(&self.exposure, &record.path, exposed_object)?;
        let filter = self.ocel_filter(arguments, &log, &record.filter)?;
        let limit = bounded_limit(arguments, 100, 200)?;
        let mut lifecycle = pm_query::inspect_ocel_object(&record.path, &filter, &object)
            .map_err(|error| error.to_string())?;
        lifecycle.source = handle.to_owned();
        Ok(expose_ocel_lifecycle(
            handle,
            &lifecycle,
            &self.exposure,
            limit,
        ))
    }

    fn measure_ocel_time_between(&self, arguments: &Value) -> Result<Value, String> {
        let handle = arguments.get("log").and_then(Value::as_str).ok_or(
            "measure_ocel_time_between requires a log identifier returned by list_ocel_logs",
        )?;
        let record = self.resolve_ocel(handle)?;
        let log = pm_query::ocel_names(&record.path).map_err(|error| error.to_string())?;
        let activities = log.activities.clone();
        let resolve_activity = |field: &str| {
            let exposed = arguments
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!("measure_ocel_time_between requires {field} returned by analyze_ocel")
                })?;
            resolve_exposed(
                &self.exposure,
                self.exposure.activity_names,
                "activity",
                exposed,
                &activities,
            )
        };
        let from_activity = resolve_activity("from_activity")?;
        let to_activity = resolve_activity("to_activity")?;
        let filter = self.ocel_filter(arguments, &log, &record.filter)?;
        let limit = bounded_limit(arguments, 50, 100)?;
        let mut report = pm_query::measure_ocel_time_between(
            &record.path,
            &filter,
            &from_activity,
            &to_activity,
        )
        .map_err(|error| error.to_string())?;
        report.source = handle.to_owned();
        Ok(expose_ocel_time_between(
            handle,
            &report,
            &self.exposure,
            limit,
        ))
    }

    fn analyze_ocel_synchronization(&self, arguments: &Value) -> Result<Value, String> {
        let handle = arguments.get("log").and_then(Value::as_str).ok_or(
            "analyze_ocel_synchronization requires a log identifier returned by list_ocel_logs",
        )?;
        let record = self.resolve_ocel(handle)?;
        let log = pm_query::ocel_names(&record.path).map_err(|error| error.to_string())?;
        let filter = self.ocel_filter(arguments, &log, &record.filter)?;
        let limit = bounded_limit(arguments, 50, 100)?;
        let mut report = pm_query::analyze_ocel_synchronization(&record.path, &filter)
            .map_err(|error| error.to_string())?;
        report.source = handle.to_owned();
        Ok(expose_ocel_synchronization(
            handle,
            &report,
            &self.exposure,
            limit,
        ))
    }

    fn rank_ocel_bottlenecks(&self, arguments: &Value) -> Result<Value, String> {
        let handle = arguments
            .get("log")
            .and_then(Value::as_str)
            .ok_or("rank_ocel_bottlenecks requires a log identifier returned by list_ocel_logs")?;
        let record = self.resolve_ocel(handle)?;
        let log = pm_query::ocel_names(&record.path).map_err(|error| error.to_string())?;
        let filter = self.ocel_filter(arguments, &log, &record.filter)?;
        let limit = bounded_limit(arguments, 10, 20)?;
        let mut report = pm_query::rank_ocel_bottlenecks(&record.path, &filter)
            .map_err(|error| error.to_string())?;
        report.source = handle.to_owned();
        Ok(expose_ocel_bottlenecks(
            handle,
            &report,
            &self.exposure,
            limit,
        ))
    }

    fn project_ocel_cases(&self, arguments: &Value) -> Result<Value, String> {
        let handle = arguments
            .get("log")
            .and_then(Value::as_str)
            .ok_or("project_ocel_cases requires a log identifier returned by list_ocel_logs")?;
        let record = self.resolve_ocel(handle)?;
        let log = pm_query::ocel_names(&record.path).map_err(|error| error.to_string())?;
        let object_types = log.object_types.clone();
        let exposed_case_type = arguments
            .get("case_object_type")
            .and_then(Value::as_str)
            .ok_or("project_ocel_cases requires a case_object_type returned by analyze_ocel")?;
        let case_object_type = resolve_exposed(
            &self.exposure,
            self.exposure.object_type_names,
            "object_type",
            exposed_case_type,
            &object_types,
        )?;
        let filter = self.ocel_filter(arguments, &log, &record.filter)?;
        let case_limit = bounded_named_limit(arguments, "case_limit", 5, 20)?;
        let event_limit_per_case =
            bounded_named_limit(arguments, "event_limit_per_case", 100, 200)?;
        let mut report = pm_query::project_ocel_cases(
            &record.path,
            &filter,
            &case_object_type,
            case_limit,
            event_limit_per_case,
        )
        .map_err(|error| error.to_string())?;
        report.source = handle.to_owned();
        Ok(expose_ocel_projection(handle, &report, &self.exposure))
    }

    fn follow_ocel_object_type(&self, arguments: &Value) -> Result<Value, String> {
        let handle = arguments.get("log").and_then(Value::as_str).ok_or(
            "follow_ocel_object_type requires a log identifier returned by list_ocel_logs",
        )?;
        let record = self.resolve_ocel(handle)?;
        let log = pm_query::ocel_names(&record.path).map_err(|error| error.to_string())?;
        let object_types = log.object_types.clone();
        let exposed = arguments
            .get("object_type")
            .and_then(Value::as_str)
            .ok_or("follow_ocel_object_type requires an object_type returned by analyze_ocel")?;
        let object_type = resolve_exposed(
            &self.exposure,
            self.exposure.object_type_names,
            "object_type",
            exposed,
            &object_types,
        )?;
        let filter = self.ocel_filter(arguments, &log, &record.filter)?;
        let mut followed = tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
        let follow =
            pm_query::follow_ocel(&record.path, &filter, &object_type, followed.as_file_mut())
                .map_err(|error| error.to_string())?;
        let outcome = pm_query::analyze_filtered(
            followed.path(),
            Some(follow.mapping()),
            &pm_ingest::finding::Policy::new(),
            Default::default(),
            Default::default(),
        )
        .map_err(|error| error.to_string())?;
        let pm_query::Outcome::Complete(analysis) = outcome else {
            return Err(
                "the followed log needs a data-quality decision; follow it in the browser".into(),
            );
        };
        let summary = &analysis.summary;
        Ok(json!({
            "log": handle,
            "object_type": exposed,
            "claim_type": "calculated_fact",
            "follow": {
                "cases": follow.cases,
                "steps": follow.events,
                "steps_without_this_type": follow.excluded_events,
                "shared_steps": follow.shared_events,
                "shared_step_copies": follow.shared_event_copies
            },
            "summary": {
                "cases": summary.cases,
                "events": summary.events,
                "activities": summary.activities,
                "variants": summary.variants,
                "top_five_coverage_percent": summary.top_five_coverage_percent,
                "mean_throughput_seconds": summary.mean_throughput_seconds,
                "median_throughput_seconds": summary.median_throughput_seconds,
                "p90_throughput_seconds": summary.p90_throughput_seconds,
                "duration_eligible_cases": summary.duration_eligible_cases,
                "rework_case_percent": analysis.rework.case_percent
            },
            "semantics": pm_query::OBJECT_HISTORY_SEMANTICS,
            "privacy_note": "Aggregates only: no identifiers, activity names, timestamps or attribute values are returned by this tool."
        }))
    }

    fn discover_ocel_model(&self, arguments: &Value) -> Result<Value, String> {
        let handle = arguments
            .get("log")
            .and_then(Value::as_str)
            .ok_or("discover_ocel_model requires a log identifier returned by list_ocel_logs")?;
        let record = self.resolve_ocel(handle)?;
        let log = pm_query::ocel_names(&record.path).map_err(|error| error.to_string())?;
        let filter = self.ocel_filter(arguments, &log, &record.filter)?;
        let config = pm_core::OcelDiscoveryConfig {
            noise_threshold_milli: threshold_milli(arguments, "noise_threshold_milli", 200)?,
            variable_arc_threshold_milli: threshold_milli(
                arguments,
                "variable_arc_threshold_milli",
                1_000,
            )?,
        };
        let max_model_nodes = bounded_named_limit(arguments, "max_model_nodes", 2_000, 10_000)?;
        let mut report = pm_query::discover_ocel_model(&record.path, &filter, config)
            .map_err(|error| error.to_string())?;
        report.source = handle.to_owned();
        let model_nodes = report.places.len().saturating_add(report.transitions.len());
        if model_nodes > max_model_nodes {
            return Err(format!(
                "the discovered model has {model_nodes} places and transitions, above max_model_nodes={max_model_nodes}; narrow the object-centric filter or explicitly raise the limit"
            ));
        }
        Ok(expose_ocel_model(handle, &report, &self.exposure))
    }

    fn check_ocel_conformance(&self, arguments: &Value) -> Result<Value, String> {
        let log_handle = arguments
            .get("log")
            .and_then(Value::as_str)
            .ok_or("check_ocel_conformance requires a log identifier returned by list_ocel_logs")?;
        let model_handle = arguments.get("model").and_then(Value::as_str).ok_or(
            "check_ocel_conformance requires a model identifier returned by list_ocel_models",
        )?;
        let log_record = self.resolve_ocel(log_handle)?;
        let model_record = self.resolve_ocel_model(model_handle)?;
        let log = pm_query::ocel_names(&log_record.path).map_err(|error| error.to_string())?;
        let filter = self.ocel_filter(arguments, &log, &log_record.filter)?;
        let detail_limit = bounded_named_limit(arguments, "detail_limit", 100, 200)?;
        let max_model_nodes = bounded_named_limit(arguments, "max_model_nodes", 2_000, 10_000)?;
        let model_size = std::fs::metadata(&model_record.path)
            .map_err(|error| format!("could not inspect the object-centric model: {error}"))?
            .len();
        if model_size > MAX_OCEL_MODEL_BYTES {
            return Err(format!(
                "the baseline model is {model_size} bytes, above the {MAX_OCEL_MODEL_BYTES}-byte local MCP safety limit"
            ));
        }
        let model_bytes = std::fs::read(&model_record.path)
            .map_err(|error| format!("could not read the object-centric model: {error}"))?;
        let model: pm_query::OcelModelReport = serde_json::from_slice(&model_bytes)
            .map_err(|error| format!("the object-centric model is not valid JSON: {error}"))?;
        let model_nodes = model.places.len().saturating_add(model.transitions.len());
        if model_nodes > max_model_nodes {
            return Err(format!(
                "the baseline model has {model_nodes} places and transitions, above max_model_nodes={max_model_nodes}; choose the intended model or explicitly raise the limit"
            ));
        }
        let mut report = pm_query::check_ocel_conformance(
            &log_record.path,
            &model_record.path,
            &filter,
            detail_limit,
        )
        .map_err(|error| error.to_string())?;
        report.source = log_handle.to_owned();
        report.model_source = model_handle.to_owned();
        report.model_training_source = model.source.clone();
        Ok(expose_ocel_conformance(
            log_handle,
            model_handle,
            &report,
            &self.exposure,
        ))
    }

    fn align_ocel_conformance(&self, arguments: &Value) -> Result<Value, String> {
        let log_handle = arguments
            .get("log")
            .and_then(Value::as_str)
            .ok_or("align_ocel_conformance requires a log identifier returned by list_ocel_logs")?;
        let model_handle = arguments.get("model").and_then(Value::as_str).ok_or(
            "align_ocel_conformance requires a model identifier returned by list_ocel_models",
        )?;
        let log_record = self.resolve_ocel(log_handle)?;
        let model_record = self.resolve_ocel_model(model_handle)?;
        let log = pm_query::ocel_names(&log_record.path).map_err(|error| error.to_string())?;
        let filter = self.ocel_filter(arguments, &log, &log_record.filter)?;
        let detail_limit = bounded_named_limit(arguments, "detail_limit", 100, 200)?;
        let max_model_nodes = bounded_named_limit(arguments, "max_model_nodes", 2_000, 10_000)?;
        let model_size = std::fs::metadata(&model_record.path)
            .map_err(|error| format!("could not inspect the object-centric model: {error}"))?
            .len();
        if model_size > MAX_OCEL_MODEL_BYTES {
            return Err(format!(
                "the baseline model is {model_size} bytes, above the {MAX_OCEL_MODEL_BYTES}-byte local MCP safety limit"
            ));
        }
        let model_bytes = std::fs::read(&model_record.path)
            .map_err(|error| format!("could not read the object-centric model: {error}"))?;
        let model: pm_query::OcelModelReport = serde_json::from_slice(&model_bytes)
            .map_err(|error| format!("the object-centric model is not valid JSON: {error}"))?;
        let model_nodes = model.places.len().saturating_add(model.transitions.len());
        if model_nodes > max_model_nodes {
            return Err(format!(
                "the baseline model has {model_nodes} places and transitions, above max_model_nodes={max_model_nodes}; choose the intended model or explicitly raise the limit"
            ));
        }
        let mut report = pm_query::align_ocel_conformance(
            &log_record.path,
            &model_record.path,
            &filter,
            detail_limit,
        )
        .map_err(|error| error.to_string())?;
        report.source = log_handle.to_owned();
        report.model_source = model_handle.to_owned();
        report.model_training_source = model.source.clone();
        Ok(expose_ocel_alignment(
            log_handle,
            model_handle,
            &report,
            &self.exposure,
        ))
    }

    fn discover_process_model(&self, arguments: &Value) -> Result<Value, String> {
        if self.exposure.activity_names == Mode::Omit {
            return Err(
                "activity names are omitted by the exposure policy, so a formal process model cannot be returned without changing its meaning"
                    .to_owned(),
            );
        }
        let project_handle = required_project(arguments, "discover_process_model")?;
        let algorithm = arguments
            .get("algorithm")
            .and_then(Value::as_str)
            .unwrap_or("imf");
        let max_model_items = bounded_named_limit(arguments, "max_model_items", 2_000, 10_000)?;
        let (project, loaded) = self.load_project(project_handle)?;

        match algorithm {
            "imf" => {
                reject_arguments(
                    arguments,
                    &[
                        "dependency_threshold_millionths",
                        "relative_to_best_millionths",
                        "and_threshold_millionths",
                        "minimum_observations",
                    ],
                    "heuristics",
                )?;
                let config = pm_core::InductiveMinerConfig {
                    noise_threshold_milli: threshold_milli(
                        arguments,
                        "noise_threshold_milli",
                        200,
                    )?,
                };
                let report = pm_query::discover_formal(
                    project_handle,
                    &loaded.log,
                    &project.view_filter,
                    config,
                )
                .map_err(|error| error.to_string())?;
                ensure_net_bound(&report.net, max_model_items, "discovered model")?;
                Ok(expose_formal_discovery(
                    project_handle,
                    self.project_name(&project),
                    &report,
                    &self.exposure,
                ))
            }
            "alpha" => {
                reject_arguments(
                    arguments,
                    &[
                        "noise_threshold_milli",
                        "dependency_threshold_millionths",
                        "relative_to_best_millionths",
                        "and_threshold_millionths",
                        "minimum_observations",
                    ],
                    "alpha",
                )?;
                let report = pm_query::discover_alpha_formal(
                    project_handle,
                    &loaded.log,
                    &project.view_filter,
                )
                .map_err(|error| error.to_string())?;
                ensure_net_bound(&report.net, max_model_items, "discovered model")?;
                Ok(expose_alpha_discovery(
                    project_handle,
                    self.project_name(&project),
                    &report,
                    &self.exposure,
                ))
            }
            "heuristics" => {
                reject_arguments(arguments, &["noise_threshold_milli"], "imf")?;
                let defaults = pm_core::HeuristicsMinerConfig::default();
                let config = pm_core::HeuristicsMinerConfig {
                    dependency_threshold_millionths: threshold_millionths(
                        arguments,
                        "dependency_threshold_millionths",
                        defaults.dependency_threshold_millionths,
                    )?,
                    relative_to_best_millionths: threshold_millionths(
                        arguments,
                        "relative_to_best_millionths",
                        defaults.relative_to_best_millionths,
                    )?,
                    and_threshold_millionths: threshold_millionths(
                        arguments,
                        "and_threshold_millionths",
                        defaults.and_threshold_millionths,
                    )?,
                    minimum_observations: positive_u64(
                        arguments,
                        "minimum_observations",
                        defaults.minimum_observations,
                    )?,
                };
                let report = pm_query::discover_heuristics_formal(
                    project_handle,
                    &loaded.log,
                    &project.view_filter,
                    config,
                )
                .map_err(|error| error.to_string())?;
                let item_count = report
                    .starts
                    .len()
                    .saturating_add(report.ends.len())
                    .saturating_add(report.arcs.len())
                    .saturating_add(report.and_splits.len())
                    .saturating_add(report.and_joins.len());
                if item_count > max_model_items {
                    return Err(format!(
                        "the discovered heuristic model has {item_count} items, above max_model_items={max_model_items}; narrow the saved active view or explicitly raise the limit"
                    ));
                }
                Ok(expose_heuristics_discovery(
                    project_handle,
                    self.project_name(&project),
                    &report,
                    &self.exposure,
                ))
            }
            _ => Err("algorithm must be imf, alpha, or heuristics".to_owned()),
        }
    }

    fn check_process_conformance(&self, arguments: &Value) -> Result<Value, String> {
        let project_handle = required_project(arguments, "check_process_conformance")?;
        let model_handle = arguments.get("model").and_then(Value::as_str).ok_or(
            "check_process_conformance requires a model identifier returned by list_process_models",
        )?;
        let detail_limit = bounded_named_limit(arguments, "detail_limit", 20, 200)?;
        let max_model_items = bounded_named_limit(arguments, "max_model_items", 2_000, 10_000)?;
        let (project, loaded) = self.load_project(project_handle)?;
        let model = self.load_process_model(model_handle, max_model_items)?;
        let report = pm_query::check_conformance(
            project_handle,
            model_handle,
            &loaded.log,
            &project.view_filter,
            &model.net,
            detail_limit,
        )
        .map_err(|error| error.to_string())?;
        Ok(expose_process_conformance(
            project_handle,
            model_handle,
            self.project_name(&project),
            &model.source,
            &report,
            &self.exposure,
        ))
    }

    fn align_process_conformance(&self, arguments: &Value) -> Result<Value, String> {
        let project_handle = required_project(arguments, "align_process_conformance")?;
        let model_handle = arguments.get("model").and_then(Value::as_str).ok_or(
            "align_process_conformance requires a model identifier returned by list_process_models",
        )?;
        let detail_limit = bounded_named_limit(arguments, "detail_limit", 20, 200)?;
        let max_model_items = bounded_named_limit(arguments, "max_model_items", 2_000, 10_000)?;
        let (project, loaded) = self.load_project(project_handle)?;
        let model = self.load_process_model(model_handle, max_model_items)?;
        let report = pm_query::align_conformance(
            project_handle,
            model_handle,
            &loaded.log,
            &project.view_filter,
            &model.net,
            detail_limit,
        )
        .map_err(|error| error.to_string())?;
        Ok(expose_process_alignment(
            project_handle,
            model_handle,
            self.project_name(&project),
            &model.source,
            &report,
            &self.exposure,
        ))
    }

    fn load_process_model(
        &self,
        model_handle: &str,
        max_model_items: usize,
    ) -> Result<pm_query::PortableFormalModel, String> {
        let record = self.resolve_process_model(model_handle)?;
        let model_size = std::fs::metadata(&record.path)
            .map_err(|error| format!("could not inspect the process model: {error}"))?
            .len();
        if model_size > MAX_CASE_MODEL_BYTES {
            return Err(format!(
                "the baseline model is {model_size} bytes, above the {MAX_CASE_MODEL_BYTES}-byte local MCP safety limit"
            ));
        }
        let bytes = std::fs::read(&record.path)
            .map_err(|error| format!("could not read the process model: {error}"))?;
        let model = pm_query::parse_formal_model_json(&bytes)
            .map_err(|error| format!("the process model cannot be used: {error}"))?;
        ensure_net_bound(&model.net, max_model_items, "baseline model")?;
        Ok(model)
    }

    fn ocel_filter(
        &self,
        arguments: &Value,
        log: &pm_query::OcelNames,
        base: &pm_core::OcelFilter,
    ) -> Result<pm_core::OcelFilter, String> {
        let object_types = log.object_types.clone();
        let e2o_qualifiers = log.event_object_qualifiers.clone();
        let o2o_qualifiers = log.object_object_qualifiers.clone();
        Ok(pm_core::OcelFilter {
            // A live cohort is an access boundary, not an optional display
            // filter. A request may narrow label selectors but never drop it.
            cohort: base.cohort.clone(),
            object_types: scoped_exposed_selection(
                arguments,
                "object_types",
                &self.exposure,
                self.exposure.object_type_names,
                "object_type",
                &object_types,
                &base.object_types,
            )?,
            event_object_qualifiers: scoped_exposed_selection(
                arguments,
                "event_object_qualifiers",
                &self.exposure,
                self.exposure.relationship_qualifiers,
                "relationship_qualifier",
                &e2o_qualifiers,
                &base.event_object_qualifiers,
            )?,
            object_object_qualifiers: scoped_exposed_selection(
                arguments,
                "object_object_qualifiers",
                &self.exposure,
                self.exposure.relationship_qualifiers,
                "relationship_qualifier",
                &o2o_qualifiers,
                &base.object_object_qualifiers,
            )?,
        })
    }

    fn inspect_log(&self, project_id: &str) -> Result<Value, String> {
        let record = self.resolve_project(project_id)?;
        let state = record.source_state.clone();
        let project = record.project;
        if !state.reproduces() {
            return Err(state.explain());
        }

        // Profile only a bounded sample. Examples are intentionally omitted
        // from MCP until exposure previews and redaction controls exist.
        let mut profile = pm_query::inspect_log(&project.source.path, false)
            .map_err(|error| error.to_string())?;
        for column in &mut profile.columns {
            column.name = self
                .exposure
                .transform(self.exposure.column_names, "column", &column.name)
                .unwrap_or_else(|| "<omitted>".to_owned());
        }
        for role in &mut profile.roles {
            role.candidates = role
                .candidates
                .iter()
                .filter_map(|name| {
                    self.exposure
                        .transform(self.exposure.column_names, "column", name)
                })
                .collect();
        }
        let mapping = redact_mapping(&project.mapping, &self.exposure);
        Ok(json!({
            "project": project_id,
            "name": self.exposure.transform(self.exposure.project_names, "project_name", &project.name),
            "source_file": self.exposure.transform(
                self.exposure.source_names,
                "source",
                &record.source_name
            ),
            "source_bytes": self.exposure.transform(
                self.exposure.source_metadata,
                "source_bytes",
                &project.source.bytes.to_string()
            ),
            "source_sha256": self.exposure.transform(
                self.exposure.source_metadata,
                "source_fingerprint",
                &project.source.sha256
            ),
            "source_state": source_state(&state),
            "mapping": mapping,
            "rows_sampled": profile.rows_sampled,
            "columns": profile.columns,
            "roles": profile.roles,
            "unfillable_roles": profile.unfillable_roles,
            "proposed_mapping": profile.proposed_mapping.as_ref().map(|mapping| redact_mapping(mapping, &self.exposure)),
            "examples_exposed": profile.examples_exposed,
            "privacy_note": "No event values or sample values are returned by this tool."
        }))
    }

    fn validate_mapping(&self, project_id: &str) -> Result<Value, String> {
        let record = self.resolve_project(project_id)?;
        let state = record.source_state;
        let project = record.project;
        if !state.reproduces() {
            return Err(state.explain());
        }
        let report = pm_query::assess_quality(
            &project.source.path,
            Some(project.mapping.clone()),
            &project.policy,
        )
        .map_err(|_| {
            "the saved mapping could not be validated; open the project locally to inspect its source"
                .to_owned()
        })?;
        let findings = report
            .findings
            .iter()
            .map(|finding| {
                let decision = finding.decision.as_ref().map(|decision| {
                    json!({
                        "option_ids": decision.options.iter().map(|option| option.id.as_str()).collect::<Vec<_>>(),
                        "answer": decision.answer,
                        "answered": decision.answered,
                        "required": decision.required
                    })
                });
                json!({
                    "kind": finding.kind,
                    "title": finding.title,
                    "severity": finding.severity,
                    "affected_rows": finding.affected_rows,
                    "affected_cases": finding.affected_cases,
                    "decision": decision
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "calculated_fact",
            "source_state": source_state(&state),
            "mapping": redact_mapping(&report.mapping, &self.exposure),
            "ready_for_analysis": report.ready_for_analysis,
            "summary": report.summary,
            "findings": findings,
            "examples_exposed": false,
            "privacy_note": "Validation scans the complete source, but event values, example values, detailed finding prose, source paths, and decision prose are not returned. Column names follow the exposure policy."
        }))
    }

    fn run_analysis(&self, project_id: &str) -> Result<Value, String> {
        let (project, analysis) = self.analyze_project(project_id)?;
        let process_model_count = self.process_model_records()?.len();
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "status": "complete",
            "claim_type": "calculated_fact",
            "history_semantics": if analysis.mapping.object_history { "Object histories may span separate transactions. First-to-last spans are not business cycle times; repeated activities are not proof of rework." } else { "Case-based process histories" },
            "view": analysis.view,
            "summary": {
                "events": analysis.summary.events,
                "cases": analysis.summary.cases,
                "activities": analysis.summary.activities,
                "variants": analysis.summary.variants
            },
            "results": {
                "process_summary": { "available": true, "tool": "get_process_summary" },
                "variants": { "available": !analysis.variants.is_empty(), "count": analysis.variants.len(), "tool": "get_variants" },
                "bottlenecks": { "available": !analysis.bottlenecks.is_empty(), "count": analysis.bottlenecks.len(), "tool": "find_bottlenecks" },
                "rework": { "available": analysis.rework.cases_affected > 0, "activity_count": analysis.rework.activities.len(), "loop_count": analysis.rework.loops.len(), "tool": "find_rework" },
                "automation_targeting": { "available": analysis.targeting.is_some(), "tool": "get_automation_targeting" },
                "case_evidence": { "available": analysis.summary.cases > 0 && self.exposure.case_ids != Mode::Omit, "tool": "get_evidence" },
                "formal_discovery": { "available": self.exposure.activity_names != Mode::Omit, "tool": "discover_process_model" },
                "process_conformance": {
                    "available": process_model_count > 0,
                    "baseline_models": process_model_count,
                    "list_tool": "list_process_models",
                    "token_replay_tool": "check_process_conformance",
                    "alignment_tool": "align_process_conformance"
                }
            },
            "preparation": {
                "excluded_events": analysis.excluded_events,
                "excluded_cases": analysis.excluded_cases
            },
            "timing": timing_description(&analysis),
            "next_step": "Call only the result tools relevant to the question; run_analysis intentionally does not duplicate their detailed payloads.",
            "privacy_note": "This compact manifest returns no activity, resource, case, timestamp, source-path, or raw-event values. Detailed tools independently enforce the exposure policy."
        }))
    }

    fn get_process_summary(&self, project_id: &str) -> Result<Value, String> {
        let (project, analysis) = self.analyze_project(project_id)?;
        let summary = &analysis.summary;
        Ok(json!({
            "project": project_id,
            "name": self.exposure.transform(
                self.exposure.project_names,
                "project_name",
                &project.name
            ),
            "claim_type": "calculated_fact",
            "summary": {
                "events": summary.events,
                "cases": summary.cases,
                "activities": summary.activities,
                "variants": summary.variants,
                "single_event_cases": summary.single_event_cases,
                "unique_variants": summary.unique_variants,
                "top_five_coverage_percent": summary.top_five_coverage_percent,
                "variants_for_80_percent": summary.variants_for_80_percent,
                "mean_throughput_seconds": summary.mean_throughput_seconds,
                "median_throughput_seconds": summary.median_throughput_seconds,
                "p90_throughput_seconds": summary.p90_throughput_seconds,
                "duration_eligible_cases": summary.duration_eligible_cases,
                "incomplete_cases": summary.incomplete_cases,
                "total_cost_microunits": summary.total_cost_microunits.and_then(|cost| {
                    self.exposure.transform(self.exposure.costs, "cost", &cost.to_string())
                })
            },
            "history_semantics": if analysis.mapping.object_history { "Object histories may span separate transactions. First-to-last spans are not business cycle times; repeated activities are not proof of rework." } else { "Case-based process histories" },
            "view": analysis.view,
            "timing": timing_description(&analysis),
            "completion_distribution": {
                "bin_width_seconds": analysis.timing.bin_width_seconds,
                "bins": analysis.timing.bins,
                "tool_for_slowest_cases": "get_slowest_cases"
            },
            "preparation": {
                "excluded_events": analysis.excluded_events,
                "excluded_cases": analysis.excluded_cases
            },
            "privacy_note": "No raw events, source path, timestamps, activity names, or case identifiers are returned by this tool. Aggregate cost follows the exposure policy."
        }))
    }

    fn get_slowest_cases(&self, project_id: &str, limit: usize) -> Result<Value, String> {
        let (project, analysis) = self.analyze_project(project_id)?;
        let cases = analysis
            .timing
            .slowest_cases
            .iter()
            .take(limit)
            .map(|case| {
                json!({
                    "case": self.exposure.transform(self.exposure.case_ids, "case", &case.case),
                    "duration_seconds": case.duration_seconds,
                    "events": case.events,
                    "route_rank": case.route_rank,
                    "repeats_step": case.repeats_step
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "calculated_fact",
            "history_semantics": if analysis.mapping.object_history { "Object histories may span separate transactions. First-to-last spans are not business cycle times; repeated activities are not proof of rework." } else { "Case-based process histories" },
            "view": analysis.view,
            "timing": timing_description(&analysis),
            "median_throughput_seconds": analysis.timing.median_seconds,
            "p90_throughput_seconds": analysis.timing.p90_seconds,
            "returned": cases.len(),
            "cases": cases,
            "privacy_note": "Case identifiers follow the exposure policy; no timestamps or activity names are returned. Cases marked incomplete are excluded, as in every duration statistic."
        }))
    }

    fn get_variants(&self, project_id: &str, limit: usize) -> Result<Value, String> {
        let (project, analysis) = self.analyze_project(project_id)?;
        let mut cumulative_percent = 0.0;
        let variants = analysis
            .variants
            .iter()
            .take(limit)
            .enumerate()
            .map(|(index, variant)| {
                cumulative_percent += variant.percent;
                let sequence = match self.exposure.activity_names {
                    Mode::Omit => None,
                    mode => Some(
                        variant
                            .sequence
                            .iter()
                            .filter_map(|activity| {
                                self.exposure.transform(mode, "activity", activity)
                            })
                            .collect::<Vec<_>>(),
                    ),
                };
                let example_cases = variant
                    .example_cases
                    .iter()
                    .filter_map(|case| {
                        self.exposure
                            .transform(self.exposure.case_ids, "case", case)
                    })
                    .collect::<Vec<_>>();
                json!({
                    "rank": index + 1,
                    "sequence": sequence,
                    "activity_count": variant.sequence.len(),
                    "cases": variant.cases,
                    "percent": variant.percent,
                    "cumulative_percent": cumulative_percent,
                    "duration_eligible_cases": variant.duration_eligible_cases,
                    "mean_throughput_seconds": variant.mean_throughput_seconds,
                    "example_cases": example_cases
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "name": self.exposure.transform(
                self.exposure.project_names,
                "project_name",
                &project.name
            ),
            "claim_type": "calculated_fact",
            "history_semantics": if analysis.mapping.object_history { "Object histories may span separate transactions. First-to-last spans are not business cycle times; repeated activities are not proof of rework." } else { "Case-based process histories" },
            "view": analysis.view,
            "total_variants": analysis.summary.variants,
            "ranked_variants_available": analysis.variants.len(),
            "returned": variants.len(),
            "variants": variants,
            "timing": timing_description(&analysis),
            "privacy_note": "Results are capped at the engine's top 20 ranked variants. Activity labels and example case identifiers follow the exposure policy; omitted sequences remain null rather than being shortened."
        }))
    }

    fn find_bottlenecks(&self, project_id: &str, limit: usize) -> Result<Value, String> {
        let (project, analysis) = self.analyze_project(project_id)?;
        let bottlenecks = analysis
            .bottlenecks
            .iter()
            .take(limit)
            .map(|bottleneck| {
                json!({
                    "rank": bottleneck.rank,
                    "from": self.activity(&bottleneck.from),
                    "to": self.activity(&bottleneck.to),
                    "total_seconds": bottleneck.total_seconds,
                    "mean_seconds": bottleneck.mean_seconds,
                    "total_occurrences": bottleneck.total_occurrences,
                    "duration_eligible_occurrences": bottleneck.duration_eligible_occurrences,
                    "share_percent": bottleneck.share_percent,
                    "example_cases": self.cases(&bottleneck.example_cases)
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "calculated_fact",
            "history_semantics": if analysis.mapping.object_history { "Object histories may span separate transactions. First-to-last spans are not business cycle times; repeated activities are not proof of rework." } else { "Case-based process histories" },
            "view": analysis.view,
            "returned": bottlenecks.len(),
            "bottlenecks": bottlenecks,
            "timing": timing_description(&analysis),
            "ranking_explanation": "Transitions are ranked by total duration across eligible occurrences, not by the slowest individual case or the largest mean.",
            "privacy_note": "Activity labels and bounded example case identifiers follow the exposure policy. Omitted endpoints remain null."
        }))
    }

    /// Field values that go with the slowest fifth of items: plain counts and
    /// medians over every case attribute and who handled the item. Field
    /// names, values and people follow the exposure policy; a driver whose
    /// field or value is withheld is left out rather than shown blank.
    fn find_drivers(&self, project_id: &str, limit: usize) -> Result<Value, String> {
        let (project, analysis) = self.analyze_project(project_id)?;
        let drivers = analysis
            .drivers
            .drivers
            .iter()
            .filter_map(|driver| {
                let (field, value) = if driver.field == "Handled by" {
                    (
                        Some("Handled by".to_owned()),
                        self.exposure.transform(
                            self.exposure.resource_names,
                            "resource",
                            &driver.value,
                        ),
                    )
                } else {
                    (
                        self.exposure.transform(
                            self.exposure.attribute_names,
                            "attribute",
                            &driver.field,
                        ),
                        if driver.at_least.is_some() {
                            (self.exposure.costs == Mode::Show).then(|| driver.value.clone())
                        } else {
                            self.exposure.transform(
                                self.exposure.attribute_values,
                                "attribute_value",
                                &driver.value,
                            )
                        },
                    )
                };
                Some(json!({
                    "field": field?,
                    "value": value?,
                    "items": driver.items,
                    "slow_items": driver.slow_items,
                    "share_of_items_percent": driver.share_of_items,
                    "share_of_slow_percent": driver.share_of_slow,
                    "times_as_likely_to_be_slow": driver.lift,
                    "median_seconds": driver.median_seconds,
                    "others_median_seconds": driver.others_median_seconds,
                    "example_cases": self.cases(&driver.example_items)
                }))
            })
            .take(limit)
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "calculated_fact",
            "view": analysis.view,
            "measured_items": analysis.drivers.measured,
            "slow_from_seconds": analysis.drivers.slow_from_seconds,
            "returned": drivers.len(),
            "drivers": drivers,
            "method": "Items are slow when they are in the slowest fifth by first-to-last time. Each field value is ranked by how many more slow items it has than its size would give by chance; at most two values per field.",
            "interpretation_note": analysis.drivers.note,
            "privacy_note": "Field names, values, people and example case identifiers follow the exposure policy; number bands appear only when amounts may be shown."
        }))
    }

    fn find_rework(
        &self,
        project_id: &str,
        activity_limit: usize,
        loop_limit: usize,
    ) -> Result<Value, String> {
        let (project, analysis) = self.analyze_project(project_id)?;
        let rework = &analysis.rework;
        let activities = rework
            .activities
            .iter()
            .take(activity_limit)
            .map(|activity| {
                json!({
                    "activity": self.activity(&activity.activity),
                    "repetitions": activity.repetitions,
                    "immediate_repetitions": activity.immediate_repetitions,
                    "returns_after_intervening": activity.returns_after_intervening,
                    "cases_affected": activity.cases_affected,
                    "duration_eligible_occurrences": activity.duration_eligible_occurrences,
                    "mean_repeat_interval_seconds": activity.mean_repeat_interval_seconds,
                    "total_repeat_interval_seconds": activity.total_repeat_interval_seconds,
                    "example_cases": self.cases(&activity.example_cases)
                })
            })
            .collect::<Vec<_>>();
        let loops = rework
            .loops
            .iter()
            .take(loop_limit)
            .map(|loop_path| {
                json!({
                    "sequence": self.activity_sequence(&loop_path.sequence),
                    "activity_count": loop_path.sequence.len(),
                    "occurrences": loop_path.occurrences,
                    "cases_affected": loop_path.cases_affected,
                    "duration_eligible_occurrences": loop_path.duration_eligible_occurrences,
                    "mean_duration_seconds": loop_path.mean_duration_seconds,
                    "total_duration_seconds": loop_path.total_duration_seconds,
                    "example_cases": self.cases(&loop_path.example_cases)
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "calculated_fact",
            "history_semantics": if analysis.mapping.object_history { "Object histories may span separate transactions. First-to-last spans are not business cycle times; repeated activities are not proof of rework." } else { "Case-based process histories" },
            "view": analysis.view,
            "summary": {
                "cases_affected": rework.cases_affected,
                "case_percent": rework.case_percent,
                "repeated_activity_occurrences": rework.repeated_activity_occurrences,
                "immediate_repetitions": rework.immediate_repetitions,
                "returns_after_intervening": rework.returns_after_intervening,
                "loop_traversals": rework.loop_traversals,
                "omitted_loop_candidates": rework.omitted_loop_candidates
            },
            "activities_available": rework.activities.len(),
            "activities_returned": activities.len(),
            "activities": activities,
            "loops_available": rework.loops.len(),
            "loops_returned": loops.len(),
            "loops": loops,
            "timing": {
                "repeat_interval": "start_to_start_elapsed",
                "unit": "seconds",
                "explanation": "Repeat intervals measure elapsed time between activity starts; they are not processing time or pure waiting time."
            },
            "privacy_note": "Activity labels, loop sequences, and bounded example case identifiers follow the exposure policy. Omitted activities and sequences remain null."
        }))
    }

    fn get_automation_targeting(&self, project_id: &str, limit: usize) -> Result<Value, String> {
        let (project, analysis) = self.analyze_project(project_id)?;
        let report = analysis.targeting.as_ref().ok_or(
            "automation targeting is unavailable because the active view has neither resource evidence nor path divergence",
        )?;
        let manual_steps = report
            .manual_steps_ranked
            .iter()
            .take(limit)
            .map(|step| self.exposed_manual_step(step))
            .collect::<Vec<_>>();
        let divergences = report
            .divergences
            .iter()
            .take(limit)
            .map(|divergence| {
                json!({
                    "after": divergence.after.as_ref().and_then(|activity| self.activity(activity)),
                    "dominant_path_next": divergence.dominant_path_next.as_ref().and_then(|activity| self.activity(activity)),
                    "took": divergence.took.as_ref().and_then(|activity| self.activity(activity)),
                    "cases": divergence.cases,
                    "example_cases": self.cases(&divergence.example_cases)
                })
            })
            .collect::<Vec<_>>();
        let unclassified_resources = report
            .unclassified_resources
            .iter()
            .filter_map(|resource| {
                self.exposure
                    .transform(self.exposure.resource_names, "resource", resource)
            })
            .collect::<Vec<_>>();
        let suggested_system = report
            .suggested_system_resources
            .iter()
            .filter_map(|resource| {
                self.exposure
                    .transform(self.exposure.resource_names, "resource", resource)
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "calculated_fact",
            "history_semantics": if analysis.mapping.object_history { "Object histories may span separate transactions. First-to-last spans are not business cycle times; repeated activities are not proof of rework." } else { "Case-based process histories" },
            "view": analysis.view,
            "classification": {
                "resources_mapped": report.resources_mapped,
                "settled": report.is_settled(),
                "coverage_percent": report.classification_coverage_percent,
                "unclassified_resources": unclassified_resources,
                "not_yet_classified_events": report.unclassified_events,
                "suggested_system_resources": suggested_system,
                "next_step": if report.resources_mapped && !report.unclassified_resources.is_empty() {
                    Some("Ask the person which accounts are systems, starting with suggested_system_resources, and record their answer as resource_kinds. Until then, report undecided steps as not yet classified, not as manual, and do not quote the straight-through rate.")
                } else {
                    None
                },
                "straight_through_percent": report.straight_through_percent,
                "straight_through_cases": report.straight_through_cases,
                "human_touched_cases": report.touched_cases,
                "undetermined_cases": report.undetermined_cases
            },
            "event_totals": {
                "manual": report.manual_events,
                "system": report.system_events,
                "unrecorded": report.unrecorded_events
            },
            "time_totals_seconds": {
                "manual_processing": report.manual_processing_seconds,
                "waiting_before_manual": report.waiting_before_manual_seconds,
                "start_to_start_before_manual": report.start_to_start_before_manual_seconds
            },
            "ranking_basis": report.ranking_basis,
            "manual_steps_available": report.manual_steps_ranked.len(),
            "manual_steps_returned": manual_steps.len(),
            "manual_steps": manual_steps,
            "dominant_observed_variant": {
                "cases": report.dominant_variant_cases,
                "percent": report.dominant_variant_percent
            },
            "divergences_available": report.divergences.len(),
            "divergences_returned": divergences.len(),
            "divergences": divergences,
            "interpretation_limits": report.interpretation_limits,
            "interpretation_note": "This is evidence for investigating automation targets, not a recommendation or readiness verdict.",
            "privacy_note": "Activity, resource, and bounded example-case identifiers follow the exposure policy. Returned activity identifiers can be passed to get_activity_context."
        }))
    }

    fn get_activity_context(
        &self,
        project_id: &str,
        exposed_activity: &str,
        limit: usize,
    ) -> Result<Value, String> {
        let (project, analysis) = self.analyze_project(project_id)?;
        let candidates = analysis
            .nodes
            .iter()
            .map(|node| node.activity.clone())
            .collect::<Vec<_>>();
        let activity = resolve_exposed(
            &self.exposure,
            self.exposure.activity_names,
            "activity",
            exposed_activity,
            &candidates,
        )?;
        let node = analysis
            .nodes
            .iter()
            .find(|node| node.activity == activity)
            .ok_or("the activity is not present in the saved active view")?;
        let transitions = analysis
            .edges
            .iter()
            .filter(|edge| edge.from == activity || edge.to == activity)
            .take(limit)
            .map(|edge| {
                json!({
                    "from": self.activity(&edge.from),
                    "to": self.activity(&edge.to),
                    "frequency": edge.frequency,
                    "case_frequency": edge.case_frequency,
                    "duration_eligible_occurrences": edge.duration_eligible_occurrences,
                    "mean_seconds": edge.mean_seconds,
                    "total_seconds": edge.total_seconds,
                    "example_cases": self.cases(&edge.example_cases)
                })
            })
            .collect::<Vec<_>>();
        let manual_evidence = analysis
            .targeting
            .as_ref()
            .and_then(|report| {
                report
                    .manual_steps_ranked
                    .iter()
                    .find(|step| step.activity == activity)
            })
            .map(|step| self.exposed_manual_step(step));
        let rework = analysis
            .rework
            .activities
            .iter()
            .find(|item| item.activity == activity)
            .map(|item| {
                json!({
                    "repetitions": item.repetitions,
                    "immediate_repetitions": item.immediate_repetitions,
                    "returns_after_intervening": item.returns_after_intervening,
                    "cases_affected": item.cases_affected,
                    "mean_repeat_interval_seconds": item.mean_repeat_interval_seconds,
                    "example_cases": self.cases(&item.example_cases)
                })
            });
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "calculated_fact",
            "history_semantics": if analysis.mapping.object_history { "Object histories may span separate transactions. First-to-last spans are not business cycle times; repeated activities are not proof of rework." } else { "Case-based process histories" },
            "view": analysis.view,
            "activity": self.activity(&activity),
            "occurrences": node.frequency,
            "cases": node.case_frequency,
            "starts": node.starts,
            "ends": node.ends,
            "example_cases": self.cases(&node.example_cases),
            "connected_transitions_returned": transitions.len(),
            "connected_transitions": transitions,
            "manual_evidence": manual_evidence,
            "rework_evidence": rework,
            "timing": timing_description(&analysis),
            "interpretation_note": "Manual evidence describes observed scale and routing regularity. It does not establish that inputs, decisions, risk, or implementation effort make the activity suitable for automation.",
            "privacy_note": "All identifiers follow the exposure policy; omitted activity identifiers make this drill-down unavailable."
        }))
    }

    fn get_process_map(
        &self,
        project_id: &str,
        selection: &pm_query::MapSelection,
    ) -> Result<Value, String> {
        let (project, loaded) = self.load_project(project_id)?;
        let filtered = project.view_filter.apply(&loaded.log);
        let map =
            pm_query::map_view(&filtered.log, selection).map_err(|error| error.to_string())?;
        let edges = map
            .edges
            .iter()
            .take(MAP_CONNECTIONS_RETURNED)
            .map(|edge| {
                json!({
                    "from": self.activity(&edge.edge.from),
                    "to": self.activity(&edge.edge.to),
                    "frequency": edge.edge.frequency,
                    "case_frequency": edge.edge.case_frequency,
                    "duration_eligible_occurrences": edge.edge.duration_eligible_occurrences,
                    "mean_seconds": edge.edge.mean_seconds,
                    "slow": edge.slow
                })
            })
            .collect::<Vec<_>>();
        let activities = map
            .activity_ranking
            .iter()
            .take(MAP_CONNECTIONS_RETURNED)
            .map(|activity| {
                json!({
                    "rank": activity.rank,
                    "activity": self.activity(&activity.activity),
                    "frequency": activity.frequency,
                    "case_frequency": activity.case_frequency,
                    "shown": activity.shown
                })
            })
            .collect::<Vec<_>>();
        let variants = map
            .variants
            .iter()
            .take(20)
            .map(|variant| {
                json!({
                    "rank": variant.rank,
                    "sequence": self.activity_sequence(&variant.sequence),
                    "cases": variant.cases,
                    "percent": variant.percent,
                    "shown": variant.shown,
                    "example_cases": self.cases(&variant.example_cases)
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "calculated_fact",
            "view": view_description(&filtered),
            "selection": map.selection,
            "top_route": self.activity_sequence(&map.top_route),
            "activities_shown": map.activities_shown,
            "activities_total": map.activities_total,
            "connections_shown": map.connections_shown,
            "connections_total": map.connections_total,
            "variants_shown": map.variants_shown,
            "variants_total": map.variants_total,
            "cases_covered": map.cases_covered,
            "cases_total": map.cases_total,
            "percent_covered": map.percent_covered,
            "slow_threshold_seconds": map.slow_threshold_seconds,
            "timing": if map.measures_true_waiting { "waiting" } else { "elapsed" },
            "connections_returned": edges.len(),
            "connections": edges,
            "activity_ranking": activities,
            "variants": variants,
            "notes": map.notes,
            "privacy_note": "Activity labels and example case identifiers follow the exposure policy. At most 100 connections and activities and 20 routes are returned; the shown and total counts are complete."
        }))
    }

    fn explore_linked_interval(&self, arguments: &Value) -> Result<Value, String> {
        let handle = required_string(arguments, "log", "explore_linked_interval")?;
        let record = self.resolve_ocel(handle)?;
        let names = pm_query::ocel_names(&record.path).map_err(|e| e.to_string())?;
        let mut interval: pm_query::linked_interval::LinkedInterval = serde_json::from_value(
            arguments
                .get("interval")
                .cloned()
                .ok_or("interval is required")?,
        )
        .map_err(|e| e.to_string())?;
        let exposed = interval.clone();
        for kind in [
            &mut interval.root_type,
            &mut interval.from_type,
            &mut interval.to_type,
        ] {
            *kind = resolve_exposed(
                &self.exposure,
                self.exposure.object_type_names,
                "object_type",
                kind,
                &names.object_types,
            )?;
        }
        for activity in [&mut interval.segment.from, &mut interval.segment.to] {
            *activity = resolve_exposed(
                &self.exposure,
                self.exposure.activity_names,
                "activity",
                activity,
                &names.activities,
            )?;
        }
        let roots = pm_query::linked_interval::native_roots(
            &record.path,
            &record.filter,
            &interval.root_type,
        )?;
        if let Some(field) = &mut interval.group_by {
            *field = resolve_exposed(
                &self.exposure,
                self.exposure.attribute_names,
                "attribute",
                field,
                &roots.attribute_names,
            )?;
        }
        let result = pm_query::linked_interval::explore_linked(
            &record.path,
            &record.filter,
            &roots,
            &interval,
        )?;
        let mut report = result.report;
        report.request = pm_query::exploration::ExplorationRequest {
            segment: Some(exposed.segment),
            group_by: exposed.group_by,
            ..Default::default()
        };
        serde_json::to_value(self.expose_exploration(report)).map_err(|e| e.to_string())
    }

    fn explore_process(&self, arguments: &Value) -> Result<Value, String> {
        let project_id = required_project(arguments, "explore_process")?;
        let (project, loaded) = self.load_project(project_id)?;
        let filtered = project.view_filter.apply(&loaded.log);
        let mut query: pm_query::exploration::ExplorationRequest = serde_json::from_value(
            json!({"segment":arguments.get("segment"),"group_by":arguments.get("group_by"),"min_micros":arguments.get("min_micros"),"max_micros":arguments.get("max_micros")}),
        )
        .map_err(|e| e.to_string())?;
        let exposed = query.clone();
        if let Some(field) = &mut query.group_by {
            *field = resolve_exposed(
                &self.exposure,
                self.exposure.attribute_names,
                "attribute",
                field,
                &filtered.log.attribute_names,
            )?;
        }
        if let Some(segment) = &mut query.segment {
            let names = (0..filtered.log.activities.len())
                .filter_map(|id| filtered.log.activities.label(id as u32).map(str::to_owned))
                .collect::<Vec<_>>();
            segment.from = resolve_exposed(
                &self.exposure,
                self.exposure.activity_names,
                "activity",
                &segment.from,
                &names,
            )?;
            segment.to = resolve_exposed(
                &self.exposure,
                self.exposure.activity_names,
                "activity",
                &segment.to,
                &names,
            )?;
        }
        let mut report = pm_query::exploration::explore(&filtered.log, &query)?;
        report.request = exposed;
        serde_json::to_value(self.expose_exploration(report)).map_err(|e| e.to_string())
    }

    fn expose_exploration(
        &self,
        mut report: pm_query::exploration::Exploration,
    ) -> pm_query::exploration::Exploration {
        for population in [
            &mut report.overall,
            &mut report.selected,
            &mut report.remainder,
        ] {
            population.examples = self.cases(&population.examples);
        }
        for group in &mut report.groups {
            group.value = group.value.as_deref().and_then(|value| {
                self.exposure
                    .transform(self.exposure.attribute_values, "attribute_value", value)
            });
            group.population.examples = self.cases(&group.population.examples);
        }
        if self.exposure.attribute_values == Mode::Omit {
            report.groups.clear();
        }
        report.contrasts.retain_mut(|contrast| {
            if let Some(name) = self.activity(&contrast.activity) {
                contrast.activity = name;
                true
            } else {
                false
            }
        });
        report
    }

    fn compare_cohorts(
        &self,
        project_id: &str,
        exposed_attribute: &str,
        exposed_left: &str,
        exposed_right: &str,
        limit: usize,
    ) -> Result<Value, String> {
        let (project, loaded) = self.load_project(project_id)?;
        let filtered = project.view_filter.apply(&loaded.log);
        let attribute = resolve_exposed(
            &self.exposure,
            self.exposure.attribute_names,
            "attribute",
            exposed_attribute,
            &filtered.log.attribute_names,
        )?;
        let values = filtered
            .log
            .attribute_options(&attribute)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let left = resolve_exposed(
            &self.exposure,
            self.exposure.attribute_values,
            "attribute_value",
            exposed_left,
            &values,
        )?;
        let right = resolve_exposed(
            &self.exposure,
            self.exposure.attribute_values,
            "attribute_value",
            exposed_right,
            &values,
        )?;
        let comparison = pm_query::compare_cohorts(&filtered.log, &attribute, &left, &right)
            .map_err(|error| error.to_string())?;
        let expose_value = |value: &str| {
            self.exposure
                .transform(self.exposure.attribute_values, "attribute_value", value)
        };
        let summary = |cohort: &pm_query::CohortSummary| {
            json!({
                "value": expose_value(&cohort.value),
                "cases": cohort.cases,
                "events": cohort.events,
                "duration_eligible_cases": cohort.duration_eligible_cases,
                "mean_throughput_seconds": cohort.mean_throughput_seconds,
                "median_throughput_seconds": cohort.median_throughput_seconds
            })
        };
        let differences = comparison
            .activity_differences
            .iter()
            .take(limit)
            .map(|difference| {
                json!({
                    "rank": difference.rank,
                    "activity": self.activity(&difference.activity),
                    "left_occurrences": difference.left_occurrences,
                    "right_occurrences": difference.right_occurrences,
                    "left_cases": difference.left_cases,
                    "right_cases": difference.right_cases,
                    "left_case_percent": difference.left_case_percent,
                    "right_case_percent": difference.right_case_percent,
                    "difference_points": difference.difference_points,
                    "left_example_cases": self.cases(&difference.left_example_cases),
                    "right_example_cases": self.cases(&difference.right_example_cases)
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "statistical_association",
            "view": view_description(&filtered),
            "attribute": self.exposure.transform(self.exposure.attribute_names, "attribute", &attribute),
            "left": summary(&comparison.left),
            "right": summary(&comparison.right),
            "mean_throughput_difference_seconds": comparison.mean_throughput_difference_seconds,
            "activity_differences_available": comparison.activity_differences.len(),
            "activity_differences_returned": differences.len(),
            "activity_differences": differences,
            "interpretation_limits": comparison.interpretation_limits,
            "privacy_note": "Attribute, value, activity, and bounded case identifiers follow the exposure policy. Omitted attribute names or values make comparison unavailable."
        }))
    }

    /// Totals of a number field such as an invoice amount. Without a field it
    /// lists the fields that hold numbers.
    fn get_attribute_choices(
        &self,
        project_id: &str,
        exposed_attribute: &str,
    ) -> Result<Value, String> {
        let (project, loaded) = self.load_project(project_id)?;
        let filtered = project.view_filter.apply(&loaded.log);
        let attribute = resolve_exposed(
            &self.exposure,
            self.exposure.attribute_names,
            "attribute",
            exposed_attribute,
            &filtered.log.attribute_names,
        )?;
        if self.exposure.attribute_values == Mode::Omit {
            return Err("attribute values are omitted by the exposure policy".into());
        }
        let choices = pm_query::attribute_choices(&filtered.log, &attribute, "", 100)?;
        let values = choices
            .values
            .iter()
            .filter_map(|choice| {
                self.exposure
                    .transform(
                        self.exposure.attribute_values,
                        "attribute_value",
                        &choice.value,
                    )
                    .map(|value| json!({"value":value,"cases":choice.cases}))
            })
            .collect::<Vec<_>>();
        Ok(
            json!({"attribute":self.exposure.transform(self.exposure.attribute_names,"attribute",&attribute),"values":values,"distinct_values":choices.distinct_values,"missing_cases":choices.missing_cases,"truncated":choices.truncated,"privacy_note":"At most 100 values in the active saved scope, with names and values governed by the exposure policy."}),
        )
    }

    fn summarize_values(
        &self,
        project_id: &str,
        arguments: &Value,
        limit: usize,
    ) -> Result<Value, String> {
        let (project, loaded) = self.load_project(project_id)?;
        let filtered = project.view_filter.apply(&loaded.log);
        let attribute_name = |name: &str| {
            self.exposure
                .transform(self.exposure.attribute_names, "attribute", name)
        };
        let fields = pm_core::number_fields(&filtered.log)
            .into_iter()
            .filter_map(|field| {
                Some(json!({
                    "field": attribute_name(&field.name)?,
                    "cases_with_number": field.numbers,
                    "cases_not_a_number": field.not_numbers,
                    "cases_missing": field.missing
                }))
            })
            .collect::<Vec<_>>();
        let Some(exposed_field) = arguments.get("field").and_then(Value::as_str) else {
            return Ok(json!({
                "project": project_id,
                "view": view_description(&filtered),
                "number_fields": fields,
                "note": if fields.is_empty() {
                    "No case attribute holds numbers in this view, so amounts cannot be totalled; use counts and times."
                } else {
                    "Pass one of these as 'field' to total it."
                }
            }));
        };
        if self.exposure.costs != Mode::Show {
            return Err(
                "amounts are hidden by the exposure policy ('costs' is not 'show'), so value totals are unavailable"
                    .to_owned(),
            );
        }
        let field = resolve_exposed(
            &self.exposure,
            self.exposure.attribute_names,
            "attribute",
            exposed_field,
            &filtered.log.attribute_names,
        )?;
        let number = |name: &str| -> Result<Option<f64>, String> {
            match arguments.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(value) => value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .map(Some)
                    .ok_or_else(|| format!("summarize_values '{name}' must be a number")),
            }
        };
        let (at_least, at_most) = (number("at_least")?, number("at_most")?);
        let view = if at_least.is_some() || at_most.is_some() {
            pm_core::Filter::new()
                .number_between(&field, at_least, at_most)
                .apply(&filtered.log)
                .log
        } else {
            filtered.log.clone()
        };
        let grouping = match arguments.get("group_by").and_then(Value::as_str) {
            None | Some("none") => pm_query::ValueGrouping::None,
            Some("variant") => pm_query::ValueGrouping::Variant,
            Some("last_step") => pm_query::ValueGrouping::LastStep,
            Some("attribute") => {
                if self.exposure.attribute_values == Mode::Omit {
                    return Err("attribute values are omitted by the exposure policy, so grouping by an attribute is unavailable".to_owned());
                }
                let exposed = required_string(arguments, "group_attribute", "summarize_values")?;
                pm_query::ValueGrouping::Attribute(resolve_exposed(
                    &self.exposure,
                    self.exposure.attribute_names,
                    "attribute",
                    exposed,
                    &filtered.log.attribute_names,
                )?)
            }
            Some(other) => {
                return Err(format!(
                    "summarize_values group_by must be none, variant, last_step or attribute, not '{other}'"
                ));
            }
        };
        let finishing = project
            .rules
            .observation_window
            .as_ref()
            .map(|window| window.end_activities.clone())
            .unwrap_or_default();
        let summary = pm_query::summarize_values(&view, &field, &grouping, &finishing, limit)
            .map_err(|error| match error {
                pm_query::ValueError::NotNumeric(..) => format!(
                    "this field does not hold numbers; fields that do: {}. Counts and times are still available.",
                    fields
                        .iter()
                        .filter_map(|field| field["field"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                other => other.to_string(),
            })?;
        let label = |label: &str| -> Option<String> {
            match &grouping {
                pm_query::ValueGrouping::Variant => self
                    .activity_sequence(&label.split(" → ").map(str::to_owned).collect::<Vec<_>>())
                    .map(|steps| steps.join(" → ")),
                pm_query::ValueGrouping::LastStep => self.activity(label),
                _ => self.exposure.transform(
                    self.exposure.attribute_values,
                    "attribute_value",
                    label,
                ),
            }
        };
        let groups = summary
            .groups
            .iter()
            .map(|group| {
                json!({
                    "group": label(&group.label),
                    "stats": group.stats,
                    "example_cases": self.cases(&group.example_case.iter().cloned().collect::<Vec<_>>())
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "view": view_description(&filtered),
            "field": exposed_field,
            "at_least": at_least,
            "at_most": at_most,
            "group_by": arguments.get("group_by").and_then(Value::as_str).unwrap_or("none"),
            "overall": summary.overall,
            "groups_available": summary.groups_available,
            "groups": groups,
            "other_groups": summary.other_groups,
            "semantics": summary.semantics
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn compare_periods(
        &self,
        project_id: &str,
        baseline_start: i64,
        baseline_end: i64,
        comparison_start: i64,
        comparison_end: i64,
        limit: usize,
    ) -> Result<Value, String> {
        let (project, loaded) = self.load_project(project_id)?;
        let filtered = project.view_filter.apply(&loaded.log);
        let comparison = pm_query::compare_periods(
            &filtered.log,
            baseline_start,
            baseline_end,
            comparison_start,
            comparison_end,
        )
        .map_err(|error| error.to_string())?;
        let summary = |period: &pm_query::PeriodSummary| {
            json!({
                "started_at_or_after_unix_micros": period.started_at_or_after,
                "started_at_or_before_unix_micros": period.started_at_or_before,
                "cases": period.cases,
                "events": period.events,
                "duration_eligible_cases": period.duration_eligible_cases,
                "mean_throughput_seconds": period.mean_throughput_seconds,
                "median_throughput_seconds": period.median_throughput_seconds
            })
        };
        let differences = comparison
            .activity_differences
            .iter()
            .take(limit)
            .map(|difference| {
                json!({
                    "rank": difference.rank,
                    "activity": self.activity(&difference.activity),
                    "baseline_occurrences": difference.left_occurrences,
                    "comparison_occurrences": difference.right_occurrences,
                    "baseline_cases": difference.left_cases,
                    "comparison_cases": difference.right_cases,
                    "baseline_case_percent": difference.left_case_percent,
                    "comparison_case_percent": difference.right_case_percent,
                    "difference_points": difference.difference_points,
                    "baseline_example_cases": self.cases(&difference.left_example_cases),
                    "comparison_example_cases": self.cases(&difference.right_example_cases)
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "statistical_association",
            "view": view_description(&filtered),
            "window_semantics": "Cases are selected by first-event timestamp; boundaries are inclusive Unix microseconds. Windows must be non-overlapping and chronologically ordered.",
            "baseline": summary(&comparison.baseline),
            "comparison": summary(&comparison.comparison),
            "mean_throughput_difference_seconds": comparison.mean_throughput_difference_seconds,
            "activity_differences_available": comparison.activity_differences.len(),
            "activity_differences_returned": differences.len(),
            "activity_differences": differences,
            "interpretation_limits": comparison.interpretation_limits,
            "privacy_note": "Window boundaries are caller-supplied query parameters and are echoed for auditability. Activity and bounded case identifiers follow the exposure policy; event timestamps are never returned."
        }))
    }

    fn exposed_manual_step(&self, step: &pm_query::ManualStep) -> Value {
        let routing_contexts = step
            .routing_contexts
            .iter()
            .map(|context| {
                json!({
                    "before": context.before.as_ref().and_then(|activity| self.activity(activity)),
                    "after": context.after.as_ref().and_then(|activity| self.activity(activity)),
                    "occurrences": context.occurrences,
                    "share_percent": context.share_percent
                })
            })
            .collect::<Vec<_>>();
        json!({
            "activity": self.activity(&step.activity),
            "occurrences": step.occurrences,
            "cases": step.cases,
            "processing_seconds": step.processing_seconds,
            "processing_observations": step.processing_observations,
            "waiting_before_seconds": step.waiting_before_seconds,
            "waiting_observations": step.waiting_observations,
            "start_to_start_before_seconds": step.start_to_start_before_seconds,
            "start_to_start_observations": step.start_to_start_observations,
            "total_cost_microunits": self.exposure.transform(
                self.exposure.costs,
                "cost",
                &step.total_cost_microunits.to_string()
            ),
            "routing_context_concentration": step.routing_context_concentration,
            "distinct_contexts": step.distinct_contexts,
            "routing_contexts": routing_contexts,
            "example_cases": self.cases(&step.example_cases)
        })
    }

    fn get_evidence(
        &self,
        project_id: &str,
        arguments: &Value,
        limit: usize,
    ) -> Result<Value, String> {
        let (project, loaded) = self.load_project(project_id)?;
        let filtered = project.view_filter.apply(&loaded.log);
        let evidence = self.parse_evidence(arguments, &filtered.log)?;
        let total = pm_query::count_cases(&filtered.log, &evidence);
        let matching_cases = pm_query::find_cases(&filtered.log, &evidence, limit);
        let cases = self.cases(&matching_cases);
        Ok(json!({
            "project": project_id,
            "name": self.project_name(&project),
            "claim_type": "calculated_fact",
            "view": view_description(&filtered),
            "evidence": exposed_evidence(&evidence, &self.exposure),
            "total": total,
            "shown": cases.len(),
            "cases": cases,
            "bounded": total > matching_cases.len(),
            "privacy_note": "Only cases in the saved active view are searched. Returned case identifiers follow the exposure policy and can be passed directly to inspect_case."
        }))
    }

    fn inspect_case(&self, project_id: &str, exposed_case: &str) -> Result<Value, String> {
        let (project, loaded) = self.load_project(project_id)?;
        let filtered = project.view_filter.apply(&loaded.log);
        let case = resolve_exposed(
            &self.exposure,
            self.exposure.case_ids,
            "case",
            exposed_case,
            &case_candidates(&filtered.log),
        )?;
        let detail = pm_query::inspect_case_read_as(&filtered.log, &case, loaded.timestamp_reading)
            .ok_or("the case is not present in the saved active view")?;
        Ok(redact_case_detail(
            project_id,
            self.project_name(&project),
            &detail,
            &self.exposure,
            view_description(&filtered),
        ))
    }

    fn parse_evidence(
        &self,
        arguments: &Value,
        log: &pm_core::EventLog,
    ) -> Result<pm_query::Evidence, String> {
        let kind = arguments
            .get("kind")
            .and_then(Value::as_str)
            .ok_or("get_evidence requires kind: activity, transition, variant, or handover")?;
        let field = |name: &str| {
            arguments
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{kind} evidence requires '{name}'"))
        };
        let activities = activity_candidates(log);
        let resources = resource_candidates(log);
        match kind {
            "activity" => Ok(pm_query::Evidence::Activity(resolve_exposed(
                &self.exposure,
                self.exposure.activity_names,
                "activity",
                field("activity")?,
                &activities,
            )?)),
            "transition" => Ok(pm_query::Evidence::Transition {
                from: resolve_exposed(
                    &self.exposure,
                    self.exposure.activity_names,
                    "activity",
                    field("from")?,
                    &activities,
                )?,
                to: resolve_exposed(
                    &self.exposure,
                    self.exposure.activity_names,
                    "activity",
                    field("to")?,
                    &activities,
                )?,
            }),
            "variant" => {
                let sequence = arguments
                    .get("sequence")
                    .and_then(Value::as_array)
                    .ok_or("variant evidence requires a sequence array")?;
                if sequence.is_empty() {
                    return Err("variant evidence sequence must not be empty".to_owned());
                }
                let sequence = sequence
                    .iter()
                    .map(|value| {
                        let value = value
                            .as_str()
                            .ok_or("every variant sequence item must be a string")?;
                        resolve_exposed(
                            &self.exposure,
                            self.exposure.activity_names,
                            "activity",
                            value,
                            &activities,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(pm_query::Evidence::Variant(sequence))
            }
            "handover" => Ok(pm_query::Evidence::Handover {
                from: resolve_exposed(
                    &self.exposure,
                    self.exposure.resource_names,
                    "resource",
                    field("from")?,
                    &resources,
                )?,
                to: resolve_exposed(
                    &self.exposure,
                    self.exposure.resource_names,
                    "resource",
                    field("to")?,
                    &resources,
                )?,
            }),
            _ => Err("kind must be activity, transition, variant, or handover".to_owned()),
        }
    }

    fn analyze_project(&self, project_id: &str) -> Result<(Project, Box<Analysis>), String> {
        let record = self.resolve_project(project_id)?;
        let state = record.source_state;
        let project = record.project;
        if !state.reproduces() {
            return Err(state.explain());
        }
        let outcome = pm_query::analyze_filtered(
            &project.source.path,
            Some(project.mapping.clone()),
            &project.policy,
            project.rules.clone(),
            project.view_filter.clone(),
        )
        .map_err(|_| {
            "analysis could not be completed; open the project locally to review the source and mapping"
                .to_owned()
        })?;
        match outcome {
            Outcome::Complete(analysis) => Ok((project, analysis)),
            Outcome::Blocked(blocked) => Err(format!(
                "analysis is blocked by {} unresolved data-quality decision(s); open the project locally to review them",
                blocked
                    .report
                    .findings
                    .iter()
                    .filter(|finding| finding.is_blocking())
                    .count()
            )),
        }
    }

    fn load_project(&self, project_id: &str) -> Result<(Project, pm_query::LoadedLog), String> {
        let record = self.resolve_project(project_id)?;
        let state = record.source_state;
        let project = record.project;
        if !state.reproduces() {
            return Err(state.explain());
        }
        match pm_query::load_log(
            &project.source.path,
            Some(project.mapping.clone()),
            &project.policy,
            project.rules.clone(),
        )
        .map_err(|_| {
            "the project could not be opened; review its source and mapping locally".to_owned()
        })? {
            Ok(loaded) => Ok((project, loaded)),
            Err(blocked) => Err(format!(
                "analysis is blocked by {} unresolved data-quality decision(s); open the project locally to review them",
                blocked
                    .report
                    .findings
                    .iter()
                    .filter(|finding| finding.is_blocking())
                    .count()
            )),
        }
    }

    fn project_name(&self, project: &Project) -> Option<String> {
        self.exposure
            .transform(self.exposure.project_names, "project_name", &project.name)
    }

    fn workspace_kind(&self) -> &'static str {
        match &self.workspace {
            Workspace::Directory(_) => "project_directory",
            Workspace::Live(_) => "running_application",
        }
    }

    fn activity(&self, activity: &str) -> Option<String> {
        self.exposure
            .transform(self.exposure.activity_names, "activity", activity)
    }

    fn activity_sequence(&self, sequence: &[String]) -> Option<Vec<String>> {
        match self.exposure.activity_names {
            Mode::Omit => None,
            mode => Some(
                sequence
                    .iter()
                    .filter_map(|activity| self.exposure.transform(mode, "activity", activity))
                    .collect(),
            ),
        }
    }

    fn cases(&self, cases: &[String]) -> Vec<String> {
        cases
            .iter()
            .filter_map(|case| {
                self.exposure
                    .transform(self.exposure.case_ids, "case", case)
            })
            .collect()
    }

    fn resolve_project(&self, id: &str) -> Result<ProjectRecord, String> {
        if !id.starts_with("project_") {
            return Err(
                "project must be an opaque identifier returned by list_projects".to_owned(),
            );
        }
        self.project_records()?
            .into_iter()
            .find(|record| self.project_handle(&record.key) == id)
            .ok_or_else(|| format!("project '{id}' was not found"))
    }

    fn project_records(&self) -> Result<Vec<ProjectRecord>, String> {
        match &self.workspace {
            Workspace::Live(workspace) => Ok(workspace
                .case_project
                .as_ref()
                .map(|live| ProjectRecord {
                    key: format!("live:{}", live.source_name),
                    source_name: live.source_name.clone(),
                    project: live.project.clone(),
                    // The running owner created and retains this 0600 random
                    // temporary file; do not hash a 100 GiB source again on
                    // every delegated tool call.
                    source_state: SourceState::Unchanged,
                })
                .into_iter()
                .collect()),
            Workspace::Directory(root) => {
                let mut records = std::fs::read_dir(root)
                    .map_err(|error| format!("could not list project directory: {error}"))?
                    .filter_map(Result::ok)
                    .filter_map(|entry| entry.path().canonicalize().ok())
                    .filter(|path| path.parent() == Some(root.as_path()))
                    .filter(|path| {
                        path.is_file()
                            && path.extension().and_then(|value| value.to_str()) == Some("json")
                    })
                    .filter_map(|path| {
                        let project = Project::load(&path).ok()?;
                        let key = path.file_name()?.to_str()?.to_owned();
                        let source_name = Path::new(&project.source.path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        Some(ProjectRecord {
                            key,
                            source_name,
                            source_state: project.check_recorded_source(),
                            project,
                        })
                    })
                    .collect::<Vec<_>>();
                records.sort_by(|left, right| left.key.cmp(&right.key));
                Ok(records)
            }
        }
    }

    fn ocel_records(&self) -> Result<Vec<OcelRecord>, String> {
        match &self.workspace {
            Workspace::Live(workspace) => Ok(workspace
                .ocel_log
                .as_ref()
                .map(|live| OcelRecord {
                    key: format!("live:{}", live.source_name),
                    source_name: live.source_name.clone(),
                    path: live.path.clone(),
                    filter: live.filter.clone(),
                })
                .into_iter()
                .collect()),
            Workspace::Directory(root) => {
                let mut records = std::fs::read_dir(root)
                    .map_err(|error| format!("could not list MCP directory: {error}"))?
                    .filter_map(Result::ok)
                    .filter_map(|entry| entry.path().canonicalize().ok())
                    .filter(|path| path.parent() == Some(root.as_path()))
                    .filter(|path| path.is_file() && pm_ingest::is_ocel(path))
                    .filter_map(|path| {
                        let source_name = path.file_name()?.to_str()?.to_owned();
                        Some(OcelRecord {
                            key: source_name.clone(),
                            source_name,
                            path,
                            filter: pm_core::OcelFilter::default(),
                        })
                    })
                    .collect::<Vec<_>>();
                records.sort_by(|left, right| left.key.cmp(&right.key));
                Ok(records)
            }
        }
    }

    fn ocel_model_records(&self) -> Result<Vec<OcelModelRecord>, String> {
        match &self.workspace {
            Workspace::Live(workspace) => Ok(workspace
                .ocel_model
                .as_ref()
                .map(|live| OcelModelRecord {
                    key: format!("live:{}", live.source_name),
                    source_name: live.source_name.clone(),
                    path: live.path.clone(),
                })
                .into_iter()
                .collect()),
            Workspace::Directory(root) => {
                let mut records = std::fs::read_dir(root)
                    .map_err(|error| format!("could not list MCP directory: {error}"))?
                    .filter_map(Result::ok)
                    .filter_map(|entry| entry.path().canonicalize().ok())
                    .filter(|path| path.parent() == Some(root.as_path()))
                    .filter(|path| {
                        path.is_file()
                            && path
                                .file_name()
                                .and_then(|value| value.to_str())
                                .is_some_and(|name| name.ends_with(".ocelmodel.json"))
                    })
                    .filter_map(|path| {
                        let source_name = path.file_name()?.to_str()?.to_owned();
                        Some(OcelModelRecord {
                            key: source_name.clone(),
                            source_name,
                            path,
                        })
                    })
                    .collect::<Vec<_>>();
                records.sort_by(|left, right| left.key.cmp(&right.key));
                Ok(records)
            }
        }
    }

    fn process_model_records(&self) -> Result<Vec<ProcessModelRecord>, String> {
        match &self.workspace {
            Workspace::Live(workspace) => Ok(workspace
                .case_model
                .as_ref()
                .map(|live| ProcessModelRecord {
                    key: format!("live:{}", live.source_name),
                    source_name: live.source_name.clone(),
                    path: live.path.clone(),
                })
                .into_iter()
                .collect()),
            Workspace::Directory(root) => {
                let mut records = std::fs::read_dir(root)
                    .map_err(|error| format!("could not list MCP directory: {error}"))?
                    .filter_map(Result::ok)
                    .filter_map(|entry| entry.path().canonicalize().ok())
                    .filter(|path| path.parent() == Some(root.as_path()))
                    .filter(|path| {
                        path.is_file()
                            && path
                                .file_name()
                                .and_then(|value| value.to_str())
                                .is_some_and(|name| {
                                    name.ends_with(".model.json")
                                        && !name.ends_with(".ocelmodel.json")
                                })
                    })
                    .filter_map(|path| {
                        let source_name = path.file_name()?.to_str()?.to_owned();
                        Some(ProcessModelRecord {
                            key: source_name.clone(),
                            source_name,
                            path,
                        })
                    })
                    .collect::<Vec<_>>();
                records.sort_by(|left, right| left.key.cmp(&right.key));
                Ok(records)
            }
        }
    }

    fn resolve_ocel(&self, id: &str) -> Result<OcelRecord, String> {
        if !id.starts_with("ocel_log_") {
            return Err("log must be an opaque identifier returned by list_ocel_logs".to_owned());
        }
        self.ocel_records()?
            .into_iter()
            .find(|record| self.ocel_handle(&record.key) == id)
            .ok_or_else(|| format!("object-centric log '{id}' was not found"))
    }

    fn resolve_ocel_model(&self, id: &str) -> Result<OcelModelRecord, String> {
        if !id.starts_with("ocel_model_") {
            return Err(
                "model must be an opaque identifier returned by list_ocel_models".to_owned(),
            );
        }
        self.ocel_model_records()?
            .into_iter()
            .find(|record| self.ocel_model_handle(&record.key) == id)
            .ok_or_else(|| format!("object-centric model '{id}' was not found"))
    }

    fn resolve_process_model(&self, id: &str) -> Result<ProcessModelRecord, String> {
        if !id.starts_with("case_model_") {
            return Err(
                "model must be an opaque identifier returned by list_process_models".to_owned(),
            );
        }
        self.process_model_records()?
            .into_iter()
            .find(|record| self.process_model_handle(&record.key) == id)
            .ok_or_else(|| format!("process model '{id}' was not found"))
    }

    fn ocel_handle(&self, file_name: &str) -> String {
        self.exposure
            .transform(Mode::Alias, "ocel_log", file_name)
            .expect("alias mode always returns a value")
    }

    fn ocel_model_handle(&self, file_name: &str) -> String {
        self.exposure
            .transform(Mode::Alias, "ocel_model", file_name)
            .expect("alias mode always returns a value")
    }

    fn process_model_handle(&self, file_name: &str) -> String {
        self.exposure
            .transform(Mode::Alias, "case_model", file_name)
            .expect("alias mode always returns a value")
    }

    fn project_handle(&self, file_name: &str) -> String {
        self.exposure
            .transform(Mode::Alias, "project", file_name)
            .expect("alias mode always returns a value")
    }
}

/// Runs newline-delimited MCP over stdio. Standard output contains protocol
/// messages only; diagnostics belong on stderr.
pub fn run_stdio(root: impl AsRef<Path>, exposure: ExposurePolicy) -> Result<(), String> {
    let server = Server::with_exposure_policy(root, exposure)?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| format!("could not read MCP request: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => server.handle(request),
            Err(_) => Some(rpc_error(Value::Null, -32700, "parse error")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut stdout, &response)
                .map_err(|error| format!("could not write MCP response: {error}"))?;
            stdout.write_all(b"\n").map_err(|error| error.to_string())?;
            stdout.flush().map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// Runs stdio MCP as a thin authenticated proxy to the explicitly enabled
/// browser owner. The proxy never receives a source path or raw event log.
pub fn run_delegated_stdio(port: u16, exposure: ExposurePolicy) -> Result<(), String> {
    run_project_delegated_stdio(port, exposure, None)
}

pub fn run_project_delegated_stdio(
    port: u16,
    exposure: ExposurePolicy,
    project_id: Option<&str>,
) -> Result<(), String> {
    exposure.validate()?;
    let token = project_id.map_or_else(
        || exposure.delegation_token(),
        |id| exposure.project_delegation_token(id),
    );
    delegated_stdio(|request| delegate_message(port, &token, request))
}

/// Reload the project session for every request, so an existing proxy can
/// reconnect after the application restarts without following a different project.
pub fn run_connection_stdio(path: &Path, project_id: &str) -> Result<(), String> {
    delegated_stdio(|request| {
        let result: Result<Option<Value>, String> = (|| {
            let meta = std::fs::symlink_metadata(path).map_err(|_| {
                "Open this project in glasstarn and enable agent access.".to_owned()
            })?;
            if !meta.is_file() || meta.len() > 64 * 1024 {
                return Err("invalid agent connection file".into());
            }
            let value: Value =
                serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if value["version"] != 1 || value["project_id"] != project_id {
                return Err("agent connection belongs to a different project".into());
            }
            if value["enabled"] != true {
                return Err(
                    "Agent access is disabled for this project. Enable it in glasstarn.".into(),
                );
            }
            let port = value["port"]
                .as_u64()
                .filter(|p| *p > 0 && *p <= u16::MAX as u64)
                .ok_or("invalid project connection port")? as u16;
            let token = value["token"]
                .as_str()
                .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or("invalid project connection credential")?;
            delegate_message(port, token, request).map_err(|error| format!("Project connection unavailable. Reopen this project in glasstarn, then retry. If access is disabled, enable it there. Detail: {error}"))
        })();
        match result {
            Ok(response) => Ok(response),
            Err(message) => Ok(request
                .get("id")
                .map(|id| rpc_error(id.clone(), -32000, &message))),
        }
    })
}

fn delegated_stdio(
    mut send: impl FnMut(&Value) -> Result<Option<Value>, String>,
) -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| format!("could not read MCP request: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<Value>(&line) {
            Ok(request) => request,
            Err(_) => {
                serde_json::to_writer(&mut stdout, &rpc_error(Value::Null, -32700, "parse error"))
                    .map_err(|error| format!("could not write MCP response: {error}"))?;
                stdout.write_all(b"\n").map_err(|error| error.to_string())?;
                stdout.flush().map_err(|error| error.to_string())?;
                continue;
            }
        };
        if let Some(response) = send(&request)? {
            serde_json::to_writer(&mut stdout, &response)
                .map_err(|error| format!("could not write MCP response: {error}"))?;
            stdout.write_all(b"\n").map_err(|error| error.to_string())?;
            stdout.flush().map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn delegate_message(port: u16, token: &str, request: &Value) -> Result<Option<Value>, String> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|error| {
        format!(
            "could not connect to the running application on 127.0.0.1:{port}: {error}; start it with --agent-access and the same exposure policy"
        )
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30 * 60)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    let body = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    write!(
        stream,
        "POST /api/mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(|error| format!("could not send delegated MCP request: {error}"))?;
    stream
        .write_all(&body)
        .map_err(|error| format!("could not send delegated MCP request: {error}"))?;
    stream
        .flush()
        .map_err(|error| format!("could not send delegated MCP request: {error}"))?;

    let mut response = Vec::new();
    stream
        .take(MAX_DELEGATED_RESPONSE_BYTES + 1)
        .read_to_end(&mut response)
        .map_err(|error| format!("could not read delegated MCP response: {error}"))?;
    if response.len() as u64 > MAX_DELEGATED_RESPONSE_BYTES {
        return Err(format!(
            "the running application returned more than the {MAX_DELEGATED_RESPONSE_BYTES}-byte delegation limit"
        ));
    }
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("the running application returned a malformed HTTP response")?;
    let head = std::str::from_utf8(&response[..split])
        .map_err(|_| "the running application returned a malformed HTTP header")?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or("the running application returned a malformed HTTP status")?;
    let body = &response[split + 4..];
    match status {
        200 => serde_json::from_slice(body)
            .map(Some)
            .map_err(|error| format!("the running application returned invalid MCP JSON: {error}")),
        204 => Ok(None),
        401 => Err(
            "the running application rejected live agent access; use the same reviewed exposure-policy file for serve and mcp"
                .to_owned(),
        ),
        _ => {
            let message = serde_json::from_slice::<Value>(body)
                .ok()
                .and_then(|value| value.get("error").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_else(|| format!("HTTP {status}"));
            Err(format!("the running application could not delegate MCP: {message}"))
        }
    }
}

fn server_info() -> Value {
    json!({ "name": "glasstarn", "version": env!("CARGO_PKG_VERSION") })
}

fn with_server_meta(mut result: Value) -> Value {
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "_meta".to_owned(),
            json!({ "io.modelcontextprotocol/serverInfo": server_info() }),
        );
    }
    result
}

/// Tools whose answers are short by construction or must stay whole.
fn budgeted(name: &str) -> bool {
    !name.starts_with("list_") && !matches!(name, "get_claim_contract" | "get_exposure_policy")
}

/// Holds a successful result to the agent budget and rewrites its text copy.
fn fit_tool_result(mut result: Value, name: &str) -> Value {
    if result["isError"] == true {
        return result;
    }
    let Some(output) = result.get_mut("structuredContent").map(Value::take) else {
        return result;
    };
    let fitted = pm_query::budget::fit(
        output,
        pm_query::budget::AGENT_BUDGET,
        &format!("call {name} again with the same arguments plus \"detail\": \"full\""),
    );
    tool_result(Ok(fitted))
}

/// Adds the `detail` choice to every budgeted tool's input schema.
fn with_detail_parameter(mut tool: Value) -> Value {
    let name = tool["name"].as_str().unwrap_or_default().to_owned();
    if budgeted(&name)
        && let Some(properties) = tool["inputSchema"]["properties"].as_object_mut()
    {
        properties.insert(
            "detail".to_owned(),
            json!({"enum":["summary","full"],"description":"full: no 4 KB list cut"}),
        );
    }
    tool
}

fn tool_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(output) => {
            // Compact: indentation would cost the agent tokens for nothing.
            let text = serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_owned());
            json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": output,
                "isError": false
            })
        }
        Err(message) => json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true
        }),
    }
}

fn instructions() -> &'static str {
    "Read-only process-mining evidence. Start with list_projects for case-centric work or list_ocel_logs for object-centric work. list_process_models and list_ocel_models supply explicit local baselines for their respective conformance tools. A model discovered from the analysis log describes that log and is not independent compliance evidence. Tool results are calculated facts or explicitly labelled statistical associations. Before writing explanations or recommendations, call get_claim_contract and label agent-authored additions as interpretations or improvement hypotheses."
}

fn claim_contract() -> Value {
    json!({
        "contract_version": 1,
        "purpose": "Keep deterministic evidence separate from explanations and proposed improvements in agent-authored output.",
        "engine_claim_types": {
            "calculated_fact": "A deterministic result produced by a named analysis tool. Preserve its metric, scope, timing semantics, denominators, and bounds.",
            "statistical_association": "A defined comparison that does not establish causation. Preserve its cohorts or periods, denominators, and interpretation limits."
        },
        "agent_claim_types": {
            "interpretation": {
                "meaning": "An explanation offered by the agent, not a finding produced by the engine.",
                "required_fields": ["claim_type", "statement", "based_on", "uncertainties"],
                "claim_type_value": "interpretation"
            },
            "improvement_hypothesis": {
                "meaning": "A proposed change whose value and feasibility require human investigation or testing.",
                "required_fields": ["claim_type", "statement", "based_on", "uncertainties", "validation_plan"],
                "claim_type_value": "improvement_hypothesis"
            }
        },
        "based_on_item": {
            "required_fields": ["tool", "claim_type", "evidence_refs"],
            "evidence_refs_note": "Use only identifiers or bounded evidence returned by MCP tools; use an empty array when the supporting result has no exposed identifier."
        },
        "field_shapes": {
            "statement": "Non-empty string that contains no unlabelled engine metric.",
            "based_on": "Non-empty array of based_on_item objects.",
            "uncertainties": "Array of material unknowns or missing evidence; use an empty array only when none remain.",
            "validation_plan": "Non-empty array of concrete checks, experiments, or human review steps."
        },
        "rules": [
            "Do not relabel an interpretation or hypothesis as a calculated fact or statistical association.",
            "Do not change, recompute, or silently combine engine metrics.",
            "Do not infer causation from sequence, timing, correlation, or cohort differences.",
            "Do not call an activity ready for automation; state missing evidence about inputs, decisions, exceptions, risk, controls, and implementation effort.",
            "Carry forward the active view, eligibility denominators, truncation, redaction, and interpretation limits that materially qualify the claim.",
            "A recommendation must be written as an improvement_hypothesis with a concrete validation_plan."
        ]
    })
}

fn discover() -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": [MODERN_PROTOCOL],
        "capabilities": { "tools": {} },
        "serverInfo": server_info(),
        "instructions": instructions()
    })
}

fn legacy_initialize(params: Option<&Value>) -> Value {
    let requested = params
        .and_then(|value| value.get("protocolVersion"))
        .and_then(Value::as_str);
    let protocol = requested
        .filter(|version| LEGACY_PROTOCOLS.contains(version))
        .unwrap_or(LEGACY_PROTOCOL);
    json!({
        "protocolVersion": protocol,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": server_info(),
        "instructions": instructions(),
    })
}

fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "list_projects",
            "title": "List process-mining projects",
            "description": "List saved projects in the configured local directory and report whether each source is unchanged. Read-only and inexpensive.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "list_process_models",
            "title": "List case-centric baseline models",
            "description": "List explicit *.model.json accepting Petri-net baselines using opaque identifiers. Directory mode lists regular local files; live mode lists only the baseline explicitly selected in the running browser. No model contents are read or returned.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "list_ocel_logs",
            "title": "List object-centric logs",
            "description": "List OCEL 2.0 JSON, XML, and SQLite files in the configured local directory using opaque identifiers. No event or object data is read or returned.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "list_ocel_models",
            "title": "List object-centric baseline models",
            "description": "List portable *.ocelmodel.json baseline files in the configured local directory using opaque identifiers. No model contents are read or returned.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "get_exposure_policy",
            "title": "Get AI exposure policy",
            "description": "Report which categories are shown, aliased, or omitted. Never returns original values or the secret alias key.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "get_claim_contract",
            "title": "Get claim-writing contract",
            "description": "Return the compact output contract for agent-authored interpretations and improvement hypotheses. Call once before writing explanations or recommendations; it reads no project or event data and is inexpensive.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "analyze_ocel",
            "title": "Analyze an object-centric event log",
            "description": "Validate and analyze one OCEL 2.0 JSON, XML, or SQLite log without flattening. Returns filtered scope, lifecycle flow and variants by object type, qualified participation, shared-event interactions, explicit object relationships, and bounded evidence. Every activity, event, object type, object, qualifier, timestamp, and source name follows the reviewed exposure policy. Use identifiers returned by this tool for repeatable filters and object inspection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Opaque identifier returned by list_ocel_logs" },
                    "object_types": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Shown or aliased object types returned by this tool" },
                    "event_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Shown or aliased event-to-object qualifiers returned by this tool" },
                    "object_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Shown or aliased object-to-object qualifiers returned by this tool" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 100, "description": "Maximum rows returned per graph family; complete totals are always included" }
                },
                "required": ["log"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "inspect_ocel_object",
            "title": "Inspect an object lifecycle",
            "description": "Inspect the bounded original lifecycle behind an object returned as object-centric evidence. Reapplies the same optional object-type and relationship filters, preserves simultaneous-event layers, and applies the reviewed exposure policy to every identifier and timestamp. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Opaque identifier returned by list_ocel_logs" },
                    "object": { "type": "string", "description": "Shown or aliased evidence object returned by analyze_ocel" },
                    "object_types": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Same shown or aliased object-type filter used for analyze_ocel" },
                    "event_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Same shown or aliased event-to-object qualifier filter used for analyze_ocel" },
                    "object_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Same shown or aliased object-to-object qualifier filter used for analyze_ocel" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 100, "description": "Maximum lifecycle events returned; the complete event count is always included" }
                },
                "required": ["log", "object"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "measure_ocel_time_between",
            "title": "Measure time between object-centric activities",
            "description": "Measure start-to-start elapsed time between two activities only through objects participating in both. Each target pairs with the most recent strictly earlier source layer on that object; simultaneous events are not ordered. Returns results by common object type with bounded evidence under the reviewed exposure policy. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Opaque identifier returned by list_ocel_logs" },
                    "from_activity": { "type": "string", "description": "Shown or aliased source activity returned by analyze_ocel" },
                    "to_activity": { "type": "string", "description": "Shown or aliased target activity returned by analyze_ocel" },
                    "object_types": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-type filter" },
                    "event_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased event-to-object qualifier filter" },
                    "object_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-to-object qualifier filter" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50, "description": "Maximum common-object-type measurements returned; complete total is always included" }
                },
                "required": ["log", "from_activity", "to_activity"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "analyze_ocel_synchronization",
            "title": "Analyze object-centric synchronization timing",
            "description": "Decompose observable timing at multi-object events into the spread between participating objects' strictly earlier lifecycle points and elapsed time after the last such point. Objects first seen at the event are disclosed separately, and insufficient-history events remain counted. Returns bounded evidence under the reviewed exposure policy. These are elapsed observations, not waiting, availability, causation, or execution duration. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Opaque identifier returned by list_ocel_logs" },
                    "object_types": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-type filter" },
                    "event_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased event-to-object qualifier filter" },
                    "object_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-to-object qualifier filter" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50, "description": "Maximum activity rows returned; complete activity total is always included" }
                },
                "required": ["log"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "rank_ocel_bottlenecks",
            "title": "Rank object-centric timing bottlenecks",
            "description": "Rank exact positive timing burden in the current object-centric view with bounded evidence. Returns lifecycle-transition total elapsed burden and multi-object prior-point spread burden as separate lists; never combines them into an opaque score. Zero-duration and eligibility denominators remain disclosed. These rankings prioritize observable timing, not waiting or causal explanations. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Opaque identifier returned by list_ocel_logs" },
                    "object_types": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-type filter" },
                    "event_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased event-to-object qualifier filter" },
                    "object_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-to-object qualifier filter" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10, "description": "Maximum rows returned per ranking; complete candidate and burden totals are always included" }
                },
                "required": ["log"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "follow_ocel_object_type",
            "title": "Follow one item type through a connected log",
            "description": "Treat each object of one type (for example each order) as a case made of every recorded step involving it, and summarise it with the same case engine and figures as a single log: case and step counts, routes, median, mean and P90 time from first to last step, and rework. Discloses how many steps were left out and how many shared steps were copied into several cases. Returns aggregates only; no identifiers, activity names or timestamps. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Opaque identifier returned by list_ocel_logs" },
                    "object_type": { "type": "string", "description": "Shown or aliased object type returned by analyze_ocel" },
                    "object_types": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-type filter applied first" },
                    "event_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased event-to-object qualifier filter applied first" },
                    "object_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-to-object qualifier filter applied first" }
                },
                "required": ["log", "object_type"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "project_ocel_cases",
            "title": "Project an OCEL view into explicit cases",
            "description": "Explicitly choose one object type as a traditional case notion and preview the resulting cases with complete convergence, divergence-risk, exclusion, and occurrence-inflation totals. Source identifiers and simultaneous timestamp layers remain traceable; case and event detail is independently bounded under the reviewed exposure policy. This is an auditable projection for case-centric algorithms, not a native object-centric result. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Opaque identifier returned by list_ocel_logs" },
                    "case_object_type": { "type": "string", "description": "Shown or aliased object type returned by analyze_ocel; each retained object of this type becomes one case" },
                    "object_types": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-type filter applied before projection" },
                    "event_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased event-to-object qualifier filter applied before projection" },
                    "object_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-to-object qualifier filter applied before projection" },
                    "case_limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 5, "description": "Maximum projected cases returned; complete totals are always included" },
                    "event_limit_per_case": { "type": "integer", "minimum": 1, "maximum": 200, "default": 100, "description": "Maximum source-event occurrences returned per projected case; availability and truncation are disclosed" }
                },
                "required": ["log", "case_object_type"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "discover_ocel_model",
            "title": "Discover a native object-centric model",
            "description": "Discover a typed object-centric Petri net from the current OCEL view. Returns the complete formal model with shared visible activities, silent routing transitions, typed initial/final places, variable-arc evidence, per-object-type mining statistics, and explicit semantics. Activity and object-type names follow the reviewed exposure policy; no event or object identifiers are returned. Scans the source and mines a model, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Opaque identifier returned by list_ocel_logs" },
                    "object_types": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-type filter" },
                    "event_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased event-to-object qualifier filter" },
                    "object_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-to-object qualifier filter" },
                    "noise_threshold_milli": { "type": "integer", "minimum": 0, "maximum": 1000, "default": 200, "description": "Local infrequent-edge filtering threshold in thousandths" },
                    "variable_arc_threshold_milli": { "type": "integer", "minimum": 0, "maximum": 1000, "default": 1000, "description": "Minimum share of exactly-one participation required for a fixed arc, in thousandths" },
                    "max_model_nodes": { "type": "integer", "minimum": 1, "maximum": 10000, "default": 2000, "description": "Safety bound for the combined number of places and transitions; oversized models are rejected intact rather than truncated" }
                },
                "required": ["log"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "check_ocel_conformance",
            "title": "Check object-centric conformance",
            "description": "Replay an OCEL log against an explicit portable *.ocelmodel.json baseline returned by list_ocel_models. Checks per-object lifecycle control flow without flattening and separately checks shared-event object participation and fixed-versus-variable multiplicity. Returns complete fitness, token, event, object, and binding totals with bounded worst-object and deviation evidence under the reviewed exposure policy. The baseline must be a raw portable model saved locally, not the privacy-transformed output of discover_ocel_model. This is diagnostic process conformance, not proof of regulatory compliance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Opaque identifier returned by list_ocel_logs" },
                    "model": { "type": "string", "description": "Opaque baseline identifier returned by list_ocel_models" },
                    "object_types": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-type filter" },
                    "event_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased event-to-object qualifier filter" },
                    "object_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-to-object qualifier filter" },
                    "detail_limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 100, "description": "Maximum worst-object rows and deviations returned independently; complete totals are always included" },
                    "max_model_nodes": { "type": "integer", "minimum": 1, "maximum": 10000, "default": 2000, "description": "Safety bound for the combined places and transitions; oversized baselines are rejected intact" }
                },
                "required": ["log", "model"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "align_ocel_conformance",
            "title": "Optimally align object-centric lifecycles",
            "description": "Use deterministic optimal A* alignment to compare every active object lifecycle with an explicit portable *.ocelmodel.json baseline, without flattening to one case notion. Reports complete lifecycle alignment cost and fitness, bounded per-object log-only and visible-model moves, and separate shared-event binding violations under the reviewed exposure policy. More expensive than token replay. This is diagnostic process conformance, not proof of regulatory compliance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Opaque identifier returned by list_ocel_logs" },
                    "model": { "type": "string", "description": "Opaque baseline identifier returned by list_ocel_models" },
                    "object_types": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-type filter" },
                    "event_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased event-to-object qualifier filter" },
                    "object_object_qualifiers": { "type": "array", "items": { "type": "string" }, "maxItems": 50, "description": "Optional shown or aliased object-to-object qualifier filter" },
                    "detail_limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 100, "description": "Maximum worst-object rows, deviations per object, and binding deviations returned; complete totals are always included" },
                    "max_model_nodes": { "type": "integer", "minimum": 1, "maximum": 10000, "default": 2000, "description": "Safety bound for the combined places and transitions; oversized baselines are rejected intact" }
                },
                "required": ["log", "model"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "discover_process_model",
            "title": "Discover a case-centric process model",
            "description": "Mine the exact saved active view with sound Inductive Miner–infrequent (default), classic Alpha Miner, or Heuristics Miner. Returns a complete privacy-transformed model within an explicit size bound. Heuristics is useful for noisy, spaghetti-like behavior but is not an executable conformance baseline. A model discovered from the analysis log describes that log and is not independent compliance evidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Opaque identifier returned by list_projects" },
                    "algorithm": { "type": "string", "enum": ["imf", "alpha", "heuristics"], "default": "imf" },
                    "noise_threshold_milli": { "type": "integer", "minimum": 0, "maximum": 1000, "default": 200, "description": "IMf-only local infrequent-edge threshold in thousandths" },
                    "dependency_threshold_millionths": { "type": "integer", "minimum": 0, "maximum": 1000000, "default": 500000, "description": "Heuristics-only minimum dependency score" },
                    "relative_to_best_millionths": { "type": "integer", "minimum": 0, "maximum": 1000000, "default": 50000, "description": "Heuristics-only relative-to-best tolerance" },
                    "and_threshold_millionths": { "type": "integer", "minimum": 0, "maximum": 1000000, "default": 650000, "description": "Heuristics-only AND-binding threshold" },
                    "minimum_observations": { "type": "integer", "minimum": 1, "default": 1, "description": "Heuristics-only minimum direct-follow observations" },
                    "max_model_items": { "type": "integer", "minimum": 1, "maximum": 10000, "default": 2000, "description": "Safety bound for places plus transitions, or all heuristic rows; oversized models are rejected intact" }
                },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "check_process_conformance",
            "title": "Check case-centric process conformance",
            "description": "Token-replay the exact saved active view against an explicit local *.model.json accepting Petri-net baseline returned by list_process_models. Returns complete aggregate fitness and token totals with bounded, privacy-controlled worst-case evidence. The baseline must be a raw local model, not privacy-transformed discovery output. This is diagnostic process conformance, not proof of regulatory compliance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Opaque identifier returned by list_projects" },
                    "model": { "type": "string", "description": "Opaque baseline identifier returned by list_process_models" },
                    "detail_limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 20, "description": "Maximum deviating case rows returned; complete totals are always included" },
                    "max_model_items": { "type": "integer", "minimum": 1, "maximum": 10000, "default": 2000, "description": "Safety bound for baseline places plus transitions; oversized models are rejected intact" }
                },
                "required": ["project", "model"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "align_process_conformance",
            "title": "Optimally align case-centric traces",
            "description": "Use deterministic optimal A* alignment to compare every trace in the exact saved active view with an explicit local *.model.json accepting Petri-net baseline. Returns complete alignment cost and fitness with bounded privacy-controlled log-only and visible-model deviations. More expensive than token replay. This is diagnostic process conformance, not proof of regulatory compliance.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Opaque identifier returned by list_projects" },
                    "model": { "type": "string", "description": "Opaque baseline identifier returned by list_process_models" },
                    "detail_limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 20, "description": "Maximum deviating case rows returned; complete totals are always included" },
                    "max_model_items": { "type": "integer", "minimum": 1, "maximum": 10000, "default": 2000, "description": "Safety bound for baseline places plus transitions; oversized models are rejected intact" }
                },
                "required": ["project", "model"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "inspect_log",
            "title": "Inspect a saved event log",
            "description": "Inspect a saved project's mapping and a bounded 200-row schema profile. Returns no event or sample values. Read-only and inexpensive.",
            "inputSchema": {
                "type": "object",
                "properties": { "project": { "type": "string", "description": "Identifier returned by list_projects" } },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "validate_mapping",
            "title": "Validate a saved event-log mapping",
            "description": "Validate the complete unchanged source with its saved mapping and interpretation policy. Returns readiness, finding kinds and counts, and stable decision option identifiers without event values, examples, paths, or decision prose. Column names follow the reviewed exposure policy. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": { "project": { "type": "string", "description": "Identifier returned by list_projects" } },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "run_analysis",
            "title": "Run a saved process analysis",
            "description": "Run the exact saved mapping, interpretation policy, preparation rules, and active view, then return a compact manifest of available result families and the follow-up tools that expose them. It does not duplicate detailed result payloads or return raw identifiers. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": { "project": { "type": "string", "description": "Identifier returned by list_projects" } },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "get_process_summary",
            "title": "Get process summary",
            "description": "Run the saved project analysis and return headline counts, variant concentration, throughput eligibility, active-view coverage, and preparation exclusions. Applies the reviewed exposure policy, returns no raw events, and scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": { "project": { "type": "string", "description": "Identifier returned by list_projects" } },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "get_variants",
            "title": "Get ranked process variants",
            "description": "Run the saved project analysis and return up to 20 ranked variants with coverage, timing eligibility, and bounded example cases. Activity and case identifiers follow the reviewed exposure policy. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10, "description": "Maximum ranked variants to return" }
                },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "get_slowest_cases",
            "title": "Get the slowest cases",
            "description": "Run the saved project analysis and return up to 50 cases with the longest first-to-last elapsed time, with event counts, route rank and whether a step repeats. Case identifiers follow the reviewed exposure policy. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10, "description": "Maximum cases to return" }
                },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "get_process_map",
            "title": "Get a readable process map",
            "description": "Return the process map for chosen routes (variant ranks, default the most common route; rank 1 is always included) or for the most frequent activities and connections. Reports shown and total activities, connections and routes, the cases whose whole path is drawn with their percentage, the activity ranking, connections marked slow by the engine rule, and up to 20 ranked routes. Activity and case identifiers follow the reviewed exposure policy. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "variants": { "type": "array", "items": { "type": "integer", "minimum": 1 }, "maxItems": 100, "description": "Route ranks to draw (routes mode)" },
                    "activities": { "type": "integer", "minimum": 0, "description": "Most frequent activities to keep (limits mode)" },
                    "connections": { "type": "integer", "minimum": 0, "description": "Most frequent connections to keep (limits mode)" }
                },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "find_bottlenecks",
            "title": "Find process bottlenecks",
            "description": "Run the saved project analysis and return up to 10 transitions ranked by total eligible duration, with denominators and bounded case evidence. Activity and case identifiers follow the reviewed exposure policy. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 10, "default": 10, "description": "Maximum ranked bottlenecks to return" }
                },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "find_drivers",
            "title": "Find what goes with slow items",
            "description": "Rank the item fields (customer, vendor, region, amount bands, who handled it) whose values are much more common among the slowest fifth of items, with counts, shares and medians. Patterns, not proven causes. Names and values follow the reviewed exposure policy. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 6, "default": 6, "description": "Maximum drivers to return" }
                },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "find_rework",
            "title": "Find rework and observed loops",
            "description": "Run the saved project analysis and return bounded repeated-activity and observed loop evidence. It does not infer loops from aggregate graph cycles. Activity and case identifiers follow the reviewed exposure policy. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "activity_limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10, "description": "Maximum repeated activities to return" },
                    "loop_limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10, "description": "Maximum observed loop paths to return" }
                },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "get_automation_targeting",
            "title": "Get automation-targeting evidence",
            "description": "Run the saved project analysis and return bounded evidence about human-touched work, classification coverage, routing regularity, observed path divergence, and timing. This is evidence for investigation, not an automation-readiness verdict. Identifiers follow the reviewed exposure policy. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10, "description": "Maximum manual steps and divergences to return" }
                },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "get_activity_context",
            "title": "Inspect activity context",
            "description": "Inspect one shown or aliased activity in the saved active view, including frequency, connected transitions, manual-work evidence, rework evidence, timing denominators, and bounded cases. It reports observations rather than an automation verdict. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "activity": { "type": "string", "description": "Activity identifier returned by an MCP tool" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10, "description": "Maximum connected transitions to return" }
                },
                "required": ["project", "activity"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name":"get_attribute_choices",
            "title":"List recorded attribute values",
            "description":"Return at most 100 most frequent recorded values of a case attribute in the saved active scope, including counts and missing-value coverage. For a field that changes between steps (a status), an item counts under every value it ever had. Names and values follow the exposure policy.",
            "inputSchema":{"type":"object","properties":{"project":{"type":"string"},"attribute":{"type":"string"}},"required":["project","attribute"],"additionalProperties":false},
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}
        }),
        json!({
            "name":"explore_linked_interval","title":"Measure intervals across linked object types",
            "description":"Measure first/last endpoints across selected object types through one direct recorded relationship or shared event per root, including split deliveries. Returns distributions, selected/remainder associations, root-attribute breakdowns and privacy-transformed evidence inside the saved native scope. Does not return raw root IDs or mutate a selection.",
            "inputSchema":{"type":"object","properties":{"log":{"type":"string"},"interval":{"type":"object","properties":{"root_type":{"type":"string"},"from_type":{"type":"string"},"to_type":{"type":"string"},"group_by":{"type":"string"},"segment":{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"},"from_occurrence":{"enum":["first","last"]},"to_occurrence":{"enum":["first","last"]},"min_micros":{"type":"integer","minimum":0},"max_micros":{"type":"integer","minimum":0}},"required":["from","to"],"additionalProperties":false}},"required":["root_type","from_type","to_type","segment"],"additionalProperties":false}},"required":["log","interval"],"additionalProperties":false},
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}
        }),
        json!({
            "name":"explore_process","title":"Analyze recorded intervals and grouped populations",
            "description":"Analyze the saved active case or followed-object scope. Keep whole-history timing by omitting segment, or choose first/last activity endpoints and optional inclusive microsecond bounds. Returns average/median/P90, distribution, up to 100 field groups, selected versus remainder associations and bounded privacy-transformed examples. Does not imply causation or arbitrary cross-object joins.",
            "inputSchema":{"type":"object","properties":{"project":{"type":"string"},"group_by":{"type":"string"},"min_micros":{"type":"integer","minimum":0},"max_micros":{"type":"integer","minimum":0},"segment":{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"},"from_occurrence":{"enum":["first","last"]},"to_occurrence":{"enum":["first","last"]},"min_micros":{"type":"integer","minimum":0},"max_micros":{"type":"integer","minimum":0}},"required":["from","to"],"additionalProperties":false}},"required":["project"],"additionalProperties":false},
            "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}
        }),
        json!({
            "name": "compare_cohorts",
            "title": "Compare two process cohorts",
            "description": "Compare two shown or aliased values of a mapped case attribute inside the saved active view. Returns cohort denominators, throughput differences, and up to 20 deterministically ranked activity-prevalence associations with bounded evidence. It does not claim causation. For a field that changes between steps, an item that had both values is in both groups; shared_items says how many. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "attribute": { "type": "string", "description": "Shown or aliased case-attribute name" },
                    "left": { "type": "string", "description": "Shown or aliased baseline cohort value" },
                    "right": { "type": "string", "description": "Shown or aliased comparison cohort value" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10 }
                },
                "required": ["project", "attribute", "left", "right"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "summarize_values",
            "title": "Total an amount field",
            "description": "Total, average, median, 90th percentile, minimum and maximum of a number field such as an invoice amount, over the saved active view, optionally limited to at_least/at_most on that field and grouped by route (variant), last step (last_step: where value waits; open amounts appear when finishing steps are confirmed) or another attribute. Cases without a number are counted, never treated as zero. Call without 'field' to list the fields that hold numbers; when none do, say so and use counts and times instead. Needs the exposure policy to show costs. Scans the source.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "field": { "type": "string", "description": "Shown or aliased case-attribute name holding numbers" },
                    "at_least": { "type": "number", "description": "Keep cases whose field is at least this (inclusive)" },
                    "at_most": { "type": "number", "description": "Keep cases whose field is at most this (inclusive)" },
                    "group_by": { "type": "string", "enum": ["none", "variant", "last_step", "attribute"], "default": "none" },
                    "group_attribute": { "type": "string", "description": "Shown or aliased attribute name when group_by is attribute" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10 }
                },
                "required": ["project"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "compare_periods",
            "title": "Compare two process periods",
            "description": "Compare two explicit, non-overlapping case-start windows in the saved active view. Boundaries are inclusive Unix microseconds. Returns denominators, throughput change, and up to 20 deterministically ranked activity-prevalence associations with bounded evidence. It does not claim causation. Scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "baseline_start": { "type": "integer", "description": "Inclusive baseline case-start boundary in Unix microseconds" },
                    "baseline_end": { "type": "integer", "description": "Inclusive baseline case-start boundary in Unix microseconds" },
                    "comparison_start": { "type": "integer", "description": "Inclusive comparison case-start boundary in Unix microseconds" },
                    "comparison_end": { "type": "integer", "description": "Inclusive comparison case-start boundary in Unix microseconds" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10 }
                },
                "required": ["project", "baseline_start", "baseline_end", "comparison_start", "comparison_end"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "get_evidence",
            "title": "Get cases supporting a finding",
            "description": "Find cases in the saved active view that support an activity, transition, exact variant, or resource handover. Accepts the shown or aliased identifiers returned by other tools, returns at most 100 case identifiers, and scans the source, so it is moderately expensive.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "kind": { "type": "string", "enum": ["activity", "transition", "variant", "handover"] },
                    "activity": { "type": "string", "description": "Activity identifier for activity evidence" },
                    "from": { "type": "string", "description": "Starting activity or resource identifier" },
                    "to": { "type": "string", "description": "Ending activity or resource identifier" },
                    "sequence": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "Exact activity sequence for variant evidence" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 }
                },
                "required": ["project", "kind"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
        json!({
            "name": "inspect_case",
            "title": "Inspect one evidence case",
            "description": "Open one case in the saved active view using a shown or aliased identifier returned by another MCP tool. Returns its ordered timeline under the exposure policy. Scans the source and may expose detailed values when the reviewed policy permits them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": { "type": "string", "description": "Identifier returned by list_projects" },
                    "case": { "type": "string", "description": "Case identifier returned by an MCP evidence result" }
                },
                "required": ["project", "case"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }),
    ]
}

/// Connections and ranked activities returned by `get_process_map`.
const MAP_CONNECTIONS_RETURNED: usize = 100;

/// Reads either route ranks or activity and connection limits.
fn map_selection(arguments: &Value) -> Result<pm_query::MapSelection, String> {
    let count = |name: &str| -> Result<Option<usize>, String> {
        match arguments.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .map(Some)
                .ok_or_else(|| format!("get_process_map {name} must be a non-negative integer")),
        }
    };
    let (activities, connections) = (count("activities")?, count("connections")?);
    if activities.is_some() || connections.is_some() {
        if arguments.get("variants").is_some() {
            return Err(
                "get_process_map takes either variants or activities and connections, not both"
                    .into(),
            );
        }
        return Ok(pm_query::MapSelection::Limits {
            activities: activities.unwrap_or(0),
            connections: connections.unwrap_or(0),
        });
    }
    let variants = match arguments.get("variants") {
        None | Some(Value::Null) => vec![1],
        Some(Value::Array(ranks)) if ranks.len() <= 100 => ranks
            .iter()
            .map(|rank| {
                rank.as_u64()
                    .and_then(|rank| usize::try_from(rank).ok())
                    .ok_or_else(|| "get_process_map variants must be route ranks".to_owned())
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(
                "get_process_map variants must be an array of at most 100 route ranks".into(),
            );
        }
    };
    Ok(pm_query::MapSelection::Routes { variants })
}

fn required_project<'a>(arguments: &'a Value, tool: &str) -> Result<&'a str, String> {
    arguments
        .get("project")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{tool} requires the project identifier returned by list_projects"))
}

fn required_string<'a>(arguments: &'a Value, name: &str, tool: &str) -> Result<&'a str, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{tool} requires '{name}'"))
}

fn required_i64(arguments: &Value, name: &str, tool: &str) -> Result<i64, String> {
    arguments
        .get(name)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{tool} requires integer Unix microseconds in '{name}'"))
}

fn bounded_limit(arguments: &Value, default: usize, maximum: usize) -> Result<usize, String> {
    bounded_named_limit(arguments, "limit", default, maximum)
}

fn bounded_named_limit(
    arguments: &Value,
    name: &str,
    default: usize,
    maximum: usize,
) -> Result<usize, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(default);
    };
    let value = value
        .as_u64()
        .ok_or_else(|| format!("{name} must be a positive integer"))?;
    let limit = usize::try_from(value).map_err(|_| format!("{name} is too large"))?;
    if !(1..=maximum).contains(&limit) {
        return Err(format!("{name} must be between 1 and {maximum}"));
    }
    Ok(limit)
}

fn threshold_milli(arguments: &Value, name: &str, default: u16) -> Result<u16, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(default);
    };
    let value = value
        .as_u64()
        .ok_or_else(|| format!("{name} must be an integer between 0 and 1000"))?;
    if value > 1_000 {
        return Err(format!("{name} must be between 0 and 1000"));
    }
    u16::try_from(value).map_err(|_| format!("{name} must be between 0 and 1000"))
}

fn threshold_millionths(arguments: &Value, name: &str, default: u64) -> Result<u64, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(default);
    };
    let value = value
        .as_u64()
        .ok_or_else(|| format!("{name} must be an integer between 0 and 1000000"))?;
    if value > 1_000_000 {
        return Err(format!("{name} must be between 0 and 1000000"));
    }
    Ok(value)
}

fn positive_u64(arguments: &Value, name: &str, default: u64) -> Result<u64, String> {
    let Some(value) = arguments.get(name) else {
        return Ok(default);
    };
    let value = value
        .as_u64()
        .ok_or_else(|| format!("{name} must be a positive integer"))?;
    if value == 0 {
        return Err(format!("{name} must be at least 1"));
    }
    Ok(value)
}

fn reject_arguments(
    arguments: &Value,
    names: &[&str],
    intended_algorithm: &str,
) -> Result<(), String> {
    if let Some(name) = names.iter().find(|name| arguments.get(**name).is_some()) {
        return Err(format!(
            "{name} applies only to the {intended_algorithm} algorithm"
        ));
    }
    Ok(())
}

fn ensure_net_bound(
    net: &pm_query::LabelledPetriNet,
    maximum: usize,
    description: &str,
) -> Result<(), String> {
    let items = net.place_count.saturating_add(net.transitions.len());
    if items > maximum {
        return Err(format!(
            "the {description} has {items} places and transitions, above max_model_items={maximum}; narrow the saved active view, choose a smaller baseline, or explicitly raise the limit"
        ));
    }
    Ok(())
}

fn exposed_selection(
    arguments: &Value,
    field: &str,
    policy: &ExposurePolicy,
    mode: Mode,
    category: &str,
    candidates: &[String],
) -> Result<Vec<String>, String> {
    let Some(value) = arguments.get(field) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("{field} must be an array of identifiers"))?;
    if values.len() > 50 {
        return Err(format!("{field} accepts at most 50 identifiers"));
    }
    values
        .iter()
        .map(|value| {
            let exposed = value
                .as_str()
                .ok_or_else(|| format!("{field} must contain only strings"))?;
            resolve_exposed(policy, mode, category, exposed, candidates)
        })
        .collect()
}

fn scoped_exposed_selection(
    arguments: &Value,
    field: &str,
    policy: &ExposurePolicy,
    mode: Mode,
    category: &str,
    candidates: &[String],
    active: &[String],
) -> Result<Vec<String>, String> {
    let Some(value) = arguments.get(field) else {
        return Ok(active.to_vec());
    };
    if value.as_array().is_some_and(Vec::is_empty) {
        return Ok(active.to_vec());
    }
    let selected = exposed_selection(arguments, field, policy, mode, category, candidates)?;
    if !active.is_empty()
        && let Some(outside) = selected.iter().find(|item| !active.contains(item))
    {
        let exposed = policy
            .transform(mode, category, outside)
            .unwrap_or_else(|| "<omitted>".to_owned());
        return Err(format!(
            "{field} contains {exposed}, which is outside the running application's active object-centric view"
        ));
    }
    Ok(selected)
}

fn expose_process_tree(tree: &pm_query::LabelledProcessTree, policy: &ExposurePolicy) -> Value {
    match tree {
        pm_query::LabelledProcessTree::Activity { activity } => json!({
            "kind": "activity",
            "activity": policy.transform(policy.activity_names, "activity", activity)
        }),
        pm_query::LabelledProcessTree::Tau => json!({ "kind": "tau" }),
        pm_query::LabelledProcessTree::Sequence { children } => json!({
            "kind": "sequence",
            "children": children.iter().map(|child| expose_process_tree(child, policy)).collect::<Vec<_>>()
        }),
        pm_query::LabelledProcessTree::Exclusive { children } => json!({
            "kind": "exclusive",
            "children": children.iter().map(|child| expose_process_tree(child, policy)).collect::<Vec<_>>()
        }),
        pm_query::LabelledProcessTree::Parallel { children } => json!({
            "kind": "parallel",
            "children": children.iter().map(|child| expose_process_tree(child, policy)).collect::<Vec<_>>()
        }),
        pm_query::LabelledProcessTree::Loop { body, redo } => json!({
            "kind": "loop",
            "body": expose_process_tree(body, policy),
            "redo": expose_process_tree(redo, policy)
        }),
    }
}

fn expose_process_net(net: &pm_query::LabelledPetriNet, policy: &ExposurePolicy) -> Value {
    json!({
        "place_count": net.place_count,
        "transitions": net.transitions.iter().map(|transition| json!({
            "id": transition.id,
            "activity": transition.activity.as_deref().and_then(|activity| {
                policy.transform(policy.activity_names, "activity", activity)
            }),
            "inputs": transition.inputs,
            "outputs": transition.outputs
        })).collect::<Vec<_>>(),
        "initial_marking": net.initial_marking,
        "final_marking": net.final_marking
    })
}

fn expose_formal_discovery(
    project_handle: &str,
    project_name: Option<String>,
    report: &pm_query::FormalDiscovery,
    policy: &ExposurePolicy,
) -> Value {
    json!({
        "project": project_handle,
        "project_name": project_name,
        "claim_type": "calculated_fact",
        "algorithm": report.algorithm,
        "view": report.view,
        "noise_threshold_milli": report.noise_threshold_milli,
        "filtered_edges": report.filtered_edges,
        "used_flower_fallback": report.used_flower_fallback,
        "model_kind": "sound_accepting_petri_net",
        "tree": expose_process_tree(&report.tree, policy),
        "net": expose_process_net(&report.net, policy),
        "baseline_guidance": "This model describes the selected log view. Do not use it as independent conformance or compliance evidence for the same data; select a separately approved *.model.json baseline through list_process_models.",
        "interpretation_note": "The discovered structure is a calculated model of observed behavior under the selected algorithm and threshold. Business meaning, causation, compliance conclusions, and improvement recommendations remain interpretations or hypotheses.",
        "privacy_note": "Activity and project names follow the reviewed exposure policy. Place and transition numbers are model-local structural identifiers. The raw event log, case identifiers, timestamps, attributes, source rows, source name, and filesystem path are not returned."
    })
}

fn expose_alpha_discovery(
    project_handle: &str,
    project_name: Option<String>,
    report: &pm_query::NetDiscovery,
    policy: &ExposurePolicy,
) -> Value {
    json!({
        "project": project_handle,
        "project_name": project_name,
        "claim_type": "calculated_fact",
        "algorithm": report.algorithm,
        "view": report.view,
        "model_kind": "accepting_petri_net_without_soundness_guarantee",
        "net": expose_process_net(&report.net, policy),
        "algorithm_warning": "Classic Alpha Miner is provided for teaching and compatibility. It is sensitive to noise, short loops, and non-free-choice behavior and does not guarantee a sound model.",
        "baseline_guidance": "This model describes the selected log view. Do not use it as independent conformance or compliance evidence for the same data; select a separately approved *.model.json baseline through list_process_models.",
        "interpretation_note": "The discovered structure is a calculated result, not proof that the represented process is complete, intended, causal, or compliant.",
        "privacy_note": "Activity and project names follow the reviewed exposure policy. Model-local structural identifiers and aggregate view counts remain available; raw cases, events, timestamps, attributes, source rows, source name, and filesystem path are not returned."
    })
}

fn expose_heuristics_discovery(
    project_handle: &str,
    project_name: Option<String>,
    report: &pm_query::HeuristicsDiscovery,
    policy: &ExposurePolicy,
) -> Value {
    let activity = |value: &str| policy.transform(policy.activity_names, "activity", value);
    json!({
        "project": project_handle,
        "project_name": project_name,
        "claim_type": "calculated_fact",
        "algorithm": report.algorithm,
        "view": report.view,
        "model_kind": "heuristic_dependency_net",
        "config": report.config,
        "starts": report.starts.iter().map(|(name, count)| json!({
            "activity": activity(name), "count": count
        })).collect::<Vec<_>>(),
        "ends": report.ends.iter().map(|(name, count)| json!({
            "activity": activity(name), "count": count
        })).collect::<Vec<_>>(),
        "arcs": report.arcs.iter().map(|arc| json!({
            "from": activity(&arc.from),
            "to": activity(&arc.to),
            "observations": arc.observations,
            "dependency_millionths": arc.dependency_millionths,
            "kind": arc.kind
        })).collect::<Vec<_>>(),
        "and_splits": report.and_splits.iter().map(|binding| json!({
            "anchor": activity(&binding.anchor),
            "activities": binding.activities.iter().filter_map(|name| activity(name)).collect::<Vec<_>>(),
            "score_millionths": binding.score_millionths
        })).collect::<Vec<_>>(),
        "and_joins": report.and_joins.iter().map(|binding| json!({
            "anchor": activity(&binding.anchor),
            "activities": binding.activities.iter().filter_map(|name| activity(name)).collect::<Vec<_>>(),
            "score_millionths": binding.score_millionths
        })).collect::<Vec<_>>(),
        "baseline_guidance": "A heuristic dependency net is not an executable accepting Petri net and cannot be used by conformance tools. It is a readable abstraction for noisy or spaghetti-like behavior.",
        "interpretation_note": "Frequencies, dependency scores, and bindings are calculated observations. They do not establish causation, model soundness, compliance, or automation readiness.",
        "privacy_note": "Activity and project names follow the reviewed exposure policy. Raw cases, events, timestamps, attributes, source rows, source name, and filesystem path are not returned."
    })
}

fn exposed_model_source(source: &str, policy: &ExposurePolicy) -> Option<String> {
    let name = Path::new(source)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    policy.transform(policy.source_names, "source", &name)
}

fn expose_process_conformance(
    project_handle: &str,
    model_handle: &str,
    project_name: Option<String>,
    model_source: &str,
    report: &pm_query::ConformanceReport,
    policy: &ExposurePolicy,
) -> Value {
    let cases = report.cases.iter().map(|case| {
        let deviations = case.deviations.iter().map(|deviation| match deviation {
            pm_query::DeviationEvidence::ActivityNotInModel { event_index, source_row, activity } => json!({
                "kind": "activity_not_in_model",
                "event_index": event_index,
                "source_row": policy.transform(policy.source_rows, "source_row", &source_row.to_string()),
                "activity": policy.transform(policy.activity_names, "activity", activity)
            }),
            pm_query::DeviationEvidence::MissingTokens { event_index, source_row, activity, count } => json!({
                "kind": "missing_tokens",
                "event_index": event_index,
                "source_row": policy.transform(policy.source_rows, "source_row", &source_row.to_string()),
                "activity": policy.transform(policy.activity_names, "activity", activity),
                "count": count
            }),
            pm_query::DeviationEvidence::FinalTokensMissing { count } => json!({
                "kind": "final_tokens_missing", "count": count
            }),
            pm_query::DeviationEvidence::TokensRemaining { count } => json!({
                "kind": "tokens_remaining", "count": count
            }),
        }).collect::<Vec<_>>();
        json!({
            "case": policy.transform(policy.case_ids, "case", &case.case),
            "fitness_millionths": case.fitness_millionths,
            "missing_tokens": case.missing_tokens,
            "remaining_tokens": case.remaining_tokens,
            "unmapped_events": case.unmapped_events,
            "deviations": deviations
        })
    }).collect::<Vec<_>>();
    json!({
        "project": project_handle,
        "project_name": project_name,
        "model": model_handle,
        "model_training_source": exposed_model_source(model_source, policy),
        "claim_type": "calculated_fact",
        "algorithm": report.algorithm,
        "view": report.view,
        "fitness_millionths": report.fitness_millionths,
        "fitness_percent": report.fitness_percent,
        "fit_cases": report.fit_cases,
        "deviating_cases": report.deviating_cases,
        "fit_case_percent": report.fit_case_percent,
        "tokens": {
            "missing": report.missing_tokens,
            "remaining": report.remaining_tokens,
            "unmapped_events": report.unmapped_events
        },
        "cases": {
            "available": report.deviating_cases,
            "returned": report.cases.len(),
            "truncated": report.cases.len() < report.deviating_cases,
            "detail_limit": report.deviation_limit,
            "items": cases
        },
        "semantics": "Token-based replay of the exact saved active view against one explicitly selected local accepting Petri-net baseline. Aggregate metrics cover the complete filtered view; case details are bounded and deterministically ranked.",
        "interpretation_note": "Fitness and replay deviations are diagnostic calculated facts against the selected baseline. They are not proof of regulatory compliance. Materiality, root cause, control effectiveness, and remediation remain interpretations or hypotheses.",
        "privacy_note": "Project, source, activity, case, and source-row identifiers follow the reviewed exposure policy. Opaque project/model handles and aggregate metrics remain available. No raw event log, attributes, timestamps, model path, or source path are returned."
    })
}

fn expose_process_alignment(
    project_handle: &str,
    model_handle: &str,
    project_name: Option<String>,
    model_source: &str,
    report: &pm_query::AlignmentReport,
    policy: &ExposurePolicy,
) -> Value {
    let cases = report.cases.iter().map(|case| {
        let deviations = case.deviations.iter().map(|deviation| match deviation {
            pm_query::AlignmentDeviation::LogOnly { event_index, source_row, activity } => json!({
                "kind": "log_only",
                "event_index": event_index,
                "source_row": policy.transform(policy.source_rows, "source_row", &source_row.to_string()),
                "activity": policy.transform(policy.activity_names, "activity", activity)
            }),
            pm_query::AlignmentDeviation::VisibleModel { transition, activity } => json!({
                "kind": "visible_model",
                "transition": transition,
                "activity": policy.transform(policy.activity_names, "activity", activity)
            }),
        }).collect::<Vec<_>>();
        json!({
            "case": policy.transform(policy.case_ids, "case", &case.case),
            "cost": case.cost,
            "fitness_millionths": case.fitness_millionths,
            "synchronous_moves": case.synchronous_moves,
            "invisible_moves": case.invisible_moves,
            "deviations": deviations
        })
    }).collect::<Vec<_>>();
    json!({
        "project": project_handle,
        "project_name": project_name,
        "model": model_handle,
        "model_training_source": exposed_model_source(model_source, policy),
        "claim_type": "calculated_fact",
        "algorithm": report.algorithm,
        "view": report.view,
        "total_cost": report.total_cost,
        "fitness_millionths": report.fitness_millionths,
        "fitness_percent": report.fitness_percent,
        "fit_cases": report.fit_cases,
        "deviating_cases": report.deviating_cases,
        "fit_case_percent": report.fit_case_percent,
        "cases": {
            "available": report.deviating_cases,
            "returned": report.cases.len(),
            "truncated": report.cases.len() < report.deviating_cases,
            "detail_limit": report.deviation_limit,
            "items": cases
        },
        "semantics": "Optimal A* trace alignment of the exact saved active view against one explicitly selected local accepting Petri-net baseline. Aggregate metrics cover the complete filtered view; case details are bounded and deterministically ranked.",
        "interpretation_note": "Alignment cost, fitness, and deviations are diagnostic calculated facts against the selected baseline. They are not proof of regulatory compliance. Materiality, root cause, control effectiveness, and remediation remain interpretations or hypotheses.",
        "privacy_note": "Project, source, activity, case, and source-row identifiers follow the reviewed exposure policy. Opaque handles, model-local transition numbers, and aggregate metrics remain available. No raw event log, attributes, timestamps, model path, or source path are returned."
    })
}

fn expose_ocel_analysis(
    handle: &str,
    source_file: &str,
    analysis: &pm_query::OcelAnalysis,
    policy: &ExposurePolicy,
    limit: usize,
) -> Value {
    let activity = |value: &str| policy.transform(policy.activity_names, "activity", value);
    let event = |value: &str| policy.transform(policy.event_ids, "event", value);
    let object_type =
        |value: &str| policy.transform(policy.object_type_names, "object_type", value);
    let object = |value: &str| policy.transform(policy.object_ids, "object", value);
    let qualifier = |value: &str| {
        policy.transform(
            policy.relationship_qualifiers,
            "relationship_qualifier",
            value,
        )
    };
    let timestamp =
        |value: i64| policy.transform(policy.timestamps, "timestamp", &value.to_string());
    let catalog = json!({
        "event_types": analysis.catalog.event_types.iter().filter_map(|value| activity(value)).collect::<Vec<_>>(),
        "object_types": analysis.catalog.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
        "event_object_qualifiers": analysis.catalog.event_object_qualifiers.iter().filter_map(|value| qualifier(value)).collect::<Vec<_>>(),
        "object_object_qualifiers": analysis.catalog.object_object_qualifiers.iter().filter_map(|value| qualifier(value)).collect::<Vec<_>>()
    });
    let findings = analysis
        .quality
        .findings
        .iter()
        .map(|finding| {
            json!({
                "kind": finding.kind,
                "title": finding.title,
                "severity": finding.severity,
                "detail": finding.detail,
                "affected": finding.affected,
                "examples_exposed": false
            })
        })
        .collect::<Vec<_>>();
    let graphs = analysis.graphs.as_ref().map(|graphs| {
        let activities = graphs
            .activities
            .iter()
            .take(limit)
            .map(|node| {
                json!({
                    "activity": activity(&node.activity),
                    "events": node.events,
                    "evidence_events": node.evidence_events.iter().filter_map(|value| event(value)).collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let directly_follows = graphs
            .directly_follows
            .iter()
            .take(limit)
            .map(|edge| {
                json!({
                    "object_type": object_type(&edge.object_type),
                    "source": activity(&edge.source),
                    "target": activity(&edge.target),
                    "occurrences": edge.occurrences,
                    "objects": edge.objects,
                    "total_elapsed_micros": edge.total_elapsed_micros,
                    "mean_elapsed_micros": edge.mean_elapsed_micros,
                    "min_elapsed_micros": edge.min_elapsed_micros,
                    "max_elapsed_micros": edge.max_elapsed_micros,
                    "evidence": edge.evidence.iter().map(|evidence| json!({
                        "object": object(&evidence.object),
                        "source_event": event(&evidence.source_event),
                        "target_event": event(&evidence.target_event),
                        "source_at_micros": timestamp(evidence.source_at_micros),
                        "target_at_micros": timestamp(evidence.target_at_micros)
                    })).collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let participation = graphs
            .participation
            .iter()
            .take(limit)
            .map(|edge| {
                json!({
                    "activity": activity(&edge.activity),
                    "object_type": object_type(&edge.object_type),
                    "qualifier": qualifier(&edge.qualifier),
                    "occurrences": edge.occurrences,
                    "evidence_events": edge.evidence_events.iter().filter_map(|value| event(value)).collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let interactions = graphs
            .interactions
            .iter()
            .take(limit)
            .map(|edge| {
                json!({
                    "left_object_type": object_type(&edge.left_object_type),
                    "right_object_type": object_type(&edge.right_object_type),
                    "events": edge.events,
                    "object_pairs": edge.object_pairs,
                    "evidence_events": edge.evidence_events.iter().filter_map(|value| event(value)).collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let relationships = graphs
            .relationships
            .iter()
            .take(limit)
            .map(|edge| {
                json!({
                    "source_object_type": object_type(&edge.source_object_type),
                    "target_object_type": object_type(&edge.target_object_type),
                    "qualifier": qualifier(&edge.qualifier),
                    "occurrences": edge.occurrences,
                    "evidence": edge.evidence.iter().map(|evidence| json!({
                        "source_object": object(&evidence.source_object),
                        "target_object": object(&evidence.target_object)
                    })).collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        json!({
            "activities": { "returned": activities.len(), "total": graphs.activities.len(), "items": activities },
            "directly_follows": { "returned": directly_follows.len(), "total": graphs.directly_follows.len(), "items": directly_follows },
            "participation": { "returned": participation.len(), "total": graphs.participation.len(), "items": participation },
            "interactions": { "returned": interactions.len(), "total": graphs.interactions.len(), "items": interactions },
            "relationships": { "returned": relationships.len(), "total": graphs.relationships.len(), "items": relationships }
        })
    });
    let variants = analysis.variants.as_ref().map(|variants| {
        let items = variants
            .iter()
            .take(limit)
            .map(|variant| {
                json!({
                    "object_type": object_type(&variant.object_type),
                    "rank": variant.rank,
                    "layers": variant.layers.iter().map(|layer| json!({
                        "activities": layer.activities.iter().map(|value| activity(value)).collect::<Vec<_>>()
                    })).collect::<Vec<_>>(),
                    "objects": variant.objects,
                    "share_millionths": variant.share_millionths,
                    "duration_eligible_objects": variant.duration_eligible_objects,
                    "mean_duration_micros": variant.mean_duration_micros,
                    "evidence_objects": variant.evidence_objects.iter().filter_map(|value| object(value)).collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        json!({
            "returned": items.len(),
            "total": variants.len(),
            "items": items,
            "semantics": "Ranks and shares are independent within each object type. Activities in the same layer are simultaneous; array order within a layer is canonical, not temporal."
        })
    });
    json!({
        "log": handle,
        "source_file": policy.transform(policy.source_names, "source", source_file),
        "claim_type": "calculated_fact",
        "ready_for_analysis": analysis.ready_for_analysis,
        "filter": {
            "cohort": analysis.filter.cohort.as_ref().map(|cohort| json!({"root_count":cohort.roots.len(),"correlation_paths":cohort.paths.len(),"pinned":true})),
            "object_types": analysis.filter.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
            "event_object_qualifiers": analysis.filter.event_object_qualifiers.iter().filter_map(|value| qualifier(value)).collect::<Vec<_>>(),
            "object_object_qualifiers": analysis.filter.object_object_qualifiers.iter().filter_map(|value| qualifier(value)).collect::<Vec<_>>()
        },
        "scope": analysis.scope,
        "catalog": catalog,
        "quality": {
            "summary": analysis.quality.summary,
            "findings": findings,
            "privacy_note": "Finding examples are omitted because they may combine several source identifiers."
        },
        "graphs": graphs,
        "variants": variants,
        "limit_per_result_family": limit,
        "interpretation_note": "Graph and variant counts are calculated facts. Explanations, causal claims, and automation recommendations remain interpretations or hypotheses.",
        "privacy_note": "Every source name, activity, event, object type, object, relationship qualifier, and timestamp follows the reviewed exposure policy."
    })
}

fn expose_ocel_lifecycle(
    handle: &str,
    lifecycle: &pm_query::LabelledOcelLifecycle,
    policy: &ExposurePolicy,
    limit: usize,
) -> Value {
    let events = lifecycle
        .events
        .iter()
        .take(limit)
        .map(|event| {
            json!({
                "event": policy.transform(policy.event_ids, "event", &event.event),
                "activity": policy.transform(policy.activity_names, "activity", &event.activity),
                "at_micros": policy.transform(policy.timestamps, "timestamp", &event.at_micros.to_string()),
                "time_layer": event.time_layer,
                "qualifiers": event.qualifiers.iter().filter_map(|value| policy.transform(
                    policy.relationship_qualifiers,
                    "relationship_qualifier",
                    value,
                )).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    json!({
        "log": handle,
        "claim_type": "calculated_fact",
        "object": policy.transform(policy.object_ids, "object", &lifecycle.object),
        "object_type": policy.transform(policy.object_type_names, "object_type", &lifecycle.object_type),
        "duration_micros": lifecycle.duration_micros,
        "events": { "returned": events.len(), "total": lifecycle.events.len(), "items": events },
        "semantics": "Events with the same time_layer are simultaneous; their display order is canonical, not temporal.",
        "privacy_note": "The object, object type, activities, event identifiers, qualifiers, and timestamps follow the reviewed exposure policy. The lifecycle is bounded and remains within the requested filtered view."
    })
}

fn expose_ocel_time_between(
    handle: &str,
    report: &pm_query::OcelTimeBetweenReport,
    policy: &ExposurePolicy,
    limit: usize,
) -> Value {
    let activity = |value: &str| policy.transform(policy.activity_names, "activity", value);
    let object_type =
        |value: &str| policy.transform(policy.object_type_names, "object_type", value);
    let object = |value: &str| policy.transform(policy.object_ids, "object", value);
    let event = |value: &str| policy.transform(policy.event_ids, "event", value);
    let timestamp =
        |value: i64| policy.transform(policy.timestamps, "timestamp", &value.to_string());
    let measurements = report
        .measurements
        .iter()
        .take(limit)
        .map(|measurement| {
            json!({
                "object_type": object_type(&measurement.object_type),
                "observations": measurement.observations,
                "objects": measurement.objects,
                "mean_elapsed_micros": measurement.mean_elapsed_micros,
                "min_elapsed_micros": measurement.min_elapsed_micros,
                "max_elapsed_micros": measurement.max_elapsed_micros,
                "evidence": measurement.evidence.iter().map(|evidence| json!({
                    "object": object(&evidence.object),
                    "source_event": event(&evidence.source_event),
                    "target_event": event(&evidence.target_event),
                    "source_at_micros": timestamp(evidence.source_at_micros),
                    "target_at_micros": timestamp(evidence.target_at_micros),
                    "elapsed_micros": evidence.elapsed_micros
                })).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    json!({
        "log": handle,
        "claim_type": "calculated_fact",
        "filter": {
            "cohort": report.filter.cohort.as_ref().map(|cohort| json!({"root_count":cohort.roots.len(),"correlation_paths":cohort.paths.len(),"pinned":true})),
            "object_types": report.filter.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
            "event_object_qualifiers": report.filter.event_object_qualifiers.iter().filter_map(|value| policy.transform(policy.relationship_qualifiers, "relationship_qualifier", value)).collect::<Vec<_>>(),
            "object_object_qualifiers": report.filter.object_object_qualifiers.iter().filter_map(|value| policy.transform(policy.relationship_qualifiers, "relationship_qualifier", value)).collect::<Vec<_>>()
        },
        "from_activity": activity(&report.from_activity),
        "to_activity": activity(&report.to_activity),
        "measurements": { "returned": measurements.len(), "total": report.measurements.len(), "items": measurements },
        "semantics": report.semantics,
        "interpretation_note": "These are calculated start-to-start elapsed times through common objects, not waiting times or causal effects.",
        "privacy_note": "Activities, object types, object and event identifiers, qualifiers, and source timestamps follow the reviewed exposure policy. Aggregate elapsed durations remain available as calculated metrics."
    })
}

fn expose_ocel_synchronization(
    handle: &str,
    report: &pm_query::OcelSynchronizationReport,
    policy: &ExposurePolicy,
    limit: usize,
) -> Value {
    let activity = |value: &str| policy.transform(policy.activity_names, "activity", value);
    let object_type =
        |value: &str| policy.transform(policy.object_type_names, "object_type", value);
    let object = |value: &str| policy.transform(policy.object_ids, "object", value);
    let event = |value: &str| policy.transform(policy.event_ids, "event", value);
    let timestamp =
        |value: i64| policy.transform(policy.timestamps, "timestamp", &value.to_string());
    let activities = report
        .activities
        .iter()
        .take(limit)
        .map(|item| {
            json!({
                "activity": activity(&item.activity),
                "object_types": item.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
                "multi_object_events": item.multi_object_events,
                "eligible_events": item.eligible_events,
                "insufficient_history_events": item.insufficient_history_events,
                "historically_observed_object_participations": item.historically_observed_object_participations,
                "first_seen_object_participations": item.first_seen_object_participations,
                "total_arrival_spread_micros": item.total_arrival_spread_micros,
                "mean_arrival_spread_micros": item.mean_arrival_spread_micros,
                "min_arrival_spread_micros": item.min_arrival_spread_micros,
                "max_arrival_spread_micros": item.max_arrival_spread_micros,
                "total_elapsed_after_last_prior_micros": item.total_elapsed_after_last_prior_micros,
                "mean_elapsed_after_last_prior_micros": item.mean_elapsed_after_last_prior_micros,
                "min_elapsed_after_last_prior_micros": item.min_elapsed_after_last_prior_micros,
                "max_elapsed_after_last_prior_micros": item.max_elapsed_after_last_prior_micros,
                "total_elapsed_from_first_prior_micros": item.total_elapsed_from_first_prior_micros,
                "mean_elapsed_from_first_prior_micros": item.mean_elapsed_from_first_prior_micros,
                "min_elapsed_from_first_prior_micros": item.min_elapsed_from_first_prior_micros,
                "max_elapsed_from_first_prior_micros": item.max_elapsed_from_first_prior_micros,
                "evidence": item.evidence.iter().map(|evidence| json!({
                    "event": event(&evidence.event),
                    "at_micros": timestamp(evidence.at_micros),
                    "participating_objects": evidence.participating_objects,
                    "historically_observed_objects": evidence.historically_observed_objects,
                    "first_seen_objects": evidence.first_seen_objects,
                    "arrival_spread_micros": evidence.arrival_spread_micros,
                    "elapsed_after_last_prior_micros": evidence.elapsed_after_last_prior_micros,
                    "elapsed_from_first_prior_micros": evidence.elapsed_from_first_prior_micros,
                    "readiness": evidence.readiness.iter().map(|readiness| json!({
                        "object": object(&readiness.object),
                        "object_type": object_type(&readiness.object_type),
                        "prior_at_micros": timestamp(readiness.prior_at_micros),
                        "prior_events": readiness.prior_events.iter().filter_map(|value| event(value)).collect::<Vec<_>>()
                    })).collect::<Vec<_>>()
                })).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    json!({
        "log": handle,
        "claim_type": "calculated_fact",
        "filter": {
            "cohort": report.filter.cohort.as_ref().map(|cohort| json!({"root_count":cohort.roots.len(),"correlation_paths":cohort.paths.len(),"pinned":true})),
            "object_types": report.filter.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
            "event_object_qualifiers": report.filter.event_object_qualifiers.iter().filter_map(|value| policy.transform(policy.relationship_qualifiers, "relationship_qualifier", value)).collect::<Vec<_>>(),
            "object_object_qualifiers": report.filter.object_object_qualifiers.iter().filter_map(|value| policy.transform(policy.relationship_qualifiers, "relationship_qualifier", value)).collect::<Vec<_>>()
        },
        "activities": { "returned": activities.len(), "total": report.activities.len(), "items": activities },
        "semantics": report.semantics,
        "interpretation_note": "The decomposition is calculated from event timestamps and object participation. It does not establish waiting, availability, causation, or execution duration.",
        "privacy_note": "Activities, object types, object and event identifiers, qualifiers, and source timestamps follow the reviewed exposure policy. Aggregate elapsed durations and counts remain available as calculated metrics."
    })
}

fn expose_ocel_bottlenecks(
    handle: &str,
    report: &pm_query::OcelBottleneckReport,
    policy: &ExposurePolicy,
    limit: usize,
) -> Value {
    let activity = |value: &str| policy.transform(policy.activity_names, "activity", value);
    let object_type =
        |value: &str| policy.transform(policy.object_type_names, "object_type", value);
    let object = |value: &str| policy.transform(policy.object_ids, "object", value);
    let event = |value: &str| policy.transform(policy.event_ids, "event", value);
    let timestamp =
        |value: i64| policy.transform(policy.timestamps, "timestamp", &value.to_string());
    let lifecycle = report
        .lifecycle
        .iter()
        .take(limit)
        .map(|item| {
            json!({
                "rank": item.rank,
                "object_type": object_type(&item.object_type),
                "source": activity(&item.source),
                "target": activity(&item.target),
                "occurrences": item.occurrences,
                "objects": item.objects,
                "total_elapsed_micros": item.total_elapsed_micros,
                "mean_elapsed_micros": item.mean_elapsed_micros,
                "share_millionths": item.share_millionths,
                "evidence": item.evidence.iter().map(|evidence| json!({
                    "object": object(&evidence.object),
                    "source_event": event(&evidence.source_event),
                    "target_event": event(&evidence.target_event),
                    "source_at_micros": timestamp(evidence.source_at_micros),
                    "target_at_micros": timestamp(evidence.target_at_micros)
                })).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let synchronization = report
        .synchronization
        .iter()
        .take(limit)
        .map(|item| {
            json!({
                "rank": item.rank,
                "activity": activity(&item.activity),
                "object_types": item.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
                "multi_object_events": item.multi_object_events,
                "eligible_events": item.eligible_events,
                "insufficient_history_events": item.insufficient_history_events,
                "total_arrival_spread_micros": item.total_arrival_spread_micros,
                "mean_arrival_spread_micros": item.mean_arrival_spread_micros,
                "share_millionths": item.share_millionths,
                "evidence": item.evidence.iter().map(|evidence| json!({
                    "event": event(&evidence.event),
                    "at_micros": timestamp(evidence.at_micros),
                    "participating_objects": evidence.participating_objects,
                    "historically_observed_objects": evidence.historically_observed_objects,
                    "first_seen_objects": evidence.first_seen_objects,
                    "arrival_spread_micros": evidence.arrival_spread_micros,
                    "elapsed_after_last_prior_micros": evidence.elapsed_after_last_prior_micros,
                    "elapsed_from_first_prior_micros": evidence.elapsed_from_first_prior_micros,
                    "readiness": evidence.readiness.iter().map(|readiness| json!({
                        "object": object(&readiness.object),
                        "object_type": object_type(&readiness.object_type),
                        "prior_at_micros": timestamp(readiness.prior_at_micros),
                        "prior_events": readiness.prior_events.iter().filter_map(|value| event(value)).collect::<Vec<_>>()
                    })).collect::<Vec<_>>()
                })).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    json!({
        "log": handle,
        "claim_type": "calculated_fact",
        "filter": {
            "cohort": report.filter.cohort.as_ref().map(|cohort| json!({"root_count":cohort.roots.len(),"correlation_paths":cohort.paths.len(),"pinned":true})),
            "object_types": report.filter.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
            "event_object_qualifiers": report.filter.event_object_qualifiers.iter().filter_map(|value| policy.transform(policy.relationship_qualifiers, "relationship_qualifier", value)).collect::<Vec<_>>(),
            "object_object_qualifiers": report.filter.object_object_qualifiers.iter().filter_map(|value| policy.transform(policy.relationship_qualifiers, "relationship_qualifier", value)).collect::<Vec<_>>()
        },
        "lifecycle": {
            "returned": lifecycle.len(),
            "ranked_total": report.lifecycle.len(),
            "candidates": report.lifecycle_candidates,
            "positive_candidates": report.lifecycle_positive_candidates,
            "total_elapsed_micros": report.total_lifecycle_elapsed_micros,
            "items": lifecycle
        },
        "synchronization": {
            "returned": synchronization.len(),
            "ranked_total": report.synchronization.len(),
            "eligible_activities": report.synchronization_eligible_activities,
            "positive_activities": report.synchronization_positive_activities,
            "total_arrival_spread_micros": report.total_arrival_spread_micros,
            "items": synchronization
        },
        "semantics": report.semantics,
        "interpretation_note": "Rank is an evidence-backed prioritization by observed total timing burden. Explanations, waiting claims, causal diagnoses, and improvement recommendations remain interpretations or hypotheses.",
        "privacy_note": "Activities, object types, object and event identifiers, qualifiers, and source timestamps follow the reviewed exposure policy. Aggregate durations, counts, ranks, and shares remain available as calculated metrics."
    })
}

fn expose_ocel_projection(
    handle: &str,
    report: &pm_query::OcelProjectionReport,
    policy: &ExposurePolicy,
) -> Value {
    let activity = |value: &str| policy.transform(policy.activity_names, "activity", value);
    let object_type =
        |value: &str| policy.transform(policy.object_type_names, "object_type", value);
    let object = |value: &str| policy.transform(policy.object_ids, "object", value);
    let event = |value: &str| policy.transform(policy.event_ids, "event", value);
    let timestamp =
        |value: i64| policy.transform(policy.timestamps, "timestamp", &value.to_string());
    let qualifier = |value: &str| {
        policy.transform(
            policy.relationship_qualifiers,
            "relationship_qualifier",
            value,
        )
    };
    let cases = report
        .cases
        .iter()
        .map(|case| {
            json!({
                "case_object": object(&case.case_object),
                "events_available": case.events_available,
                "events_returned": case.events_returned,
                "truncated": case.truncated,
                "layers": case.layers.iter().map(|layer| json!({
                    "at_micros": timestamp(layer.at_micros),
                    "events": layer.events.iter().map(|item| json!({
                        "source_event": event(&item.source_event),
                        "activity": activity(&item.activity)
                    })).collect::<Vec<_>>()
                })).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let convergence = report
        .convergence_evidence
        .iter()
        .map(|evidence| {
            json!({
                "source_event": event(&evidence.source_event),
                "activity": activity(&evidence.activity),
                "projected_case_count": evidence.projected_case_count,
                "additional_occurrences": evidence.additional_occurrences,
                "evidence_cases": evidence.evidence_cases.iter().filter_map(|value| object(value)).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let divergence = report
        .divergence_evidence
        .iter()
        .map(|evidence| {
            json!({
                "projected_case": object(&evidence.projected_case),
                "activity": activity(&evidence.activity),
                "related_object_type": object_type(&evidence.related_object_type),
                "projected_occurrences": evidence.projected_occurrences,
                "distinct_related_objects": evidence.distinct_related_objects,
                "evidence_events": evidence.evidence_events.iter().filter_map(|value| event(value)).collect::<Vec<_>>(),
                "evidence_objects": evidence.evidence_objects.iter().filter_map(|value| object(value)).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    json!({
        "log": handle,
        "claim_type": "calculated_fact",
        "filter": {
            "cohort": report.filter.cohort.as_ref().map(|cohort| json!({"root_count":cohort.roots.len(),"correlation_paths":cohort.paths.len(),"pinned":true})),
            "object_types": report.filter.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
            "event_object_qualifiers": report.filter.event_object_qualifiers.iter().filter_map(|value| qualifier(value)).collect::<Vec<_>>(),
            "object_object_qualifiers": report.filter.object_object_qualifiers.iter().filter_map(|value| qualifier(value)).collect::<Vec<_>>()
        },
        "case_object_type": object_type(&report.case_object_type),
        "source_events": report.source_events,
        "projected_cases": report.projected_cases,
        "included_unique_events": report.included_unique_events,
        "excluded_events": report.excluded_events,
        "projected_event_occurrences": report.projected_event_occurrences,
        "convergence": {
            "events": report.convergence_events,
            "additional_occurrences": report.convergence_additional_occurrences,
            "evidence": convergence
        },
        "divergence": {
            "candidates": report.divergence_candidates,
            "evidence": divergence
        },
        "cases": {
            "available": report.cases_available,
            "returned": report.cases_returned,
            "event_limit_per_case": report.event_limit_per_case,
            "items": cases
        },
        "semantics": report.semantics,
        "interpretation_note": "Projection counts and evidence are calculated facts. Divergence entries are risk candidates, not proof of false loops; explanations and decisions about using a projected model remain interpretations.",
        "privacy_note": "The selected case notion, filters, activities, event identifiers, object identifiers, object types, qualifiers, and timestamps follow the reviewed exposure policy. Complete aggregate distortion counts remain available."
    })
}

fn expose_ocel_model(
    handle: &str,
    report: &pm_query::OcelModelReport,
    policy: &ExposurePolicy,
) -> Value {
    let activity = |value: &str| policy.transform(policy.activity_names, "activity", value);
    let object_type =
        |value: &str| policy.transform(policy.object_type_names, "object_type", value);
    json!({
        "log": handle,
        "claim_type": "calculated_fact",
        "filter": {
            "cohort": report.filter.cohort.as_ref().map(|cohort| json!({"root_count":cohort.roots.len(),"correlation_paths":cohort.paths.len(),"pinned":true})),
            "object_types": report.filter.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
            "event_object_qualifiers": report.filter.event_object_qualifiers.iter().filter_map(|value| policy.transform(policy.relationship_qualifiers, "relationship_qualifier", value)).collect::<Vec<_>>(),
            "object_object_qualifiers": report.filter.object_object_qualifiers.iter().filter_map(|value| policy.transform(policy.relationship_qualifiers, "relationship_qualifier", value)).collect::<Vec<_>>()
        },
        "places": report.places.iter().map(|place| json!({
            "id": place.id,
            "object_type": object_type(&place.object_type),
            "initial": place.initial,
            "final_place": place.final_place
        })).collect::<Vec<_>>(),
        "transitions": report.transitions.iter().map(|transition| json!({
            "id": transition.id,
            "activity": transition.activity.as_deref().and_then(activity),
            "inputs": transition.inputs.iter().map(|arc| json!({
                "place": arc.place,
                "variable": arc.variable
            })).collect::<Vec<_>>(),
            "outputs": transition.outputs.iter().map(|arc| json!({
                "place": arc.place,
                "variable": arc.variable
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "object_types": report.object_types.iter().map(|stats| json!({
            "object_type": object_type(&stats.object_type),
            "objects": stats.objects,
            "objects_with_events": stats.objects_with_events,
            "lifecycle_event_occurrences": stats.lifecycle_event_occurrences,
            "filtered_edges": stats.filtered_edges,
            "used_flower_fallback": stats.used_flower_fallback
        })).collect::<Vec<_>>(),
        "variable_arcs": report.variable_arcs,
        "fixed_arcs": report.fixed_arcs,
        "noise_threshold_milli": report.noise_threshold_milli,
        "variable_arc_threshold_milli": report.variable_arc_threshold_milli,
        "semantics": report.semantics,
        "interpretation_note": "The formal structure and mining statistics are calculated results. Causal explanations, compliance conclusions, and improvement recommendations remain interpretations until separately supported.",
        "privacy_note": "Activity and object-type names follow the reviewed exposure policy. Place and transition numbers are model-local structural identifiers. No source event identifiers, object identifiers, timestamps, attributes, or source path are returned."
    })
}

fn expose_ocel_conformance(
    log_handle: &str,
    model_handle: &str,
    report: &pm_query::OcelConformanceReport,
    policy: &ExposurePolicy,
) -> Value {
    let activity = |value: &str| policy.transform(policy.activity_names, "activity", value);
    let object_type =
        |value: &str| policy.transform(policy.object_type_names, "object_type", value);
    let object = |value: &str| policy.transform(policy.object_ids, "object", value);
    let event = |value: &str| policy.transform(policy.event_ids, "event", value);
    let qualifier = |value: &str| {
        policy.transform(
            policy.relationship_qualifiers,
            "relationship_qualifier",
            value,
        )
    };
    let objects = report
        .objects
        .iter()
        .map(|item| {
            json!({
                "object": object(&item.object),
                "object_type": object_type(&item.object_type),
                "events": item.events,
                "fitness_millionths": item.fitness_millionths,
                "consumed_tokens": item.consumed_tokens,
                "produced_tokens": item.produced_tokens,
                "missing_tokens": item.missing_tokens,
                "remaining_tokens": item.remaining_tokens,
                "unmapped_events": item.unmapped_events,
                "silent_firings": item.silent_firings,
                "lifecycle_fit": item.lifecycle_fit
            })
        })
        .collect::<Vec<_>>();
    let deviations = report
        .deviations
        .iter()
        .map(|deviation| match deviation {
            pm_query::LabelledOcelConformanceDeviation::ActivityNotInModel {
                event: source_event,
                activity: name,
                object: source_object,
            } => json!({
                "kind": "activity_not_in_model",
                "event": event(source_event),
                "activity": activity(name),
                "object": object(source_object)
            }),
            pm_query::LabelledOcelConformanceDeviation::MissingTokens {
                event: source_event,
                activity: name,
                object: source_object,
                count,
            } => json!({
                "kind": "missing_tokens",
                "event": event(source_event),
                "activity": activity(name),
                "object": object(source_object),
                "count": count
            }),
            pm_query::LabelledOcelConformanceDeviation::FinalTokensMissing {
                object: source_object,
                count,
            } => json!({
                "kind": "final_tokens_missing",
                "object": object(source_object),
                "count": count
            }),
            pm_query::LabelledOcelConformanceDeviation::TokensRemaining {
                object: source_object,
                count,
            } => json!({
                "kind": "tokens_remaining",
                "object": object(source_object),
                "count": count
            }),
            pm_query::LabelledOcelConformanceDeviation::MissingObjectParticipation {
                event: source_event,
                activity: name,
                object_type: kind,
                variable,
            } => json!({
                "kind": "missing_object_participation",
                "event": event(source_event),
                "activity": name.as_deref().and_then(activity),
                "object_type": object_type(kind),
                "variable": variable
            }),
            pm_query::LabelledOcelConformanceDeviation::FixedArcMultiplicity {
                event: source_event,
                activity: name,
                object_type: kind,
                observed_objects,
            } => json!({
                "kind": "fixed_arc_multiplicity",
                "event": event(source_event),
                "activity": name.as_deref().and_then(activity),
                "object_type": object_type(kind),
                "observed_objects": observed_objects
            }),
            pm_query::LabelledOcelConformanceDeviation::UnexpectedObjectParticipation {
                event: source_event,
                activity: name,
                object_type: kind,
                observed_objects,
            } => json!({
                "kind": "unexpected_object_participation",
                "event": event(source_event),
                "activity": name.as_deref().and_then(activity),
                "object_type": object_type(kind),
                "observed_objects": observed_objects
            }),
        })
        .collect::<Vec<_>>();
    let training_source_name = Path::new(&report.model_training_source)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    json!({
        "log": log_handle,
        "model": model_handle,
        "claim_type": "calculated_fact",
        "model_training_source": policy.transform(
            policy.source_names,
            "source",
            &training_source_name
        ),
        "filter": {
            "cohort": report.filter.cohort.as_ref().map(|cohort| json!({"root_count":cohort.roots.len(),"correlation_paths":cohort.paths.len(),"pinned":true})),
            "object_types": report.filter.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
            "event_object_qualifiers": report.filter.event_object_qualifiers.iter().filter_map(|value| qualifier(value)).collect::<Vec<_>>(),
            "object_object_qualifiers": report.filter.object_object_qualifiers.iter().filter_map(|value| qualifier(value)).collect::<Vec<_>>()
        },
        "fitness_millionths": report.fitness_millionths,
        "lifecycle_fit_objects": report.lifecycle_fit_objects,
        "replayed_objects": report.replayed_objects,
        "fit_events": report.fit_events,
        "replayed_events": report.replayed_events,
        "out_of_scope_events": report.out_of_scope_events,
        "tokens": {
            "consumed": report.consumed_tokens,
            "produced": report.produced_tokens,
            "missing": report.missing_tokens,
            "remaining": report.remaining_tokens,
            "unmapped_event_occurrences": report.unmapped_event_occurrences
        },
        "binding_violations": report.binding_violations,
        "objects": {
            "available": report.objects_available,
            "returned": report.objects_returned,
            "truncated": report.objects_truncated,
            "items": objects
        },
        "deviations": {
            "available": report.deviations_available,
            "returned": report.deviations_returned,
            "truncated": report.deviations_truncated,
            "items": deviations
        },
        "semantics": report.semantics,
        "interpretation_note": "Fitness, token counts, binding violations, and replay deviations are calculated against the selected explicit baseline. Explanations, materiality judgments, regulatory conclusions, and remediation recommendations remain interpretations or hypotheses.",
        "privacy_note": "Source names, activities, event identifiers, object identifiers, object types, and relationship qualifiers follow the reviewed exposure policy. Opaque log/model handles and aggregate counts remain available; no raw event log, attributes, timestamps, source rows, or filesystem paths are returned."
    })
}

fn expose_ocel_alignment(
    log_handle: &str,
    model_handle: &str,
    report: &pm_query::OcelAlignmentConformanceReport,
    policy: &ExposurePolicy,
) -> Value {
    let activity = |value: &str| policy.transform(policy.activity_names, "activity", value);
    let object_type =
        |value: &str| policy.transform(policy.object_type_names, "object_type", value);
    let object = |value: &str| policy.transform(policy.object_ids, "object", value);
    let event = |value: &str| policy.transform(policy.event_ids, "event", value);
    let qualifier = |value: &str| {
        policy.transform(
            policy.relationship_qualifiers,
            "relationship_qualifier",
            value,
        )
    };
    let objects = report
        .objects
        .iter()
        .map(|item| {
            let deviations = item
                .deviations
                .iter()
                .map(|deviation| match deviation {
                    pm_query::LabelledOcelAlignmentDeviation::LogOnly {
                        event: source_event,
                        activity: name,
                    } => json!({
                        "kind": "log_only",
                        "event": event(source_event),
                        "activity": activity(name)
                    }),
                    pm_query::LabelledOcelAlignmentDeviation::VisibleModel {
                        transition,
                        activity: name,
                    } => json!({
                        "kind": "visible_model",
                        "transition": transition,
                        "activity": activity(name)
                    }),
                })
                .collect::<Vec<_>>();
            json!({
                "object": object(&item.object),
                "object_type": object_type(&item.object_type),
                "events": item.events,
                "cost": item.cost,
                "denominator": item.denominator,
                "fitness_millionths": item.fitness_millionths,
                "synchronous_moves": item.synchronous_moves,
                "invisible_moves": item.invisible_moves,
                "log_moves": item.log_moves,
                "visible_model_moves": item.visible_model_moves,
                "lifecycle_fit": item.lifecycle_fit,
                "deviations": {
                    "available": item.deviations_available,
                    "returned": item.deviations_returned,
                    "truncated": item.deviations_truncated,
                    "items": deviations
                }
            })
        })
        .collect::<Vec<_>>();
    let binding_deviations = report
        .binding_deviations
        .iter()
        .map(|deviation| match deviation {
            pm_query::LabelledOcelConformanceDeviation::MissingObjectParticipation {
                event: source_event,
                activity: name,
                object_type: kind,
                variable,
            } => json!({
                "kind": "missing_object_participation",
                "event": event(source_event),
                "activity": name.as_deref().and_then(activity),
                "object_type": object_type(kind),
                "variable": variable
            }),
            pm_query::LabelledOcelConformanceDeviation::FixedArcMultiplicity {
                event: source_event,
                activity: name,
                object_type: kind,
                observed_objects,
            } => json!({
                "kind": "fixed_arc_multiplicity",
                "event": event(source_event),
                "activity": name.as_deref().and_then(activity),
                "object_type": object_type(kind),
                "observed_objects": observed_objects
            }),
            pm_query::LabelledOcelConformanceDeviation::UnexpectedObjectParticipation {
                event: source_event,
                activity: name,
                object_type: kind,
                observed_objects,
            } => json!({
                "kind": "unexpected_object_participation",
                "event": event(source_event),
                "activity": name.as_deref().and_then(activity),
                "object_type": object_type(kind),
                "observed_objects": observed_objects
            }),
            _ => json!({ "kind": "unexpected_lifecycle_replay_deviation" }),
        })
        .collect::<Vec<_>>();
    let training_source_name = Path::new(&report.model_training_source)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    json!({
        "log": log_handle,
        "model": model_handle,
        "claim_type": "calculated_fact",
        "algorithm": "optimal_astar_alignment",
        "model_training_source": policy.transform(policy.source_names, "source", &training_source_name),
        "filter": {
            "cohort": report.filter.cohort.as_ref().map(|cohort| json!({"root_count":cohort.roots.len(),"correlation_paths":cohort.paths.len(),"pinned":true})),
            "object_types": report.filter.object_types.iter().filter_map(|value| object_type(value)).collect::<Vec<_>>(),
            "event_object_qualifiers": report.filter.event_object_qualifiers.iter().filter_map(|value| qualifier(value)).collect::<Vec<_>>(),
            "object_object_qualifiers": report.filter.object_object_qualifiers.iter().filter_map(|value| qualifier(value)).collect::<Vec<_>>()
        },
        "total_cost": report.total_cost,
        "denominator": report.denominator,
        "fitness_millionths": report.fitness_millionths,
        "lifecycle_fit_objects": report.lifecycle_fit_objects,
        "aligned_objects": report.aligned_objects,
        "fit_events": report.fit_events,
        "aligned_events": report.aligned_events,
        "out_of_scope_events": report.out_of_scope_events,
        "binding_violations": report.binding_violations,
        "objects": {
            "available": report.objects_available,
            "returned": report.objects_returned,
            "truncated": report.objects_truncated,
            "items": objects
        },
        "binding_deviations": {
            "available": report.binding_deviations_available,
            "returned": report.binding_deviations_returned,
            "truncated": report.binding_deviations_truncated,
            "items": binding_deviations
        },
        "semantics": report.semantics,
        "interpretation_note": "Alignment cost and fitness are calculated against the selected explicit baseline. Shared-event binding is a separate calculated check. Explanations, materiality judgments, regulatory conclusions, and remediation recommendations remain interpretations or hypotheses.",
        "privacy_note": "Source names, activities, event identifiers, object identifiers, object types, and relationship qualifiers follow the reviewed exposure policy. Opaque handles, model-local transition numbers, and aggregate counts remain available; no raw event log, attributes, timestamps, source rows, or filesystem paths are returned."
    })
}

fn timing_description(analysis: &Analysis) -> Value {
    json!({
        "transition_duration": if analysis.measures_true_waiting { "waiting" } else { "elapsed" },
        "transition_duration_explanation": if analysis.measures_true_waiting {
            "End timestamps were mapped, so transition durations exclude the preceding activity's processing time."
        } else {
            "No end timestamps were mapped, so transition durations are start-to-start elapsed time rather than pure waiting."
        },
        "throughput_unit": "seconds"
    })
}

fn view_description(filtered: &pm_core::Filtered) -> Value {
    json!({
        "cases_shown": filtered.log.case_count(),
        "cases_total": filtered.cases_before,
        "events_total": filtered.events_before,
        "percent_shown": filtered.cases_kept_percent(),
        "narrowed": filtered.narrowed()
    })
}

/// The item an exposed identifier names, searched on disk for a large log.
fn resolve_exposed_object(
    policy: &ExposurePolicy,
    path: &std::path::Path,
    exposed: &str,
) -> Result<String, String> {
    if policy.object_ids == Mode::Omit {
        return Err(
            "object identifiers are omitted by the exposure policy, so this drill-down is unavailable"
                .into(),
        );
    }
    pm_query::find_ocel_object(path, |candidate| {
        policy.transform(policy.object_ids, "object", candidate).as_deref() == Some(exposed)
    })
    .map_err(|error| error.to_string())?
    .ok_or_else(|| {
        "the object identifier was not found in the saved active view; use an identifier returned by an MCP tool".to_owned()
    })
}

fn resolve_exposed(
    policy: &ExposurePolicy,
    mode: Mode,
    category: &str,
    exposed: &str,
    candidates: &[String],
) -> Result<String, String> {
    if mode == Mode::Omit {
        return Err(format!(
            "{category} identifiers are omitted by the exposure policy, so this drill-down is unavailable"
        ));
    }
    candidates
        .iter()
        .find(|candidate| policy.transform(mode, category, candidate).as_deref() == Some(exposed))
        .cloned()
        .ok_or_else(|| {
            format!(
                "the {category} identifier was not found in the saved active view; use an identifier returned by an MCP tool"
            )
        })
}

fn case_candidates(log: &pm_core::EventLog) -> Vec<String> {
    log.traces
        .iter()
        .map(|trace| log.case_label(trace.case).to_owned())
        .collect()
}

fn activity_candidates(log: &pm_core::EventLog) -> Vec<String> {
    log.traces
        .iter()
        .flat_map(|trace| trace.events.iter())
        .map(|event| log.activity_label(event.activity).to_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn resource_candidates(log: &pm_core::EventLog) -> Vec<String> {
    log.traces
        .iter()
        .flat_map(|trace| trace.events.iter())
        .filter_map(|event| event.resource().map(|id| log.resource_label(id).to_owned()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn exposed_evidence(evidence: &pm_query::Evidence, policy: &ExposurePolicy) -> Value {
    match evidence {
        pm_query::Evidence::Activity(activity) => json!({
            "kind": "activity",
            "activity": policy.transform(policy.activity_names, "activity", activity)
        }),
        pm_query::Evidence::Transition { from, to } => json!({
            "kind": "transition",
            "from": policy.transform(policy.activity_names, "activity", from),
            "to": policy.transform(policy.activity_names, "activity", to)
        }),
        pm_query::Evidence::Variant(sequence) => json!({
            "kind": "variant",
            "sequence": match policy.activity_names {
                Mode::Omit => None,
                mode => Some(sequence.iter().filter_map(|activity| {
                    policy.transform(mode, "activity", activity)
                }).collect::<Vec<_>>())
            }
        }),
        pm_query::Evidence::Handover { from, to } => json!({
            "kind": "handover",
            "from": policy.transform(policy.resource_names, "resource", from),
            "to": policy.transform(policy.resource_names, "resource", to)
        }),
    }
}

fn redact_case_detail(
    project: &str,
    project_name: Option<String>,
    detail: &pm_query::CaseDetail,
    policy: &ExposurePolicy,
    view: Value,
) -> Value {
    let attributes = detail
        .attributes
        .iter()
        .map(|(name, value)| {
            json!({
                "name": policy.transform(policy.attribute_names, "attribute", name),
                "value": policy.transform(policy.attribute_values, "attribute_value", value)
            })
        })
        .collect::<Vec<_>>();
    let events = detail
        .events
        .iter()
        .enumerate()
        .map(|(index, event)| {
            json!({
                "position": index + 1,
                "activity": policy.transform(policy.activity_names, "activity", &event.activity),
                "timestamp": policy.transform(policy.timestamps, "timestamp", &event.timestamp),
                "end_timestamp": event.end_timestamp.as_ref().and_then(|timestamp| {
                    policy.transform(policy.timestamps, "timestamp", timestamp)
                }),
                "processing_seconds": event.processing_seconds,
                "since_previous_seconds": event.since_previous_seconds,
                "resource": event.resource.as_ref().and_then(|resource| {
                    policy.transform(policy.resource_names, "resource", resource)
                }),
                "cost_microunits": event.cost_microunits.and_then(|cost| {
                    policy.transform(policy.costs, "cost", &cost.to_string())
                }),
                "source_row": policy.transform(
                    policy.source_rows,
                    "source_row",
                    &event.source_row.to_string()
                )
            })
        })
        .collect::<Vec<_>>();
    json!({
        "project": project,
        "name": project_name,
        "claim_type": "calculated_fact",
        "view": view,
        "case": policy.transform(policy.case_ids, "case", &detail.case),
        "event_count": detail.events.len(),
        "variant": match policy.activity_names {
            Mode::Omit => None,
            mode => Some(detail.variant.iter().filter_map(|activity| {
                policy.transform(mode, "activity", activity)
            }).collect::<Vec<_>>())
        },
        "throughput_seconds": detail.throughput_seconds,
        "throughput_eligible": detail.throughput_eligible,
        "incomplete": detail.incomplete,
        "attributes": attributes,
        "total_cost_microunits": detail.total_cost_microunits.and_then(|cost| {
            policy.transform(policy.costs, "cost", &cost.to_string())
        }),
        "source_rows": detail.source_rows.iter().filter_map(|row| {
            policy.transform(policy.source_rows, "source_row", &row.to_string())
        }).collect::<Vec<_>>(),
        "gap_measure": if detail.gap_is_waiting { "waiting" } else { "elapsed" },
        "timestamp_reading": if policy.timestamps == Mode::Omit {
            None
        } else {
            Some(detail.timestamp_reading.clone())
        },
        "events": events,
        "privacy_note": "Every identifier and value category follows the reviewed exposure policy. Timing durations remain calculated facts even when timestamp values are omitted."
    })
}

fn redact_mapping(mapping: &pm_ingest::Mapping, policy: &ExposurePolicy) -> serde_json::Value {
    let column = |value: &str| policy.transform(policy.column_names, "column", value);
    json!({
        "case_id": column(&mapping.case_id),
        "activity": column(&mapping.activity),
        "timestamp": column(&mapping.timestamp),
        "event_order": mapping.event_order.as_deref().and_then(column),
        "end_timestamp": mapping.end_timestamp.as_deref().and_then(column),
        "resource": mapping.resource.as_deref().and_then(column),
        "lifecycle": mapping.lifecycle.as_deref().and_then(column),
        "cost": mapping.cost.as_deref().and_then(column),
        "case_attributes": mapping.case_attributes.iter().filter_map(|name| {
            policy.transform(policy.attribute_names, "attribute", name)
        }).collect::<Vec<_>>()
    })
}

/// Builds a local-only preview of original values beside exactly what MCP
/// would transmit. Callers must display this before starting the MCP server;
/// the original side of this structure must never itself become a tool result.
pub fn preview_project(
    project: &Project,
    policy: &ExposurePolicy,
) -> Result<ExposurePreview, String> {
    policy.validate()?;
    let state = project.check_recorded_source();
    if !state.reproduces() {
        return Err(state.explain());
    }
    let profile =
        pm_query::inspect_log(&project.source.path, true).map_err(|error| error.to_string())?;
    let source_file = Path::new(&project.source.path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut mapped = vec![
        (
            "case id",
            project.mapping.case_id.as_str(),
            policy.case_ids,
            "case",
        ),
        (
            "activity",
            project.mapping.activity.as_str(),
            policy.activity_names,
            "activity",
        ),
        (
            "timestamp",
            project.mapping.timestamp.as_str(),
            policy.timestamps,
            "timestamp",
        ),
    ];
    for (role, column, mode, category) in [
        (
            "event order",
            project.mapping.event_order.as_deref(),
            Mode::Omit,
            "order",
        ),
        (
            "end timestamp",
            project.mapping.end_timestamp.as_deref(),
            policy.timestamps,
            "timestamp",
        ),
        (
            "resource",
            project.mapping.resource.as_deref(),
            policy.resource_names,
            "resource",
        ),
        (
            "lifecycle",
            project.mapping.lifecycle.as_deref(),
            Mode::Alias,
            "lifecycle",
        ),
        (
            "cost",
            project.mapping.cost.as_deref(),
            policy.costs,
            "cost",
        ),
    ] {
        if let Some(column) = column {
            mapped.push((role, column, mode, category));
        }
    }
    for attribute in &project.mapping.case_attributes {
        mapped.push((
            "case attribute",
            attribute,
            policy.attribute_values,
            "attribute_value",
        ));
    }
    let fields = mapped
        .into_iter()
        .map(|(role, column, mode, category)| {
            let column_mode = if role == "case attribute" {
                policy.attribute_names
            } else {
                policy.column_names
            };
            let examples = profile
                .columns
                .iter()
                .find(|item| item.name == column)
                .map(|item| {
                    item.examples
                        .iter()
                        .take(4)
                        .map(|value| ValuePreview {
                            local_value: value.clone(),
                            outbound_value: policy.transform(mode, category, value),
                        })
                        .collect()
                })
                .unwrap_or_default();
            FieldPreview {
                role: role.to_owned(),
                local_column: column.to_owned(),
                outbound_column: policy.transform(column_mode, "column", column),
                mode,
                examples,
            }
        })
        .collect();
    Ok(ExposurePreview {
        project_name: ValuePreview {
            local_value: project.name.clone(),
            outbound_value: policy.transform(policy.project_names, "project_name", &project.name),
        },
        source_file: ValuePreview {
            local_value: source_file.clone(),
            outbound_value: policy.transform(policy.source_names, "source", &source_file),
        },
        policy: policy.public_summary(),
        fields,
        note: "The left side is shown only in this local terminal preview. MCP receives only the outbound side.".to_owned(),
    })
}

fn source_state(state: &SourceState) -> &'static str {
    match state {
        SourceState::Unchanged => "unchanged",
        SourceState::Changed { .. } => "changed",
        SourceState::Missing => "missing",
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_ingest::finding::Policy;
    use pm_ingest::validate::Mapping;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn stdio_delegation_forwards_only_authenticated_json_rpc() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let responder = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let read = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..read]);
                let Some(split) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                    continue;
                };
                let head = std::str::from_utf8(&request[..split]).unwrap();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                if request.len() >= split + 4 + length {
                    break;
                }
            }
            let request = std::str::from_utf8(&request).unwrap();
            assert!(request.starts_with("POST /api/mcp HTTP/1.1\r\n"));
            assert!(request.contains("Authorization: Bearer derived-agent-token\r\n"));
            let request_body = request.split("\r\n\r\n").nth(1).unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(request_body).unwrap(),
                json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" })
            );
            let body = r#"{"jsonrpc":"2.0","id":7,"result":{}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let response = delegate_message(
            port,
            "derived-agent-token",
            &json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(response["id"], 7);
        assert_eq!(response["result"], json!({}));
        responder.join().unwrap();
    }

    fn fixture() -> (PathBuf, Server) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pm_mcp_{}_{}_{}",
            std::process::id(),
            unique,
            sequence
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("orders.csv");
        std::fs::write(
            &source,
            "case_id,activity,timestamp,secret\nC1,Start,2026-01-01 10:00:00,private\n",
        )
        .unwrap();
        Project::new(
            &source,
            Mapping::new("case_id", "activity", "timestamp"),
            Policy::new(),
            pm_ingest::Rules::new(),
        )
        .unwrap()
        .named("Orders")
        .save(root.join("orders.json"))
        .unwrap();
        let server = Server::new(&root).unwrap();
        (root, server)
    }

    fn ocel_fixture() -> (PathBuf, Server) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pm_mcp_ocel_{}_{}_{}",
            std::process::id(),
            unique,
            sequence
        ));
        std::fs::create_dir(&root).unwrap();
        let log_path = root.join("order-management.jsonocel");
        std::fs::write(
            &log_path,
            include_bytes!("../../../testdata/ocel/order-management.jsonocel"),
        )
        .unwrap();
        std::fs::write(
            root.join("order-management.xmlocel"),
            include_bytes!("../../../testdata/ocel/order-management.xmlocel"),
        )
        .unwrap();
        std::fs::write(
            root.join("order-management.sqlite"),
            include_bytes!("../../../testdata/ocel/order-management.sqlite"),
        )
        .unwrap();
        let baseline = pm_query::discover_ocel_model(
            &log_path,
            &pm_core::OcelFilter::default(),
            pm_core::OcelDiscoveryConfig::default(),
        )
        .unwrap();
        std::fs::write(
            root.join("approved.ocelmodel.json"),
            serde_json::to_vec(&baseline).unwrap(),
        )
        .unwrap();
        let server = Server::new(&root).unwrap();
        (root, server)
    }

    fn formal_model_fixture() -> (PathBuf, Server) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pm_mcp_formal_{}_{}_{}",
            std::process::id(),
            unique,
            sequence
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("private-claims.csv");
        std::fs::write(
            &source,
            "case_id,activity,timestamp\n\
             C1,A,2026-01-01 09:00:00\n\
             C1,B,2026-01-01 10:00:00\n\
             C1,C,2026-01-01 11:00:00\n\
             C2,A,2026-01-02 09:00:00\n\
             C2,C,2026-01-02 10:00:00\n\
             C3,A,2026-01-03 09:00:00\n\
             C3,Private Exception,2026-01-03 10:00:00\n\
             C3,B,2026-01-03 11:00:00\n\
             C3,C,2026-01-03 12:00:00\n",
        )
        .unwrap();
        let mut project = Project::new(
            &source,
            Mapping::new("case_id", "activity", "timestamp"),
            Policy::new(),
            pm_ingest::Rules::new(),
        )
        .unwrap()
        .named("Secret Claims");
        // C2 is deliberately outside the saved active view. Every formal MCP
        // tool must operate on C1 and C3 only, without partial traces.
        project.view_filter = pm_core::Filter::new().with_activity("B");
        project.save(root.join("claims.json")).unwrap();

        let baseline = pm_query::PortableFormalModel {
            source: "/confidential/designs/approved-process.bpmn".to_owned(),
            net: pm_query::LabelledPetriNet {
                place_count: 4,
                transitions: vec![
                    pm_query::LabelledTransition {
                        id: 0,
                        activity: Some("A".to_owned()),
                        inputs: vec![0],
                        outputs: vec![1],
                    },
                    pm_query::LabelledTransition {
                        id: 1,
                        activity: Some("B".to_owned()),
                        inputs: vec![1],
                        outputs: vec![2],
                    },
                    pm_query::LabelledTransition {
                        id: 2,
                        activity: Some("C".to_owned()),
                        inputs: vec![2],
                        outputs: vec![3],
                    },
                ],
                initial_marking: vec![1, 0, 0, 0],
                final_marking: vec![0, 0, 0, 1],
            },
        };
        std::fs::write(
            root.join("approved.model.json"),
            serde_json::to_vec(&baseline).unwrap(),
        )
        .unwrap();
        let server = Server::new(&root).unwrap();
        (root, server)
    }

    #[test]
    fn mcp_discovers_and_analyzes_xml_and_sqlite_ocel_logs() {
        let (root, server) = ocel_fixture();
        let listed = server
            .handle(request(
                1,
                "tools/call",
                json!({ "name": "list_ocel_logs", "arguments": {} }),
            ))
            .unwrap();
        assert_eq!(
            listed["result"]["structuredContent"]["logs"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let analyzed = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "analyze_ocel",
                    "arguments": { "log": server.ocel_handle("order-management.xmlocel") }
                }),
            ))
            .unwrap();
        assert_eq!(analyzed["result"]["isError"], false);
        assert_eq!(
            analyzed["result"]["structuredContent"]["scope"]["events_shown"],
            3
        );
        let sqlite = server
            .handle(request(
                3,
                "tools/call",
                json!({
                    "name": "analyze_ocel",
                    "arguments": { "log": server.ocel_handle("order-management.sqlite") }
                }),
            ))
            .unwrap();
        assert_eq!(sqlite["result"]["isError"], false);
        assert_eq!(
            sqlite["result"]["structuredContent"]["scope"]["events_shown"],
            3
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn live_object_centric_workspace_defaults_to_and_cannot_widen_the_active_view() {
        let (root, _) = ocel_fixture();
        let path = root.join("order-management.jsonocel");
        let mut policy = ExposurePolicy::safe();
        policy.object_type_names = Mode::Show;
        let server = Server::with_live_workspace(
            LiveWorkspace {
                ocel_log: Some(LiveOcelLog {
                    path,
                    source_name: "order-management.jsonocel".to_owned(),
                    filter: pm_core::OcelFilter {
                        object_types: vec!["package".to_owned()],
                        ..pm_core::OcelFilter::default()
                    },
                }),
                ..LiveWorkspace::default()
            },
            policy,
        )
        .unwrap();
        let handle = server.ocel_handle("live:order-management.jsonocel");
        let analyzed = server
            .handle(request(
                1,
                "tools/call",
                json!({ "name": "analyze_ocel", "arguments": { "log": handle.clone() } }),
            ))
            .unwrap();
        assert_eq!(analyzed["result"]["isError"], false);
        assert_eq!(
            analyzed["result"]["structuredContent"]["scope"]["objects_shown"],
            1
        );
        assert_eq!(
            analyzed["result"]["structuredContent"]["scope"]["events_shown"],
            2
        );
        assert_eq!(
            analyzed["result"]["structuredContent"]["scope"]["narrowed"],
            true
        );

        let widened = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "analyze_ocel",
                    "arguments": { "log": handle, "object_types": ["item"] }
                }),
            ))
            .unwrap();
        assert_eq!(widened["result"]["isError"], true);
        assert!(
            widened["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("outside the running application's active object-centric view")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn connected_cohort_is_pinned_without_exposing_root_ids() {
        let (root, _) = ocel_fixture();
        let server = Server::with_live_workspace(
            LiveWorkspace {
                ocel_log: Some(LiveOcelLog {
                    path: root.join("order-management.jsonocel"),
                    source_name: "cohort".into(),
                    filter: pm_core::OcelFilter {
                        cohort: Some(pm_core::OcelCohort {
                            root_type: "item".into(),
                            roots: std::collections::BTreeSet::from(["item-1".into()]),
                            paths: vec![vec!["item".into(), "package".into()]],
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                }),
                ..Default::default()
            },
            ExposurePolicy::safe(),
        )
        .unwrap();
        let handle = server.ocel_handle("live:cohort");
        for selectors in [
            json!({"log":handle}),
            json!({"log":handle,"object_types":[]}),
        ] {
            let result = server
                .handle(request(
                    1,
                    "tools/call",
                    json!({"name":"analyze_ocel","arguments":selectors}),
                ))
                .unwrap();
            assert_eq!(result["result"]["isError"], false, "{result}");
            assert_eq!(
                result["result"]["structuredContent"]["scope"]["objects_shown"],
                2
            );
            assert_eq!(
                result["result"]["structuredContent"]["filter"]["cohort"]["root_count"],
                1
            );
            assert!(!result.to_string().contains("item-1"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    fn finding_fixture() -> (PathBuf, Server) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pm_mcp_findings_{}_{}_{}",
            std::process::id(),
            unique,
            sequence
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("claims.csv");
        std::fs::write(
            &source,
            "case_id,activity,timestamp\n\
             C1,Submit,2026-01-01 09:00:00\n\
             C1,Review,2026-01-01 10:00:00\n\
             C1,Request Documents,2026-01-01 11:00:00\n\
             C1,Review,2026-01-03 10:00:00\n\
             C1,Approve,2026-01-03 12:00:00\n\
             C2,Submit,2026-01-01 10:00:00\n\
             C2,Review,2026-01-01 12:00:00\n\
             C2,Approve,2026-01-01 13:00:00\n\
             C3,Submit,2026-01-02 09:00:00\n\
             C3,Review,2026-01-02 10:00:00\n\
             C3,Request Documents,2026-01-02 11:00:00\n\
             C3,Review,2026-01-04 10:00:00\n\
             C3,Approve,2026-01-04 12:00:00\n",
        )
        .unwrap();
        Project::new(
            &source,
            Mapping::new("case_id", "activity", "timestamp"),
            Policy::new(),
            pm_ingest::Rules::new(),
        )
        .unwrap()
        .named("Claims")
        .save(root.join("claims.json"))
        .unwrap();
        let server = Server::new(&root).unwrap();
        (root, server)
    }

    fn automation_fixture() -> (PathBuf, Server) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pm_mcp_automation_{}_{}_{}",
            std::process::id(),
            unique,
            sequence
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("invoices.csv");
        std::fs::write(
            &source,
            "case_id,activity,timestamp,end_timestamp,resource,cost\n\
             C1,Receive,2026-01-01 09:00:00,2026-01-01 09:01:00,SYSTEM,0\n\
             C1,Post,2026-01-01 09:02:00,2026-01-01 09:03:00,WF-BATCH,0\n\
             C2,Receive,2026-01-01 10:00:00,2026-01-01 10:01:00,SYSTEM,0\n\
             C2,Fix Data,2026-01-01 10:05:00,2026-01-01 10:15:00,alice,12.50\n\
             C2,Post,2026-01-01 10:16:00,2026-01-01 10:17:00,WF-BATCH,0\n\
             C3,Receive,2026-01-01 11:00:00,2026-01-01 11:01:00,SYSTEM,0\n\
             C3,Fix Data,2026-01-01 11:05:00,2026-01-01 11:15:00,bob,12.50\n\
             C3,Post,2026-01-01 11:16:00,2026-01-01 11:17:00,WF-BATCH,0\n",
        )
        .unwrap();
        let rules = pm_ingest::Rules::new()
            .classify_resource("SYSTEM", pm_core::automation::ResourceKind::System)
            .classify_resource("WF-BATCH", pm_core::automation::ResourceKind::System)
            .classify_resource("alice", pm_core::automation::ResourceKind::Human)
            .classify_resource("bob", pm_core::automation::ResourceKind::Human);
        Project::new(
            &source,
            Mapping::new("case_id", "activity", "timestamp")
                .with_end_timestamp("end_timestamp")
                .with_resource("resource")
                .with_cost("cost"),
            Policy::new(),
            rules,
        )
        .unwrap()
        .named("Invoice Handling")
        .save(root.join("invoices.json"))
        .unwrap();
        let server = Server::new(&root).unwrap();
        (root, server)
    }

    fn validation_fixture() -> (PathBuf, Server) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pm_mcp_validation_{}_{}_{}",
            std::process::id(),
            unique,
            sequence
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("sensitive.csv");
        std::fs::write(
            &source,
            "case_id,activity,timestamp\nC1,Private Review,\nC1,Approve,2026-01-01 10:00:00\n",
        )
        .unwrap();
        Project::new(
            &source,
            Mapping::new("case_id", "activity", "timestamp"),
            Policy::new(),
            pm_ingest::Rules::new(),
        )
        .unwrap()
        .named("Sensitive Review")
        .save(root.join("sensitive.json"))
        .unwrap();
        let server = Server::new(&root).unwrap();
        (root, server)
    }

    fn comparison_fixture() -> (PathBuf, Server) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pm_mcp_comparison_{}_{}_{}",
            std::process::id(),
            unique,
            sequence
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("regions.csv");
        std::fs::write(&source, "case_id,activity,timestamp,region\nE1,Start,2026-01-01 09:00:00,East\nE1,Review,2026-01-01 10:00:00,East\nE1,End,2026-01-01 11:00:00,East\nE2,Start,2026-01-02 09:00:00,East\nE2,Review,2026-01-02 10:00:00,East\nE2,End,2026-01-02 12:00:00,East\nW1,Start,2026-01-03 09:00:00,West\nW1,Escalate,2026-01-03 11:00:00,West\nW1,End,2026-01-03 15:00:00,West\nW2,Start,2026-01-04 09:00:00,West\nW2,Escalate,2026-01-04 11:00:00,West\nW2,End,2026-01-04 17:00:00,West\n").unwrap();
        Project::new(
            &source,
            Mapping::new("case_id", "activity", "timestamp")
                .with_case_attributes(vec!["region".to_owned()]),
            Policy::new(),
            pm_ingest::Rules::new(),
        )
        .unwrap()
        .named("Regional Flow")
        .save(root.join("regions.json"))
        .unwrap();
        let mut policy = ExposurePolicy::safe();
        policy.attribute_values = Mode::Alias;
        let server = Server::with_exposure_policy(&root, policy).unwrap();
        (root, server)
    }

    fn request(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn supports_modern_discovery_and_legacy_initialization() {
        let (_root, server) = fixture();
        let modern = server
            .handle(request(1, "server/discover", json!({})))
            .unwrap();
        assert_eq!(modern["result"]["supportedVersions"][0], MODERN_PROTOCOL);
        let legacy = server.handle(request(2, "initialize", json!({}))).unwrap();
        assert_eq!(legacy["result"]["protocolVersion"], LEGACY_PROTOCOL);
        let older = server
            .handle(request(
                3,
                "initialize",
                json!({ "protocolVersion": "2025-06-18" }),
            ))
            .unwrap();
        assert_eq!(older["result"]["protocolVersion"], "2025-06-18");
        assert!(
            modern["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("get_claim_contract")
        );
        assert!(
            legacy["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("improvement hypotheses")
        );
    }

    #[test]
    fn claim_contract_is_discoverable_structured_and_requires_evidence() {
        let (_root, server) = fixture();
        let listed = server.handle(request(1, "tools/list", json!({}))).unwrap();
        let tool = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "get_claim_contract")
            .unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert!(tool["description"].as_str().unwrap().contains("Call once"));

        let response = server
            .handle(request(
                2,
                "tools/call",
                json!({ "name": "get_claim_contract", "arguments": {} }),
            ))
            .unwrap();
        let contract = &response["result"]["structuredContent"];
        assert_eq!(contract["contract_version"], 1);
        assert_eq!(
            contract["agent_claim_types"]["interpretation"]["claim_type_value"],
            "interpretation"
        );
        assert_eq!(
            contract["agent_claim_types"]["improvement_hypothesis"]["claim_type_value"],
            "improvement_hypothesis"
        );
        assert!(
            contract["based_on_item"]["required_fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field == "evidence_refs")
        );
        assert!(
            contract["agent_claim_types"]["improvement_hypothesis"]["required_fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field == "validation_plan")
        );
        assert!(
            contract["field_shapes"]["based_on"]
                .as_str()
                .unwrap()
                .contains("Non-empty array")
        );

        let text: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(text, *contract);
    }

    #[test]
    fn formal_model_tools_are_discoverable_with_bounded_closed_schemas() {
        let (_root, server) = fixture();
        let listed = server.handle(request(1, "tools/list", json!({}))).unwrap();
        let tools = listed["result"]["tools"].as_array().unwrap();
        for name in [
            "list_process_models",
            "discover_process_model",
            "check_process_conformance",
            "align_process_conformance",
        ] {
            let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
        let discovery = tools
            .iter()
            .find(|tool| tool["name"] == "discover_process_model")
            .unwrap();
        assert_eq!(
            discovery["inputSchema"]["properties"]["max_model_items"]["maximum"],
            10_000
        );
        assert!(
            discovery["description"]
                .as_str()
                .unwrap()
                .contains("not independent compliance evidence")
        );
    }

    #[test]
    fn mcp_discovers_all_case_model_families_on_the_exact_saved_view() {
        let (root, server) = formal_model_fixture();
        let project = server.project_handle("claims.json");
        for algorithm in ["imf", "alpha", "heuristics"] {
            let response = server
                .handle(request(
                    1,
                    "tools/call",
                    json!({
                        "name": "discover_process_model",
                        "arguments": { "project": project, "algorithm": algorithm }
                    }),
                ))
                .unwrap();
            assert_eq!(response["result"]["isError"], false, "{response:#}");
            let output = &response["result"]["structuredContent"];
            assert_eq!(output["claim_type"], "calculated_fact");
            assert_eq!(output["view"]["cases_shown"], 2);
            assert_eq!(output["view"]["cases_total"], 3);
            assert_eq!(output["view"]["events_total"], 9);
            assert_eq!(output["view"]["narrowed"], true);
            let serialized = response.to_string();
            assert!(!serialized.contains("Private Exception"));
            assert!(!serialized.contains("Secret Claims"));
            assert!(!serialized.contains("private-claims.csv"));
            assert!(!serialized.contains(&root.display().to_string()));
            if algorithm == "heuristics" {
                assert_eq!(output["model_kind"], "heuristic_dependency_net");
                assert!(
                    output["baseline_guidance"]
                        .as_str()
                        .unwrap()
                        .contains("cannot be used")
                );
            } else {
                assert!(
                    output["baseline_guidance"]
                        .as_str()
                        .unwrap()
                        .contains("same data")
                );
                assert!(
                    output["net"]["transitions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|transition| transition["activity"].as_str())
                        .all(|activity| activity.starts_with("activity_"))
                );
            }
        }

        let bad_parameter = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "discover_process_model",
                    "arguments": {
                        "project": project,
                        "algorithm": "alpha",
                        "noise_threshold_milli": 100
                    }
                }),
            ))
            .unwrap();
        assert_eq!(bad_parameter["result"]["isError"], true);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn case_conformance_uses_an_explicit_opaque_baseline_and_bounds_details() {
        let (root, server) = formal_model_fixture();
        let listed = server
            .handle(request(
                1,
                "tools/call",
                json!({ "name": "list_process_models", "arguments": {} }),
            ))
            .unwrap();
        let list_output = &listed["result"]["structuredContent"];
        assert_eq!(list_output["count"], 1);
        let model = list_output["models"][0]["model"].as_str().unwrap();
        assert!(model.starts_with("case_model_"));
        assert!(!listed.to_string().contains("approved.model.json"));
        let project = server.project_handle("claims.json");

        let replay = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "check_process_conformance",
                    "arguments": { "project": project, "model": model, "detail_limit": 1 }
                }),
            ))
            .unwrap();
        assert_eq!(replay["result"]["isError"], false, "{replay:#}");
        let output = &replay["result"]["structuredContent"];
        assert_eq!(output["view"]["cases_shown"], 2);
        assert_eq!(output["view"]["cases_total"], 3);
        assert_eq!(output["fit_cases"], 1);
        assert_eq!(output["deviating_cases"], 1);
        assert_eq!(output["cases"]["available"], 1);
        assert_eq!(output["cases"]["returned"], 1);
        assert!(
            output["cases"]["items"][0]["case"]
                .as_str()
                .unwrap()
                .starts_with("case_")
        );
        assert!(
            output["cases"]["items"][0]["deviations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|deviation| deviation["source_row"].is_null())
        );
        let replay_text = replay.to_string();
        for private in [
            "C1",
            "C2",
            "C3",
            "Private Exception",
            "approved-process.bpmn",
            "approved.model.json",
            "private-claims.csv",
        ] {
            assert!(!replay_text.contains(private), "leaked {private}");
        }

        let alignment = server
            .handle(request(
                3,
                "tools/call",
                json!({
                    "name": "align_process_conformance",
                    "arguments": { "project": project, "model": model, "detail_limit": 1 }
                }),
            ))
            .unwrap();
        assert_eq!(alignment["result"]["isError"], false, "{alignment:#}");
        assert_eq!(alignment["result"]["structuredContent"]["fit_cases"], 1);
        assert_eq!(
            alignment["result"]["structuredContent"]["deviating_cases"],
            1
        );
        assert!(
            alignment["result"]["structuredContent"]["total_cost"]
                .as_u64()
                .unwrap()
                > 0
        );

        let too_small = server
            .handle(request(
                4,
                "tools/call",
                json!({
                    "name": "check_process_conformance",
                    "arguments": { "project": project, "model": model, "max_model_items": 6 }
                }),
            ))
            .unwrap();
        assert_eq!(too_small["result"]["isError"], true);
        assert!(
            too_small["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("above max_model_items=6")
        );

        let arbitrary_path = server
            .handle(request(
                5,
                "tools/call",
                json!({
                    "name": "check_process_conformance",
                    "arguments": { "project": project, "model": "../approved.model.json" }
                }),
            ))
            .unwrap();
        assert_eq!(arbitrary_path["result"]["isError"], true);
        assert!(
            arbitrary_path["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("opaque identifier")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn activity_omission_blocks_discovery_but_preserves_conformance_aggregates() {
        let (root, _) = formal_model_fixture();
        let mut policy = ExposurePolicy::safe();
        policy.activity_names = Mode::Omit;
        let server = Server::with_exposure_policy(&root, policy).unwrap();
        let project = server.project_handle("claims.json");
        let model = server.process_model_handle("approved.model.json");
        let discovery = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "discover_process_model",
                    "arguments": { "project": project }
                }),
            ))
            .unwrap();
        assert_eq!(discovery["result"]["isError"], true);
        assert!(
            discovery["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("cannot be returned without changing its meaning")
        );

        let replay = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "check_process_conformance",
                    "arguments": { "project": project, "model": model }
                }),
            ))
            .unwrap();
        assert_eq!(replay["result"]["isError"], false);
        let output = &replay["result"]["structuredContent"];
        assert_eq!(output["fit_cases"], 1);
        assert_eq!(output["deviating_cases"], 1);
        assert!(
            output["cases"]["items"][0]["deviations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|deviation| deviation["activity"].is_null())
        );
        assert!(!replay.to_string().contains("Private Exception"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_case_model_fails_without_disclosing_its_local_path() {
        let (root, server) = formal_model_fixture();
        std::fs::write(root.join("broken.model.json"), br#"{"source":42}"#).unwrap();
        let response = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "check_process_conformance",
                    "arguments": {
                        "project": server.project_handle("claims.json"),
                        "model": server.process_model_handle("broken.model.json")
                    }
                }),
            ))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("model has no string 'source' field"));
        assert!(!text.contains("broken.model.json"));
        assert!(!text.contains(&root.display().to_string()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn live_case_discovery_uses_the_browser_view_and_discloses_missing_baseline_selection() {
        let (root, _) = formal_model_fixture();
        let project = Project::load(root.join("claims.json")).unwrap();
        let server = Server::with_live_workspace(
            LiveWorkspace {
                case_project: Some(LiveProject {
                    project,
                    source_name: "private-claims.csv".to_owned(),
                }),
                ..LiveWorkspace::default()
            },
            ExposurePolicy::safe(),
        )
        .unwrap();
        let project = server.project_handle("live:private-claims.csv");
        let discovered = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "discover_process_model",
                    "arguments": { "project": project }
                }),
            ))
            .unwrap();
        assert_eq!(discovered["result"]["isError"], false);
        assert_eq!(
            discovered["result"]["structuredContent"]["view"]["cases_shown"],
            2
        );
        let models = server
            .handle(request(
                2,
                "tools/call",
                json!({ "name": "list_process_models", "arguments": {} }),
            ))
            .unwrap();
        assert_eq!(models["result"]["structuredContent"]["count"], 0);
        assert!(
            models["result"]["structuredContent"]["availability_note"]
                .as_str()
                .unwrap()
                .contains("no selected case-centric baseline")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lists_only_valid_saved_projects_deterministically() {
        let (root, server) = fixture();
        std::fs::write(root.join("not-a-project.json"), "{}").unwrap();
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "list_projects", "arguments": {}
                }),
            ))
            .unwrap();
        assert_eq!(result["result"]["structuredContent"]["count"], 1);
        let handle = result["result"]["structuredContent"]["projects"][0]["project"]
            .as_str()
            .unwrap();
        assert!(handle.starts_with("project_"));
        assert!(!result.to_string().contains("orders.json"));
        assert!(!result.to_string().contains("orders.csv"));
        assert!(!result.to_string().contains("Orders"));

        let inspected = server
            .handle(request(
                2,
                "tools/call",
                json!({ "name": "inspect_log", "arguments": { "project": handle } }),
            ))
            .unwrap();
        assert_eq!(inspected["result"]["isError"], false);
    }

    #[test]
    fn inspection_is_bounded_and_does_not_expose_values() {
        let (_root, server) = fixture();
        let handle = server.project_handle("orders.json");
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "inspect_log", "arguments": { "project": handle }
                }),
            ))
            .unwrap();
        let output = &result["result"]["structuredContent"];
        assert_eq!(output["rows_sampled"], 1);
        assert_eq!(output["examples_exposed"], false);
        assert!(!result.to_string().contains("private"));
        assert!(!result.to_string().contains("orders.csv"));
        assert!(
            output["mapping"]["case_id"]
                .as_str()
                .unwrap()
                .starts_with("column_")
        );
        assert!(output["columns"].as_array().unwrap().iter().all(|column| {
            column["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("column_"))
        }));
    }

    #[test]
    fn mapping_validation_reports_decisions_without_source_values() {
        let (_root, server) = validation_fixture();
        let handle = server.project_handle("sensitive.json");
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "validate_mapping",
                    "arguments": { "project": handle }
                }),
            ))
            .unwrap();
        let output = &result["result"]["structuredContent"];
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(output["claim_type"], "calculated_fact");
        assert_eq!(output["ready_for_analysis"], false);
        assert_eq!(output["summary"]["open_decisions"], 1);
        assert_eq!(output["findings"][0]["kind"], "blank_timestamps");
        assert_eq!(output["findings"][0]["decision"]["required"], true);
        assert!(
            output["mapping"]["case_id"]
                .as_str()
                .is_some_and(|column| column.starts_with("column_"))
        );
        assert_eq!(output["examples_exposed"], false);
        assert!(!result.to_string().contains("Private Review"));
        assert!(!result.to_string().contains("sensitive.csv"));
        assert!(!result.to_string().contains("row 2"));
    }

    #[test]
    fn project_ids_cannot_escape_the_configured_root() {
        let (_root, server) = fixture();
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "inspect_log", "arguments": { "project": "../orders.json" }
                }),
            ))
            .unwrap();
        assert_eq!(result["result"]["isError"], true);
        assert!(
            result["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("opaque identifier")
        );
    }

    #[test]
    fn notifications_do_not_receive_responses() {
        let (_root, server) = fixture();
        assert!(
            server
                .handle(json!({
                    "jsonrpc": "2.0", "method": "notifications/initialized"
                }))
                .is_none()
        );
    }

    #[test]
    fn local_preview_shows_original_and_outbound_sides() {
        let (root, _server) = fixture();
        let project = Project::load(root.join("orders.json")).unwrap();
        let policy = ExposurePolicy::safe();
        let preview = preview_project(&project, &policy).unwrap();
        let activity = preview
            .fields
            .iter()
            .find(|field| field.role == "activity")
            .unwrap();
        assert_eq!(activity.examples[0].local_value, "Start");
        assert_ne!(
            activity.examples[0].outbound_value.as_deref(),
            Some("Start")
        );
        assert_eq!(preview.source_file.local_value, "orders.csv");
        assert_eq!(preview.source_file.outbound_value, None);
    }

    #[test]
    fn mcp_reports_modes_but_never_the_alias_key() {
        let (_root, server) = fixture();
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({ "name": "get_exposure_policy", "arguments": {} }),
            ))
            .unwrap();
        assert_eq!(
            result["result"]["structuredContent"]["policy"]["case_ids"],
            "alias"
        );
        assert!(!result.to_string().contains(&server.exposure.alias_key));
    }

    #[test]
    fn summary_and_variants_are_calculated_bounded_and_private_by_default() {
        let (_root, server) = fixture();
        let handle = server.project_handle("orders.json");
        let summary = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "get_process_summary",
                    "arguments": { "project": handle }
                }),
            ))
            .unwrap();
        let output = &summary["result"]["structuredContent"];
        assert_eq!(summary["result"]["isError"], false);
        assert_eq!(output["claim_type"], "calculated_fact");
        assert_eq!(output["summary"]["events"], 1);
        assert_eq!(output["summary"]["cases"], 1);
        assert_eq!(output["timing"]["transition_duration"], "elapsed");
        assert!(
            output["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("project_name_"))
        );
        assert!(!summary.to_string().contains("orders.csv"));
        assert!(!summary.to_string().contains("Orders"));

        let variants = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "get_variants",
                    "arguments": { "project": handle, "limit": 1 }
                }),
            ))
            .unwrap();
        let output = &variants["result"]["structuredContent"];
        assert_eq!(variants["result"]["isError"], false);
        assert_eq!(output["returned"], 1);
        assert_eq!(output["variants"][0]["activity_count"], 1);
        assert!(
            output["variants"][0]["sequence"][0]
                .as_str()
                .is_some_and(|activity| activity.starts_with("activity_"))
        );
        assert!(
            output["variants"][0]["example_cases"][0]
                .as_str()
                .is_some_and(|case| case.starts_with("case_"))
        );
        assert!(!variants.to_string().contains("Start"));
        assert!(!variants.to_string().contains("C1"));

        let map = server
            .handle(request(
                3,
                "tools/call",
                json!({ "name": "get_process_map", "arguments": { "project": handle } }),
            ))
            .unwrap();
        let output = &map["result"]["structuredContent"];
        assert_eq!(map["result"]["isError"], false);
        assert_eq!(output["claim_type"], "calculated_fact");
        assert_eq!(output["selection"]["variants"], json!([1]));
        assert!(output["cases_covered"].as_u64().unwrap() >= 1);
        assert!(
            output["top_route"][0]
                .as_str()
                .is_some_and(|activity| activity.starts_with("activity_"))
        );
        assert!(!map.to_string().contains("Start"));
        assert!(!map.to_string().contains("C1"));
        let slowest = server
            .handle(request(
                5,
                "tools/call",
                json!({ "name": "get_slowest_cases", "arguments": { "project": handle, "limit": 1 } }),
            ))
            .unwrap();
        let output = &slowest["result"]["structuredContent"];
        assert_eq!(output["returned"], 1);
        assert!(
            output["cases"][0]["case"]
                .as_str()
                .is_some_and(|case| case.starts_with("case_"))
        );
        assert!(!slowest.to_string().contains("C1"));
        let both = server
            .handle(request(
                4,
                "tools/call",
                json!({ "name": "get_process_map", "arguments": { "project": handle, "variants": [1], "activities": 2 } }),
            ))
            .unwrap();
        assert!(both.to_string().contains("not both"));
    }

    #[test]
    fn run_analysis_returns_a_compact_private_result_manifest() {
        let (_root, server) = finding_fixture();
        let handle = server.project_handle("claims.json");
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "run_analysis",
                    "arguments": { "project": handle }
                }),
            ))
            .unwrap();
        let output = &result["result"]["structuredContent"];
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(output["status"], "complete");
        assert_eq!(output["claim_type"], "calculated_fact");
        assert_eq!(output["summary"]["cases"], 3);
        assert_eq!(output["results"]["variants"]["available"], true);
        assert_eq!(output["results"]["variants"]["tool"], "get_variants");
        assert_eq!(output["results"]["rework"]["available"], true);
        assert_eq!(output["results"]["automation_targeting"]["available"], true);
        assert_eq!(
            output["results"]["formal_discovery"]["tool"],
            "discover_process_model"
        );
        assert_eq!(output["results"]["process_conformance"]["available"], false);
        assert!(!result.to_string().contains("Request Documents"));
        assert!(!result.to_string().contains("claims.csv"));
        assert!(!result.to_string().contains("C1"));
    }

    #[test]
    fn value_totals_follow_the_exposure_policy() {
        let root = std::env::temp_dir().join(format!(
            "pm_mcp_values_{}_{}",
            std::process::id(),
            FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("invoices.csv");
        std::fs::write(&source, "case_id,activity,timestamp,amount,region\nI1,Receive,2026-01-01 09:00:00,75000,East\nI1,Pay,2026-01-02 09:00:00,75000,East\nI2,Receive,2026-01-01 09:00:00,52000,West\nI3,Receive,2026-01-01 09:00:00,900,West\nI3,Pay,2026-01-02 09:00:00,900,West\n").unwrap();
        Project::new(
            &source,
            Mapping::new("case_id", "activity", "timestamp")
                .with_case_attributes(vec!["amount".to_owned(), "region".to_owned()]),
            Policy::new(),
            pm_ingest::Rules::new(),
        )
        .unwrap()
        .named("Invoices")
        .save(root.join("invoices.json"))
        .unwrap();
        let call = |server: &Server, arguments: Value| {
            server
                .handle(request(
                    1,
                    "tools/call",
                    json!({"name":"summarize_values","arguments":arguments}),
                ))
                .unwrap()["result"]
                .clone()
        };

        let server = Server::with_exposure_policy(&root, ExposurePolicy::safe()).unwrap();
        let handle = server.project_handle("invoices.json");
        let listed = call(&server, json!({"project":handle}));
        let fields = &listed["structuredContent"]["number_fields"];
        assert_eq!(fields.as_array().unwrap().len(), 1, "{listed}");
        let amount = fields[0]["field"].as_str().unwrap().to_owned();
        assert_ne!(amount, "amount");
        let result = call(
            &server,
            json!({"project":handle,"field":amount,"at_least":50000,"group_by":"last_step"}),
        );
        assert_eq!(result["isError"], false, "{result}");
        let output = &result["structuredContent"];
        assert_eq!(output["overall"]["cases"], 2);
        assert_eq!(output["overall"]["total"], 127_000.0);
        assert_eq!(output["groups"].as_array().unwrap().len(), 2);
        assert!(!result.to_string().contains("Receive"));
        let grouped = call(
            &server,
            json!({"project":handle,"field":amount,"group_by":"attribute","group_attribute":amount}),
        );
        assert_eq!(grouped["isError"], true);

        let mut hidden = ExposurePolicy::safe();
        hidden.costs = Mode::Omit;
        let server = Server::with_exposure_policy(&root, hidden).unwrap();
        let handle = server.project_handle("invoices.json");
        let amount = server
            .exposure
            .transform(Mode::Alias, "attribute", "amount")
            .unwrap();
        let refused = call(&server, json!({"project":handle,"field":amount}));
        assert_eq!(refused["isError"], true);
        assert!(refused.to_string().contains("exposure policy"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn answers_are_held_to_the_agent_budget_unless_full_is_asked() {
        let big = json!({"catalog": {"event_types": ["a", "b"]},
            "routes": (0..2000).map(|n| json!({"rank": n})).collect::<Vec<_>>()});
        let fitted = fit_tool_result(tool_result(Ok(big.clone())), "get_variants");
        let output = &fitted["structuredContent"];
        assert!(
            output.to_string().len() < 5000,
            "{}",
            output.to_string().len()
        );
        assert_eq!(
            output["catalog"]["event_types"].as_array().unwrap().len(),
            2
        );
        let kept = output["routes"].as_array().unwrap().len() as u64;
        assert_eq!(kept + output["routes_omitted"].as_u64().unwrap(), 2000);
        assert!(kept < 500);
        assert!(
            output["shortened"]["how_to_get_everything"]
                .as_str()
                .unwrap()
                .contains("\"detail\": \"full\"")
        );
        // The text copy is the same compact JSON.
        assert_eq!(fitted["content"][0]["text"], output.to_string());
        // Lists of what exists stay whole.
        assert!(!budgeted("list_projects") && budgeted("analyze_ocel"));

        let (_root, server) = comparison_fixture();
        let handle = server.project_handle("regions.json");
        let listed = server.handle(request(1, "tools/list", json!({}))).unwrap();
        let variants = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "get_variants")
            .unwrap();
        assert_eq!(
            variants["inputSchema"]["properties"]["detail"]["enum"],
            json!(["summary", "full"])
        );
        let full = server
            .handle(request(
                2,
                "tools/call",
                json!({"name":"get_variants","arguments":{"project":handle,"detail":"full"}}),
            ))
            .unwrap();
        assert_eq!(full["result"]["isError"], false);
        let wrong = server
            .handle(request(
                3,
                "tools/call",
                json!({"name":"get_variants","arguments":{"project":handle,"detail":"all"}}),
            ))
            .unwrap();
        assert!(wrong.to_string().contains("detail must be"));
    }

    #[test]
    fn aliased_cohort_comparison_is_bounded_and_labelled_as_association() {
        let (_root, server) = comparison_fixture();
        let handle = server.project_handle("regions.json");
        let attribute = server
            .exposure
            .transform(Mode::Alias, "attribute", "region")
            .unwrap();
        let left = server
            .exposure
            .transform(Mode::Alias, "attribute_value", "East")
            .unwrap();
        let right = server
            .exposure
            .transform(Mode::Alias, "attribute_value", "West")
            .unwrap();
        let choices = server.get_attribute_choices(&handle, &attribute).unwrap();
        assert_eq!(choices["values"].as_array().unwrap().len(), 2);
        assert!(!choices.to_string().contains("East"));
        assert!(!choices.to_string().contains("West"));
        let result = server.handle(request(1, "tools/call", json!({"name":"compare_cohorts","arguments":{"project":handle,"attribute":attribute,"left":left,"right":right,"limit":1}}))).unwrap();
        let output = &result["result"]["structuredContent"];
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(output["claim_type"], "statistical_association");
        assert_eq!(output["left"]["cases"], 2);
        assert_eq!(output["right"]["cases"], 2);
        assert_eq!(output["activity_differences_returned"], 1);
        assert_eq!(
            output["activity_differences"][0]["difference_points"]
                .as_f64()
                .unwrap()
                .abs(),
            100.0
        );
        assert!(!result.to_string().contains("East"));
        assert!(!result.to_string().contains("West"));
        assert!(!result.to_string().contains("Escalate"));
        assert!(!result.to_string().contains("W1"));
    }

    #[test]
    fn exploration_transforms_group_names_values_and_evidence() {
        let (_root, server) = comparison_fixture();
        let handle = server.project_handle("regions.json");
        let attribute = server
            .exposure
            .transform(Mode::Alias, "attribute", "region")
            .unwrap();
        let result = server
            .explore_process(&json!({"project":handle,"group_by":attribute}))
            .unwrap();
        assert_eq!(result["overall"]["items"], 4);
        let text = result.to_string();
        for raw in ["East", "West", "Escalate", "E1", "W1"] {
            assert!(!text.contains(raw), "{raw} leaked");
        }
        assert!(
            result["groups"][0]["value"]
                .as_str()
                .unwrap()
                .starts_with("attribute_value_")
        );
        assert!(
            result["overall"]["examples"][0]
                .as_str()
                .unwrap()
                .starts_with("case_")
        );
    }

    #[test]
    fn period_comparison_uses_exact_windows_and_private_evidence() {
        let (_root, server) = comparison_fixture();
        let handle = server.project_handle("regions.json");
        let (_project, loaded) = server.load_project(&handle).unwrap();
        let starts = loaded
            .log
            .traces
            .iter()
            .map(|trace| trace.start())
            .collect::<Vec<_>>();
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "compare_periods",
                    "arguments": {
                        "project": handle,
                        "baseline_start": starts[0],
                        "baseline_end": starts[1],
                        "comparison_start": starts[2],
                        "comparison_end": starts[3],
                        "limit": 1
                    }
                }),
            ))
            .unwrap();
        let output = &result["result"]["structuredContent"];
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(output["claim_type"], "statistical_association");
        assert_eq!(output["baseline"]["cases"], 2);
        assert_eq!(output["comparison"]["cases"], 2);
        assert_eq!(output["activity_differences_returned"], 1);
        assert_eq!(
            output["baseline"]["started_at_or_after_unix_micros"],
            starts[0]
        );
        assert!(!result.to_string().contains("Escalate"));
        assert!(!result.to_string().contains("W1"));
        assert!(!result.to_string().contains("2026-01-03"));

        let invalid = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "compare_periods",
                    "arguments": {
                        "project": handle,
                        "baseline_start": starts[0],
                        "baseline_end": starts[2],
                        "comparison_start": starts[2],
                        "comparison_end": starts[3]
                    }
                }),
            ))
            .unwrap();
        assert_eq!(invalid["result"]["isError"], true);
    }

    #[test]
    fn variant_limit_and_omission_modes_are_enforced() {
        let (root, _server) = fixture();
        let mut policy = ExposurePolicy::safe();
        policy.activity_names = Mode::Omit;
        policy.case_ids = Mode::Omit;
        let server = Server::with_exposure_policy(&root, policy).unwrap();
        let handle = server.project_handle("orders.json");
        let omitted = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "get_variants",
                    "arguments": { "project": handle }
                }),
            ))
            .unwrap();
        let variant = &omitted["result"]["structuredContent"]["variants"][0];
        assert!(variant["sequence"].is_null());
        assert_eq!(variant["activity_count"], 1);
        assert_eq!(variant["example_cases"], json!([]));

        let rejected = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "get_variants",
                    "arguments": { "project": handle, "limit": 21 }
                }),
            ))
            .unwrap();
        assert_eq!(rejected["error"]["code"], -32602);
    }

    #[test]
    fn drivers_answer_with_method_and_counts_even_when_there_are_too_few_items() {
        let (_root, server) = finding_fixture();
        let handle = server.project_handle("claims.json");
        let drivers = server
            .handle(request(
                1,
                "tools/call",
                json!({ "name": "find_drivers", "arguments": { "project": handle } }),
            ))
            .unwrap();
        let output = &drivers["result"]["structuredContent"];
        assert_eq!(drivers["result"]["isError"], false);
        assert_eq!(output["claim_type"], "calculated_fact");
        assert_eq!(
            output["returned"], 0,
            "a handful of items is too few to rank fields"
        );
        assert!(
            output["method"]
                .as_str()
                .is_some_and(|text| text.contains("slowest fifth"))
        );
        assert!(
            output["interpretation_note"]
                .as_str()
                .is_some_and(|text| text.contains("not proven causes"))
        );
    }

    #[test]
    fn bottlenecks_and_rework_are_evidence_bounded_and_private() {
        let (_root, server) = finding_fixture();
        let handle = server.project_handle("claims.json");
        let bottlenecks = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "find_bottlenecks",
                    "arguments": { "project": handle, "limit": 1 }
                }),
            ))
            .unwrap();
        let output = &bottlenecks["result"]["structuredContent"];
        assert_eq!(bottlenecks["result"]["isError"], false);
        assert_eq!(output["claim_type"], "calculated_fact");
        assert_eq!(output["returned"], 1);
        assert_eq!(output["timing"]["transition_duration"], "elapsed");
        assert!(
            output["bottlenecks"][0]["from"]
                .as_str()
                .is_some_and(|activity| activity.starts_with("activity_"))
        );
        assert!(
            output["bottlenecks"][0]["example_cases"][0]
                .as_str()
                .is_some_and(|case| case.starts_with("case_"))
        );
        assert!(!bottlenecks.to_string().contains("Request Documents"));
        assert!(!bottlenecks.to_string().contains("C1"));

        let rework = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "find_rework",
                    "arguments": {
                        "project": handle,
                        "activity_limit": 1,
                        "loop_limit": 1
                    }
                }),
            ))
            .unwrap();
        let output = &rework["result"]["structuredContent"];
        assert_eq!(rework["result"]["isError"], false);
        assert_eq!(output["summary"]["cases_affected"], 2);
        assert_eq!(output["activities_returned"], 1);
        assert_eq!(output["loops_returned"], 1);
        assert_eq!(output["loops"][0]["activity_count"], 3);
        assert!(
            output["activities"][0]["activity"]
                .as_str()
                .is_some_and(|activity| activity.starts_with("activity_"))
        );
        assert!(!rework.to_string().contains("Review"));
        assert!(!rework.to_string().contains("C1"));
    }

    #[test]
    fn automation_targeting_is_bounded_private_and_not_a_verdict() {
        let (_root, server) = automation_fixture();
        let handle = server.project_handle("invoices.json");
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "get_automation_targeting",
                    "arguments": { "project": handle, "limit": 1 }
                }),
            ))
            .unwrap();
        let output = &result["result"]["structuredContent"];
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(output["claim_type"], "calculated_fact");
        assert_eq!(output["classification"]["settled"], true);
        assert_eq!(output["classification"]["not_yet_classified_events"], 0);
        assert!(output["classification"]["next_step"].is_null());
        assert_eq!(output["classification"]["straight_through_cases"], 1);
        assert_eq!(output["classification"]["human_touched_cases"], 2);
        assert_eq!(output["manual_steps_returned"], 1);
        assert_eq!(output["manual_steps"][0]["processing_seconds"], 1200);
        assert_eq!(output["manual_steps"][0]["processing_observations"], 2);
        assert_eq!(output["manual_steps"][0]["waiting_before_seconds"], 480);
        assert_eq!(
            output["manual_steps"][0]["total_cost_microunits"],
            "25000000"
        );
        assert!(
            output["manual_steps"][0]["activity"]
                .as_str()
                .is_some_and(|activity| activity.starts_with("activity_"))
        );
        assert!(
            output["interpretation_note"]
                .as_str()
                .unwrap()
                .contains("not a recommendation")
        );
        assert!(!result.to_string().contains("Fix Data"));
        assert!(!result.to_string().contains("alice"));
        assert!(!result.to_string().contains("C2"));
    }

    #[test]
    fn aliased_target_activity_round_trips_into_context() {
        let (_root, server) = automation_fixture();
        let handle = server.project_handle("invoices.json");
        let targeting = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "get_automation_targeting",
                    "arguments": { "project": handle }
                }),
            ))
            .unwrap();
        let activity = targeting["result"]["structuredContent"]["manual_steps"][0]["activity"]
            .as_str()
            .unwrap();
        let context = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "get_activity_context",
                    "arguments": { "project": handle, "activity": activity }
                }),
            ))
            .unwrap();
        let output = &context["result"]["structuredContent"];
        assert_eq!(context["result"]["isError"], false);
        assert_eq!(output["activity"], activity);
        assert_eq!(output["occurrences"], 2);
        assert_eq!(output["cases"], 2);
        assert_eq!(output["manual_evidence"]["processing_seconds"], 1200);
        assert_eq!(output["connected_transitions_returned"], 2);
        assert_eq!(output["timing"]["transition_duration"], "waiting");
        assert!(!context.to_string().contains("Fix Data"));
    }

    #[test]
    fn omitted_activity_names_block_activity_context() {
        let (root, _server) = automation_fixture();
        let mut policy = ExposurePolicy::safe();
        policy.activity_names = Mode::Omit;
        let server = Server::with_exposure_policy(&root, policy).unwrap();
        let handle = server.project_handle("invoices.json");
        let context = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "get_activity_context",
                    "arguments": { "project": handle, "activity": "Fix Data" }
                }),
            ))
            .unwrap();
        assert_eq!(context["result"]["isError"], true);
        assert!(
            context["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("omitted")
        );
    }

    #[test]
    fn finding_omission_modes_and_limits_do_not_reconnect_hidden_activities() {
        let (root, _server) = finding_fixture();
        let mut policy = ExposurePolicy::safe();
        policy.activity_names = Mode::Omit;
        policy.case_ids = Mode::Omit;
        let server = Server::with_exposure_policy(&root, policy).unwrap();
        let handle = server.project_handle("claims.json");
        let bottlenecks = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "find_bottlenecks",
                    "arguments": { "project": handle }
                }),
            ))
            .unwrap();
        let bottleneck = &bottlenecks["result"]["structuredContent"]["bottlenecks"][0];
        assert!(bottleneck["from"].is_null());
        assert!(bottleneck["to"].is_null());
        assert_eq!(bottleneck["example_cases"], json!([]));

        let rework = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "find_rework",
                    "arguments": { "project": handle }
                }),
            ))
            .unwrap();
        let output = &rework["result"]["structuredContent"];
        assert!(output["activities"][0]["activity"].is_null());
        assert!(output["loops"][0]["sequence"].is_null());
        assert_eq!(output["loops"][0]["activity_count"], 3);
        assert_eq!(output["loops"][0]["example_cases"], json!([]));

        let rejected = server
            .handle(request(
                3,
                "tools/call",
                json!({
                    "name": "find_rework",
                    "arguments": { "project": handle, "activity_limit": 21 }
                }),
            ))
            .unwrap();
        assert_eq!(rejected["error"]["code"], -32602);
    }

    #[test]
    fn aliased_variant_evidence_flows_into_private_case_inspection() {
        let (_root, server) = finding_fixture();
        let handle = server.project_handle("claims.json");
        let variants = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "get_variants",
                    "arguments": { "project": handle, "limit": 1 }
                }),
            ))
            .unwrap();
        let sequence = variants["result"]["structuredContent"]["variants"][0]["sequence"].clone();
        let evidence = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "get_evidence",
                    "arguments": {
                        "project": handle,
                        "kind": "variant",
                        "sequence": sequence,
                        "limit": 1
                    }
                }),
            ))
            .unwrap();
        let output = &evidence["result"]["structuredContent"];
        assert_eq!(evidence["result"]["isError"], false);
        assert_eq!(output["claim_type"], "calculated_fact");
        assert_eq!(output["shown"], 1);
        let exposed_case = output["cases"][0].as_str().unwrap();
        assert!(exposed_case.starts_with("case_"));

        let inspected = server
            .handle(request(
                3,
                "tools/call",
                json!({
                    "name": "inspect_case",
                    "arguments": { "project": handle, "case": exposed_case }
                }),
            ))
            .unwrap();
        let detail = &inspected["result"]["structuredContent"];
        assert_eq!(inspected["result"]["isError"], false);
        assert_eq!(detail["case"], exposed_case);
        assert!(
            detail["events"][0]["activity"]
                .as_str()
                .is_some_and(|activity| activity.starts_with("activity_"))
        );
        assert!(detail["events"][0]["timestamp"].is_null());
        assert!(detail["events"][0]["source_row"].is_null());
        assert!(detail["source_rows"].as_array().unwrap().is_empty());
        assert!(detail["timestamp_reading"].is_null());
        assert!(!inspected.to_string().contains("Submit"));
        assert!(!inspected.to_string().contains("2026-01-01"));
        assert!(!inspected.to_string().contains("C1"));
    }

    #[test]
    fn evidence_respects_the_saved_filter_and_reports_full_match_count() {
        let (root, server) = finding_fixture();
        let project_path = root.join("claims.json");
        let mut project = Project::load(&project_path).unwrap();
        project.view_filter = pm_core::Filter::new().with_activity("Request Documents");
        project.save(&project_path).unwrap();
        let handle = server.project_handle("claims.json");
        let activity = server
            .exposure
            .transform(Mode::Alias, "activity", "Review")
            .unwrap();
        let evidence = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "get_evidence",
                    "arguments": {
                        "project": handle,
                        "kind": "activity",
                        "activity": activity,
                        "limit": 1
                    }
                }),
            ))
            .unwrap();
        let output = &evidence["result"]["structuredContent"];
        assert_eq!(output["view"]["cases_shown"], 2);
        assert_eq!(output["view"]["cases_total"], 3);
        assert_eq!(output["view"]["narrowed"], true);
        assert_eq!(output["total"], 2);
        assert_eq!(output["shown"], 1);
        assert_eq!(output["bounded"], true);

        let excluded_case = server
            .exposure
            .transform(Mode::Alias, "case", "C2")
            .unwrap();
        let inspected = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "inspect_case",
                    "arguments": { "project": handle, "case": excluded_case }
                }),
            ))
            .unwrap();
        assert_eq!(inspected["result"]["isError"], true);
        assert!(
            inspected["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("saved active view")
        );
    }

    #[test]
    fn mcp_reproduces_the_saved_composable_trace_view() {
        let (root, server) = finding_fixture();
        let project_path = root.join("claims.json");
        let mut project = Project::load(&project_path).unwrap();
        project.view_filter = pm_core::Filter::new()
            .starts_with("Submit")
            .ends_with("Approve")
            .directly_follows("Submit", "Review")
            .eventually_follows("Request Documents", "Approve");
        project.save(&project_path).unwrap();
        let handle = server.project_handle("claims.json");

        let summary = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "get_process_summary",
                    "arguments": { "project": handle }
                }),
            ))
            .unwrap();
        let output = &summary["result"]["structuredContent"];
        assert_eq!(summary["result"]["isError"], false);
        assert_eq!(output["view"]["cases_shown"], 2);
        assert_eq!(output["view"]["cases_total"], 3);
        assert_eq!(output["summary"]["events"], 10);
        assert_eq!(output["summary"]["variants"], 1);

        let variants = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "get_variants",
                    "arguments": { "project": handle, "limit": 10 }
                }),
            ))
            .unwrap();
        let output = &variants["result"]["structuredContent"];
        assert_eq!(output["view"]["cases_shown"], 2);
        assert_eq!(output["returned"], 1);
        assert_eq!(output["variants"][0]["cases"], 2);
    }

    #[test]
    fn detailed_show_modes_are_explicitly_enforced() {
        let (root, _server) = finding_fixture();
        let mut policy = ExposurePolicy::safe();
        policy.project_names = Mode::Show;
        policy.activity_names = Mode::Show;
        policy.case_ids = Mode::Show;
        policy.timestamps = Mode::Show;
        policy.source_rows = Mode::Show;
        let server = Server::with_exposure_policy(&root, policy).unwrap();
        let handle = server.project_handle("claims.json");
        let evidence = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "get_evidence",
                    "arguments": {
                        "project": handle,
                        "kind": "transition",
                        "from": "Submit",
                        "to": "Review"
                    }
                }),
            ))
            .unwrap();
        assert_eq!(evidence["result"]["structuredContent"]["total"], 3);
        let case = evidence["result"]["structuredContent"]["cases"][0]
            .as_str()
            .unwrap();
        let inspected = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "inspect_case",
                    "arguments": { "project": handle, "case": case }
                }),
            ))
            .unwrap();
        let detail = &inspected["result"]["structuredContent"];
        assert_eq!(detail["case"], "C1");
        assert_eq!(detail["events"][0]["activity"], "Submit");
        assert_eq!(detail["events"][0]["timestamp"], "2026-01-01 09:00:00");
        assert_eq!(detail["events"][0]["source_row"], "2");
        assert_eq!(
            detail["timestamp_reading"],
            "exactly as recorded, with no timezone applied"
        );
    }

    #[test]
    fn omitted_identifiers_cannot_be_used_to_bypass_drilldown_policy() {
        let (root, _server) = finding_fixture();
        let mut policy = ExposurePolicy::safe();
        policy.activity_names = Mode::Omit;
        policy.case_ids = Mode::Omit;
        let server = Server::with_exposure_policy(&root, policy).unwrap();
        let handle = server.project_handle("claims.json");
        let evidence = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "get_evidence",
                    "arguments": {
                        "project": handle,
                        "kind": "activity",
                        "activity": "Review"
                    }
                }),
            ))
            .unwrap();
        assert_eq!(evidence["result"]["isError"], true);
        assert!(
            evidence["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("omitted")
        );

        let inspected = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "inspect_case",
                    "arguments": { "project": handle, "case": "C1" }
                }),
            ))
            .unwrap();
        assert_eq!(inspected["result"]["isError"], true);
    }

    #[test]
    fn tool_catalogue_describes_summary_and_variant_costs() {
        let (_root, server) = fixture();
        let listed = server.handle(request(1, "tools/list", json!({}))).unwrap();
        let tools = listed["result"]["tools"].as_array().unwrap();
        for name in [
            "validate_mapping",
            "run_analysis",
            "get_process_summary",
            "get_variants",
            "get_process_map",
            "get_slowest_cases",
            "find_bottlenecks",
            "find_rework",
            "get_automation_targeting",
            "get_activity_context",
            "compare_cohorts",
            "compare_periods",
            "get_evidence",
            "inspect_case",
        ] {
            let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
            let description = tool["description"].as_str().unwrap().to_lowercase();
            assert!(description.contains("scans the source"));
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
        }
    }

    #[test]
    fn object_centric_tools_are_discoverable_private_and_filterable_by_alias() {
        let (_root, server) = ocel_fixture();
        let tools = server.handle(request(1, "tools/list", json!({}))).unwrap();
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "analyze_ocel")
        );
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "inspect_ocel_object")
        );
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "measure_ocel_time_between")
        );
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "analyze_ocel_synchronization")
        );
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "rank_ocel_bottlenecks")
        );
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "project_ocel_cases")
        );
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "discover_ocel_model")
        );
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "list_ocel_models")
        );
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "check_ocel_conformance")
        );
        assert!(
            tools["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "align_ocel_conformance")
        );

        let listed = server
            .handle(request(
                2,
                "tools/call",
                json!({ "name": "list_ocel_logs", "arguments": {} }),
            ))
            .unwrap();
        let handle = listed["result"]["structuredContent"]["logs"][0]["log"]
            .as_str()
            .unwrap();
        assert!(handle.starts_with("ocel_log_"));
        assert!(!listed.to_string().contains("order-management.jsonocel"));

        let listed_models = server
            .handle(request(
                3,
                "tools/call",
                json!({ "name": "list_ocel_models", "arguments": {} }),
            ))
            .unwrap();
        let model_handle = listed_models["result"]["structuredContent"]["models"][0]["model"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(model_handle.starts_with("ocel_model_"));
        assert!(
            !listed_models
                .to_string()
                .contains("approved.ocelmodel.json")
        );

        let analyzed = server
            .handle(request(
                3,
                "tools/call",
                json!({ "name": "analyze_ocel", "arguments": { "log": handle } }),
            ))
            .unwrap();
        let output = &analyzed["result"]["structuredContent"];
        assert_eq!(analyzed["result"]["isError"], false);
        assert_eq!(output["ready_for_analysis"], true);
        assert_eq!(output["scope"]["events_shown"], 3);
        assert_eq!(
            output["graphs"]["directly_follows"]["items"][0]["mean_elapsed_micros"],
            86_400_000_000_i64
        );
        assert!(
            output["catalog"]["event_types"][0]
                .as_str()
                .is_some_and(|value| value.starts_with("activity_"))
        );
        assert!(
            output["catalog"]["object_types"][0]
                .as_str()
                .is_some_and(|value| value.starts_with("object_type_"))
        );
        for secret in ["e-place", "order-1", "package-1", "place order"] {
            assert!(!analyzed.to_string().contains(secret));
        }

        let from_activity = output["catalog"]["event_types"][0].clone();
        let to_activity = output["catalog"]["event_types"][1].clone();
        let measured = server
            .handle(request(
                4,
                "tools/call",
                json!({
                    "name": "measure_ocel_time_between",
                    "arguments": {
                        "log": handle,
                        "from_activity": from_activity,
                        "to_activity": to_activity
                    }
                }),
            ))
            .unwrap();
        let timing = &measured["result"]["structuredContent"];
        assert_eq!(measured["result"]["isError"], false);
        assert_eq!(timing["measurements"]["total"], 1);
        assert_eq!(timing["measurements"]["items"][0]["objects"], 2);
        assert_eq!(
            timing["measurements"]["items"][0]["mean_elapsed_micros"],
            86_400_000_000_i64
        );
        assert_eq!(
            timing["measurements"]["items"][0]["evidence"][0]["source_at_micros"],
            Value::Null
        );
        for secret in ["place order", "create package", "item-1", "e-place"] {
            assert!(!measured.to_string().contains(secret));
        }

        let synchronized = server
            .handle(request(
                5,
                "tools/call",
                json!({
                    "name": "analyze_ocel_synchronization",
                    "arguments": { "log": handle }
                }),
            ))
            .unwrap();
        let synchronization = &synchronized["result"]["structuredContent"];
        assert_eq!(synchronized["result"]["isError"], false);
        assert_eq!(synchronization["activities"]["total"], 2);
        let package = synchronization["activities"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["eligible_events"] == 1)
            .unwrap();
        assert_eq!(package["first_seen_object_participations"], 1);
        assert_eq!(package["mean_arrival_spread_micros"], 0);
        assert_eq!(
            package["mean_elapsed_after_last_prior_micros"],
            86_400_000_000_i64
        );
        assert_eq!(package["evidence"][0]["at_micros"], Value::Null);
        assert!(
            package["activity"]
                .as_str()
                .is_some_and(|value| value.starts_with("activity_"))
        );
        for secret in ["create package", "item-1", "e-pack", "e-place"] {
            assert!(!synchronized.to_string().contains(secret));
        }

        let ranked = server
            .handle(request(
                6,
                "tools/call",
                json!({
                    "name": "rank_ocel_bottlenecks",
                    "arguments": { "log": handle, "limit": 1 }
                }),
            ))
            .unwrap();
        let ranking = &ranked["result"]["structuredContent"];
        assert_eq!(ranked["result"]["isError"], false);
        assert_eq!(ranking["lifecycle"]["returned"], 1);
        assert_eq!(ranking["lifecycle"]["ranked_total"], 2);
        assert_eq!(ranking["lifecycle"]["candidates"], 2);
        assert_eq!(ranking["lifecycle"]["items"][0]["rank"], 1);
        assert_eq!(
            ranking["lifecycle"]["items"][0]["total_elapsed_micros"],
            172_800_000_000_i64
        );
        assert_eq!(
            ranking["lifecycle"]["items"][0]["share_millionths"],
            666_666
        );
        assert_eq!(ranking["synchronization"]["eligible_activities"], 1);
        assert_eq!(ranking["synchronization"]["positive_activities"], 0);
        assert!(
            ranking["lifecycle"]["items"][0]["source"]
                .as_str()
                .is_some_and(|value| value.starts_with("activity_"))
        );
        for secret in ["place order", "create package", "item-1", "e-place"] {
            assert!(!ranked.to_string().contains(secret));
        }

        let item_alias = output["catalog"]["object_types"][1].clone();
        let projected = server
            .handle(request(
                7,
                "tools/call",
                json!({
                    "name": "project_ocel_cases",
                    "arguments": {
                        "log": handle,
                        "case_object_type": item_alias,
                        "case_limit": 1,
                        "event_limit_per_case": 1
                    }
                }),
            ))
            .unwrap();
        let projection = &projected["result"]["structuredContent"];
        assert_eq!(projected["result"]["isError"], false);
        assert_eq!(projection["source_events"], 3);
        assert_eq!(projection["projected_cases"], 2);
        assert_eq!(projection["included_unique_events"], 2);
        assert_eq!(projection["excluded_events"], 1);
        assert_eq!(projection["projected_event_occurrences"], 4);
        assert_eq!(projection["convergence"]["events"], 2);
        assert_eq!(projection["convergence"]["additional_occurrences"], 2);
        assert_eq!(projection["divergence"]["candidates"], 0);
        assert_eq!(projection["cases"]["available"], 2);
        assert_eq!(projection["cases"]["returned"], 1);
        assert_eq!(projection["cases"]["items"][0]["events_available"], 2);
        assert_eq!(projection["cases"]["items"][0]["events_returned"], 1);
        assert_eq!(projection["cases"]["items"][0]["truncated"], true);
        assert_eq!(
            projection["cases"]["items"][0]["layers"][0]["at_micros"],
            Value::Null
        );
        assert!(
            projection["case_object_type"]
                .as_str()
                .is_some_and(|value| value.starts_with("object_type_"))
        );
        assert!(!projected.to_string().contains("\"item\""));
        for secret in ["item-1", "e-place", "place order"] {
            assert!(!projected.to_string().contains(secret));
        }

        // Following the same type gives case-engine figures for those items,
        // with the projection's accounting, and still no identifiers.
        let followed = server
            .handle(request(
                70,
                "tools/call",
                json!({
                    "name": "follow_ocel_object_type",
                    "arguments": { "log": handle, "object_type": item_alias }
                }),
            ))
            .unwrap();
        let follow = &followed["result"]["structuredContent"];
        assert_eq!(followed["result"]["isError"], false, "{followed}");
        assert_eq!(follow["follow"]["cases"], 2);
        assert_eq!(follow["follow"]["steps"], 4);
        assert_eq!(follow["follow"]["shared_step_copies"], 2);
        assert_eq!(follow["summary"]["cases"], 2);
        assert_eq!(follow["summary"]["events"], 4);
        for secret in ["item-1", "e-place", "place order", "\"item\""] {
            assert!(!followed.to_string().contains(secret));
        }

        let discovered = server
            .handle(request(
                8,
                "tools/call",
                json!({
                    "name": "discover_ocel_model",
                    "arguments": { "log": handle }
                }),
            ))
            .unwrap();
        let model = &discovered["result"]["structuredContent"];
        assert_eq!(discovered["result"]["isError"], false);
        assert_eq!(model["object_types"].as_array().unwrap().len(), 3);
        assert!(
            model["places"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            model["transitions"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            model["variable_arcs"]
                .as_u64()
                .is_some_and(|value| value > 0)
        );
        assert!(
            model["transitions"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|transition| transition["activity"].as_str())
                .all(|value| value.starts_with("activity_"))
        );
        assert!(!discovered.to_string().contains("\"item\""));
        for secret in ["place order", "create package", "package-1", "e-place"] {
            assert!(!discovered.to_string().contains(secret));
        }
        let conformance = server
            .handle(request(
                9,
                "tools/call",
                json!({
                    "name": "check_ocel_conformance",
                    "arguments": {
                        "log": handle,
                        "model": model_handle,
                        "detail_limit": 1
                    }
                }),
            ))
            .unwrap();
        let conformance_output = &conformance["result"]["structuredContent"];
        assert_eq!(conformance["result"]["isError"], false);
        assert_eq!(conformance_output["claim_type"], "calculated_fact");
        assert_eq!(conformance_output["fitness_millionths"], 1_000_000);
        assert_eq!(conformance_output["lifecycle_fit_objects"], 4);
        assert_eq!(conformance_output["replayed_objects"], 4);
        assert_eq!(conformance_output["binding_violations"], 0);
        assert_eq!(conformance_output["objects"]["available"], 4);
        assert_eq!(conformance_output["objects"]["returned"], 1);
        assert_eq!(conformance_output["objects"]["truncated"], true);
        assert!(
            conformance_output["objects"]["items"][0]["object"]
                .as_str()
                .is_some_and(|value| value.starts_with("object_"))
        );
        assert!(
            conformance_output["objects"]["items"][0]["object_type"]
                .as_str()
                .is_some_and(|value| value.starts_with("object_type_"))
        );
        for secret in [
            "approved.ocelmodel.json",
            "order-management.jsonocel",
            "place order",
            "item-1",
            "e-place",
        ] {
            assert!(!conformance.to_string().contains(secret));
        }
        let alignment = server
            .handle(request(
                10,
                "tools/call",
                json!({
                    "name": "align_ocel_conformance",
                    "arguments": {
                        "log": handle,
                        "model": model_handle,
                        "detail_limit": 1
                    }
                }),
            ))
            .unwrap();
        let alignment_output = &alignment["result"]["structuredContent"];
        assert_eq!(alignment["result"]["isError"], false);
        assert_eq!(alignment_output["algorithm"], "optimal_astar_alignment");
        assert_eq!(alignment_output["total_cost"], 0);
        assert_eq!(alignment_output["fitness_millionths"], 1_000_000);
        assert_eq!(alignment_output["lifecycle_fit_objects"], 4);
        assert_eq!(alignment_output["aligned_objects"], 4);
        assert_eq!(alignment_output["binding_violations"], 0);
        assert_eq!(alignment_output["objects"]["available"], 4);
        assert_eq!(alignment_output["objects"]["returned"], 1);
        for secret in ["place order", "package-1", "e-place"] {
            assert!(!alignment.to_string().contains(secret));
        }
        let invalid_threshold = server
            .handle(request(
                9,
                "tools/call",
                json!({
                    "name": "discover_ocel_model",
                    "arguments": { "log": handle, "noise_threshold_milli": 1001 }
                }),
            ))
            .unwrap();
        assert_eq!(invalid_threshold["result"]["isError"], true);
        let bounded_model = server
            .handle(request(
                10,
                "tools/call",
                json!({
                    "name": "discover_ocel_model",
                    "arguments": { "log": handle, "max_model_nodes": 1 }
                }),
            ))
            .unwrap();
        assert_eq!(bounded_model["result"]["isError"], true);

        let package_alias = output["catalog"]["object_types"][2].clone();
        let filtered = server
            .handle(request(
                5,
                "tools/call",
                json!({
                    "name": "analyze_ocel",
                    "arguments": { "log": handle, "object_types": [package_alias] }
                }),
            ))
            .unwrap();
        let filtered = &filtered["result"]["structuredContent"];
        assert_eq!(filtered["scope"]["objects_shown"], 1);
        assert_eq!(filtered["scope"]["events_shown"], 2);
        assert_eq!(filtered["scope"]["narrowed"], true);
        assert_eq!(filtered["variants"]["total"], 1);
        assert_eq!(filtered["variants"]["items"][0]["objects"], 1);
        let package = filtered["variants"]["items"][0]["evidence_objects"][0].clone();
        assert!(
            package
                .as_str()
                .is_some_and(|value| value.starts_with("object_"))
        );

        let inspected = server
            .handle(request(
                6,
                "tools/call",
                json!({
                    "name": "inspect_ocel_object",
                    "arguments": {
                        "log": handle,
                        "object": package,
                        "object_types": [package_alias],
                        "limit": 1
                    }
                }),
            ))
            .unwrap();
        let lifecycle = &inspected["result"]["structuredContent"];
        assert_eq!(inspected["result"]["isError"], false);
        assert_eq!(lifecycle["events"]["returned"], 1);
        assert_eq!(lifecycle["events"]["total"], 2);
        assert_eq!(lifecycle["events"]["items"][0]["at_micros"], Value::Null);
        assert!(
            lifecycle["events"]["items"][0]["activity"]
                .as_str()
                .is_some_and(|value| value.starts_with("activity_"))
        );
        for secret in ["package-1", "e-pack", "create package"] {
            assert!(!inspected.to_string().contains(secret));
        }
    }

    #[test]
    fn object_centric_conformance_deviations_are_bounded_and_private() {
        let (root, server) = ocel_fixture();
        let log_path = root.join("order-management.jsonocel");
        let mut source: Value = serde_json::from_slice(&std::fs::read(&log_path).unwrap()).unwrap();
        source["events"][1]["relationships"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        std::fs::write(&log_path, serde_json::to_vec(&source).unwrap()).unwrap();

        let log_handle = server.ocel_handle("order-management.jsonocel");
        let model_handle = server.ocel_model_handle("approved.ocelmodel.json");
        let checked = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "check_ocel_conformance",
                    "arguments": {
                        "log": log_handle,
                        "model": model_handle,
                        "detail_limit": 1
                    }
                }),
            ))
            .unwrap();
        let output = &checked["result"]["structuredContent"];
        assert_eq!(checked["result"]["isError"], false);
        assert!(output["fitness_millionths"].as_u64().unwrap() < 1_000_000);
        assert!(output["binding_violations"].as_u64().unwrap() > 0);
        assert!(output["deviations"]["available"].as_u64().unwrap() > 1);
        assert_eq!(output["deviations"]["returned"], 1);
        assert_eq!(output["deviations"]["truncated"], true);
        let deviation = &output["deviations"]["items"][0];
        assert_eq!(deviation["kind"], "missing_object_participation");
        assert!(
            deviation["event"]
                .as_str()
                .is_some_and(|value| value.starts_with("event_"))
        );
        assert!(
            deviation["activity"]
                .as_str()
                .is_some_and(|value| value.starts_with("activity_"))
        );
        assert!(
            deviation["object_type"]
                .as_str()
                .is_some_and(|value| value.starts_with("object_type_"))
        );
        for secret in [
            "approved.ocelmodel.json",
            "order-management.jsonocel",
            "create package",
            "package-1",
            "e-pack",
            "\"package\"",
        ] {
            assert!(!checked.to_string().contains(secret));
        }

        let mut omitted = ExposurePolicy::safe();
        omitted.source_names = Mode::Omit;
        omitted.activity_names = Mode::Omit;
        omitted.event_ids = Mode::Omit;
        omitted.object_ids = Mode::Omit;
        omitted.object_type_names = Mode::Omit;
        omitted.relationship_qualifiers = Mode::Omit;
        let omitted_server = Server::with_exposure_policy(&root, omitted).unwrap();
        let omitted_result = omitted_server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "check_ocel_conformance",
                    "arguments": {
                        "log": omitted_server.ocel_handle("order-management.jsonocel"),
                        "model": omitted_server.ocel_model_handle("approved.ocelmodel.json"),
                        "detail_limit": 1
                    }
                }),
            ))
            .unwrap();
        let omitted_output = &omitted_result["result"]["structuredContent"];
        assert!(omitted_output["model_training_source"].is_null());
        assert!(omitted_output["objects"]["items"][0]["object"].is_null());
        assert!(omitted_output["objects"]["items"][0]["object_type"].is_null());
        assert!(omitted_output["deviations"]["items"][0]["event"].is_null());
        assert!(omitted_output["deviations"]["items"][0]["activity"].is_null());
        assert!(omitted_output["deviations"]["items"][0]["object_type"].is_null());
        assert!(omitted_output["binding_violations"].as_u64().unwrap() > 0);
        for secret in ["create package", "package-1", "e-pack", "\"package\""] {
            assert!(!omitted_result.to_string().contains(secret));
        }
    }

    #[test]
    fn object_centric_conformance_accepts_only_bounded_local_model_handles() {
        let (root, server) = ocel_fixture();
        let log_handle = server.ocel_handle("order-management.jsonocel");
        let model_handle = server.ocel_model_handle("approved.ocelmodel.json");

        let raw_path = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "check_ocel_conformance",
                    "arguments": {
                        "log": log_handle,
                        "model": root.join("approved.ocelmodel.json").display().to_string()
                    }
                }),
            ))
            .unwrap();
        assert_eq!(raw_path["result"]["isError"], true);
        assert!(
            raw_path["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("opaque identifier")
        );

        let bounded = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "check_ocel_conformance",
                    "arguments": {
                        "log": log_handle,
                        "model": model_handle,
                        "max_model_nodes": 1
                    }
                }),
            ))
            .unwrap();
        assert_eq!(bounded["result"]["isError"], true);
        assert!(
            bounded["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("above max_model_nodes=1")
        );

        std::fs::write(root.join("broken.ocelmodel.json"), b"not json").unwrap();
        let broken_handle = server.ocel_model_handle("broken.ocelmodel.json");
        let broken = server
            .handle(request(
                3,
                "tools/call",
                json!({
                    "name": "check_ocel_conformance",
                    "arguments": { "log": log_handle, "model": broken_handle }
                }),
            ))
            .unwrap();
        assert_eq!(broken["result"]["isError"], true);
        assert!(
            broken["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("not valid JSON")
        );
    }

    #[test]
    fn omitted_object_identifiers_cannot_be_used_as_filter_inputs() {
        let (root, _) = ocel_fixture();
        let mut policy = ExposurePolicy::safe();
        policy.object_type_names = Mode::Omit;
        let server = Server::with_exposure_policy(root, policy).unwrap();
        let handle = server.ocel_handle("order-management.jsonocel");
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "analyze_ocel",
                    "arguments": { "log": handle, "object_types": ["package"] }
                }),
            ))
            .unwrap();
        assert_eq!(result["result"]["isError"], true);

        let result = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "project_ocel_cases",
                    "arguments": { "log": handle, "case_object_type": "item" }
                }),
            ))
            .unwrap();
        assert_eq!(result["result"]["isError"], true);
        assert!(!result.to_string().contains("item"));
    }

    #[test]
    fn omitted_object_ids_disable_object_lifecycle_drilldown() {
        let (root, _) = ocel_fixture();
        let mut policy = ExposurePolicy::safe();
        policy.object_ids = Mode::Omit;
        let server = Server::with_exposure_policy(root, policy).unwrap();
        let handle = server.ocel_handle("order-management.jsonocel");
        let analyzed = server
            .handle(request(
                1,
                "tools/call",
                json!({ "name": "analyze_ocel", "arguments": { "log": handle } }),
            ))
            .unwrap();
        assert_eq!(
            analyzed["result"]["structuredContent"]["variants"]["items"][0]["evidence_objects"],
            json!([])
        );
        let inspected = server
            .handle(request(
                2,
                "tools/call",
                json!({
                    "name": "inspect_ocel_object",
                    "arguments": { "log": handle, "object": "package-1" }
                }),
            ))
            .unwrap();
        assert_eq!(inspected["result"]["isError"], true);
        assert!(!inspected.to_string().contains("package-1"));
    }

    #[test]
    fn omitted_activity_names_disable_object_centric_time_queries() {
        let (root, _) = ocel_fixture();
        let mut policy = ExposurePolicy::safe();
        policy.activity_names = Mode::Omit;
        let server = Server::with_exposure_policy(root, policy).unwrap();
        let handle = server.ocel_handle("order-management.jsonocel");
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "measure_ocel_time_between",
                    "arguments": {
                        "log": handle,
                        "from_activity": "place order",
                        "to_activity": "create package"
                    }
                }),
            ))
            .unwrap();
        assert_eq!(result["result"]["isError"], true);
        assert!(!result.to_string().contains("place order"));
        assert!(!result.to_string().contains("create package"));
    }

    #[test]
    fn synchronization_keeps_aggregates_but_omits_protected_evidence() {
        let (root, _) = ocel_fixture();
        let mut policy = ExposurePolicy::safe();
        policy.activity_names = Mode::Omit;
        policy.object_type_names = Mode::Omit;
        policy.object_ids = Mode::Omit;
        policy.event_ids = Mode::Omit;
        policy.timestamps = Mode::Omit;
        let server = Server::with_exposure_policy(root, policy).unwrap();
        let handle = server.ocel_handle("order-management.jsonocel");
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "analyze_ocel_synchronization",
                    "arguments": { "log": handle }
                }),
            ))
            .unwrap();
        let output = &result["result"]["structuredContent"];
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(output["activities"]["total"], 2);
        let package = output["activities"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["eligible_events"] == 1)
            .unwrap();
        assert_eq!(package["activity"], Value::Null);
        assert_eq!(package["object_types"], json!([]));
        assert_eq!(
            package["mean_elapsed_after_last_prior_micros"],
            86_400_000_000_i64
        );
        assert_eq!(package["evidence"][0]["event"], Value::Null);
        assert_eq!(package["evidence"][0]["at_micros"], Value::Null);
        assert_eq!(
            package["evidence"][0]["readiness"][0]["object"],
            Value::Null
        );
        assert_eq!(
            package["evidence"][0]["readiness"][0]["prior_events"],
            json!([])
        );
        for secret in ["create package", "item-1", "e-pack", "e-place"] {
            assert!(!result.to_string().contains(secret));
        }
    }

    #[test]
    fn bottleneck_ranking_keeps_metrics_but_omits_protected_evidence() {
        let (root, _) = ocel_fixture();
        let mut policy = ExposurePolicy::safe();
        policy.activity_names = Mode::Omit;
        policy.object_type_names = Mode::Omit;
        policy.object_ids = Mode::Omit;
        policy.event_ids = Mode::Omit;
        policy.timestamps = Mode::Omit;
        let server = Server::with_exposure_policy(root, policy).unwrap();
        let handle = server.ocel_handle("order-management.jsonocel");
        let result = server
            .handle(request(
                1,
                "tools/call",
                json!({
                    "name": "rank_ocel_bottlenecks",
                    "arguments": { "log": handle }
                }),
            ))
            .unwrap();
        let output = &result["result"]["structuredContent"];
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(output["lifecycle"]["ranked_total"], 2);
        assert_eq!(output["lifecycle"]["items"][0]["source"], Value::Null);
        assert_eq!(output["lifecycle"]["items"][0]["target"], Value::Null);
        assert_eq!(output["lifecycle"]["items"][0]["object_type"], Value::Null);
        assert_eq!(
            output["lifecycle"]["items"][0]["total_elapsed_micros"],
            172_800_000_000_i64
        );
        assert_eq!(output["lifecycle"]["items"][0]["share_millionths"], 666_666);
        let evidence = &output["lifecycle"]["items"][0]["evidence"][0];
        assert_eq!(evidence["object"], Value::Null);
        assert_eq!(evidence["source_event"], Value::Null);
        assert_eq!(evidence["source_at_micros"], Value::Null);
        for secret in ["place order", "create package", "item-1", "e-place"] {
            assert!(!result.to_string().contains(secret));
        }
    }

    #[test]
    fn reviewed_show_modes_are_enforced() {
        let (root, _server) = fixture();
        let mut policy = ExposurePolicy::safe();
        policy.project_names = Mode::Show;
        policy.source_names = Mode::Show;
        policy.column_names = Mode::Show;
        policy.activity_names = Mode::Show;
        policy.case_ids = Mode::Show;
        let server = Server::with_exposure_policy(&root, policy).unwrap();
        let listed = server
            .handle(request(
                1,
                "tools/call",
                json!({ "name": "list_projects", "arguments": {} }),
            ))
            .unwrap();
        assert!(listed.to_string().contains("Orders"));
        assert!(listed.to_string().contains("orders.csv"));
        let handle = listed["result"]["structuredContent"]["projects"][0]["project"]
            .as_str()
            .unwrap();
        let inspected = server
            .handle(request(
                2,
                "tools/call",
                json!({ "name": "inspect_log", "arguments": { "project": handle } }),
            ))
            .unwrap();
        assert_eq!(
            inspected["result"]["structuredContent"]["mapping"]["case_id"],
            "case_id"
        );

        let variants = server
            .handle(request(
                3,
                "tools/call",
                json!({ "name": "get_variants", "arguments": { "project": handle } }),
            ))
            .unwrap();
        assert_eq!(
            variants["result"]["structuredContent"]["variants"][0]["sequence"][0],
            "Start"
        );
    }
}
