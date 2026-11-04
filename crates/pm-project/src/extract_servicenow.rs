//! ServiceNow extracts: records of a chosen table with activity in a window,
//! the records of child tables that belong to them (such as the change tasks
//! of a change), and their audit history (`sys_audit`) up to the window end,
//! read through the Table API with a read-only user's own credentials.
//!
//! The transport is the system `curl`, given its URL and credentials on
//! standard input, as for Jira. Credentials come from `SERVICENOW_USER` and
//! `SERVICENOW_PASSWORD` in the app's environment and are never written.
//!
//! Audit rows hold stored values, such as state `2`; the extract adds each
//! value's display label from the choice list, from the records' own display
//! values, or from the referenced record's name for common references. People
//! are kept as stored IDs, never names.
//!
//! Like every extract, a spec is only filed by an agent; a person approves it
//! in the app, and the app runs it.
use crate::extract_jira::{Client, encode};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::Path;

/// Records per page; the Table API answers up to 10,000.
const PAGE: usize = 1000;
/// Record IDs per `IN` query.
const BATCH: usize = 100;
/// Records an extract may read across its tables.
const RECORD_LIMIT: usize = 500_000;

/// The tables an extract may read, with the file each one is written to.
pub const TABLES: [(&str, &str); 8] = [
    ("incident", "ServiceNowIncident"),
    ("problem", "ServiceNowProblem"),
    ("problem_task", "ServiceNowProblemTask"),
    ("change_request", "ServiceNowChangeRequest"),
    ("change_task", "ServiceNowChangeTask"),
    ("sc_request", "ServiceNowRequest"),
    ("sc_req_item", "ServiceNowRequestItem"),
    ("sc_task", "ServiceNowCatalogTask"),
];

pub const AUDIT_FILE: &str = "ServiceNowAudit";

/// Any other ticket table built on ServiceNow's `task` (HR cases, customer
/// service cases, facilities requests, a company's own tables) is read as a
/// ticket, and a task table that belongs to it as its tasks.
pub const TICKET_FILE: &str = "ServiceNowTicket";
pub const TICKET_TASK_FILE: &str = "ServiceNowTicketTask";

/// Fields that point at people; they are kept as IDs and never labelled.
const PEOPLE: [&str; 9] = [
    "assigned_to",
    "opened_by",
    "caller_id",
    "requested_for",
    "resolved_by",
    "closed_by",
    "sys_created_by",
    "sys_updated_by",
    "requested_by",
];

/// Common references whose labels come from the referenced record's name.
const REFERENCES: [(&str, &str); 7] = [
    ("assignment_group", "sys_user_group"),
    ("cmdb_ci", "cmdb_ci"),
    ("business_service", "cmdb_ci_service"),
    ("service_offering", "service_offering"),
    ("company", "core_company"),
    ("location", "cmn_location"),
    ("cat_item", "sc_cat_item"),
];

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub version: u32,
    pub system: String,
    /// The instance, such as `https://acme.service-now.com`.
    pub instance: String,
    pub start: String,
    pub end: String,
    pub tables: Vec<TableSpec>,
    /// Audited fields to keep, by name; empty keeps all.
    #[serde(default)]
    pub history_fields: Vec<String>,
    pub history_gaps: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TableSpec {
    pub table: String,
    /// Fields to write as stored values, by name.
    pub fields: Vec<String>,
    /// Fields that also get a `<field>_label` column with display values,
    /// and labels in the audit history.
    #[serde(default)]
    pub label_fields: Vec<String>,
    /// For a child table: the reference field that points at an earlier
    /// table's record, such as `change_request` on `change_task`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Parent>,
    /// `ticket` or `ticket_task`: read a table built on `task` that is not
    /// one of [`TABLES`], such as `sn_hr_core_case`, as tickets or as the
    /// tasks of those tickets.
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Parent {
    pub field: String,
    pub table: String,
}

pub struct Plan {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub tables: Vec<TablePlan>,
}

pub struct TablePlan {
    pub table: String,
    pub file: String,
    /// Columns written, in order, labels last.
    pub columns: Vec<String>,
    /// Fields asked of the API.
    pub fields: Vec<String>,
    pub labels: Vec<String>,
    /// For a child: its reference field and the parent's position.
    pub parent: Option<(String, usize)>,
    /// The filter for a window-scoped table, before paging.
    pub query: String,
    /// Read as a ticket or ticket task: checked to be built on `task` first.
    pub ticket: bool,
}

fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// A ServiceNow date-time literal: `2026-01-05 09:00:00`, in UTC.
fn literal(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%d %H:%M:%S").to_string()
}

pub(crate) fn instance_ok(instance: &str) -> bool {
    instance
        .strip_prefix("https://")
        .map(|h| h.trim_end_matches('/'))
        .is_some_and(|h| {
            !h.is_empty()
                && h.len() <= 200
                && h.contains('.')
                && h.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
        })
}

/// The file a table is written to.
pub fn file_of(table: &str) -> Option<&'static str> {
    TABLES.iter().find(|(t, _)| *t == table).map(|(_, f)| *f)
}

pub fn plan(spec: &Spec) -> Result<Plan, String> {
    if spec.version != 1 || spec.system != "servicenow" {
        return Err("this runner supports ServiceNow extract spec version 1".into());
    }
    if !instance_ok(&spec.instance) {
        return Err(
            "instance must be a plain https address such as https://acme.service-now.com".into(),
        );
    }
    let instant = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|_| "start and end must be RFC3339 timestamps".to_owned())
    };
    let (start, end) = (instant(&spec.start)?, instant(&spec.end)?);
    if start >= end {
        return Err("start must be before end".into());
    }
    if spec.tables.is_empty() || spec.tables.len() > TABLES.len() + 2 {
        return Err("name one or more tables".into());
    }
    if spec.history_fields.len() > 60 || !spec.history_fields.iter().all(|f| name(f)) {
        return Err("history_fields must be up to sixty field names".into());
    }
    let mut tables: Vec<TablePlan> = Vec::new();
    for table in &spec.tables {
        let file = match table.role.as_deref() {
            Some("ticket") => TICKET_FILE,
            Some("ticket_task") => TICKET_TASK_FILE,
            Some(_) => return Err("as must be ticket or ticket_task".into()),
            None => file_of(&table.table).ok_or(format!(
                "table '{}' is not one this extract reads: {}; read any other ticket table built on task with \"as\": \"ticket\" (and its task table with \"as\": \"ticket_task\")",
                table.table,
                TABLES.map(|(t, _)| t).join(", ")
            ))?,
        };
        if !name(&table.table) {
            return Err("table must be a table name such as sn_hr_core_case".into());
        }
        if tables.iter().any(|t| t.file == file) {
            return Err(format!("only one table can be read as {file}"));
        }
        if table.role.as_deref() == Some("ticket_task") && table.parent.is_none() {
            return Err("a ticket_task table needs its parent: the field naming its ticket".into());
        }
        if tables.iter().any(|t| t.table == table.table) {
            return Err(format!("table '{}' is listed twice", table.table));
        }
        if table.fields.len() > 60
            || !table
                .fields
                .iter()
                .chain(&table.label_fields)
                .all(|f| name(f))
        {
            return Err("fields must be up to sixty field names such as priority".into());
        }
        if let Some(person) = table
            .label_fields
            .iter()
            .find(|f| PEOPLE.contains(&f.as_str()))
        {
            return Err(format!(
                "{person} points at people, who are kept as IDs; remove it from label_fields"
            ));
        }
        let mut fields = vec![
            "sys_id".to_owned(),
            "number".into(),
            "sys_created_on".into(),
        ];
        let mut columns = vec!["sys_id".to_owned(), "number".into(), "created".into()];
        let parent = match &table.parent {
            None => None,
            Some(parent) => {
                let position = tables
                    .iter()
                    .position(|t| t.table == parent.table)
                    .ok_or("a child table's parent must be listed before it")?;
                if !name(&parent.field) {
                    return Err("parent field must be a field name".into());
                }
                Some((parent.field.clone(), position))
            }
        };
        let wanted = parent
            .iter()
            .map(|(field, _)| field)
            .chain(&table.fields)
            .chain(&table.label_fields);
        // A ticket task names its ticket in a column called `ticket`, whatever
        // the table's own field is called (`parent`, `u_case` …).
        let ticket_field = parent
            .as_ref()
            .filter(|_| table.role.as_deref() == Some("ticket_task"))
            .map(|(field, _)| field.clone());
        for field in wanted {
            if !fields.contains(field) && field != "created" {
                fields.push(field.clone());
                columns.push(if Some(field) == ticket_field.as_ref() {
                    "ticket".to_owned()
                } else {
                    field.clone()
                });
            }
        }
        let labels = table.label_fields.clone();
        columns.extend(labels.iter().map(|f| format!("{f}_label")));
        // The Table API compares stored UTC times; the window is widened by a
        // day each side and applied exactly to the returned times.
        let query = format!(
            "sys_updated_on>={}^sys_created_on<{}",
            literal(start - Duration::days(1)),
            literal(end + Duration::days(1))
        );
        tables.push(TablePlan {
            table: table.table.clone(),
            file: file.into(),
            columns,
            fields,
            labels,
            parent,
            query,
            ticket: table.role.is_some(),
        });
    }
    if tables[0].parent.is_some() {
        return Err("the first table must be one the window applies to".into());
    }
    Ok(Plan { start, end, tables })
}

pub fn preview(spec: &Spec, plan: &Plan, sha256: &str) -> Value {
    let tables = plan
        .tables
        .iter()
        .map(|t| {
            json!({"table":t.table,"file":t.file,"fields":t.fields,"labels":t.labels,
                "scope":match &t.parent {
                    None => format!("records with activity in the window ({})", t.query),
                    Some((field, parent)) => format!("records whose {field} is one of the {} records read", plan.tables[*parent].table),
                }})
        })
        .collect::<Vec<_>>();
    json!({"status":"needs_approval","system":"servicenow","instance":spec.instance,"spec_sha256":sha256,
        "start":spec.start,"end":spec.end,
        "scope":"Records with recorded activity in the window, the child records that belong to them, and their full audit history up to the window end. History before the window is included.",
        "tables":tables,
        "history_fields":if spec.history_fields.is_empty() { json!("all") } else { json!(spec.history_fields) },
        "endpoints":["GET /api/now/table/{table} (records)","GET /api/now/table/sys_audit (field history)","GET /api/now/table/sys_choice (state and choice labels)","GET /api/now/table/{group, item or other referenced table} (names for labels)"],
        "credentials":"SERVICENOW_USER and SERVICENOW_PASSWORD from the app's environment; never stored",
        "history_coverage":"unverified; only audited fields leave history, and administrators can exclude fields or tables from auditing",
        "history_gaps":spec.history_gaps})
}

/// `user:password` from `SERVICENOW_USER` and `SERVICENOW_PASSWORD`.
pub fn user_from_environment() -> Result<String, String> {
    let user = std::env::var("SERVICENOW_USER").unwrap_or_default();
    let password = std::env::var("SERVICENOW_PASSWORD").unwrap_or_default();
    if user.trim().is_empty() || password.is_empty() {
        return Err("set SERVICENOW_USER and SERVICENOW_PASSWORD (a read-only user that can read the tables and sys_audit) in the app's environment, then approve again".into());
    }
    Ok(format!("{}:{}", user.trim(), password))
}

/// ServiceNow writes `2026-01-05 09:00:00` in UTC; this writes RFC 3339.
pub fn time(value: &str) -> Option<String> {
    let parsed = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok()?;
    Some(parsed.and_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// A stored value as text, with date-times in RFC 3339.
fn stored(value: &Value) -> String {
    let raw = match value {
        Value::Object(map) => map.get("value").cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };
    match raw {
        Value::Null => String::new(),
        Value::String(s) => time(&s).unwrap_or(s),
        other => other.to_string(),
    }
}

fn display(value: &Value) -> String {
    match &value["display_value"] {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn csv(values: &[String]) -> String {
    values
        .iter()
        .map(|v| {
            if v.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", v.replace('"', "\"\""))
            } else {
                v.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Reads every record matching `query`, paging by `sys_id`.
fn read_all(
    client: &mut Client,
    table: &str,
    query: &str,
    fields: &[String],
    display_values: bool,
    read: &mut usize,
) -> Result<Vec<Value>, String> {
    let mut records = Vec::new();
    let mut last: Option<String> = None;
    loop {
        let mut q = query.to_owned();
        if let Some(last) = &last {
            q.push_str(&format!("^sys_id>{last}"));
        }
        q.push_str("^ORDERBYsys_id");
        let page = client.get(&format!(
            "/api/now/table/{table}?sysparm_query={}&sysparm_fields={}&sysparm_limit={PAGE}&sysparm_exclude_reference_link=true{}",
            encode(&q),
            encode(&fields.join(",")),
            if display_values { "&sysparm_display_value=all" } else { "" }
        ))?;
        let rows = page["result"].as_array().cloned().unwrap_or_default();
        *read += rows.len();
        if *read > RECORD_LIMIT {
            return Err(format!(
                "more than {RECORD_LIMIT} records match; narrow the window or the tables"
            ));
        }
        let next = rows.last().map(|r| stored(&r["sys_id"]));
        let done = rows.len() < PAGE;
        records.extend(rows);
        match next {
            Some(next) if !done && !next.is_empty() => last = Some(next),
            _ => break,
        }
    }
    Ok(records)
}

/// Reads records in batches of IDs: `<field>IN<ids>` plus `rest`.
fn read_in(
    client: &mut Client,
    table: &str,
    (field, ids): (&str, &[String]),
    rest: &str,
    fields: &[String],
    display_values: bool,
    read: &mut usize,
) -> Result<Vec<Value>, String> {
    let mut records = Vec::new();
    for batch in ids.chunks(BATCH) {
        let query = format!("{field}IN{}{rest}", batch.join(","));
        records.extend(read_all(
            client,
            table,
            &query,
            fields,
            display_values,
            read,
        )?);
    }
    Ok(records)
}

/// One audit row: (record, audit_id, created, user, field, old, new).
type AuditRow = (String, String, String, String, String, String, String);

/// Downloads into `stage`: one file per table and the audit file. Returns a
/// report per file.
pub fn download(
    spec: &Spec,
    plan: &Plan,
    curl: &Path,
    user: &str,
    stage: &Path,
) -> Result<Vec<Value>, String> {
    let mut client = Client::new(curl, &spec.instance, user).for_system("ServiceNow");
    let mut read = 0usize;
    let keep = |field: &str| {
        !field.is_empty()
            && field != "DELETED"
            && (spec.history_fields.is_empty() || spec.history_fields.iter().any(|f| f == field))
    };
    let audit_fields = [
        "sys_id",
        "documentkey",
        "fieldname",
        "oldvalue",
        "newvalue",
        "sys_created_on",
        "user",
    ]
    .map(String::from);
    let audit_end = format!("^sys_created_on<{}", literal(plan.end));
    let mut kept: Vec<Vec<Value>> = Vec::new();
    let mut audits: Vec<Vec<AuditRow>> = Vec::new();
    let (mut before, mut initial) = (0u64, 0u64);
    // A table read as a ticket must be a ticket: built on task.
    for table in plan.tables.iter().filter(|t| t.ticket) {
        if !built_on_task(&mut client, &table.table)? {
            return Err(format!(
                "{} is not built on ServiceNow's task table, so it is not read as tickets",
                table.table
            ));
        }
    }
    for table in &plan.tables {
        let candidates = match &table.parent {
            None => read_all(
                &mut client,
                &table.table,
                &table.query,
                &table.fields,
                true,
                &mut read,
            )?,
            Some((field, parent)) => {
                let ids = kept[*parent]
                    .iter()
                    .map(|r| stored(&r["sys_id"]))
                    .collect::<Vec<_>>();
                let rest = format!("^sys_created_on<{}", literal(plan.end));
                read_in(
                    &mut client,
                    &table.table,
                    (field, &ids),
                    &rest,
                    &table.fields,
                    true,
                    &mut read,
                )?
            }
        };
        let created = |r: &Value| {
            DateTime::parse_from_rfc3339(&stored(&r["sys_created_on"]))
                .map(|d| d.with_timezone(&Utc))
                .ok()
        };
        let candidates = candidates
            .into_iter()
            .filter(|r| created(r).is_some_and(|c| c < plan.end))
            .collect::<Vec<_>>();
        let ids = candidates
            .iter()
            .map(|r| stored(&r["sys_id"]))
            .collect::<Vec<_>>();
        let rest = format!("^tablename={}{audit_end}", table.table);
        let entries = read_in(
            &mut client,
            "sys_audit",
            ("documentkey", &ids),
            &rest,
            &audit_fields,
            false,
            &mut read,
        )?;
        let mut by_record: HashMap<String, Vec<AuditRow>> = HashMap::new();
        for entry in &entries {
            let Ok(at) = DateTime::parse_from_rfc3339(&stored(&entry["sys_created_on"])) else {
                continue;
            };
            if at >= plan.end {
                continue;
            }
            let record = stored(&entry["documentkey"]);
            by_record.entry(record.clone()).or_default().push((
                record,
                stored(&entry["sys_id"]),
                at.with_timezone(&Utc)
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string(),
                stored(&entry["user"]),
                stored(&entry["fieldname"]),
                stored(&entry["oldvalue"]),
                stored(&entry["newvalue"]),
            ));
        }
        let mut records = Vec::new();
        let mut rows = Vec::new();
        for record in candidates {
            let id = stored(&record["sys_id"]);
            let opened = created(&record).expect("filtered above");
            let entries = by_record.remove(&id).unwrap_or_default();
            // A window-scoped record is kept when opened or changed in the
            // window; a child belongs to a kept parent.
            let active = table.parent.is_some()
                || opened >= plan.start
                || entries
                    .iter()
                    .any(|e| DateTime::parse_from_rfc3339(&e.2).is_ok_and(|t| t >= plan.start));
            if !active {
                continue;
            }
            if opened < plan.start {
                before += 1;
            }
            let opened = opened.format("%Y-%m-%dT%H:%M:%SZ").to_string();
            for entry in entries {
                if !keep(&entry.4) {
                    continue;
                }
                // Values set when the record was created are not changes.
                if entry.5.is_empty() && entry.2 == opened {
                    initial += 1;
                    continue;
                }
                rows.push(entry);
            }
            records.push(record);
        }
        rows.sort_by(|a, b| (&a.2, &a.0, &a.1).cmp(&(&b.2, &b.0, &b.1)));
        kept.push(records);
        audits.push(rows);
    }

    // Labels: the choice lists, the records' own display values, then the
    // names of referenced records not yet seen.
    let mut labels: Vec<HashMap<String, HashMap<String, String>>> = Vec::new();
    for (position, table) in plan.tables.iter().enumerate() {
        let mut map: HashMap<String, HashMap<String, String>> = HashMap::new();
        if !table.labels.is_empty() {
            let query = format!(
                "nameIN{},task^elementIN{}^inactive=false",
                table.table,
                table.labels.join(",")
            );
            let choices = client.get(&format!(
                "/api/now/table/sys_choice?sysparm_query={}&sysparm_fields=name,element,value,label&sysparm_limit=10000",
                encode(&query)
            ))?;
            let mut rows = choices["result"].as_array().cloned().unwrap_or_default();
            // The table's own choices win over those it inherits from task.
            rows.sort_by_key(|r| r["name"] == table.table.as_str());
            for row in rows {
                map.entry(stored(&row["element"]))
                    .or_default()
                    .insert(stored(&row["value"]), stored(&row["label"]));
            }
        }
        for record in &kept[position] {
            for field in &table.labels {
                let value = stored(&record[field]);
                if !value.is_empty() {
                    map.entry(field.clone())
                        .or_default()
                        .entry(value)
                        .or_insert_with(|| display(&record[field]));
                }
            }
        }
        for field in &table.labels {
            let Some((_, target)) = REFERENCES.iter().find(|(f, _)| f == field) else {
                continue;
            };
            let known = map.entry(field.clone()).or_default();
            let missing = audits[position]
                .iter()
                .filter(|e| &e.4 == field)
                .flat_map(|e| [e.5.clone(), e.6.clone()])
                .filter(|v| !v.is_empty() && !known.contains_key(v))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let found = read_in(
                &mut client,
                target,
                ("sys_id", &missing),
                "",
                &["sys_id".to_owned(), "name".to_owned()],
                false,
                &mut read,
            )?;
            for row in found {
                known.insert(stored(&row["sys_id"]), stored(&row["name"]));
            }
        }
        labels.push(map);
    }

    let mut reports = Vec::new();
    let write = |name: &str, rows: Vec<String>| -> Result<(), String> {
        let mut file = fs::File::create(stage.join(format!("{name}.csv")))
            .map_err(|_| "cannot stage export")?;
        for row in rows {
            writeln!(file, "{row}").map_err(|_| "cannot stage export")?;
        }
        Ok(())
    };
    let mut audit_rows = vec![csv(&[
        "table",
        "record",
        "audit_id",
        "created",
        "user",
        "field",
        "old_value",
        "old_label",
        "new_value",
        "new_label",
    ]
    .map(String::from))];
    for (position, table) in plan.tables.iter().enumerate() {
        let map = &labels[position];
        let mut rows = vec![csv(&table.columns)];
        let (mut first, mut last): (Option<String>, Option<String>) = (None, None);
        for record in &kept[position] {
            let mut row = table
                .fields
                .iter()
                .map(|f| stored(&record[f]))
                .collect::<Vec<_>>();
            for field in &table.labels {
                let value = stored(&record[field]);
                row.push(
                    map.get(field)
                        .and_then(|m| m.get(&value))
                        .cloned()
                        .unwrap_or_else(|| display(&record[field])),
                );
            }
            let opened = row[2].clone();
            if first.as_ref().is_none_or(|f| opened < *f) {
                first = Some(opened.clone());
            }
            if last.as_ref().is_none_or(|l| opened > *l) {
                last = Some(opened);
            }
            rows.push(csv(&row));
        }
        let label = |field: &str, value: &str| {
            if value.is_empty() {
                return String::new();
            }
            map.get(field)
                .and_then(|m| m.get(value))
                .cloned()
                .unwrap_or_else(|| readable(value))
        };
        for (record, id, at, user, field, old, new) in &audits[position] {
            audit_rows.push(csv(&[
                table.table.clone(),
                record.clone(),
                id.clone(),
                at.clone(),
                user.clone(),
                field.clone(),
                old.clone(),
                label(field, old),
                new.clone(),
                label(field, new),
            ]));
        }
        let count = rows.len() - 1;
        write(&table.file, rows)?;
        reports.push(json!({"object":table.file,"table":table.table,"kind":"records",
            "scope":if table.parent.is_none() { "items_active_in_window" } else { "children_of_kept_records" },
            "rows":count,"observed_start":first,"observed_end":last,"fields":table.columns,
            "history_gaps":spec.history_gaps,"history_coverage":"unverified","empty":count == 0}));
    }
    let history = audit_rows.len() - 1;
    write(AUDIT_FILE, audit_rows)?;
    if let Some(first) = reports.first_mut() {
        first["rows_before_window"] = json!(before);
        first["records_read"] = json!(read);
        first["api_calls"] = json!(client.calls());
    }
    reports.push(json!({"object":AUDIT_FILE,"kind":"history","rows":history,
        "history_lower_bound":"none: full audit history up to the window end",
        "initial_values_left_out":initial,
        "history_fields":if spec.history_fields.is_empty() { json!("all") } else { json!(spec.history_fields) },
        "history_coverage":"unverified","empty":history == 0}));
    Ok(reports)
}

/// What discovery reads for one table, as the extract needs it: the
/// company's own fields, the state labels and whether audit history can be
/// read.
/// Whether a table extends `task`, following its parents in `sys_db_object`.
pub(crate) fn built_on_task(client: &mut Client, table: &str) -> Result<bool, String> {
    let mut current = table.to_owned();
    for _ in 0..12 {
        if current == "task" {
            return Ok(true);
        }
        let found = client.get(&format!(
            "/api/now/table/sys_db_object?sysparm_query={}&sysparm_fields=name,super_class.name&sysparm_limit=1",
            encode(&format!("name={current}"))
        ))?;
        let parent = found["result"][0]["super_class.name"]
            .as_str()
            .unwrap_or("");
        if parent.is_empty() || !name(parent) {
            return Ok(false);
        }
        current = parent.to_owned();
    }
    Ok(false)
}

pub(crate) fn discover_table(client: &mut Client, table: &str) -> Result<Value, String> {
    let query = format!("nameIN{table},task^elementSTARTSWITHu_");
    let own = client.get(&format!(
        "/api/now/table/sys_dictionary?sysparm_query={}&sysparm_fields=name,element,column_label,internal_type&sysparm_limit=200&sysparm_exclude_reference_link=true",
        encode(&query)
    ))?;
    let custom = own["result"]
        .as_array()
        .into_iter()
        .flatten()
        .take(80)
        .map(|f| json!({"name":stored(&f["element"]),"label":stored(&f["column_label"]),"type":stored(&f["internal_type"]),"on":stored(&f["name"])}))
        .collect::<Vec<_>>();
    let query = format!("nameIN{table},task^element=state^inactive=false");
    let states = client.get(&format!(
        "/api/now/table/sys_choice?sysparm_query={}&sysparm_fields=name,value,label,sequence&sysparm_limit=200",
        encode(&query)
    ))?;
    let mut by_value = BTreeMap::new();
    let mut rows = states["result"].as_array().cloned().unwrap_or_default();
    rows.sort_by_key(|r| r["name"] == table);
    for row in rows {
        by_value.insert(stored(&row["value"]), stored(&row["label"]));
    }
    let query = format!("tablename={table}");
    let audit = client.get(&format!(
        "/api/now/table/sys_audit?sysparm_query={}&sysparm_fields=sys_id&sysparm_limit=1",
        encode(&query)
    ));
    let gap = match &audit {
        Err(error) => Some(format!(
            "This user cannot read the audit history (sys_audit) for {table}, so no steps after creation can be seen: {error}. A role that can read sys_audit is needed."
        )),
        Ok(found) if found["result"].as_array().is_none_or(|r| r.is_empty()) => Some(format!(
            "No audit history was found for {table}. Check that auditing is on for it; history is only recorded from when auditing is on."
        )),
        Ok(_) => None,
    };
    Ok(json!({"table":table,"file":file_of(table),
        "states":by_value.into_iter().map(|(value, label)| json!({"value":value,"label":label})).collect::<Vec<_>>(),
        "custom_fields":custom,"audit_readable":gap.is_none(),"gap":gap}))
}

/// A stored value no choice list or record names, such as a catalog item's
/// past workflow stage `request_approved`: written as words, "Request
/// approved". Anything else (numbers, IDs, mixed case) stays as stored.
fn readable(value: &str) -> String {
    let words = value.contains('_')
        && value.starts_with(|c: char| c.is_ascii_lowercase())
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_');
    if !words {
        return value.to_owned();
    }
    let text = value
        .split('_')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn unlabelled_stages_read_as_words() {
        assert_eq!(super::readable("request_approved"), "Request approved");
        assert_eq!(
            super::readable("waiting_for_approval"),
            "Waiting for approval"
        );
        // Numbers and IDs stay as stored.
        assert_eq!(super::readable("2"), "2");
        assert_eq!(
            super::readable("6816f79cc0a8016401c5a33be04be441"),
            "6816f79cc0a8016401c5a33be04be441"
        );
        assert_eq!(super::readable("Fulfillment"), "Fulfillment");
    }

    use super::*;

    #[test]
    fn writes_servicenow_times_as_rfc3339() {
        assert_eq!(
            time("2026-01-05 09:00:00").as_deref(),
            Some("2026-01-05T09:00:00Z")
        );
        assert_eq!(time("2026-01-05"), None);
        assert_eq!(
            stored(&json!({"value":"2026-01-05 09:00:00","display_value":"05/01/2026 10:00"})),
            "2026-01-05T09:00:00Z"
        );
        assert_eq!(
            display(&json!({"value":"2","display_value":"In Progress"})),
            "In Progress"
        );
    }

    #[test]
    fn plans_bounded_queries_and_rejects_unsafe_specs() {
        let spec: Spec = serde_json::from_value(json!({"version":1,"system":"servicenow",
            "instance":"https://acme.service-now.com","start":"2026-01-01T00:00:00Z","end":"2026-04-01T00:00:00Z",
            "tables":[{"table":"change_request","fields":["state","approval"],"label_fields":["state"]},
                      {"table":"change_task","fields":["state"],"parent":{"field":"change_request","table":"change_request"}}],
            "history_gaps":[]}))
        .unwrap();
        let plan = plan(&spec).unwrap();
        assert_eq!(
            plan.tables[0].query,
            "sys_updated_on>=2025-12-31 00:00:00^sys_created_on<2026-04-02 00:00:00"
        );
        assert_eq!(
            plan.tables[0].columns,
            [
                "sys_id",
                "number",
                "created",
                "state",
                "approval",
                "state_label"
            ]
        );
        assert_eq!(
            plan.tables[1].columns,
            ["sys_id", "number", "created", "change_request", "state"]
        );
        assert_eq!(plan.tables[1].parent, Some(("change_request".into(), 0)));
        for bad in [
            json!({"instance":"http://acme.service-now.com"}),
            json!({"instance":"https://acme.service-now.com/../x"}),
            json!({"tables":[{"table":"sys_user","fields":["name"]}]}),
            json!({"tables":[{"table":"incident","fields":["state^ORsys_id!=x"]}]}),
            json!({"tables":[{"table":"incident","fields":["state"],"label_fields":["assigned_to"]}]}),
            json!({"tables":[{"table":"change_task","fields":["state"],"parent":{"field":"change_request","table":"change_request"}}]}),
        ] {
            let mut value = serde_json::to_value(&spec).unwrap();
            for (k, v) in bad.as_object().unwrap() {
                value[k] = v.clone();
            }
            let spec: Spec = serde_json::from_value(value).unwrap();
            assert!(super::plan(&spec).is_err(), "{bad}");
        }
    }
}
