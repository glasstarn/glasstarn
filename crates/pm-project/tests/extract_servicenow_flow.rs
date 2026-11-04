//! A ServiceNow extract from request to registered files, against a stand-in
//! for curl that answers like ServiceNow's Table API. No instance or
//! credentials.
#![cfg(unix)]
use pm_project::extract;
use pm_project::workspace::{InvestigationPurpose, WorkspaceFolder};
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;

/// Window: January 2026. CHG1 was raised in it; CHG2 before it and changed
/// in it; CHG3 had no activity in it; CHG4 was raised after it. CHG1 has a
/// change task. Audit rows include CHG1's initial state (not a change), a
/// work note (not kept) and a CHG2 change after the window.
const CURL: &str = r#"#!/bin/sh
config=$(cat)
printf '%s\n---\n' "$config" >> "$(dirname "$0")/calls.log"
v() { printf '{"value":"%s","display_value":"%s"}' "$1" "$2"; }
case "$config" in
  *"table/sys_choice"*"nameINchange_task"*)
    printf '{"result":[{"name":"task","element":"state","value":"1","label":"Open"},{"name":"task","element":"state","value":"2","label":"Work in Progress"}]}' ;;
  *"table/sys_choice"*"nameINchange_request"*)
    printf '{"result":[
      {"name":"task","element":"state","value":"-2","label":"Pending"},
      {"name":"change_request","element":"state","value":"-5","label":"New"},
      {"name":"change_request","element":"state","value":"-4","label":"Assess"},
      {"name":"change_request","element":"state","value":"-3","label":"Authorize"},
      {"name":"change_request","element":"state","value":"-2","label":"Scheduled"}]}' ;;
  *"table/sys_user_group"*)
    printf '{"result":[{"sys_id":"g1","name":"Service Desk"}]}' ;;
  *"table/sys_audit"*"tablename%3Dchange_request"*)
    printf '{"result":[
      {"sys_id":"a1","documentkey":"c1","fieldname":"state","oldvalue":"","newvalue":"-5","sys_created_on":"2026-01-05 09:00:00","user":"admin"},
      {"sys_id":"a2","documentkey":"c1","fieldname":"state","oldvalue":"-5","newvalue":"-2","sys_created_on":"2026-01-07 10:00:00","user":"admin"},
      {"sys_id":"a3","documentkey":"c1","fieldname":"assignment_group","oldvalue":"","newvalue":"g1","sys_created_on":"2026-01-07 10:00:00","user":"admin"},
      {"sys_id":"a4","documentkey":"c1","fieldname":"work_notes","oldvalue":"","newvalue":"checked","sys_created_on":"2026-01-08 10:00:00","user":"admin"},
      {"sys_id":"a5","documentkey":"c2","fieldname":"state","oldvalue":"-4","newvalue":"-3","sys_created_on":"2026-01-10 10:00:00","user":"admin"},
      {"sys_id":"a6","documentkey":"c2","fieldname":"state","oldvalue":"-5","newvalue":"-4","sys_created_on":"2025-12-11 10:00:00","user":"admin"},
      {"sys_id":"a7","documentkey":"c3","fieldname":"state","oldvalue":"-5","newvalue":"-4","sys_created_on":"2025-12-01 10:00:00","user":"admin"},
      {"sys_id":"a8","documentkey":"c2","fieldname":"state","oldvalue":"-3","newvalue":"-2","sys_created_on":"2026-02-10 10:00:00","user":"admin"}]}' ;;
  *"table/sys_audit"*"tablename%3Dchange_task"*)
    printf '{"result":[{"sys_id":"b1","documentkey":"t1","fieldname":"state","oldvalue":"1","newvalue":"2","sys_created_on":"2026-01-08 10:00:00","user":"admin"}]}' ;;
  *"table/change_task"*"change_requestINc1%2Cc2"*)
    printf '{"result":[{"sys_id":%s,"number":%s,"sys_created_on":%s,"change_request":%s,"state":%s}]}' \
      "$(v t1 t1)" "$(v CTASK1 CTASK1)" "$(v '2026-01-06 09:00:00' x)" "$(v c1 CHG1)" "$(v 2 'Work in Progress')" ;;
  *"table/change_request"*)
    row() { printf '{"sys_id":%s,"number":%s,"sys_created_on":%s,"state":%s,"assignment_group":%s}' \
      "$(v $1 $1)" "$(v $2 $2)" "$(v "$3" x)" "$(v $4 "$5")" "$(v g2 Network)"; }
    printf '{"result":[%s,%s,%s,%s]}' \
      "$(row c1 CHG1 '2026-01-05 09:00:00' -2 Scheduled)" "$(row c2 CHG2 '2025-12-10 09:00:00' -3 Authorize)" \
      "$(row c3 CHG3 '2025-11-01 09:00:00' -4 Assess)" "$(row c4 CHG4 '2026-02-03 09:00:00' -5 New)" ;;
  *) printf '{"error":{"message":"unexpected request"}}'; exit 22 ;;
esac
"#;

fn stand_in(dir: &std::path::Path, script: &str) -> std::path::PathBuf {
    let tools = dir.join("tools");
    fs::create_dir_all(&tools).unwrap();
    let curl = tools.join("curl");
    fs::write(&curl, script).unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).unwrap();
    curl
}

#[test]
fn a_servicenow_extract_reads_records_active_in_the_window_with_children_and_audit() {
    let dir = tempfile::tempdir().unwrap();
    let mut folder = WorkspaceFolder::create_named(
        &dir.path().join("projects"),
        "Change management",
        InvestigationPurpose::Improvement,
    )
    .unwrap();
    let curl = stand_in(dir.path(), CURL);
    let spec = json!({"version":1,"system":"servicenow","instance":"https://acme.service-now.com",
        "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z",
        "tables":[{"table":"change_request","fields":["state","assignment_group"],"label_fields":["state","assignment_group"]},
                  {"table":"change_task","fields":["state"],"label_fields":["state"],"parent":{"field":"change_request","table":"change_request"}}],
        "history_fields":["state","assignment_group"],"history_gaps":[]})
    .to_string()
    .into_bytes();
    let reviewed = extract::review(&spec).unwrap();
    assert_eq!(reviewed.preview()["system"], "servicenow");
    let root = folder.root.clone();
    extract::request(&root, &reviewed).unwrap();
    let tools = extract::Tools {
        salesforce_cli: "sf".into(),
        curl: curl.clone(),
        jira_user: None,
        servicenow_user: Some("reader:SECRET_PASSWORD".into()),
        hubspot_token: None,
    };
    // Nothing runs before a person approves.
    assert!(extract::run(&root, &reviewed.sha256, &tools).is_err());
    extract::approve(&root, &reviewed.sha256, "Pat Lee").unwrap();
    let staged = extract::run(&root, &reviewed.sha256, &tools).unwrap();
    let result = extract::register(&mut folder, staged).unwrap();
    assert_eq!(result["status"], "complete");
    let report: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join(result["report"].as_str().expect("report path"))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["objects"][2]["initial_values_left_out"], 1,
        "{report}"
    );

    let source = |name: &str| {
        let asset = result["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap();
        fs::read_to_string(root.join(asset["relative_path"].as_str().unwrap())).unwrap()
    };
    // The table's own state labels win over those inherited from task.
    assert_eq!(
        source("ServiceNowChangeRequest.csv"),
        "sys_id,number,created,state,assignment_group,state_label,assignment_group_label\n\
         c1,CHG1,2026-01-05T09:00:00Z,-2,g2,Scheduled,Network\n\
         c2,CHG2,2025-12-10T09:00:00Z,-3,g2,Authorize,Network\n"
    );
    assert_eq!(
        source("ServiceNowChangeTask.csv"),
        "sys_id,number,created,change_request,state,state_label\n\
         t1,CTASK1,2026-01-06T09:00:00Z,c1,2,Work in Progress\n"
    );
    // CHG1's initial state, the work note, CHG3 and the change after the
    // window are left out; CHG2's December change is kept as history. The
    // first group's name comes from the group itself.
    assert_eq!(
        source("ServiceNowAudit.csv"),
        "table,record,audit_id,created,user,field,old_value,old_label,new_value,new_label\n\
         change_request,c2,a6,2025-12-11T10:00:00Z,admin,state,-5,New,-4,Assess\n\
         change_request,c1,a2,2026-01-07T10:00:00Z,admin,state,-5,New,-2,Scheduled\n\
         change_request,c1,a3,2026-01-07T10:00:00Z,admin,assignment_group,,,g1,Service Desk\n\
         change_request,c2,a5,2026-01-10T10:00:00Z,admin,state,-4,Assess,-3,Authorize\n\
         change_task,t1,b1,2026-01-08T10:00:00Z,admin,state,1,Open,2,Work in Progress\n"
    );
    // The password went to curl on standard input only, and is nowhere in
    // the project.
    let calls = fs::read_to_string(curl.parent().unwrap().join("calls.log")).unwrap();
    assert!(calls.contains("user = \"reader:SECRET_PASSWORD\""));
    assert!(walk(&root).iter().all(|f| {
        !fs::read(f)
            .unwrap()
            .windows(15)
            .any(|w| w == b"SECRET_PASSWORD")
    }));
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

/// Discovery reads metadata only: states, the company's own fields, whether
/// audit history can be read, and the assignment groups.
#[test]
fn servicenow_discovery_lists_states_own_fields_groups_and_audit_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let curl = stand_in(
        dir.path(),
        r#"#!/bin/sh
config=$(cat)
case "$config" in
  *"table/sys_dictionary"*) printf '{"result":[{"name":"incident","element":"u_business_impact","column_label":"Business impact","internal_type":"choice"}]}' ;;
  *"table/sys_choice"*) printf '{"result":[{"name":"incident","value":"1","label":"New"},{"name":"incident","value":"2","label":"In Progress"}]}' ;;
  *"table/sys_audit"*"tablename%3Dincident"*) printf '{"result":[{"sys_id":"a1"}]}' ;;
  *"table/sys_audit"*) printf '{"error":{"message":"User Not Authorized"}}'; exit 22 ;;
  *"table/sys_user_group"*) printf '{"result":[{"name":"Network"},{"name":"Service Desk"}]}' ;;
  *) exit 22 ;;
esac
"#,
    );
    let found = pm_project::discover::servicenow(
        &curl,
        "reader:pw",
        "https://acme.service-now.com",
        &["incident".into(), "problem".into()],
    )
    .unwrap();
    let incident = &found["tables"][0];
    assert_eq!(incident["file"], "ServiceNowIncident");
    assert_eq!(incident["states"][1]["label"], "In Progress");
    assert_eq!(incident["custom_fields"][0]["label"], "Business impact");
    assert_eq!(incident["audit_readable"], true);
    assert_eq!(found["tables"][1]["audit_readable"], false);
    let gaps = found["gaps"].as_array().unwrap();
    assert_eq!(gaps.len(), 1);
    assert!(gaps[0].as_str().unwrap().contains("User Not Authorized"));
    assert_eq!(
        found["assignment_groups"],
        json!(["Network", "Service Desk"])
    );
    assert!(
        pm_project::discover::servicenow(
            &curl,
            "u:p",
            "https://acme.service-now.com",
            &["sys_user".into()]
        )
        .is_err()
    );
}

/// An HR case table and its tasks, read as tickets; sys_user is not built on
/// task.
const TICKET_CURL: &str = r#"#!/bin/sh
config=$(cat)
v() { printf '{"value":"%s","display_value":"%s"}' "$1" "$2"; }
case "$config" in
  *"table/sys_db_object"*"name%3Dsn_hr_core_case"*) printf '{"result":[{"name":"sn_hr_core_case","super_class.name":"task"}]}' ;;
  *"table/sys_db_object"*"name%3Dsn_hr_core_task"*) printf '{"result":[{"name":"sn_hr_core_task","super_class.name":"task"}]}' ;;
  *"table/sys_db_object"*"name%3Dsys_user"*) printf '{"result":[{"name":"sys_user","super_class.name":""}]}' ;;
  *"table/sys_choice"*"nameINsn_hr_core_task"*)
    printf '{"result":[{"name":"task","element":"state","value":"3","label":"Closed Complete"}]}' ;;
  *"table/sys_choice"*"nameINsn_hr_core_case"*)
    printf '{"result":[{"name":"sn_hr_core_case","element":"state","value":"10","label":"Ready"},{"name":"sn_hr_core_case","element":"state","value":"18","label":"Awaiting Acceptance"}]}' ;;
  *"table/sys_audit"*"tablename%3Dsn_hr_core_case"*)
    printf '{"result":[{"sys_id":"a1","documentkey":"h1","fieldname":"state","oldvalue":"10","newvalue":"18","sys_created_on":"2026-01-07 10:00:00","user":"admin"}]}' ;;
  *"table/sys_audit"*"tablename%3Dsn_hr_core_task"*) printf '{"result":[]}' ;;
  *"table/sn_hr_core_task"*"parentINh1"*)
    printf '{"result":[{"sys_id":%s,"number":%s,"sys_created_on":%s,"parent":%s,"state":%s}]}' \
      "$(v k1 k1)" "$(v HRT1 HRT1)" "$(v '2026-01-06 09:00:00' x)" "$(v h1 HRC1)" "$(v 3 'Closed Complete')" ;;
  *"table/sn_hr_core_case"*)
    printf '{"result":[{"sys_id":%s,"number":%s,"sys_created_on":%s,"state":%s}]}' \
      "$(v h1 h1)" "$(v HRC1 HRC1)" "$(v '2026-01-05 09:00:00' x)" "$(v 18 'Awaiting Acceptance')" ;;
  *) printf '{"error":{"message":"unexpected request"}}'; exit 22 ;;
esac
"#;

fn ticket_run(spec: serde_json::Value) -> Result<(WorkspaceFolder, serde_json::Value), String> {
    let dir = tempfile::tempdir().unwrap().keep();
    let folder = WorkspaceFolder::create_named(
        &dir.join("projects"),
        "HR cases",
        InvestigationPurpose::Improvement,
    )
    .unwrap();
    let curl = stand_in(&dir, TICKET_CURL);
    let reviewed = extract::review(spec.to_string().as_bytes())?;
    let root = folder.root.clone();
    extract::request(&root, &reviewed)?;
    extract::approve(&root, &reviewed.sha256, "Pat Lee")?;
    let tools = extract::Tools {
        salesforce_cli: "sf".into(),
        curl,
        jira_user: None,
        servicenow_user: Some("reader:pw".into()),
        hubspot_token: None,
    };
    let staged = extract::run(&root, &reviewed.sha256, &tools)?;
    let mut folder = folder;
    let result = extract::register(&mut folder, staged)?;
    Ok((folder, result))
}

#[test]
fn any_ticket_table_built_on_task_is_read_as_tickets() {
    let spec = json!({"version":1,"system":"servicenow","instance":"https://acme.service-now.com",
        "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z",
        "tables":[{"table":"sn_hr_core_case","as":"ticket","fields":["state"],"label_fields":["state"]},
                  {"table":"sn_hr_core_task","as":"ticket_task","fields":["state"],"label_fields":["state"],
                   "parent":{"field":"parent","table":"sn_hr_core_case"}}],
        "history_fields":["state"],"history_gaps":[]});
    let (folder, result) = ticket_run(spec).unwrap();
    let source = |name: &str| {
        let asset = result["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap();
        fs::read_to_string(folder.root.join(asset["relative_path"].as_str().unwrap())).unwrap()
    };
    assert_eq!(
        source("ServiceNowTicket.csv"),
        "sys_id,number,created,state,state_label\nh1,HRC1,2026-01-05T09:00:00Z,18,Awaiting Acceptance\n"
    );
    // The task's own parent field is written as `ticket`.
    assert_eq!(
        source("ServiceNowTicketTask.csv"),
        "sys_id,number,created,ticket,state,state_label\nk1,HRT1,2026-01-06T09:00:00Z,h1,3,Closed Complete\n"
    );
    assert!(source("ServiceNowAudit.csv").contains(
        "sn_hr_core_case,h1,a1,2026-01-07T10:00:00Z,admin,state,10,Ready,18,Awaiting Acceptance"
    ));
}

#[test]
fn a_table_not_built_on_task_is_not_read_as_tickets() {
    let spec = json!({"version":1,"system":"servicenow","instance":"https://acme.service-now.com",
        "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z",
        "tables":[{"table":"sys_user","as":"ticket","fields":["state"],"label_fields":[]}],
        "history_fields":[],"history_gaps":[]});
    let error = ticket_run(spec).err().unwrap();
    assert!(
        error.contains("not built on ServiceNow's task table"),
        "{error}"
    );
    // A table outside the list without "as" is refused before anything runs.
    let unnamed = json!({"version":1,"system":"servicenow","instance":"https://acme.service-now.com",
        "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z",
        "tables":[{"table":"sn_hr_core_case","fields":["state"],"label_fields":[]}],
        "history_fields":[],"history_gaps":[]});
    let error = extract::review(unnamed.to_string().as_bytes())
        .err()
        .unwrap();
    assert!(error.contains("\"as\": \"ticket\""), "{error}");
}
