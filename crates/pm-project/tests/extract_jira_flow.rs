//! A Jira extract from request to registered files, against a stand-in for
//! curl that answers like Jira Cloud's REST API. No site or credentials.
#![cfg(unix)]
use pm_project::extract;
use pm_project::workspace::{InvestigationPurpose, WorkspaceFolder};
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;

/// Reads curl's config from standard input, keeps a copy, and answers by URL.
/// Window: January 2026. OPS-1 was created in it; OPS-2 was created before
/// it and changed in it, with a long history fetched separately; OPS-3 had no
/// activity in it; OPS-4 was created after it; OPS-5 is on the second page.
const CURL: &str = r#"#!/bin/sh
config=$(cat)
printf '%s\n---\n' "$config" >> "$(dirname "$0")/calls.log"
case "$config" in
  *"/issue/OPS-2/changelog"*)
    printf '{"values":[
      {"id":"21","created":"2025-12-20T10:00:00.000+0100","author":{"accountId":"acc-a"},"items":[{"field":"status","fieldId":"status","from":"1","fromString":"To Do","to":"3","toString":"In Progress"}]},
      {"id":"22","created":"2026-01-10T10:00:00.000+0100","author":{"accountId":"acc-a"},"items":[{"field":"status","fieldId":"status","from":"3","fromString":"In Progress","to":"10001","toString":"Done"},{"field":"resolution","fieldId":"resolution","from":null,"fromString":null,"to":"1","toString":"Done"}]},
      {"id":"23","created":"2026-02-10T10:00:00.000+0100","author":{"accountId":"acc-a"},"items":[{"field":"status","fieldId":"status","from":"10001","fromString":"Done","to":"4","toString":"Reopened"}]}],
      "isLast":true}' ;;
  *"nextPageToken=p2"*)
    printf '{"issues":[
      {"id":"5","key":"OPS-5","fields":{"created":"2026-01-20T09:00:00.000+0100","project":{"key":"OPS"},"status":{"name":"To Do"},"priority":{"name":"Medium"}},"changelog":{"total":0,"histories":[]}}],
      "isLast":true}' ;;
  *"/search/jql"*)
    printf '{"issues":[
      {"id":"1","key":"OPS-1","fields":{"created":"2026-01-05T09:00:00.000+0100","project":{"key":"OPS"},"status":{"name":"In Progress"},"priority":{"name":"High"}},
       "changelog":{"total":1,"histories":[{"id":"11","created":"2026-01-06T09:30:00.000+0100","author":{"accountId":"acc-b"},"items":[{"field":"status","fieldId":"status","from":"1","fromString":"To Do","to":"3","toString":"In Progress"},{"field":"description","fieldId":"description","from":null,"fromString":"a","to":null,"toString":"b"},{"field":"status","fieldId":"status","from":"3","fromString":"In Progress","to":"3","toString":"In Progress"}]}]}},
      {"id":"2","key":"OPS-2","fields":{"created":"2025-12-15T09:00:00.000+0100","project":{"key":"OPS"},"status":{"name":"Reopened"},"priority":{"name":"Medium"}},
       "changelog":{"total":3,"histories":[{"id":"21","created":"2025-12-20T10:00:00.000+0100","author":{"accountId":"acc-a"},"items":[]}]}},
      {"id":"3","key":"OPS-3","fields":{"created":"2025-11-01T09:00:00.000+0100","project":{"key":"OPS"},"status":{"name":"Done"},"priority":{"name":"Low"}},"changelog":{"total":0,"histories":[]}},
      {"id":"4","key":"OPS-4","fields":{"created":"2026-02-03T09:00:00.000+0100","project":{"key":"OPS"},"status":{"name":"To Do"},"priority":{"name":"Low"}},"changelog":{"total":0,"histories":[]}}],
      "nextPageToken":"p2","isLast":false}' ;;
  *) printf '{"errorMessages":["unexpected request"]}'; exit 22 ;;
esac
"#;

#[test]
fn a_jira_extract_reads_issues_active_in_the_window_with_their_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut folder = WorkspaceFolder::create_named(
        &dir.path().join("projects"),
        "Bug resolution",
        InvestigationPurpose::Improvement,
    )
    .unwrap();
    let tools_dir = dir.path().join("tools");
    fs::create_dir_all(&tools_dir).unwrap();
    let curl = tools_dir.join("curl");
    fs::write(&curl, CURL).unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).unwrap();
    let spec = json!({"version":1,"system":"jira","site":"https://acme.atlassian.net",
        "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z","projects":["OPS"],
        "fields":["status","priority"],"history_fields":["status","resolution"],"history_gaps":[]})
    .to_string()
    .into_bytes();
    let reviewed = extract::review(&spec).unwrap();
    assert_eq!(reviewed.preview()["system"], "jira");
    let root = folder.root.clone();
    extract::request(&root, &reviewed).unwrap();
    let tools = extract::Tools {
        salesforce_cli: "sf".into(),
        curl: curl.clone(),
        jira_user: Some("analyst@acme.example:SECRET_TOKEN".into()),
        servicenow_user: None,
        hubspot_token: None,
    };
    // Nothing runs before a person approves.
    assert!(extract::run(&root, &reviewed.sha256, &tools).is_err());
    extract::approve(&root, &reviewed.sha256, "Pat Lee").unwrap();
    let staged = extract::run(&root, &reviewed.sha256, &tools).unwrap();
    let result = extract::register(&mut folder, staged).unwrap();
    assert_eq!(result["status"], "complete");

    let source = |name: &str| {
        let asset = result["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap();
        fs::read_to_string(root.join(asset["relative_path"].as_str().unwrap())).unwrap()
    };
    let issues = source("JiraIssue.csv");
    assert_eq!(
        issues,
        "key,id,project,created,status,priority\n\
         OPS-1,1,OPS,2026-01-05T09:00:00.000+01:00,In Progress,High\n\
         OPS-2,2,OPS,2025-12-15T09:00:00.000+01:00,Reopened,Medium\n\
         OPS-5,5,OPS,2026-01-20T09:00:00.000+01:00,To Do,Medium\n"
    );
    let history = source("JiraIssueHistory.csv");
    // The description edit and the status change that changed nothing are
    // left out; OPS-2's change after the window too;
    // its December change is kept as history before the window.
    assert_eq!(
        history,
        "issue_key,history_id,item,created,author,field,field_id,from,from_string,to,to_string\n\
         OPS-1,11,0,2026-01-06T09:30:00.000+01:00,acc-b,status,status,1,To Do,3,In Progress\n\
         OPS-2,21,0,2025-12-20T10:00:00.000+01:00,acc-a,status,status,1,To Do,3,In Progress\n\
         OPS-2,22,0,2026-01-10T10:00:00.000+01:00,acc-a,status,status,3,In Progress,10001,Done\n\
         OPS-2,22,1,2026-01-10T10:00:00.000+01:00,acc-a,resolution,resolution,,,1,Done\n"
    );
    // The token went to curl on standard input, never as an argument, and
    // is nowhere in the project.
    let calls = fs::read_to_string(tools_dir.join("calls.log")).unwrap();
    assert!(calls.contains("user = \"analyst@acme.example:SECRET_TOKEN\""));
    let mut found = Vec::new();
    for entry in walk(&root) {
        if fs::read(&entry)
            .unwrap()
            .windows(12)
            .any(|w| w == b"SECRET_TOKEN")
        {
            found.push(entry);
        }
    }
    assert!(found.is_empty(), "{found:?}");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

/// Discovery reads metadata only: issue types with their statuses, and the
/// site's own fields with their names.
#[test]
fn jira_discovery_lists_workflows_and_own_fields() {
    let dir = tempfile::tempdir().unwrap();
    let curl = dir.path().join("curl");
    fs::write(
        &curl,
        r#"#!/bin/sh
config=$(cat)
case "$config" in
  *"/project/OPS/statuses"*) printf '[{"name":"Bug","statuses":[{"name":"To Do"},{"name":"In Progress"},{"name":"Done"}]}]' ;;
  *"/rest/api/3/field"*) printf '[{"id":"status","name":"Status","custom":false},{"id":"customfield_10200","name":"Customer impact","custom":true,"schema":{"type":"option"}}]' ;;
  *) exit 22 ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).unwrap();
    let found = pm_project::discover::jira(
        &curl,
        "a@b.example:token",
        "https://acme.atlassian.net",
        &["OPS".into()],
    )
    .unwrap();
    assert_eq!(
        found["projects"][0]["issue_types"][0]["statuses"][1],
        "In Progress"
    );
    assert_eq!(
        found["custom_fields"],
        json!([{"id":"customfield_10200","name":"Customer impact","type":"option"}])
    );
    assert!(
        pm_project::discover::jira(&curl, "u", "http://acme.atlassian.net", &["OPS".into()])
            .is_err()
    );
}
