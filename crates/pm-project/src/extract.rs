//! Reviewed extraction plans. Bulk records stay in files, never command output.
//!
//! The approved window selects *items*, not rows. An item is in scope when it
//! has recorded activity inside the window; the extract then takes its full
//! recorded history up to the window end, including history from before the
//! window. Cutting history at the window start would make every item that was
//! already under way look as if it began there, and would drop the record of
//! an item whose history falls in the window but whose creation does not.
//! Where each item sits against the window is labelled later, never used to
//! drop it.
//!
//! A spec is only ever *requested* by an agent. A person approves it in the
//! app, and the app runs it: see [`request`], [`approve`] and [`download`].
//! Every step is appended to the project's audit log.
use crate::audit;
use crate::extract_hubspot as hubspot;
use crate::extract_jira as jira;
use crate::extract_servicenow as servicenow;
use crate::workspace::WorkspaceFolder;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The Salesforce CLI as the system starts it. On Windows its installer
/// adds `sf.cmd`, which is only found under that full name.
pub fn salesforce_cli() -> std::path::PathBuf {
    std::path::PathBuf::from(if cfg!(windows) { "sf.cmd" } else { "sf" })
}

#[cfg(test)]
mod salesforce_cli_name {
    #[test]
    fn is_sf_cmd_on_windows_and_sf_elsewhere() {
        let name = super::salesforce_cli();
        if cfg!(windows) {
            assert_eq!(name, std::path::PathBuf::from("sf.cmd"));
        } else {
            assert_eq!(name, std::path::PathBuf::from("sf"));
        }
    }
}

const SPEC_LIMIT: usize = 64 * 1024;

/// Item identifiers per query. Each Salesforce ID adds about 21 characters,
/// keeping a query well inside SOQL's 100,000-character limit.
const ID_CHUNK: usize = 3_000;
const ITEMS_PLACEHOLDER: &str = "<items active in the window>";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    version: u32,
    system: String,
    target_org: String,
    api_version: u32,
    start: String,
    end: String,
    objects: Vec<Object>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Object {
    name: String,
    fields: Vec<String>,
    time_field: String,
    /// Whether `time_field` records a time of day or only a date.
    #[serde(default)]
    time_type: TimeType,
    kind: Kind,
    /// History only: the field naming the item each row belongs to.
    #[serde(default)]
    item_field: Option<String>,
    /// History only: the records object whose items this history describes.
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    filters: Vec<Filter>,
    // These are user-supplied scope limitations, not verified API capabilities.
    history_gaps: Vec<String>,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Records,
    History,
}
#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TimeType {
    #[default]
    Datetime,
    /// A calendar date with no time of day, such as an opportunity close date.
    Date,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Filter {
    field: String,
    equals: String,
}

/// One query the runner will issue, with the IDs left open until scanning.
#[derive(Debug, Clone, PartialEq)]
struct Query {
    /// Spec object index.
    object: usize,
    /// Scope group whose item IDs fill `{ids}`; `None` for a plain window query.
    group: Option<String>,
    /// SOQL, with `{ids}` standing for a chunk of item IDs.
    soql: String,
}

/// The reviewed plan: scans find items active in the window, exports read them.
#[derive(Debug)]
struct Plan {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    scans: Vec<Query>,
    exports: Vec<Query>,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.as_bytes()[0].is_ascii_alphabetic()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
fn instant(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&Utc))
        .map_err(|_| "start and end must be RFC3339 timestamps".into())
}
/// Reads an exported time value of the declared type as an instant.
fn exported(value: &str, kind: TimeType) -> Result<DateTime<Utc>, String> {
    match kind {
        TimeType::Datetime => instant(value)
            .map_err(|_| "export contains a value that is not a date and time".to_owned()),
        TimeType::Date => NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map(|date| date.and_time(chrono::NaiveTime::MIN).and_utc())
            .map_err(|_| "export contains a value that is not a YYYY-MM-DD date".into()),
    }
}
fn literal(value: &str) -> Result<String, String> {
    if value.len() > 256 || value.chars().any(char::is_control) {
        return Err(
            "filter values must be at most 256 characters with no control characters".into(),
        );
    }
    Ok(format!(
        "'{}'",
        value.replace('\\', "\\\\").replace('\'', "\\'")
    ))
}
fn bound(at: DateTime<Utc>, kind: TimeType) -> String {
    match kind {
        TimeType::Datetime => at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        TimeType::Date => at.format("%Y-%m-%d").to_string(),
    }
}

fn plan(spec: &Spec) -> Result<Plan, String> {
    if spec.version != 2 || spec.system != "salesforce" {
        return Err("this runner supports extraction spec v2 for salesforce".into());
    }
    if spec.target_org.is_empty()
        || spec.target_org.len() > 100
        || !spec
            .target_org
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(
            "target_org must be a saved Salesforce CLI alias, not a credential or URL".into(),
        );
    }
    if !(60..=99).contains(&spec.api_version) {
        return Err("api_version must be an explicit Salesforce API version (60–99); tenant support is checked by Salesforce".into());
    }
    let start = instant(&spec.start)?;
    let end = instant(&spec.end)?;
    if start >= end {
        return Err("start must precede end (end is exclusive)".into());
    }
    // Do not silently discard fractional seconds in an approved time window.
    if start.timestamp_subsec_nanos() != 0 || end.timestamp_subsec_nanos() != 0 {
        return Err("time bounds must use whole seconds".into());
    }
    if spec.objects.is_empty() || spec.objects.len() > 16 {
        return Err("spec needs 1–16 objects".into());
    }
    let mut names = BTreeSet::new();
    for object in &spec.objects {
        if !identifier(&object.name)
            || !names.insert(&object.name)
            || !identifier(&object.time_field)
            || object.fields.is_empty()
            || object.fields.len() > 64
            || object.fields.iter().any(|field| !identifier(field))
            || object.fields.iter().collect::<BTreeSet<_>>().len() != object.fields.len()
            || !object.fields.contains(&object.time_field)
            || object.filters.len() > 16
        {
            return Err("objects need unique identifiers, 1–64 unique fields including time_field, and at most 16 filters".into());
        }
        if object.history_gaps.len() > 16
            || object
                .history_gaps
                .iter()
                .any(|gap| gap.len() > 512 || gap.chars().any(char::is_control))
        {
            return Err("history_gaps must be bounded plain-language limitations".into());
        }
        if object
            .filters
            .iter()
            .any(|filter| !identifier(&filter.field))
        {
            return Err("filter field must be an identifier".into());
        }
        if object.time_type == TimeType::Date
            && [start, end]
                .iter()
                .any(|at| at.time() != chrono::NaiveTime::MIN)
        {
            return Err(format!(
                "{} has a date-only time field, so start and end must fall at midnight UTC",
                object.name
            ));
        }
        match object.kind {
            Kind::History => {
                let item = object.item_field.as_ref().ok_or_else(|| {
                    format!(
                        "{} is history and needs item_field, the field naming its item",
                        object.name
                    )
                })?;
                if !object.fields.contains(item) {
                    return Err(format!(
                        "{}: item_field must be one of the selected fields",
                        object.name
                    ));
                }
                if let Some(parent) = &object.parent {
                    let records = spec.objects.iter().find(|candidate| {
                        &candidate.name == parent && candidate.kind == Kind::Records
                    });
                    if !records.is_some_and(|records| records.fields.iter().any(|f| f == "Id")) {
                        return Err(format!(
                            "{}: parent must name a records object in this spec that selects Id",
                            object.name
                        ));
                    }
                }
            }
            Kind::Records => {
                if object.item_field.is_some() || object.parent.is_some() {
                    return Err(format!(
                        "{}: item_field and parent apply to history objects only",
                        object.name
                    ));
                }
            }
        }
    }

    let filters = |object: &Object| -> Result<Vec<String>, String> {
        object
            .filters
            .iter()
            .map(|filter| Ok(format!("{} = {}", filter.field, literal(&filter.equals)?)))
            .collect()
    };
    let window = |object: &Object| {
        vec![
            format!(
                "{} >= {}",
                object.time_field,
                bound(start, object.time_type)
            ),
            format!("{} < {}", object.time_field, bound(end, object.time_type)),
        ]
    };
    let select = |fields: &str, object: &Object, clauses: Vec<String>| {
        format!(
            "SELECT {fields} FROM {} WHERE {}",
            object.name,
            clauses.join(" AND ")
        )
    };
    // A records object is item-scoped when some history describes its items.
    let parents: BTreeSet<&str> = spec
        .objects
        .iter()
        .filter_map(|object| object.parent.as_deref())
        .collect();
    let mut scans = Vec::new();
    let mut exports = Vec::new();
    for (index, object) in spec.objects.iter().enumerate() {
        let fields = object.fields.join(",");
        match object.kind {
            Kind::Records if parents.contains(object.name.as_str()) => {
                let group = object.name.clone();
                scans.push(Query {
                    object: index,
                    group: Some(group.clone()),
                    soql: select("Id", object, [window(object), filters(object)?].concat()),
                });
                exports.push(Query {
                    object: index,
                    group: Some(group),
                    soql: select(
                        &fields,
                        object,
                        [vec!["Id IN ({ids})".to_owned()], filters(object)?].concat(),
                    ),
                });
            }
            Kind::Records => exports.push(Query {
                object: index,
                group: None,
                soql: select(&fields, object, [window(object), filters(object)?].concat()),
            }),
            Kind::History => {
                let item = object.item_field.as_deref().expect("validated");
                let group = object.parent.clone().unwrap_or_else(|| object.name.clone());
                // Id keeps every scan row non-blank, so the downloaded row
                // count can be reconciled even when an item reference is empty.
                scans.push(Query {
                    object: index,
                    group: Some(group.clone()),
                    soql: select(
                        &format!("Id,{item}"),
                        object,
                        [window(object), filters(object)?].concat(),
                    ),
                });
                let before_end =
                    format!("{} < {}", object.time_field, bound(end, object.time_type));
                exports.push(Query {
                    object: index,
                    group: Some(group),
                    soql: select(
                        &fields,
                        object,
                        [
                            vec![format!("{item} IN ({{ids}})"), before_end],
                            filters(object)?,
                        ]
                        .concat(),
                    ),
                });
            }
        }
    }
    Ok(Plan {
        start,
        end,
        scans,
        exports,
    })
}

/// A spec that passed review: its exact bytes, fingerprint and query plan.
#[derive(Debug)]
pub struct Reviewed {
    source: Source,
    bytes: Vec<u8>,
    /// SHA-256 of the exact spec bytes a person approves.
    pub sha256: String,
}

/// The system a reviewed spec reads, with its plan.
enum Source {
    Salesforce {
        spec: Spec,
        plan: Plan,
    },
    Jira {
        spec: jira::Spec,
        plan: jira::Plan,
    },
    ServiceNow {
        spec: servicenow::Spec,
        plan: servicenow::Plan,
    },
    HubSpot {
        spec: hubspot::Spec,
        plan: hubspot::Plan,
    },
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Salesforce { .. } => "Salesforce",
            Self::Jira { .. } => "Jira",
            Self::ServiceNow { .. } => "ServiceNow",
            Self::HubSpot { .. } => "HubSpot",
        })
    }
}

impl Reviewed {
    /// What the person approving reviews; no record is read to build it.
    pub fn preview(&self) -> Value {
        match &self.source {
            Source::Salesforce { spec, plan } => preview(spec, plan, &self.sha256),
            Source::Jira { spec, plan } => jira::preview(spec, plan, &self.sha256),
            Source::ServiceNow { spec, plan } => servicenow::preview(spec, plan, &self.sha256),
            Source::HubSpot { spec, plan } => hubspot::preview(spec, plan, &self.sha256),
        }
    }

    /// The system, the connection, the window and the files, for the audit log.
    fn summary(&self) -> Value {
        match &self.source {
            Source::Salesforce { spec, .. } => {
                json!({"system":spec.system,"target_org":spec.target_org,
                "window":{"start":spec.start,"end_exclusive":spec.end},
                "objects":spec.objects.iter().map(|o| &o.name).collect::<Vec<_>>()})
            }
            Source::Jira { spec, .. } => json!({"system":"jira","site":spec.site,
                "window":{"start":spec.start,"end_exclusive":spec.end},
                "objects":["JiraIssue","JiraIssueHistory"],"projects":spec.projects}),
            Source::ServiceNow { spec, plan } => {
                json!({"system":"servicenow","instance":spec.instance,
                "window":{"start":spec.start,"end_exclusive":spec.end},
                "objects":Self::servicenow_files(plan),"tables":plan.tables.iter().map(|t| &t.table).collect::<Vec<_>>()})
            }
            Source::HubSpot { spec, plan } => json!({"system":"hubspot","site":hubspot::API,
                "window":{"start":spec.start,"end_exclusive":spec.end},
                "objects":hubspot::files(plan)}),
        }
    }

    /// The files an extract writes, by name without extension.
    fn files(&self) -> Vec<String> {
        match &self.source {
            Source::Salesforce { spec, .. } => {
                spec.objects.iter().map(|o| o.name.clone()).collect()
            }
            Source::Jira { .. } => vec!["JiraIssue".into(), "JiraIssueHistory".into()],
            Source::ServiceNow { plan, .. } => Self::servicenow_files(plan),
            Source::HubSpot { plan, .. } => hubspot::files(plan),
        }
    }

    fn servicenow_files(plan: &servicenow::Plan) -> Vec<String> {
        plan.tables
            .iter()
            .map(|t| t.file.clone())
            .chain([servicenow::AUDIT_FILE.to_owned()])
            .collect()
    }
}

/// Checks a spec and plans its queries without contacting the system.
pub fn review(bytes: &[u8]) -> Result<Reviewed, String> {
    if bytes.len() > SPEC_LIMIT {
        return Err("extract spec exceeds 64 KiB".into());
    }
    let system = serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|v| v["system"].as_str().map(str::to_owned));
    let source = if system.as_deref() == Some("servicenow") {
        let spec: servicenow::Spec = serde_json::from_slice(bytes)
            .map_err(|_| "invalid extract spec; unknown fields and credentials are not accepted")?;
        let plan = servicenow::plan(&spec)?;
        Source::ServiceNow { spec, plan }
    } else if system.as_deref() == Some("hubspot") {
        let spec: hubspot::Spec = serde_json::from_slice(bytes)
            .map_err(|_| "invalid extract spec; unknown fields and credentials are not accepted")?;
        let plan = hubspot::plan(&spec)?;
        Source::HubSpot { spec, plan }
    } else if system.as_deref() == Some("jira") {
        let spec: jira::Spec = serde_json::from_slice(bytes)
            .map_err(|_| "invalid extract spec; unknown fields and credentials are not accepted")?;
        let plan = jira::plan(&spec)?;
        Source::Jira { spec, plan }
    } else {
        let spec: Spec = serde_json::from_slice(bytes)
            .map_err(|_| "invalid extract spec; unknown fields and credentials are not accepted")?;
        let plan = plan(&spec)?;
        Source::Salesforce { spec, plan }
    };
    Ok(Reviewed {
        source,
        bytes: bytes.to_vec(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
    })
}

fn requests_dir(root: &Path) -> PathBuf {
    root.join("extraction").join("requests")
}

fn request_dir(root: &Path, sha256: &str) -> Result<PathBuf, String> {
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err("unknown extract request".into());
    }
    Ok(requests_dir(root).join(sha256))
}

fn regular_dir(path: &Path) -> Result<(), String> {
    if path.exists()
        && !fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
    {
        return Err("extraction directory is not a regular project directory".into());
    }
    Ok(())
}

fn read_request(root: &Path, sha256: &str) -> Result<Value, String> {
    let path = request_dir(root, sha256)?.join("request.json");
    let bytes = fs::read(&path).map_err(|_| "unknown extract request")?;
    serde_json::from_slice(&bytes).map_err(|_| "unreadable extract request".into())
}

fn write_request(root: &Path, request: &Value) -> Result<(), String> {
    let sha256 = request["spec_sha256"]
        .as_str()
        .ok_or("invalid extract request")?;
    let path = request_dir(root, sha256)?.join("request.json");
    let mut file = atomic_write_file::AtomicWriteFile::open(&path)
        .map_err(|_| "cannot save extract request")?;
    file.write_all(&serde_json::to_vec_pretty(request).map_err(|e| e.to_string())?)
        .map_err(|_| "cannot save extract request")?;
    file.commit()
        .map_err(|_| "cannot save extract request".into())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Files an extract for a person to approve in the app.
///
/// The agent's tools stop here: nothing is downloaded until someone approves
/// this exact spec in the browser, and the app, not the agent, then runs it.
/// Needs no project lock, so it works while the project is open in the app.
/// Filing the same spec again returns the open request; after a finished,
/// failed or declined run it opens a new request needing a new approval.
pub fn request(root: &Path, reviewed: &Reviewed) -> Result<Value, String> {
    regular_dir(&root.join("extraction"))?;
    regular_dir(&requests_dir(root))?;
    let dir = request_dir(root, &reviewed.sha256)?;
    if let Ok(existing) = read_request(root, &reviewed.sha256)
        && matches!(existing["status"].as_str(), Some("pending" | "running"))
    {
        return Ok(existing);
    }
    fs::create_dir_all(&dir).map_err(|_| "cannot create extract request")?;
    fs::write(dir.join("spec.json"), &reviewed.bytes).map_err(|_| "cannot save extract spec")?;
    let request = json!({"spec_sha256":reviewed.sha256,"status":"pending","requested_at":now(),
        "preview":reviewed.preview()});
    write_request(root, &request)?;
    let mut details = reviewed.summary();
    details["spec_sha256"] = json!(reviewed.sha256);
    audit::append(root, "extract_requested", details)?;
    Ok(request)
}

/// Every extract request in the project, newest first.
pub fn requests(root: &Path) -> Result<Vec<Value>, String> {
    let dir = requests_dir(root);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    regular_dir(&dir)?;
    let mut found = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|_| "cannot read extract requests")? {
        let name = entry
            .map_err(|_| "cannot read extract requests")?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if let Ok(request) = read_request(root, &name) {
            found.push(request);
        }
    }
    found.sort_by(|a, b| b["requested_at"].as_str().cmp(&a["requested_at"].as_str()));
    Ok(found)
}

fn person(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
        return Err("enter the name of the person approving, up to 120 characters".into());
    }
    Ok(name.to_owned())
}

fn os_user() -> Value {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .map_or(Value::Null, Value::from)
}

/// Records a person's approval of a pending request and marks it running.
pub fn approve(root: &Path, sha256: &str, approver: &str) -> Result<Value, String> {
    let approver = person(approver)?;
    let mut request = read_request(root, sha256)?;
    if request["status"] != "pending" {
        return Err("this extract is not waiting for approval".into());
    }
    // Approval covers the exact bytes on disk, checked again here.
    let bytes = fs::read(request_dir(root, sha256)?.join("spec.json"))
        .map_err(|_| "cannot read the requested spec")?;
    if review(&bytes)?.sha256 != sha256 {
        return Err("the requested spec changed after it was filed; ask for a new request".into());
    }
    request["status"] = json!("running");
    request["approved_by"] = json!(approver);
    request["approved_at"] = json!(now());
    request["approved_os_user"] = os_user();
    write_request(root, &request)?;
    audit::append(
        root,
        "extract_approved",
        json!({"spec_sha256":sha256,"approved_by":approver}),
    )?;
    Ok(request)
}

/// Records that a person declined a pending request.
pub fn decline(root: &Path, sha256: &str, by: &str) -> Result<Value, String> {
    let by = person(by)?;
    let mut request = read_request(root, sha256)?;
    if request["status"] != "pending" {
        return Err("this extract is not waiting for approval".into());
    }
    request["status"] = json!("declined");
    request["declined_by"] = json!(by);
    request["declined_at"] = json!(now());
    write_request(root, &request)?;
    audit::append(
        root,
        "extract_declined",
        json!({"spec_sha256":sha256,"declined_by":by}),
    )?;
    Ok(request)
}

/// Marks an approved run failed. The message is the runner's sanitized one.
pub fn fail(root: &Path, sha256: &str, reason: &str) -> Result<(), String> {
    let mut request = read_request(root, sha256)?;
    request["status"] = json!("failed");
    request["finished_at"] = json!(now());
    request["error"] = json!(reason);
    write_request(root, &request)?;
    audit::append(
        root,
        "extract_failed",
        json!({"spec_sha256":sha256,"reason":reason}),
    )
}

/// Downloaded, reconciled exports waiting to be registered with the project.
#[derive(Debug)]
pub struct Staged {
    stage: tempfile::TempDir,
    reviewed: Reviewed,
    reports: Vec<Value>,
}

/// Runs an approved request: scans for items active in the window, then reads
/// their records and full history, into a staging folder inside the project.
///
/// Needs no project lock, so a long download never blocks the app. `sf` is
/// the Salesforce CLI to run.
pub fn download(root: &Path, sha256: &str, sf: &Path) -> Result<Staged, String> {
    run(
        root,
        sha256,
        &Tools {
            salesforce_cli: sf.to_owned(),
            curl: PathBuf::from("curl"),
            jira_user: None,
            servicenow_user: None,
            hubspot_token: None,
        },
    )
}

/// The external programs extracts use.
#[derive(Debug, Clone)]
pub struct Tools {
    /// The official Salesforce CLI.
    pub salesforce_cli: PathBuf,
    /// curl, for REST APIs such as Jira's.
    pub curl: PathBuf,
    /// `email:token` for Jira; `None` reads `JIRA_EMAIL` and `JIRA_API_TOKEN`
    /// from the environment.
    pub jira_user: Option<String>,
    /// `user:password` for ServiceNow; `None` reads `SERVICENOW_USER` and
    /// `SERVICENOW_PASSWORD` from the environment.
    pub servicenow_user: Option<String>,
    /// A HubSpot service key (or legacy private app token); `None` reads `HUBSPOT_TOKEN` from the
    /// environment.
    pub hubspot_token: Option<String>,
}

/// Runs an approved request with the transport its system needs.
pub fn run(root: &Path, sha256: &str, tools: &Tools) -> Result<Staged, String> {
    let sf = tools.salesforce_cli.as_path();
    let request = read_request(root, sha256)?;
    if request["status"] != "running" || !request["approved_by"].is_string() {
        return Err("this extract has not been approved in the app".into());
    }
    let bytes = fs::read(request_dir(root, sha256)?.join("spec.json"))
        .map_err(|_| "cannot read the approved spec")?;
    let reviewed = review(&bytes)?;
    if reviewed.sha256 != sha256 {
        return Err("the approved spec changed; nothing was downloaded".into());
    }
    let stage = tempfile::Builder::new()
        .prefix(".extract-")
        .tempdir_in(root)
        .map_err(|_| "cannot create project staging directory")?;
    fs::write(stage.path().join("spec.json"), &bytes)
        .map_err(|_| "cannot save extract specification")?;
    let (spec, plan) = match &reviewed.source {
        Source::Salesforce { spec, plan } => (spec, plan),
        Source::ServiceNow { spec, plan } => {
            let user = match &tools.servicenow_user {
                Some(user) => user.clone(),
                None => servicenow::user_from_environment()?,
            };
            let reports = servicenow::download(spec, plan, &tools.curl, &user, stage.path())?;
            return Ok(Staged {
                stage,
                reviewed,
                reports,
            });
        }
        Source::HubSpot { spec, plan } => {
            let token = match &tools.hubspot_token {
                Some(token) => token.clone(),
                None => hubspot::token_from_environment()?,
            };
            let reports = hubspot::download(spec, plan, &tools.curl, &token, stage.path())?;
            return Ok(Staged {
                stage,
                reviewed,
                reports,
            });
        }
        Source::Jira { spec, plan } => {
            let user = match &tools.jira_user {
                Some(user) => user.clone(),
                None => jira::user_from_environment()?,
            };
            let reports = jira::download(spec, plan, &tools.curl, &user, stage.path())?;
            return Ok(Staged {
                stage,
                reviewed,
                reports,
            });
        }
    };

    // Pass one: which items had activity inside the window.
    let mut items: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (number, scan) in plan.scans.iter().enumerate() {
        let output = stage.path().join(format!("scan-{number}.csv"));
        let expected = export(sf, spec, &scan.soql, &output, stage.path())?;
        let column = usize::from(spec.objects[scan.object].kind == Kind::History);
        let found = scanned_items(&output, expected, column)?;
        items
            .entry(scan.group.clone().expect("scans are grouped"))
            .or_default()
            .extend(found);
        fs::remove_file(&output).map_err(|_| "cannot remove staged scan")?;
    }

    // Pass two: the records and full history of those items.
    let mut reports = Vec::new();
    for query in &plan.exports {
        let object = &spec.objects[query.object];
        let merged = stage.path().join(format!("{}.csv", object.name));
        fs::write(&merged, format!("{}\n", object.fields.join(",")))
            .map_err(|_| "cannot stage export")?;
        let mut tally = Tally::default();
        let chunks: Vec<Option<String>> = match &query.group {
            None => vec![None],
            Some(group) => {
                let ids: Vec<&String> = items.get(group).into_iter().flatten().collect();
                ids.chunks(ID_CHUNK)
                    .map(|chunk| {
                        let list = chunk
                            .iter()
                            .map(|id| literal(id))
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok(Some(list.join(",")))
                    })
                    .collect::<Result<_, String>>()?
            }
        };
        for (number, ids) in chunks.iter().enumerate() {
            let soql = match ids {
                Some(ids) => query.soql.replace("{ids}", ids),
                None => query.soql.clone(),
            };
            let output = stage.path().join(format!("part-{number}.csv"));
            let expected = export(sf, spec, &soql, &output, stage.path())?;
            let lower = query.group.is_none().then_some(plan.start);
            let upper = (query.group.is_none() || object.kind == Kind::History).then_some(plan.end);
            let part = inspect_csv(&output, object, lower, upper, plan.start)?;
            if part.rows != expected {
                return Err("downloaded row count does not match Salesforce's completion count; extraction stopped".into());
            }
            append_rows(&output, &merged)?;
            tally.add(part);
            fs::remove_file(&output).map_err(|_| "cannot remove staged export part")?;
        }
        reports.push(json!({"object":object.name,"kind":object.kind,
            "scope":if query.group.is_some() {"items_active_in_window"} else {"window"},
            "items_in_scope":query.group.as_ref().map(|group| items.get(group).map_or(0, BTreeSet::len)),
            "history_lower_bound":(object.kind == Kind::History && query.group.is_some()).then_some("none: full recorded history up to the window end"),
            "fields":object.fields,"time_field":object.time_field,"time_type":object.time_type,
            "jobs":chunks.len(),"rows":tally.rows,"rows_before_window":tally.before,
            "observed_start":tally.minimum.map(|v| v.to_rfc3339()),"observed_end":tally.maximum.map(|v| v.to_rfc3339()),
            "history_gaps":object.history_gaps,"history_coverage":"unverified","empty":tally.rows == 0}));
    }
    Ok(Staged {
        stage,
        reviewed,
        reports,
    })
}

/// Registers a completed download: hashed sources, the window, the report and
/// the audit entry. Holds the project lock only for this short step.
pub fn register(folder: &mut WorkspaceFolder, staged: Staged) -> Result<Value, String> {
    let Staged {
        stage,
        reviewed,
        reports,
    } = staged;
    let files = reviewed.files();
    let mut assets = Vec::new();
    for name in &files {
        let asset = folder
            .store_asset(
                &stage.path().join(format!("{name}.csv")),
                &format!("{name}.csv"),
                ".csv",
                "sources",
            )
            .map_err(|error| error.to_string())?;
        assets.push(asset);
    }
    let (window_start, window_end) = match &reviewed.source {
        Source::Salesforce { plan, .. } => (plan.start, plan.end),
        Source::Jira { plan, .. } => (plan.start, plan.end),
        Source::ServiceNow { plan, .. } => (plan.start, plan.end),
        Source::HubSpot { plan, .. } => (plan.start, plan.end),
    };
    let mut state = folder.state().map_err(|error| error.to_string())?;
    for asset in &assets {
        if !state
            .preparation_sources
            .iter()
            .any(|existing| existing.sha256 == asset.sha256)
        {
            state.preparation_sources.push(asset.clone());
        }
    }
    let end_activities = state
        .observation_window
        .take()
        .map(|window| window.end_activities)
        .unwrap_or_default();
    state.observation_window = Some(pm_core::ObservationWindow {
        start_micros: window_start.timestamp_micros(),
        end_micros: window_end.timestamp_micros(),
        end_activities,
    });
    let request = read_request(&folder.root, &reviewed.sha256)?;
    let report = match &reviewed.source {
        Source::Salesforce { spec, .. } => {
            json!({"version":2,"status":"complete","system":"salesforce","transport":"official_sf_cli_bulk_api_2",
        "api_version":spec.api_version,"target_org":spec.target_org,"spec_sha256":reviewed.sha256,
        "approved_by":request["approved_by"],"approved_at":request["approved_at"],
        "requested_range":{"start":spec.start,"end_exclusive":spec.end},
        "scope":"Items with recorded activity in the window, with their full recorded history up to the window end.",
        "objects":reports,"sources":assets,
        "api_calls":null,"truncated":false,"history_coverage":"unverified",
        "limits":["History tracking and retention must be verified separately.","Objects are exported in separate jobs; this is not a cross-object transactional snapshot.","The connector does not infer activities or causes."]})
        }
        Source::Jira { spec, .. } => {
            json!({"version":2,"status":"complete","system":"jira","transport":"jira_cloud_rest_v3_over_curl",
        "site":spec.site,"target_org":spec.site,"projects":spec.projects,"spec_sha256":reviewed.sha256,
        "approved_by":request["approved_by"],"approved_at":request["approved_at"],
        "requested_range":{"start":spec.start,"end_exclusive":spec.end},
        "scope":"Issues with recorded activity in the window, with their full change history up to the window end.",
        "objects":reports,"sources":assets,
        "api_calls":reports.first().map(|r| r["api_calls"].clone()),"truncated":false,"history_coverage":"unverified",
        "limits":["Issues and history are read page by page, not as one snapshot; changes made during the read may be missed or included.","The connector does not infer activities or causes."]})
        }
        Source::HubSpot { spec, .. } => {
            json!({"version":2,"status":"complete","system":"hubspot","transport":"hubspot_crm_api_over_curl",
        "site":hubspot::API,"target_org":hubspot::API,"spec_sha256":reviewed.sha256,
        "approved_by":request["approved_by"],"approved_at":request["approved_at"],
        "requested_range":{"start":spec.start,"end_exclusive":spec.end},
        "scope":"Records changed in the window, with the full history of the listed properties up to the window end, and the links between them.",
        "objects":reports,"sources":assets,
        "api_calls":reports.first().map(|r| r["api_calls"].clone()),"truncated":false,"history_coverage":"unverified",
        "limits":["Records are read page by page, not as one snapshot; changes made during the read may be missed or included.","The connector does not infer activities or causes."]})
        }
        Source::ServiceNow { spec, .. } => {
            json!({"version":2,"status":"complete","system":"servicenow","transport":"servicenow_table_api_over_curl",
        "instance":spec.instance,"target_org":spec.instance,"spec_sha256":reviewed.sha256,
        "approved_by":request["approved_by"],"approved_at":request["approved_at"],
        "requested_range":{"start":spec.start,"end_exclusive":spec.end},
        "scope":"Records with recorded activity in the window, their child records, and their full audit history up to the window end.",
        "objects":reports,"sources":assets,
        "api_calls":reports.first().map(|r| r["api_calls"].clone()),"truncated":false,"history_coverage":"unverified",
        "limits":["Records and audit history are read page by page, not as one snapshot; changes made during the read may be missed or included.","Only audited fields leave history.","The connector does not infer activities or causes."]})
        }
    };
    fs::write(
        stage.path().join("report.json"),
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|_| "cannot write extraction report")?;
    for name in &files {
        fs::remove_file(stage.path().join(format!("{name}.csv")))
            .map_err(|_| "cannot remove staged source copy")?;
    }
    let extraction = folder.root.join("extraction");
    regular_dir(&extraction)?;
    fs::create_dir_all(&extraction).map_err(|_| "cannot create extraction report directory")?;
    let name = stage
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("invalid extraction run name")?
        .trim_start_matches('.')
        .to_owned();
    let destination = extraction.join(&name);
    // Publish a complete report before registering inputs. A failed state save leaves
    // a recoverable report, never a claim that the inputs were registered successfully.
    fs::rename(stage.path(), &destination).map_err(|_| "cannot publish extraction report")?;
    folder
        .save_state(&state)
        .map_err(|error| error.to_string())?;
    let row_counts = reports
        .iter()
        .map(|r| json!({"object":r["object"],"rows":r["rows"],"rows_before_window":r["rows_before_window"]}))
        .collect::<Vec<_>>();
    let result = json!({"status":"complete","spec_sha256":reviewed.sha256,
        "report":format!("extraction/{name}/report.json"),"sources":assets,"row_counts":row_counts,
        "history_coverage":"unverified"});
    let mut request = request;
    request["status"] = json!("complete");
    request["finished_at"] = json!(now());
    request["result"] = result.clone();
    write_request(&folder.root, &request)?;
    audit::append(
        &folder.root,
        "extract_completed",
        json!({"spec_sha256":reviewed.sha256,
        "row_counts":row_counts,"report":result["report"],
        "outputs":assets.iter().map(|a| json!({"name":a.name,"path":a.relative_path,"sha256":a.sha256})).collect::<Vec<_>>()}),
    )?;
    Ok(result)
}

/// What the person approving the spec reviews: every query, with item lists
/// shown as the placeholder the scans will fill.
fn preview(spec: &Spec, plan: &Plan, sha256: &str) -> Value {
    let shown = |query: &Query| json!({"object":spec.objects[query.object].name,"soql":query.soql.replace("{ids}", ITEMS_PLACEHOLDER)});
    json!({"status":"needs_approval","system":spec.system,"target_org":spec.target_org,
        "api_version":spec.api_version,"spec_sha256":sha256,"start":spec.start,"end":spec.end,
        "scope":"Items with recorded activity in the window, with their full recorded history up to the window end. History before the window is included.",
        "scans":plan.scans.iter().map(shown).collect::<Vec<_>>(),
        "exports":plan.exports.iter().map(shown).collect::<Vec<_>>(),
        "history_coverage":"unverified; inspect history tracking and retention before approval",
        "objects":spec.objects })
}

/// Runs one bulk query into `output` and returns Salesforce's row count.
fn export(sf: &Path, spec: &Spec, soql: &str, output: &Path, stage: &Path) -> Result<u64, String> {
    let metadata = stage.join("cli-result.json");
    let log = File::create(&metadata).map_err(|_| "cannot stage CLI completion metadata")?;
    let status = Command::new(sf).args(["data", "export", "bulk", "--target-org", &spec.target_org,
        "--api-version", &format!("{}.0", spec.api_version), "--query", soql, "--output-file"])
        .arg(output).args(["--result-format", "csv", "--wait", "10", "--json"])
        .stdin(Stdio::null()).stdout(Stdio::from(log)).stderr(Stdio::null()).status()
        .map_err(|_| "Salesforce CLI could not start. Install the official sf CLI and authenticate an approved read-only org alias.")?;
    let object = soql
        .split(" FROM ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or("the object");
    if !status.success() {
        let cause = fs::read(&metadata)
            .ok()
            .filter(|bytes| bytes.len() <= 64 * 1024)
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|reply| failure_cause(&reply))
            .map(|cause| format!(" {cause}"))
            .unwrap_or_default();
        return Err(format!(
            "Salesforce export failed for {object}.{cause} Check CLI/API support, read permissions and history availability. No preparation inputs were registered; vendor output is withheld."
        ));
    }
    let metadata_size = fs::metadata(&metadata)
        .map_err(|_| "missing CLI completion metadata")?
        .len();
    if metadata_size > 1024 * 1024 {
        return Err("unexpected CLI completion response; extraction stopped".into());
    }
    let completion: Value = serde_json::from_slice(
        &fs::read(&metadata).map_err(|_| "cannot read CLI completion metadata")?,
    )
    .map_err(|_| "invalid CLI completion response")?;
    fs::remove_file(&metadata).map_err(|_| "cannot remove vendor completion metadata")?;
    let expected = completion.pointer("/result/totalSize").and_then(Value::as_u64)
        .ok_or("export is not confirmed complete (possibly timed out); retry after reviewing the Salesforce job")?;
    if completion["status"].as_u64() != Some(0) {
        return Err("Salesforce did not confirm successful export".into());
    }
    Ok(expected)
}

/// What went wrong, in words an agent can act on, from the CLI's error reply.
/// Only the error's name and the object or field names Salesforce reports are
/// used; the rest of the vendor's message is withheld, as are all records.
fn failure_cause(reply: &Value) -> Option<String> {
    let name = reply["name"].as_str().unwrap_or_default();
    let message = reply["message"].as_str().unwrap_or_default();
    let identifier = |s: &str| {
        !s.is_empty() && s.len() <= 80 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    };
    let quoted = |after: &str| {
        message
            .split(after)
            .nth(1)
            .and_then(|rest| rest.split('\'').nth(1))
            .filter(|s| identifier(s))
            .map(str::to_owned)
    };
    if let (Some(field), Some(object)) = (quoted("No such column"), quoted("on entity")) {
        return Some(format!(
            "Salesforce reports no field {field} on {object}: it does not exist in this org or is hidden from this user (field-level security)."
        ));
    }
    if let Some(object) = quoted("sObject type") {
        return Some(format!(
            "Salesforce reports the object {object} is not available: it is not enabled in this org or this user cannot read it."
        ));
    }
    match name {
        "InvalidApiVersionError" => Some(
            "The Salesforce CLI refused the API version (InvalidApiVersionError); check the spec's api_version against what the org and CLI support.".into(),
        ),
        "NamedOrgNotFoundError" | "NoAuthInfoFound" | "NoOrgFound" => Some(format!(
            "The Salesforce CLI has no sign-in for this org alias ({name}); sign in with `sf org login web --alias <alias>`."
        )),
        "INVALID_SESSION_ID" | "RefreshTokenAuthError" => Some(format!(
            "The Salesforce sign-in has expired ({name}); sign in again with `sf org login web`."
        )),
        _ if identifier(name) => Some(format!("The Salesforce CLI reported {name}.")),
        _ => None,
    }
}

/// Item IDs from a scan's `column`. Blank references name no item.
fn scanned_items(path: &Path, expected: u64, column: usize) -> Result<BTreeSet<String>, String> {
    let mut reader = csv::Reader::from_path(path).map_err(|_| "cannot read scan result")?;
    let mut items = BTreeSet::new();
    let mut rows = 0u64;
    for record in reader.records() {
        let record = record.map_err(|_| "invalid scan record; extraction stopped")?;
        rows += 1;
        let id = record.get(column).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        if id.len() > 40 || !id.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err("a scan returned an unexpected item identifier; extraction stopped".into());
        }
        items.insert(id.to_owned());
    }
    if rows != expected {
        return Err(
            "scan row count does not match Salesforce's completion count; extraction stopped"
                .into(),
        );
    }
    Ok(items)
}

/// Appends a part's data rows, byte for byte, after the merged header.
fn append_rows(part: &Path, merged: &Path) -> Result<(), String> {
    let bytes = fs::read(part).map_err(|_| "cannot read staged export part")?;
    let body = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(&[][..], |at| &bytes[at + 1..]);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(merged)
        .map_err(|_| "cannot extend staged export")?;
    file.write_all(body)
        .map_err(|_| "cannot extend staged export")?;
    if !body.is_empty() && !body.ends_with(b"\n") {
        file.write_all(b"\n")
            .map_err(|_| "cannot extend staged export")?;
    }
    Ok(())
}

#[derive(Default)]
struct Tally {
    rows: u64,
    before: u64,
    minimum: Option<DateTime<Utc>>,
    maximum: Option<DateTime<Utc>>,
}
impl Tally {
    fn add(&mut self, part: Tally) {
        self.rows += part.rows;
        self.before += part.before;
        self.minimum = self.minimum.into_iter().chain(part.minimum).min();
        self.maximum = self.maximum.into_iter().chain(part.maximum).max();
    }
}

/// Checks one export's columns and time values; `lower`/`upper` bound them
/// when the query did.
fn inspect_csv(
    path: &Path,
    object: &Object,
    lower: Option<DateTime<Utc>>,
    upper: Option<DateTime<Utc>>,
    window_start: DateTime<Utc>,
) -> Result<Tally, String> {
    if !fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink()) {
        return Err("export did not produce a regular CSV file".into());
    }
    let mut reader = csv::Reader::from_path(path).map_err(|_| "cannot read exported CSV")?;
    let headers = reader.headers().map_err(|_| "invalid CSV header")?.clone();
    if headers.iter().collect::<Vec<_>>()
        != object.fields.iter().map(String::as_str).collect::<Vec<_>>()
    {
        return Err("exported columns differ from the approved fields".into());
    }
    let index = headers
        .iter()
        .position(|name| name == object.time_field)
        .ok_or("missing time field")?;
    let mut tally = Tally::default();
    for record in reader.records() {
        let record = record.map_err(|_| "invalid CSV record; extraction stopped")?;
        let value = record.get(index).unwrap_or("");
        // An item-scoped record may lack an optional time value; history and
        // window queries filtered on it, so theirs is always present.
        if value.is_empty() && lower.is_none() && upper.is_none() {
            tally.rows += 1;
            continue;
        }
        let date = exported(value, object.time_type)?;
        if lower.is_some_and(|start| date < start) || upper.is_some_and(|end| date >= end) {
            return Err("export contains timestamps outside the approved range".into());
        }
        tally.before += u64::from(date < window_start);
        tally.minimum = Some(tally.minimum.map_or(date, |current| current.min(date)));
        tally.maximum = Some(tally.maximum.map_or(date, |current| current.max(date)));
        tally.rows += 1;
    }
    Ok(tally)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> Spec {
        serde_json::from_value(json!({"version":2,"system":"salesforce","target_org":"test-org","api_version":66,
        "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z","objects":[{"name":"CaseHistory","fields":["Id","CaseId","CreatedDate","Field"],"time_field":"CreatedDate","kind":"history","item_field":"CaseId","filters":[{"field":"Field","equals":"Status"}],"history_gaps":[]}]})).unwrap()
    }
    fn connected() -> Spec {
        serde_json::from_value(json!({"version":2,"system":"salesforce","target_org":"test-org","api_version":66,
        "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z","objects":[
            {"name":"Opportunity","fields":["Id","CreatedDate","CloseDate"],"time_field":"CreatedDate","kind":"records","history_gaps":[]},
            {"name":"OpportunityFieldHistory","fields":["Id","OpportunityId","CreatedDate"],"time_field":"CreatedDate","kind":"history","item_field":"OpportunityId","parent":"Opportunity","history_gaps":[]}]})).unwrap()
    }
    #[test]
    fn validates_read_only_queries_and_escapes_values() {
        let mut s = spec();
        s.objects[0].filters[0].equals = "a' OR Name = 'b".into();
        assert!(
            plan(&s).unwrap().scans[0]
                .soql
                .contains("'a\\' OR Name = \\'b'")
        );
        s.objects[0].name = "Case; DELETE".into();
        assert!(plan(&s).is_err());
    }
    #[test]
    fn history_scope_reads_full_history_of_items_active_in_the_window() {
        let p = plan(&connected()).unwrap();
        let soql: Vec<&str> = p.scans.iter().map(|q| q.soql.as_str()).collect();
        assert_eq!(
            soql,
            [
                "SELECT Id FROM Opportunity WHERE CreatedDate >= 2026-01-01T00:00:00Z AND CreatedDate < 2026-02-01T00:00:00Z",
                "SELECT Id,OpportunityId FROM OpportunityFieldHistory WHERE CreatedDate >= 2026-01-01T00:00:00Z AND CreatedDate < 2026-02-01T00:00:00Z",
            ]
        );
        let soql: Vec<&str> = p.exports.iter().map(|q| q.soql.as_str()).collect();
        assert_eq!(
            soql,
            [
                "SELECT Id,CreatedDate,CloseDate FROM Opportunity WHERE Id IN ({ids})",
                // No lower bound: history from before the window is kept.
                "SELECT Id,OpportunityId,CreatedDate FROM OpportunityFieldHistory WHERE OpportunityId IN ({ids}) AND CreatedDate < 2026-02-01T00:00:00Z",
            ]
        );
        assert!(
            p.exports
                .iter()
                .all(|q| q.group.as_deref() == Some("Opportunity"))
        );
    }
    #[test]
    fn date_only_fields_use_date_literals_on_whole_days() {
        let mut s: Spec = serde_json::from_value(json!({"version":2,"system":"salesforce","target_org":"test-org","api_version":66,
            "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z","objects":[
            {"name":"Opportunity","fields":["Id","CloseDate"],"time_field":"CloseDate","time_type":"date","kind":"records","history_gaps":[]}]})).unwrap();
        assert_eq!(
            plan(&s).unwrap().exports[0].soql,
            "SELECT Id,CloseDate FROM Opportunity WHERE CloseDate >= 2026-01-01 AND CloseDate < 2026-02-01"
        );
        s.start = "2026-01-01T09:00:00Z".into();
        assert!(plan(&s).unwrap_err().contains("midnight"));
        assert_eq!(
            exported("2026-01-05", TimeType::Date).unwrap(),
            instant("2026-01-05T00:00:00Z").unwrap()
        );
        assert!(exported("2026-01-05T10:00:00Z", TimeType::Date).is_err());
    }
    #[test]
    fn rejects_ambiguous_and_changed_contracts() {
        let mut s = spec();
        s.start = "01/02/2026".into();
        assert!(plan(&s).is_err());
        let mut s = spec();
        s.end = s.start.clone();
        assert!(plan(&s).is_err());
        let mut s = spec();
        s.objects
            .push(serde_json::from_value(serde_json::to_value(&s.objects[0]).unwrap()).unwrap());
        assert!(plan(&s).is_err());
        let mut s = spec();
        s.objects[0].item_field = None;
        assert!(plan(&s).unwrap_err().contains("item_field"));
        let mut s = connected();
        s.objects[1].parent = Some("Account".into());
        assert!(plan(&s).unwrap_err().contains("parent"));
        let mut value = serde_json::to_value(spec()).unwrap();
        value["access_token"] = json!("secret");
        assert!(serde_json::from_value::<Spec>(value).is_err());
        let mut value = serde_json::to_value(spec()).unwrap();
        value["version"] = json!(1);
        assert!(plan(&serde_json::from_value(value).unwrap()).is_err());
    }
    #[test]
    fn streams_records_and_rejects_incomplete_schema_or_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("records.csv");
        let s = spec();
        let start = instant(&s.start).unwrap();
        let end = instant(&s.end).unwrap();
        let object = &s.objects[0];
        fs::write(
            &path,
            "Id,CaseId,CreatedDate,Field\n1,500A,2025-12-02T00:00:00.000Z,Status\n2,500A,2026-01-02T00:00:00Z,Status\n",
        )
        .unwrap();
        // Full history: earlier rows are allowed and counted.
        let tally = inspect_csv(&path, object, None, Some(end), start).unwrap();
        assert_eq!((tally.rows, tally.before), (2, 1));
        // A window query must not return them.
        assert!(inspect_csv(&path, object, Some(start), Some(end), start).is_err());
        fs::write(&path, "Id,Wrong,CreatedDate,Field\n").unwrap();
        assert!(inspect_csv(&path, object, None, Some(end), start).is_err());
    }
    #[test]
    fn scans_collect_distinct_items_and_reject_unexpected_identifiers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scan.csv");
        fs::write(&path, "Id,OpportunityId\n1,006A\n2,006A\n3,\n4,006B\n").unwrap();
        assert_eq!(scanned_items(&path, 4, 1).unwrap().len(), 2);
        assert!(scanned_items(&path, 5, 1).is_err());
        fs::write(&path, "Id,OpportunityId\n1,006A') OR (Id != '\n").unwrap();
        assert!(scanned_items(&path, 1, 1).is_err());
    }

    #[test]
    fn a_failed_export_says_what_salesforce_objected_to_without_its_message() {
        let field = json!({"status":1,"name":"API_ERROR","message":
            "SELECT Id, Secret FROM Lead WHERE Id IN ('00Q1')\n ^\nERROR at Row:1:Column:12\nNo such column 'ContractId' on entity 'Opportunity'. If you are attempting..."});
        let cause = failure_cause(&field).unwrap();
        assert!(
            cause.contains("no field ContractId on Opportunity"),
            "{cause}"
        );
        assert!(cause.contains("field-level security"));
        // Nothing else from the vendor's message comes through.
        assert!(!cause.contains("00Q1") && !cause.contains("Secret"));
        let object = json!({"status":1,"name":"API_ERROR","message":"sObject type 'Quote' is not supported."});
        assert!(
            failure_cause(&object)
                .unwrap()
                .contains("object Quote is not available")
        );
        let version = json!({"status":2,"name":"InvalidApiVersionError","message":"66 is not a valid API version."});
        assert!(failure_cause(&version).unwrap().contains("api_version"));
        let alias = json!({"status":2,"name":"NamedOrgNotFoundError","message":"No authorization information found for approved-org."});
        assert!(failure_cause(&alias).unwrap().contains("sf org login web"));
        // An unknown name is passed on only when it is a plain identifier.
        let odd = json!({"status":1,"name":"<script>","message":"x"});
        assert_eq!(failure_cause(&odd), None);
    }
}
