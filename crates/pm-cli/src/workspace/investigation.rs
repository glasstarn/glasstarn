//! Harness-neutral follow-up queries and durable investigation notes.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Readiness,
    LinkedInterval {
        interval: pm_query::linked_interval::LinkedInterval,
    },
    Explore {
        #[serde(default)]
        object_type: Option<String>,
        #[serde(flatten)]
        exploration: pm_query::exploration::ExplorationRequest,
    },
    AttributeChoices {
        attribute: String,
        #[serde(default)]
        search: String,
        #[serde(default = "default_value_limit")]
        limit: usize,
        #[serde(default)]
        object_type: Option<String>,
    },
    Case {
        id: String,
    },
    Activity {
        activity: String,
        limit: usize,
    },
    Transition {
        from: String,
        to: String,
        limit: usize,
    },
    Cohorts {
        attribute: String,
        left: String,
        right: String,
    },
    Discover,
    Map {
        #[serde(default)]
        selection: pm_query::MapSelection,
    },
    Conformance {
        model: Option<PathBuf>,
        algorithm: Algorithm,
        limit: usize,
    },
    Object {
        id: String,
    },
    Bottlenecks,
    /// Follow one item type of a connected log as a case log, through the
    /// same engine and with the same figures as any single log.
    Follow {
        object_type: String,
        /// `headline` (the default) returns the figures an agent needs first;
        /// `full` returns the whole analysis.
        #[serde(default)]
        detail: Detail,
    },
    /// The stages from one item type to the linked types after it, with
    /// hand-off times between them.
    Journey {
        object_type: String,
    },
    /// The ranked calculated findings. On a connected log, `object_type`
    /// names the item type to follow; its journey drop-outs are included.
    Findings {
        #[serde(default)]
        object_type: Option<String>,
    },
    /// The rules the process follows and the cases that break them, who does
    /// each step, and each step's automation bucket. On a connected log,
    /// `object_type` names the item type to follow.
    Improvement {
        #[serde(default)]
        object_type: Option<String>,
    },
    /// Totals of an amount field, optionally bounded and grouped. Without a
    /// field, lists the fields that hold numbers. On a connected log,
    /// `object_type` names the item type to follow first.
    Values {
        #[serde(default)]
        object_type: Option<String>,
        #[serde(default)]
        field: Option<String>,
        #[serde(default)]
        at_least: Option<f64>,
        #[serde(default)]
        at_most: Option<f64>,
        #[serde(default)]
        grouping: pm_query::ValueGrouping,
        #[serde(default = "default_value_limit")]
        limit: usize,
    },
}

fn default_value_limit() -> usize {
    10
}

/// Findings with the item name filled in.
fn findings_value(
    findings: &[pm_query::KeyFinding],
    singular: &str,
    plural: &str,
) -> serde_json::Value {
    let rendered = findings
        .iter()
        .map(|finding| {
            let mut finding = finding.clone();
            finding.title = pm_query::findings::render(&finding.title, singular, plural);
            finding.detail = pm_query::findings::render(&finding.detail, singular, plural);
            finding.impact = pm_query::findings::render(&finding.impact, singular, plural);
            finding
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "findings": rendered,
        "semantics": "Calculated facts, most important first. They state what was measured, not why; label any cause you propose as a hypothesis."
    })
}

/// Totals of the followed items' money field, when they carry one.
fn headline_value(
    path: &Path,
    summary: &pm_query::FollowSummary,
    analysis: &pm_query::Analysis,
) -> Result<Option<pm_query::HeadlineValue>, String> {
    let Some(field) = pm_core::main_number_field(&analysis.number_fields) else {
        return Ok(None);
    };
    let loaded = pm_query::load_log(
        path,
        Some(summary.mapping()),
        &pm_ingest::finding::Policy::new(),
        Default::default(),
    )
    .map_err(|e| e.to_string())?
    .map_err(|_| "the followed log did not pass its data checks")?;
    let values = pm_query::summarize_values(
        &loaded.log,
        &field.name,
        &pm_query::ValueGrouping::None,
        &Default::default(),
        1,
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(pm_query::HeadlineValue {
        field: values.field,
        stats: values.overall,
    }))
}

/// Answers a [`Request::Values`] over a case log.
fn values(
    log: &pm_core::EventLog,
    field: Option<&str>,
    (at_least, at_most): (Option<f64>, Option<f64>),
    grouping: &pm_query::ValueGrouping,
    finishing: &std::collections::BTreeSet<String>,
    count: usize,
) -> Result<serde_json::Value, String> {
    let fields = pm_core::number_fields(log);
    let Some(field) = field else {
        return Ok(serde_json::json!({
            "number_fields": fields,
            "note": if fields.is_empty() {
                "No field in this view holds numbers, so amounts cannot be totalled. Answer with counts and times."
            } else {
                "Pass one of these as field to total it."
            }
        }));
    };
    if [at_least, at_most]
        .iter()
        .flatten()
        .any(|bound| !bound.is_finite())
    {
        return Err("amount bounds must be finite numbers".into());
    }
    let bounded;
    let log = if at_least.is_some() || at_most.is_some() {
        bounded = pm_core::Filter::new()
            .number_between(field, at_least, at_most)
            .apply(log)
            .log;
        &bounded
    } else {
        log
    };
    serialize(
        pm_query::summarize_values(log, field, grouping, finishing, limit(count)?)
            .map_err(|e| e.to_string())?,
    )
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Detail {
    #[default]
    Headline,
    Full,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Algorithm {
    Token,
    Alignment,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Note {
    version: u32,
    question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intention: Option<pm_query::process_model::Intention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process_model: Option<pm_query::process_model::ProcessModel>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    clear_process_model: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    visuals: Vec<VisualSpec>,
    #[serde(default)]
    assumptions: Vec<String>,
    #[serde(default)]
    missing_data: Vec<String>,
    #[serde(default)]
    findings: Vec<Finding>,
    /// The headline answer the browser Summary shows for this question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    answer: Option<String>,
    /// The agent's one-line read of the process and its business goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process: Option<String>,
    /// Follow-up questions worth asking next.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    next_questions: Vec<String>,
    /// The kind of process, from the playbooks; the browser frames Overview
    /// around its goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process_type: Option<pm_query::ProcessType>,
    /// What the open page should show for this answer ("show me the main
    /// path"): the page moves there by itself when the answer is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show: Option<Show>,
}

/// A view of the results page, as an answer asks for it.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Show {
    view: ShowView,
    /// On the Process page: `main` (the main path only) or `all`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<ShowDetail>,
    /// A step to pick on the Process page, by its recorded name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    step: Option<String>,
    /// Variant numbers to select on the Variants page, as the page numbers them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    variants: Vec<u32>,
    /// A filter to put on, in the same form as a finding's filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    filter: Option<pm_core::Filter>,
    /// An Overview finding to open, by (part of) its title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finding: Option<String>,
    /// Two groups of one field to compare side by side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compare: Option<ShowCompare>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShowCompare {
    attribute: String,
    left: String,
    right: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ShowView {
    Overview,
    Process,
    Variants,
    Conformance,
    Improve,
    Items,
    Compare,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ShowDetail {
    Main,
    All,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VisualSpec {
    title: String,
    evidence: String,
    kind: VisualKind,
    metric: VisualMetric,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VisualKind {
    Bar,
    Table,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VisualMetric {
    Cases,
    Total,
    Mean,
    Median,
    P90,
}

// Rows come only from the validated Rust query snapshot. The agent chooses
// presentation and metric, never chart numbers, executable markup or URLs.
fn visual_result(spec: &VisualSpec, evidence: &Value) -> Result<Value, String> {
    let result = &evidence["result"];
    let metric = match spec.metric {
        VisualMetric::Cases => "cases",
        VisualMetric::Total => "total",
        VisualMetric::Mean => "mean",
        VisualMetric::Median => "median",
        VisualMetric::P90 => "p90",
    };
    let mut rows = Vec::new();
    match evidence["request"]["type"].as_str() {
        Some("values") => {
            let groups = result["groups"]
                .as_array()
                .ok_or("visual evidence needs grouped numeric values")?;
            if groups.is_empty() {
                return Err("visual evidence has no groups".into());
            }
            for group in groups.iter().take(100) {
                rows.push(json!({"label":group["label"],"value":group["stats"][metric],"cases":group["stats"]["cases"],"cases_with_value":group["stats"]["cases_with_value"],"example_case":group["example_case"]}));
            }
        }
        Some("cohorts") => {
            let field = match spec.metric {
                VisualMetric::Cases => "cases",
                VisualMetric::Mean => "mean_throughput_seconds",
                VisualMetric::Median => "median_throughput_seconds",
                VisualMetric::P90 => "p90_throughput_seconds",
                VisualMetric::Total => {
                    return Err("cohort comparisons do not measure amount totals".into());
                }
            };
            for group in [&result["left"], &result["right"]] {
                rows.push(
                    json!({"label":group["value"],"value":group[field],"cases":group["cases"]}),
                );
            }
        }
        Some("explore") | Some("linked_interval") => {
            let report = if evidence["request"]["type"] == "linked_interval" {
                &result["report"]
            } else {
                result
            };
            let field = match spec.metric {
                VisualMetric::Cases => "items",
                VisualMetric::Mean => "mean_seconds",
                VisualMetric::Median => "median_seconds",
                VisualMetric::P90 => "p90_seconds",
                VisualMetric::Total => {
                    return Err("elapsed-time analysis does not measure amount totals".into());
                }
            };
            let groups = report["groups"]
                .as_array()
                .ok_or("analysis visual needs grouped populations")?;
            if groups.is_empty() {
                return Err("choose group_by for this analysis visual".into());
            }
            for group in groups {
                rows.push(json!({"label":group["value"].as_str().unwrap_or("Missing value"),"value":group["population"][field],"cases":group["population"]["items"],"cases_with_value":group["population"]["measurable"],"example_case":group["population"]["examples"][0]}));
            }
        }
        _ => return Err("visual evidence must be grouped values, cohorts or exploration".into()),
    }
    if rows
        .iter()
        .any(|row| !row["value"].as_f64().is_some_and(f64::is_finite))
    {
        return Err("selected visual metric is not measurable for every group".into());
    }
    Ok(
        json!({"title":spec.title,"evidence":spec.evidence,"kind":spec.kind,"metric":metric,"rows":rows,"groups_available":result["groups_available"],"other_groups":result["other_groups"],"field":result["field"],"grouping":result["grouping"],"semantics":result["semantics"],"request":evidence["request"]}),
    )
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Finding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    statement: String,
    kind: FindingKind,
    #[serde(default)]
    evidence: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FindingKind {
    Observed,
    Hypothesis,
}

fn input(args: &[String], action: &str) -> Result<(WorkspaceFolder, Value), String> {
    let (root, args) = root_option(args)?;
    let mut project = None;
    let mut file = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--request" => {
                i += 1;
                if file.is_some() {
                    return Err("--request may be supplied once".into());
                }
                file = Some(args.get(i).ok_or("--request needs a JSON file")?);
            }
            "--json" => {}
            value if !value.starts_with('-') && project.is_none() => project = Some(value),
            _ => {
                return Err(format!(
                    "workspace {action} needs one project and --request <JSON file>"
                ));
            }
        }
        i += 1;
    }
    let project = project.ok_or("missing project")?;
    let folder = match open_project(&root, project) {
        Ok(folder) => folder,
        Err(original) => {
            let given = PathBuf::from(project);
            let path = if given.join("workspace.json").is_file() {
                given
            } else if valid_workspace_id(project) {
                pm_project::folders::all_roots(&root)
                    .into_iter()
                    .map(|root| root.join(project))
                    .find(|path| path.join("workspace.json").is_file())
                    .unwrap_or_else(|| root.join(project))
            } else {
                return Err(original);
            };
            WorkspaceFolder::open_for_investigation(&path).map_err(|error| error.to_string())?
        }
    };
    let file = Path::new(file.ok_or("missing --request <JSON file>")?);
    if std::fs::metadata(file).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
        return Err("request exceeds 1 MiB".into());
    }
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    Ok((
        folder,
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
    ))
}

fn limit(value: usize) -> Result<usize, String> {
    if !(1..=100).contains(&value) {
        return Err("limit must be between 1 and 100".into());
    }
    Ok(value)
}

fn serialize(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

fn snapshot(
    folder: &WorkspaceFolder,
    category: &str,
    prefix: &str,
    value: &Value,
) -> Result<String, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let name = format!("{prefix}-{stamp}.json");
    // One writer owns the folder. Never overwrite earlier evidence.
    if folder.root.join(category).join(&name).exists() {
        return Err("snapshot identity collision; retry".into());
    }
    save_artifact_json(folder, category, &name, value)?;
    Ok(format!("{category}/{name}"))
}

pub(super) fn query(args: &[String]) -> Result<ExitCode, String> {
    let (mut folder, value) = input(args, "query")?;
    let request: Request =
        serde_json::from_value(value).map_err(|e| format!("invalid query: {e}"))?;
    let mut state = folder.state().map_err(|e| e.to_string())?;
    let asset = state
        .source
        .as_ref()
        .ok_or("project has no imported data")?;
    let source = folder.resolve_asset(asset).map_err(|e| e.to_string())?;
    let source_label = source.display().to_string();
    let mut selected_model = None;
    let result = if let Some(project) = &state.case_project {
        let loaded = pm_query::load_log(
            &source,
            Some(project.mapping.clone()),
            &project.policy,
            project.rules.clone(),
        )
        .map_err(|e| e.to_string())?;
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(blocked) => {
                let outcome = Outcome::Blocked(blocked);
                let (value, code) =
                    outcome_value(&folder, &outcome, asset, &source, &project.policy)?;
                print_json(&value)?;
                return Ok(code);
            }
        };
        let filtered = project.view_filter.apply(&loaded.log);
        if filtered.log.case_count() == 0
            && !matches!(
                request,
                Request::Readiness | Request::AttributeChoices { .. }
            )
        {
            return Err(
                "active view has no cases; no finding or conformance conclusion can be produced"
                    .into(),
            );
        }
        match &request {
            Request::AttributeChoices {
                attribute,
                search,
                limit: count,
                object_type: None,
            } => serialize(pm_query::attribute_choices(
                &filtered.log,
                attribute,
                search,
                limit(*count)?,
            )?)?,
            Request::Readiness => {
                serialize(pm_query::inspection::history_readiness(&filtered.log))?
            }
            Request::Case { id } => serialize(
                pm_query::inspect_case_read_as(&filtered.log, id, loaded.timestamp_reading)
                    .ok_or("case is absent from the active view")?,
            )?,
            Request::Activity {
                activity,
                limit: count,
            } => evidence(
                &filtered.log,
                pm_query::Evidence::Activity(activity.clone()),
                limit(*count)?,
            )?,
            Request::Transition {
                from,
                to,
                limit: count,
            } => evidence(
                &filtered.log,
                pm_query::Evidence::Transition {
                    from: from.clone(),
                    to: to.clone(),
                },
                limit(*count)?,
            )?,
            Request::Cohorts {
                attribute,
                left,
                right,
            } => serialize(
                pm_query::compare_cohorts(&filtered.log, attribute, left, right)
                    .map_err(|e| e.to_string())?,
            )?,
            Request::Explore { exploration, .. } => {
                serialize(pm_query::exploration::explore(&filtered.log, exploration)?)?
            }
            Request::Map { selection } => {
                serialize(pm_query::map_view(&filtered.log, selection).map_err(|e| e.to_string())?)?
            }
            Request::Improvement { object_type: None } => {
                serialize(pm_query::improvement::improvement_report(
                    &pm_query::improvement::CaseLog(&filtered.log),
                    "case",
                    &pm_project::improvement::Improvement::load(&folder.root)?,
                    &project.rules.resource_kinds,
                    &Default::default(),
                )?)?
            }
            Request::Findings { object_type: None } => {
                let outcome = pm_query::analyze_filtered(
                    &source,
                    Some(project.mapping.clone()),
                    &project.policy,
                    project.rules.clone(),
                    project.view_filter.clone(),
                )
                .map_err(|e| e.to_string())?;
                let Outcome::Complete(analysis) = outcome else {
                    return Err("the log needs a data-quality decision first".into());
                };
                findings_value(&analysis.key_findings, "item", "items")
            }
            Request::Values {
                object_type: None,
                field,
                at_least,
                at_most,
                grouping,
                limit: count,
            } => values(
                &filtered.log,
                field.as_deref(),
                (*at_least, *at_most),
                grouping,
                &project
                    .rules
                    .observation_window
                    .as_ref()
                    .map(|window| window.end_activities.clone())
                    .unwrap_or_default(),
                *count,
            )?,
            Request::Discover => serialize(
                pm_query::discover_formal(
                    &source_label,
                    &loaded.log,
                    &project.view_filter,
                    Default::default(),
                )
                .map_err(|e| e.to_string())?,
            )?,
            Request::Conformance {
                model,
                algorithm,
                limit: count,
            } => {
                let count = limit(*count)?;
                let (path, candidate) = model_path(&folder, model, &state.case_model)?;
                let bytes = read_bounded(&path)?;
                if bytes.len() > 10 * 1024 * 1024 {
                    return Err("case baseline exceeds the shared browser limit of 10 MiB".into());
                }
                let parsed =
                    pm_query::parse_formal_model_json(&bytes).map_err(|e| e.to_string())?;
                pm_query::validate_formal_model(&parsed.net).map_err(|e| e.to_string())?;
                // Token replay guesses around optional steps and can flag
                // compliant cases, so a model with them is checked exactly.
                let algorithm = if pm_query::has_silent_steps(&parsed.net) {
                    &Algorithm::Alignment
                } else {
                    algorithm
                };
                let report = match algorithm {
                    Algorithm::Token => serialize(
                        pm_query::check_conformance(
                            &source_label,
                            &parsed.source,
                            &loaded.log,
                            &project.view_filter,
                            &parsed.net,
                            count,
                        )
                        .map_err(|e| e.to_string())?,
                    )?,
                    Algorithm::Alignment => serialize(
                        pm_query::align_conformance(
                            &source_label,
                            &parsed.source,
                            &loaded.log,
                            &project.view_filter,
                            &parsed.net,
                            count,
                        )
                        .map_err(|e| e.to_string())?,
                    )?,
                };
                // The step sequences behind each group are for the page's
                // click-through; an agent needs every group's count, so they
                // must not crowd the groups out of its answer.
                let mut report = report;
                if let Some(groups) = report.get_mut("by_step").and_then(Value::as_array_mut) {
                    for group in groups {
                        if let Some(group) = group.as_object_mut() {
                            group.remove("variants");
                        }
                    }
                }
                selected_model = candidate;
                report
            }
            _ => return Err("this query requires native OCEL data".into()),
        }
    } else if let Some(filter) = &state.ocel_filter {
        match &request {
            Request::LinkedInterval { interval } => serialize(
                pm_query::linked_interval::explore_native(&source, filter, interval)?,
            )?,
            Request::Object { id } => serialize(
                pm_query::inspect_ocel_object(&source, filter, id).map_err(|e| e.to_string())?,
            )?,
            Request::Bottlenecks => serialize(
                pm_query::rank_ocel_bottlenecks(&source, filter).map_err(|e| e.to_string())?,
            )?,
            Request::Journey { object_type } => serialize(
                pm_query::ocel_journey(&source, filter, object_type).map_err(|e| e.to_string())?,
            )?,
            Request::Follow {
                object_type,
                detail,
            } => {
                if matches!(detail, Detail::Full) {
                    print_everything();
                }
                let mut followed = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
                let summary =
                    pm_query::follow_ocel(&source, filter, object_type, followed.as_file_mut())
                        .map_err(|e| e.to_string())?;
                let outcome = pm_query::analyze_filtered(
                    followed.path(),
                    Some(summary.mapping()),
                    &pm_ingest::finding::Policy::new(),
                    Default::default(),
                    Default::default(),
                )
                .map_err(|e| e.to_string())?;
                match outcome {
                    Outcome::Complete(mut analysis) => {
                        analysis.source = format!("{object_type} followed in {source_label}");
                        match detail {
                            Detail::Full => {
                                serde_json::json!({"follow":summary,"analysis":analysis})
                            }
                            Detail::Headline => {
                                let value = headline_value(followed.path(), &summary, &analysis)?;
                                let mut headline = serde_json::json!(pm_query::headline(
                                    &analysis,
                                    value,
                                    HEADLINE_ITEMS
                                ));
                                super::drop_empty_filter_parts(&mut headline);
                                serde_json::json!({
                                    "follow": summary,
                                    "headline": headline,
                                    "more": {
                                        "full_analysis": {"type":"follow","object_type":object_type,"detail":"full"},
                                        "findings": {"type":"findings","object_type":object_type},
                                        "amounts": {"type":"values","object_type":object_type},
                                        "journey": {"type":"journey","object_type":object_type}
                                    }
                                })
                            }
                        }
                    }
                    // A projection of a quality-checked log has fixed, valid
                    // columns, so a block here is a data finding to report.
                    Outcome::Blocked(blocked) => serde_json::json!({"follow":summary,
                        "blocked":blocked.report.findings.iter().map(|f| format!("{}: {}", f.kind.title(), f.detail)).collect::<Vec<_>>()}),
                }
            }
            Request::Improvement {
                object_type: Some(object_type),
            } => serialize(pm_query::improvement::improvement_ocel(
                &source,
                filter,
                object_type,
                &pm_project::improvement::Improvement::load(&folder.root)?,
            )?)?,
            Request::Improvement { object_type: None } => {
                return Err(
                    "on a connected log, improvement needs object_type: the item type to follow"
                        .into(),
                );
            }
            Request::Findings {
                object_type: Some(object_type),
            } => {
                let mut followed = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
                let summary =
                    pm_query::follow_ocel(&source, filter, object_type, followed.as_file_mut())
                        .map_err(|e| e.to_string())?;
                let outcome = pm_query::analyze_filtered(
                    followed.path(),
                    Some(summary.mapping()),
                    &pm_ingest::finding::Policy::new(),
                    Default::default(),
                    Default::default(),
                )
                .map_err(|e| e.to_string())?;
                let Outcome::Complete(analysis) = outcome else {
                    return Err("the followed log needs a data-quality decision".into());
                };
                let journey = pm_query::ocel_journey(&source, filter, object_type)
                    .map_err(|e| e.to_string())?;
                let mut all = analysis.key_findings.clone();
                all.extend(journey.findings);
                let emphasis = analysis
                    .process
                    .as_ref()
                    .map(|profile| profile.emphasis.clone())
                    .unwrap_or_default();
                let ranked = pm_query::findings::rank_for(all, &emphasis, 8);
                findings_value(
                    &ranked,
                    object_type,
                    &pm_query::findings::plural(object_type),
                )
            }
            Request::Findings { .. } => {
                return Err(
                    "on a connected log, findings needs object_type: the item type to follow"
                        .into(),
                );
            }
            Request::Explore {
                object_type: Some(object_type),
                exploration,
            } => {
                let mut followed = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
                let summary =
                    pm_query::follow_ocel(&source, filter, object_type, followed.as_file_mut())
                        .map_err(|e| e.to_string())?;
                let loaded = pm_query::load_log(
                    followed.path(),
                    Some(summary.mapping()),
                    &pm_ingest::finding::Policy::new(),
                    Default::default(),
                )
                .map_err(|e| e.to_string())?
                .map_err(|_| "followed log failed data checks")?;
                serialize(pm_query::exploration::explore(&loaded.log, exploration)?)?
            }
            Request::AttributeChoices {
                object_type: Some(object_type),
                attribute,
                search,
                limit: count,
            } => {
                let mut followed = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
                let summary =
                    pm_query::follow_ocel(&source, filter, object_type, followed.as_file_mut())
                        .map_err(|e| e.to_string())?;
                let loaded = pm_query::load_log(
                    followed.path(),
                    Some(summary.mapping()),
                    &pm_ingest::finding::Policy::new(),
                    Default::default(),
                )
                .map_err(|e| e.to_string())?
                .map_err(|_| "followed log failed data checks")?;
                serialize(pm_query::attribute_choices(
                    &loaded.log,
                    attribute,
                    search,
                    limit(*count)?,
                )?)?
            }
            Request::AttributeChoices { .. } => {
                return Err("connected attribute_choices needs object_type".into());
            }
            Request::Values {
                object_type: Some(object_type),
                field,
                at_least,
                at_most,
                grouping,
                limit: count,
            } => {
                let mut followed = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
                let summary =
                    pm_query::follow_ocel(&source, filter, object_type, followed.as_file_mut())
                        .map_err(|e| e.to_string())?;
                let loaded = pm_query::load_log(
                    followed.path(),
                    Some(summary.mapping()),
                    &pm_ingest::finding::Policy::new(),
                    Default::default(),
                )
                .map_err(|e| e.to_string())?
                .map_err(|_| "the followed log did not pass its data checks")?;
                values(
                    &loaded.log,
                    field.as_deref(),
                    (*at_least, *at_most),
                    grouping,
                    &Default::default(),
                    *count,
                )?
            }
            Request::Values { .. } => {
                return Err("on a connected log, values needs object_type: the item type whose amounts to total".into());
            }
            Request::Discover => serialize(
                pm_query::discover_ocel_model(&source, filter, Default::default())
                    .map_err(|e| e.to_string())?,
            )?,
            Request::Conformance {
                model,
                algorithm,
                limit: count,
            } => {
                let count = limit(*count)?;
                let (path, candidate) = model_path(&folder, model, &state.ocel_model)?;
                let parsed: pm_query::OcelModelReport =
                    serde_json::from_slice(&read_bounded(&path)?).map_err(|e| e.to_string())?;
                let nodes = parsed.places.len().saturating_add(parsed.transitions.len());
                if nodes == 0 || nodes > 10_000 {
                    return Err(
                        "object baseline must have 1–10000 nodes for browser handoff".into(),
                    );
                }
                let report = match algorithm {
                    Algorithm::Token => {
                        let report =
                            pm_query::check_ocel_conformance(&source, &path, filter, count)
                                .map_err(|e| e.to_string())?;
                        if report.replayed_objects == 0 {
                            return Err("active view has no replayable objects".into());
                        }
                        serialize(report)?
                    }
                    Algorithm::Alignment => {
                        let report =
                            pm_query::align_ocel_conformance(&source, &path, filter, count)
                                .map_err(|e| e.to_string())?;
                        if report.aligned_objects == 0 {
                            return Err("active view has no alignable objects".into());
                        }
                        serialize(report)?
                    }
                };
                selected_model = candidate;
                report
            }
            _ => {
                return Err(
                    "case query requires a case log; use object or bottlenecks for native OCEL"
                        .into(),
                );
            }
        }
    } else {
        return Err("project has no saved analysis settings".into());
    };
    // Checked while the project was open in a page: leave the model for that
    // page to adopt, so its Conformance tab shows the same check.
    if folder.is_append_only()
        && let Request::Conformance {
            model: Some(path), ..
        } = &request
        && state.case_project.is_some()
    {
        let bytes = read_bounded(path)?;
        std::fs::write(folder.root.join(PENDING_MODEL), bytes).map_err(|e| e.to_string())?;
    }
    if let Some(model) = selected_model {
        if state.case_project.is_some() {
            state.case_model = Some(model);
        } else {
            state.ocel_model = Some(model);
        }
        folder.save_state(&state).map_err(|e| e.to_string())?;
    }
    let result = json!({"version":1,"status":"complete","project":folder_info(&folder),"request":request,"scope":state,"result":result});
    if !folder.settings_are_current().map_err(|e| e.to_string())? {
        return Err("project settings changed during the query; rerun it".into());
    }
    let path = snapshot(&folder, "exports", "query", &result)?;
    print_json(&json!({"artifact":path,"response":result}))?;
    Ok(ExitCode::SUCCESS)
}

fn evidence(
    log: &pm_core::EventLog,
    query: pm_query::Evidence,
    limit: usize,
) -> Result<Value, String> {
    let total = pm_query::count_cases(log, &query);
    let cases = pm_query::find_cases(log, &query, limit);
    Ok(
        json!({"cases_in_view":log.case_count(),"matching_cases":total,"cases":cases,"truncated":total > limit}),
    )
}

/// A model checked while the project's page was open, waiting for that page.
pub(crate) const PENDING_MODEL: &str = "pending-model.model.json";

fn model_path(
    folder: &WorkspaceFolder,
    supplied: &Option<PathBuf>,
    saved: &Option<WorkspaceAsset>,
) -> Result<(PathBuf, Option<WorkspaceAsset>), String> {
    match supplied {
        // The project is open in a page, which owns its models: check against
        // the supplied model as it is; the page adopts it afterwards.
        Some(path) if folder.is_append_only() => {
            read_bounded(path)?;
            Ok((path.clone(), None))
        }
        Some(path) => {
            read_bounded(path)?;
            let asset = folder
                .store_asset(path, "baseline.json", ".json", "models")
                .map_err(|e| e.to_string())?;
            let stored = folder.resolve_asset(&asset).map_err(|e| e.to_string())?;
            Ok((stored, Some(asset)))
        }
        None => folder
            .resolve_asset(
                saved
                    .as_ref()
                    .ok_or("supply an explicit baseline model; the project has none")?,
            )
            .map(|path| (path, None))
            .map_err(|e| e.to_string()),
    }
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > 64 * 1024 * 1024 {
        return Err("artifact must be a regular file no larger than 64 MiB".into());
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

pub(super) fn record(args: &[String]) -> Result<ExitCode, String> {
    let (folder, value) = input(args, "record")?;
    let note: Note =
        serde_json::from_value(value).map_err(|e| format!("invalid investigation record: {e}"))?;
    if note.version != 1 || note.question.trim().is_empty() {
        return Err("record needs version 1 and a nonempty question".into());
    }
    let bounded = |text: &Option<String>, limit: usize| {
        text.as_ref()
            .is_none_or(|text| !text.trim().is_empty() && text.chars().count() <= limit)
    };
    if !bounded(&note.answer, 400) {
        return Err("answer must contain 1–400 characters".into());
    }
    if !bounded(&note.process, 200) {
        return Err("process must contain 1–200 characters".into());
    }
    if note.next_questions.len() > 5
        || note
            .next_questions
            .iter()
            .any(|question| question.trim().is_empty() || question.chars().count() > 300)
    {
        return Err("next_questions holds up to five questions of 1–300 characters".into());
    }
    let state = folder.state().map_err(|e| e.to_string())?;
    let state_json = serialize(&state)?;
    // The browser shows what was measured, documented or reported. Advice
    // about what to change stays in the conversation, so a saved answer
    // carries no proposal, future process or step treatment.
    if let Some(model) = &note.process_model
        && (model.improvement.is_some()
            || matches!(
                model.view,
                pm_query::process_model::ProcessView::ProposedFuture
            )
            || model.steps.iter().any(|step| step.assessment.is_some()))
    {
        return Err(
            "improvement suggestions belong in the conversation, not in the project: \
                    save the facts they rest on and give the suggestions in your answer"
                .into(),
        );
    }
    if note.clear_process_model && note.process_model.is_some() {
        return Err("supply a process model or clear it, not both".into());
    }
    let previous = records(&folder)?
        .last()
        .map(|reference| -> Result<(String, Value), String> {
            let value: Value = serde_json::from_slice(&read_bounded(&folder.root.join(reference))?)
                .map_err(|error| error.to_string())?;
            if value["project"]["id"] != folder.manifest.id || value["version"] != 1 {
                return Err("previous investigation has a different identity or version".into());
            }
            Ok((reference.clone(), value))
        })
        .transpose()?;
    let previous_model = previous
        .as_ref()
        .filter(|(_, value)| value["process_model"].is_object());
    let previous_origin = previous_model.map(|(reference, value)| {
        value["process_model_origin"]
            .as_str()
            .unwrap_or(reference)
            .to_owned()
    });
    let mut process_model = note.process_model.clone();
    let mut process_model_origin = None;
    let mut process_model_recorded_at = None;
    if let Some(model) = process_model.as_mut() {
        if model.supersedes.is_some() && model.supersedes != previous_origin {
            return Err("process model supersedes must name the preceding model revision".into());
        }
        model.supersedes.clone_from(&previous_origin);
    } else if let Some((_, previous)) = previous_model
        .filter(|(_, previous)| !note.clear_process_model && previous["scope"] == state_json)
    {
        process_model = Some(
            serde_json::from_value(previous["process_model"].clone()).map_err(|e| e.to_string())?,
        );
        process_model_origin = previous_origin;
        process_model_recorded_at = previous["process_model_recorded_at"]
            .as_str()
            .map(str::to_owned);
    }
    let current_process_model: Option<pm_query::process_model::ProcessModel> =
        if note.clear_process_model {
            None
        } else if process_model.as_ref().is_some_and(|model| {
            matches!(
                model.view,
                pm_query::process_model::ProcessView::CombinedCurrent
            )
        }) {
            process_model.clone()
        } else {
            previous
                .as_ref()
                .filter(|(_, value)| value["scope"] == state_json)
                .and_then(|(_, value)| {
                    serde_json::from_value(value["current_process_model"].clone()).ok()
                })
        };
    let intention = note.intention.clone().or_else(|| {
        previous
            .as_ref()
            .and_then(|(_, previous)| serde_json::from_value(previous["intention"].clone()).ok())
    });
    for asset in state
        .source
        .iter()
        .chain(state.case_model.iter())
        .chain(state.ocel_model.iter())
    {
        folder.resolve_asset(asset).map_err(|e| e.to_string())?;
    }
    if note.visuals.len() > 6
        || note
            .visuals
            .iter()
            .any(|visual| visual.title.trim().is_empty() || visual.title.chars().count() > 160)
    {
        return Err("use at most six visuals, each with a title of 1–160 characters".into());
    }
    let mut supporting = BTreeMap::new();
    for finding in &note.findings {
        if finding
            .title
            .as_ref()
            .is_some_and(|title| title.trim().is_empty() || title.chars().count() > 160)
        {
            return Err("finding title must contain 1–160 characters".into());
        }
        if finding.statement.trim().is_empty() {
            return Err("finding statement is empty".into());
        }
        if matches!(finding.kind, FindingKind::Observed) && finding.evidence.is_empty() {
            return Err("observed findings need saved query evidence".into());
        }
    }
    for reference in note
        .findings
        .iter()
        .flat_map(|finding| finding.evidence.iter())
        .chain(note.visuals.iter().map(|visual| &visual.evidence))
        .chain(process_model.iter().flat_map(|model| model.queries()))
        .chain(
            current_process_model
                .iter()
                .flat_map(|model| model.queries()),
        )
    {
        let name = reference
            .strip_prefix("exports/query-")
            .and_then(|v| v.strip_suffix(".json"))
            .ok_or("evidence must reference exports/query-<id>.json")?;
        if name.is_empty() || !name.bytes().all(|c| c.is_ascii_digit()) {
            return Err("invalid evidence reference".into());
        }
        let directory = folder.root.join("exports");
        if directory.canonicalize().map_err(|e| e.to_string())? != directory {
            return Err("evidence directory cannot be a symbolic link".into());
        }
        let evidence: Value = serde_json::from_slice(&read_bounded(&folder.root.join(reference))?)
            .map_err(|e| e.to_string())?;
        if evidence["project"]["id"] != folder.manifest.id
            || evidence["scope"] != serialize(&state)?
            || evidence["status"] != "complete"
        {
            return Err(
                "evidence belongs to another project or older settings; rerun the query".into(),
            );
        }
        supporting.insert(reference.clone(), evidence);
    }
    let visuals = note
        .visuals
        .iter()
        .map(|visual| visual_result(visual, &supporting[&visual.evidence]))
        .collect::<Result<Vec<_>, _>>()?;
    let recorded_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    if let Some(model) = &process_model {
        model.validate(&supporting)?;
        if process_model_recorded_at.is_none() {
            process_model_recorded_at = Some(recorded_at.clone());
        }
    }
    if let Some(model) = &current_process_model {
        model.validate(&supporting)?;
    }
    let process_profile = note.process_type.map(|kind| kind.profile("agent"));
    let result = json!({"version":1,"project":folder_info(&folder),"scope":state,"note":note,"process_profile":process_profile,"recorded_at":recorded_at,"supporting_evidence":supporting,"visuals":visuals,
        "intention":intention,"process_model":process_model,"current_process_model":current_process_model,"process_model_origin":process_model_origin,"process_model_recorded_at":process_model_recorded_at,
        "provenance":"Agent-authored interpretation; engine validates references and scope, not the truth of the narrative."});
    if !folder.settings_are_current().map_err(|e| e.to_string())? {
        return Err("project settings changed while saving the answer; rerun its queries".into());
    }
    let path = snapshot(&folder, "findings", "investigation", &result)?;
    print_json(&json!({"status":"saved","project":folder_info(&folder),"artifact":path}))?;
    Ok(ExitCode::SUCCESS)
}

pub(super) fn records(folder: &WorkspaceFolder) -> Result<Vec<String>, String> {
    let directory = folder.root.join("findings");
    if directory.canonicalize().map_err(|e| e.to_string())? != directory {
        return Err("findings directory cannot be a symbolic link".into());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().map_err(|e| e.to_string())?.is_file()
            && name.starts_with("investigation-")
            && name.ends_with(".json")
        {
            names.push(format!("findings/{name}"));
        }
    }
    names.sort();
    Ok(names)
}
