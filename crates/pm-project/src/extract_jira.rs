//! Jira Cloud extracts: issues of chosen projects with activity in a window,
//! with their full change history up to the window end, read through Jira's
//! REST API (version 3) with the person's own API token.
//!
//! The transport is the system `curl`, given its URL and credentials on
//! standard input so the token never appears in a process list, a file or a
//! log. Credentials come from `JIRA_EMAIL` and `JIRA_API_TOKEN` in the app's
//! environment and are never written anywhere.
//!
//! Like every extract, a spec is only filed by an agent; a person approves it
//! in the app, and the app runs it.
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Issues per search page; Jira caps it at 100.
const PAGE: usize = 100;
/// Issues an extract may read; a larger scope needs a narrower spec.
const ISSUE_LIMIT: usize = 500_000;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub version: u32,
    pub system: String,
    /// The Jira Cloud site, such as `https://acme.atlassian.net`.
    pub site: String,
    pub start: String,
    pub end: String,
    /// Project keys.
    pub projects: Vec<String>,
    /// Issue type names to keep, such as `Bug`; empty keeps every type.
    #[serde(default)]
    pub issue_types: Vec<String>,
    /// Issue fields to write, by field ID (`priority`, `customfield_10016`).
    pub fields: Vec<String>,
    /// Changed fields to keep from the history, by field ID; empty keeps all.
    #[serde(default)]
    pub history_fields: Vec<String>,
    pub history_gaps: Vec<String>,
}

pub struct Plan {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub jql: String,
    pub fields: Vec<String>,
}

fn field_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn quoted(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 100 || value.chars().any(char::is_control) {
        return Err("issue type names must be 1 to 100 characters".into());
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

/// Base columns every issue row starts with.
const BASE: [&str; 4] = ["key", "id", "project", "created"];

pub fn plan(spec: &Spec) -> Result<Plan, String> {
    if spec.version != 1 || spec.system != "jira" {
        return Err("this runner supports Jira extract spec version 1".into());
    }
    let host = spec
        .site
        .strip_prefix("https://")
        .ok_or("site must be the https address of the Jira site")?
        .trim_end_matches('/');
    if host.is_empty()
        || host.len() > 200
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
        || !host.contains('.')
    {
        return Err("site must be a plain https address such as https://acme.atlassian.net".into());
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
    if spec.projects.is_empty()
        || spec.projects.len() > 50
        || !spec.projects.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 20
                && p.bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        })
    {
        return Err("projects must be one to fifty Jira project keys such as OPS".into());
    }
    if spec.fields.len() > 60 || !spec.fields.iter().all(|f| field_id(f)) {
        return Err("fields must be up to sixty Jira field IDs".into());
    }
    if spec.history_fields.len() > 60 || !spec.history_fields.iter().all(|f| field_id(f)) {
        return Err("history_fields must be up to sixty Jira field IDs".into());
    }
    let mut fields = BASE.iter().map(|f| (*f).to_owned()).collect::<Vec<_>>();
    for field in &spec.fields {
        if !fields.contains(field) {
            fields.push(field.clone());
        }
    }
    // JQL reads dates in the user's own time zone, so the window is widened by
    // a day each side here and applied exactly to the returned times.
    let mut jql = format!(
        "project in ({})",
        spec.projects
            .iter()
            .map(|p| format!("\"{p}\""))
            .collect::<Vec<_>>()
            .join(",")
    );
    if !spec.issue_types.is_empty() {
        let types = spec
            .issue_types
            .iter()
            .map(|t| quoted(t))
            .collect::<Result<Vec<_>, _>>()?;
        jql.push_str(&format!(" AND issuetype in ({})", types.join(",")));
    }
    jql.push_str(&format!(
        " AND updated >= \"{}\" AND created < \"{}\" ORDER BY created ASC, key ASC",
        (start - Duration::days(1)).format("%Y-%m-%d"),
        (end + Duration::days(1)).format("%Y-%m-%d")
    ));
    Ok(Plan {
        start,
        end,
        jql,
        fields,
    })
}

pub fn preview(spec: &Spec, plan: &Plan, sha256: &str) -> Value {
    json!({"status":"needs_approval","system":"jira","site":spec.site,"spec_sha256":sha256,
        "start":spec.start,"end":spec.end,
        "scope":"Issues of these projects with recorded activity in the window, with their full change history up to the window end. History before the window is included.",
        "jql":plan.jql,"issue_fields":plan.fields,
        "history_fields":if spec.history_fields.is_empty() { json!("all") } else { json!(spec.history_fields) },
        "endpoints":["GET /rest/api/3/search/jql (issues with change history)","GET /rest/api/3/issue/{key}/changelog (longer histories)"],
        "credentials":"JIRA_EMAIL and JIRA_API_TOKEN from the app's environment; never stored",
        "history_coverage":"unverified; Jira keeps issue history for the life of the issue unless an admin removed it",
        "history_gaps":spec.history_gaps})
}

/// `%`-encodes a query value.
pub(crate) fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A curl config value, quoted.
fn config(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A read-only REST client over curl, shared by the Jira, ServiceNow and
/// HubSpot extracts. Credentials go to curl on standard input only.
pub(crate) struct Client<'a> {
    curl: &'a Path,
    site: String,
    /// `user:password` for basic sign-in, or a bearer token.
    user: String,
    bearer: bool,
    calls: u64,
    system: &'static str,
}

impl Client<'_> {
    pub(crate) fn new<'a>(curl: &'a Path, site: &str, user: &str) -> Client<'a> {
        Client {
            curl,
            site: site.trim_end_matches('/').to_owned(),
            user: user.to_owned(),
            bearer: false,
            calls: 0,
            system: "Jira",
        }
    }

    /// A client that signs in with a bearer token, such as a HubSpot private
    /// app token.
    pub(crate) fn bearer<'a>(curl: &'a Path, site: &str, token: &str) -> Client<'a> {
        Client {
            bearer: true,
            ..Client::new(curl, site, token)
        }
    }

    /// The same client, naming another system in its errors.
    pub(crate) fn for_system(mut self, system: &'static str) -> Self {
        self.system = system;
        self
    }

    pub(crate) fn calls(&self) -> u64 {
        self.calls
    }

    pub(crate) fn get(&mut self, path_and_query: &str) -> Result<Value, String> {
        self.send(path_and_query, None)
    }

    /// A POST that only reads, such as a search or a batch read.
    pub(crate) fn post(&mut self, path: &str, body: &Value) -> Result<Value, String> {
        self.send(path, Some(body))
    }

    fn send(&mut self, path_and_query: &str, body: Option<&Value>) -> Result<Value, String> {
        self.calls += 1;
        let mut child = Command::new(self.curl)
            .args([
                "--silent",
                "--show-error",
                "--fail-with-body",
                "--max-time",
                "120",
                "--config",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| {
                format!(
                    "curl could not start; it is needed to read from {}",
                    self.system
                )
            })?;
        {
            let mut stdin = child.stdin.take().ok_or("curl could not start")?;
            let url = format!("{}{}", self.site, path_and_query);
            let sign_in = if self.bearer {
                format!(
                    "header = {}",
                    config(&format!("Authorization: Bearer {}", self.user))
                )
            } else {
                format!("user = {}", config(&self.user))
            };
            write!(
                stdin,
                "url = {}\n{sign_in}\nheader = \"Accept: application/json\"\n",
                config(&url),
            )
            .map_err(|_| "curl could not start")?;
            if let Some(body) = body {
                write!(
                    stdin,
                    "request = \"POST\"\nheader = \"Content-Type: application/json\"\ndata = {}\n",
                    config(&body.to_string())
                )
                .map_err(|_| "curl could not start")?;
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|_| "curl did not finish")?;
        let body: Option<Value> = serde_json::from_slice(&output.stdout).ok();
        if !output.status.success() {
            let message = body
                .as_ref()
                .and_then(|b| {
                    b["errorMessages"]
                        .get(0)
                        .and_then(Value::as_str)
                        .or(b["error"]["message"].as_str())
                        .or(b["message"].as_str())
                })
                .map(|m| m.chars().take(200).collect::<String>())
                .unwrap_or_else(|| "no answer; check the site address and network".into());
            return Err(format!("{} refused the request: {message}", self.system));
        }
        body.ok_or_else(|| format!("{}'s answer was not readable", self.system))
    }
}

/// Jira writes `2026-01-05T10:15:30.000+0100`; this writes RFC 3339.
pub fn time(value: &str) -> Option<String> {
    let parsed = DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z")
        .or_else(|_| DateTime::parse_from_rfc3339(value))
        .ok()?;
    Some(parsed.to_rfc3339_opts(chrono::SecondsFormat::Millis, false))
}

/// A field value as one text: names, option values, account IDs, project
/// keys; lists joined with `;`; dates as RFC 3339.
fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => time(s)
            .filter(|_| s.len() >= 20 && s.as_bytes()[10] == b'T')
            .unwrap_or_else(|| s.clone()),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(items) => items.iter().map(text).collect::<Vec<_>>().join(";"),
        Value::Object(map) => ["key", "name", "value", "accountId", "id"]
            .iter()
            .find_map(|k| map.get(*k).filter(|v| !v.is_null()).map(text))
            .unwrap_or_default(),
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

/// Downloads into `stage`: JiraIssue.csv and JiraIssueHistory.csv. Returns
/// a report per file.
/// `email:token` from `JIRA_EMAIL` and `JIRA_API_TOKEN`.
pub fn user_from_environment() -> Result<String, String> {
    let email = std::env::var("JIRA_EMAIL").unwrap_or_default();
    let token = std::env::var("JIRA_API_TOKEN").unwrap_or_default();
    if email.trim().is_empty() || token.trim().is_empty() {
        return Err("set JIRA_EMAIL and JIRA_API_TOKEN (a read-only user's API token) in the app's environment, then approve again".into());
    }
    Ok(format!("{}:{}", email.trim(), token.trim()))
}

pub fn download(
    spec: &Spec,
    plan: &Plan,
    curl: &Path,
    user: &str,
    stage: &Path,
) -> Result<Vec<Value>, String> {
    let mut client = Client::new(curl, &spec.site, user);
    let keep = |field_id: &str| {
        spec.history_fields.is_empty() || spec.history_fields.iter().any(|f| f == field_id)
    };
    let mut issues = vec![csv(&plan.fields.clone())];
    let mut history = vec![csv(&[
        "issue_key",
        "history_id",
        "item",
        "created",
        "author",
        "field",
        "field_id",
        "from",
        "from_string",
        "to",
        "to_string",
    ]
    .map(String::from))];
    let (mut seen, mut scanned, mut before) = (BTreeSet::new(), 0u64, 0u64);
    let (mut first, mut last): (Option<String>, Option<String>) = (None, None);
    let mut token_page: Option<String> = None;
    let requested = plan
        .fields
        .iter()
        .filter(|f| !matches!(f.as_str(), "key" | "id"))
        .cloned()
        .collect::<Vec<_>>()
        .join(",");
    loop {
        let mut query = format!(
            "/rest/api/3/search/jql?jql={}&fields={}&expand=changelog&maxResults={PAGE}",
            encode(&plan.jql),
            encode(&requested)
        );
        if let Some(token) = &token_page {
            query.push_str(&format!("&nextPageToken={}", encode(token)));
        }
        let page = client.get(&query)?;
        for issue in page["issues"].as_array().into_iter().flatten() {
            scanned += 1;
            if scanned as usize > ISSUE_LIMIT {
                return Err(format!(
                    "more than {ISSUE_LIMIT} issues match; narrow the projects or the window"
                ));
            }
            let key = issue["key"].as_str().unwrap_or_default().to_owned();
            if key.is_empty() || !seen.insert(key.clone()) {
                continue;
            }
            let fields = &issue["fields"];
            let created = fields["created"].as_str().and_then(time);
            let Some(created) = created else { continue };
            let created_at =
                DateTime::parse_from_rfc3339(&created).map_err(|_| "unreadable created time")?;
            if created_at >= plan.end {
                continue;
            }
            // The issue's full history: the page holds the first part.
            let mut entries = issue["changelog"]["histories"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let total = issue["changelog"]["total"]
                .as_u64()
                .unwrap_or(entries.len() as u64);
            if (entries.len() as u64) < total {
                entries.clear();
                let mut start = 0;
                loop {
                    let part = client.get(&format!(
                        "/rest/api/3/issue/{}/changelog?startAt={start}&maxResults={PAGE}",
                        encode(&key)
                    ))?;
                    let values = part["values"].as_array().cloned().unwrap_or_default();
                    start += values.len();
                    let done = values.is_empty() || part["isLast"].as_bool().unwrap_or(true);
                    entries.extend(values);
                    if done {
                        break;
                    }
                }
            }
            // Only changes up to the window end; active if anything falls in it.
            let mut rows = Vec::new();
            let mut active = created_at >= plan.start;
            for entry in &entries {
                let Some(at) = entry["created"].as_str().and_then(time) else {
                    continue;
                };
                let when =
                    DateTime::parse_from_rfc3339(&at).map_err(|_| "unreadable change time")?;
                if when >= plan.end {
                    continue;
                }
                active |= when >= plan.start;
                for (item, change) in entry["items"].as_array().into_iter().flatten().enumerate() {
                    let id = change["fieldId"]
                        .as_str()
                        .or(change["field"].as_str())
                        .unwrap_or_default();
                    // Jira records some no-op changes, such as a new board's
                    // issues moving from To Do to To Do; they are not steps.
                    if !keep(id)
                        || (change["from"] == change["to"]
                            && change["fromString"] == change["toString"])
                    {
                        continue;
                    }
                    rows.push(csv(&[
                        key.clone(),
                        text(&entry["id"]),
                        item.to_string(),
                        at.clone(),
                        text(&entry["author"]["accountId"]),
                        text(&change["field"]),
                        id.to_owned(),
                        text(&change["from"]),
                        text(&change["fromString"]),
                        text(&change["to"]),
                        text(&change["toString"]),
                    ]));
                }
            }
            if !active {
                continue;
            }
            if created_at < plan.start {
                before += 1;
            }
            if first.as_ref().is_none_or(|f| created < *f) {
                first = Some(created.clone());
            }
            if last.as_ref().is_none_or(|l| created > *l) {
                last = Some(created.clone());
            }
            let row = plan
                .fields
                .iter()
                .map(|field| match field.as_str() {
                    "key" => key.clone(),
                    "id" => text(&issue["id"]),
                    "project" => text(&fields["project"]),
                    "created" => created.clone(),
                    other => text(&fields[other]),
                })
                .collect::<Vec<_>>();
            issues.push(csv(&row));
            history.extend(rows);
        }
        token_page = page["nextPageToken"].as_str().map(str::to_owned);
        if page["isLast"].as_bool().unwrap_or(token_page.is_none()) || token_page.is_none() {
            break;
        }
    }
    let issue_rows = issues.len() - 1;
    let history_rows = history.len() - 1;
    let write = |name: &str, rows: Vec<String>| -> Result<(), String> {
        let mut file = fs::File::create(stage.join(format!("{name}.csv")))
            .map_err(|_| "cannot stage export")?;
        for row in rows {
            writeln!(file, "{row}").map_err(|_| "cannot stage export")?;
        }
        Ok(())
    };
    write("JiraIssue", issues)?;
    write("JiraIssueHistory", history)?;
    Ok(vec![
        json!({"object":"JiraIssue","kind":"records","scope":"items_active_in_window","rows":issue_rows,
            "rows_before_window":before,"observed_start":first,"observed_end":last,"fields":plan.fields,
            "issues_scanned":scanned,"api_calls":client.calls,"history_gaps":spec.history_gaps,
            "history_coverage":"unverified","empty":issue_rows == 0}),
        json!({"object":"JiraIssueHistory","kind":"history","rows":history_rows,
            "history_lower_bound":"none: full recorded history up to the window end",
            "history_fields":if spec.history_fields.is_empty() { json!("all") } else { json!(spec.history_fields) },
            "history_coverage":"unverified","empty":history_rows == 0}),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_jira_times_as_rfc3339() {
        assert_eq!(
            time("2026-01-05T10:15:30.000+0100").as_deref(),
            Some("2026-01-05T10:15:30.000+01:00")
        );
        assert_eq!(time("not a time"), None);
    }

    #[test]
    fn flattens_field_values_to_text() {
        assert_eq!(text(&json!({"name":"High","id":"2"})), "High");
        assert_eq!(
            text(&json!({"accountId":"acc-1","displayName":"Ana"})),
            "acc-1"
        );
        assert_eq!(text(&json!([{"value":"A"},{"value":"B"}])), "A;B");
        assert_eq!(text(&json!(5)), "5");
        assert_eq!(text(&Value::Null), "");
    }

    #[test]
    fn plans_a_bounded_query_and_rejects_unsafe_specs() {
        let spec: Spec = serde_json::from_value(json!({"version":1,"system":"jira","site":"https://acme.atlassian.net",
            "start":"2026-01-01T00:00:00Z","end":"2026-04-01T00:00:00Z","projects":["OPS"],"issue_types":["Bug"],
            "fields":["status","priority"],"history_gaps":[]}))
        .unwrap();
        let plan = plan(&spec).unwrap();
        assert_eq!(
            plan.jql,
            "project in (\"OPS\") AND issuetype in (\"Bug\") AND updated >= \"2025-12-31\" AND created < \"2026-04-02\" ORDER BY created ASC, key ASC"
        );
        assert_eq!(
            plan.fields,
            ["key", "id", "project", "created", "status", "priority"]
        );
        for bad in [
            json!({"site":"http://acme.atlassian.net"}),
            json!({"site":"https://acme.atlassian.net/../x"}),
            json!({"projects":["ops) OR 1=1"]}),
            json!({"fields":["status,summary"]}),
        ] {
            let mut value = serde_json::to_value(&spec).unwrap();
            for (k, v) in bad.as_object().unwrap() {
                value[k] = v.clone();
            }
            let spec: Spec = serde_json::from_value(value).unwrap();
            assert!(super::plan(&spec).is_err());
        }
    }
}
