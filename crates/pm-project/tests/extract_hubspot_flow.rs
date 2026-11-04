//! A HubSpot extract from request to registered files, against a stand-in
//! for curl that answers like HubSpot's CRM API. No account or token.
#![cfg(unix)]
use pm_project::extract;
use pm_project::workspace::{InvestigationPurpose, WorkspaceFolder};
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;

/// Window: January 2026. Deal 1 was created in it and moved stage twice;
/// deal 2 was created before it and won in it; deal 3 changed after the
/// window (its later change is not kept). Contact 9 is linked to deal 1.
const CURL: &str = r#"#!/bin/sh
config=$(cat)
printf '%s\n---\n' "$config" >> "$(dirname "$0")/calls.log"
case "$config" in
  *"objects/deals/search"*)
    printf '{"results":[{"id":"1"},{"id":"2"}]}' ;;
  *"objects/contacts/search"*)
    printf '{"results":[{"id":"9"}]}' ;;
  *"pipelines/deals"*)
    printf '{"results":[{"id":"default","label":"Sales","stages":[{"id":"appointmentscheduled","label":"Appointment scheduled"},{"id":"closedwon","label":"Closed won"},{"id":"qualifiedtobuy","label":"Qualified to buy"}]}]}' ;;
  *"properties/contacts/lifecyclestage"*)
    printf '{"options":[{"value":"lead","label":"Lead"},{"value":"customer","label":"Customer"}]}' ;;
  *"properties/contacts/hs_lead_status"*)
    printf '{"options":[{"value":"NEW","label":"New"}]}' ;;
  *"objects/deals/batch/read"*)
    printf '{"results":[
      {"id":"1","properties":{"createdate":"2026-01-05T09:00:00.000Z","amount":"5000","dealstage":"qualifiedtobuy"},
       "propertiesWithHistory":{"dealstage":[
         {"value":"qualifiedtobuy","timestamp":"2026-01-07T10:00:00.000Z","sourceType":"CRM_UI","updatedByUserId":"77"},
         {"value":"appointmentscheduled","timestamp":"2026-01-05T09:00:00.000Z","sourceType":"CRM_UI","updatedByUserId":"77"}]}},
      {"id":"2","properties":{"createdate":"2025-12-10T09:00:00.000Z","amount":"900","dealstage":"closedwon"},
       "propertiesWithHistory":{"dealstage":[
         {"value":"appointmentscheduled","timestamp":"2025-12-10T09:00:00.000Z","sourceType":"CRM_UI"},
         {"value":"closedwon","timestamp":"2026-01-20T10:00:00.000Z","sourceType":"WORKFLOW"},
         {"value":"qualifiedtobuy","timestamp":"2026-02-10T10:00:00.000Z","sourceType":"CRM_UI"}]}}]}' ;;
  *"objects/contacts/batch/read"*)
    printf '{"results":[{"id":"9","properties":{"createdate":"2026-01-02T08:00:00.000Z","lifecyclestage":"customer"},
       "propertiesWithHistory":{"lifecyclestage":[{"value":"lead","timestamp":"2026-01-02T08:00:00.000Z"},{"value":"customer","timestamp":"2026-01-21T08:00:00.000Z"}],"hs_lead_status":[{"value":"NEW","timestamp":"2026-01-12T08:00:00.000Z"}]}}]}' ;;
  *"associations/deals/contacts/batch/read"*)
    printf '{"results":[{"from":{"id":"1"},"to":[{"toObjectId":9},{"toObjectId":555}]}]}' ;;
  *) printf '{"message":"unexpected request"}'; exit 22 ;;
esac
"#;

#[test]
fn quotes_use_their_own_creation_property_in_search_and_batch_reads() {
    let dir = tempfile::tempdir().unwrap();
    let mut folder = WorkspaceFolder::create_named(
        &dir.path().join("projects"),
        "Quotes",
        InvestigationPurpose::Improvement,
    )
    .unwrap();
    let curl = dir.path().join("curl");
    // Actual CRM schema: quotes expose hs_createdate, not createdate.
    // Literal source dates and expected CSV are independent of the extractor.
    fs::write(&curl, r#"#!/bin/sh
config=$(cat)
case "$config" in
  *"objects/quotes/search"*)
    case "$config" in *"hs_createdate"*) printf '{"results":[{"id":"9"}]}' ;; *) exit 22 ;; esac ;;
  *"properties/quotes/hs_status"*)
    printf '{"options":[{"value":"DRAFT","label":"Draft"}]}' ;;
  *"objects/quotes/batch/read"*)
    case "$config" in *"hs_createdate"*) printf '{"results":[{"id":"9","properties":{"hs_createdate":"2026-01-05T09:00:00.000Z","hs_status":"DRAFT"},"propertiesWithHistory":{"hs_status":[{"value":"DRAFT","timestamp":"2026-01-05T09:00:00.000Z"}]}}]}' ;; *) exit 22 ;; esac ;;
  *) exit 22 ;;
esac
"#).unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).unwrap();
    let spec = json!({"version":1,"system":"hubspot","start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z",
        "objects":[{"object":"quotes","properties":["hs_status"],"history":["hs_status"]}],"history_gaps":[]}).to_string();
    let reviewed = extract::review(spec.as_bytes()).unwrap();
    assert_eq!(reviewed.preview()["objects"][0]["created"], "hs_createdate");
    extract::request(&folder.root, &reviewed).unwrap();
    extract::approve(&folder.root, &reviewed.sha256, "Fixture author").unwrap();
    let tools = extract::Tools {
        salesforce_cli: "sf".into(),
        curl,
        jira_user: None,
        servicenow_user: None,
        hubspot_token: Some("test-token".into()),
    };
    let staged = extract::run(&folder.root, &reviewed.sha256, &tools).unwrap();
    let result = extract::register(&mut folder, staged).unwrap();
    let asset = result["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "HubSpotQuote.csv")
        .unwrap();
    assert_eq!(
        fs::read_to_string(folder.root.join(asset["relative_path"].as_str().unwrap())).unwrap(),
        "id,created,hs_status,hs_status_label\n9,2026-01-05T09:00:00.000Z,DRAFT,Draft\n"
    );
    // A schema mismatch must not silently become an empty successful extract.
    let script = fs::read_to_string(&tools.curl).unwrap().replace(
        r#""hs_createdate":"2026-01-05T09:00:00.000Z""#,
        r#""createdate":"2026-01-05T09:00:00.000Z""#,
    );
    fs::write(&tools.curl, script).unwrap();
    extract::request(&folder.root, &reviewed).unwrap();
    extract::approve(&folder.root, &reviewed.sha256, "Fixture author").unwrap();
    let error = extract::run(&folder.root, &reviewed.sha256, &tools)
        .err()
        .unwrap();
    assert!(error.contains("without a valid hs_createdate"), "{error}");
}

#[test]
fn a_hubspot_extract_reads_changed_records_their_history_labels_and_links() {
    let dir = tempfile::tempdir().unwrap();
    let mut folder = WorkspaceFolder::create_named(
        &dir.path().join("projects"),
        "Deals",
        InvestigationPurpose::Improvement,
    )
    .unwrap();
    let tools_dir = dir.path().join("tools");
    fs::create_dir_all(&tools_dir).unwrap();
    let curl = tools_dir.join("curl");
    fs::write(&curl, CURL).unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o700)).unwrap();
    let spec = json!({"version":1,"system":"hubspot","start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z",
        "objects":[{"object":"deals","properties":["amount"],"history":["dealstage"]},
                   {"object":"contacts","properties":[],"history":["lifecyclestage","hs_lead_status"]}],
        "associations":[["deals","contacts"]],"history_gaps":[]})
    .to_string();
    let reviewed = extract::review(spec.as_bytes()).unwrap();
    assert_eq!(reviewed.preview()["system"], "hubspot");
    let root = folder.root.clone();
    extract::request(&root, &reviewed).unwrap();
    let tools = extract::Tools {
        salesforce_cli: "sf".into(),
        curl: curl.clone(),
        jira_user: None,
        servicenow_user: None,
        hubspot_token: Some("pat-SECRET_TOKEN".into()),
    };
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
    assert_eq!(
        source("HubSpotDeal.csv"),
        "id,created,amount,dealstage,dealstage_label\n\
         1,2026-01-05T09:00:00.000Z,5000,qualifiedtobuy,Qualified to buy\n\
         2,2025-12-10T09:00:00.000Z,900,closedwon,Closed won\n"
    );
    // History in time order with labels; the first value is marked; the
    // change after the window is left out, the one before it kept.
    assert_eq!(
        source("HubSpotHistory.csv"),
        "object,record,property,value,label,timestamp,source,user,first\n\
         deals,1,dealstage,appointmentscheduled,Appointment scheduled,2026-01-05T09:00:00.000Z,CRM_UI,77,true\n\
         deals,1,dealstage,qualifiedtobuy,Qualified to buy,2026-01-07T10:00:00.000Z,CRM_UI,77,false\n\
         deals,2,dealstage,appointmentscheduled,Appointment scheduled,2025-12-10T09:00:00.000Z,CRM_UI,,true\n\
         deals,2,dealstage,closedwon,Closed won,2026-01-20T10:00:00.000Z,WORKFLOW,,false\n\
         contacts,9,lifecyclestage,lead,Lead,2026-01-02T08:00:00.000Z,,,true\n\
         contacts,9,lifecyclestage,customer,Customer,2026-01-21T08:00:00.000Z,,,false\n\
         contacts,9,hs_lead_status,NEW,New,2026-01-12T08:00:00.000Z,,,false\n"
    );
    // Only links between records read.
    assert_eq!(
        source("HubSpotAssociation.csv"),
        "from_object,from_id,to_object,to_id\ndeals,1,contacts,9\n"
    );
    // The token went to curl on standard input only, and is nowhere in the
    // project.
    let calls = fs::read_to_string(tools_dir.join("calls.log")).unwrap();
    assert!(calls.contains("Authorization: Bearer pat-SECRET_TOKEN"));
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                assert!(
                    !fs::read(&path)
                        .unwrap()
                        .windows(12)
                        .any(|w| w == b"SECRET_TOKEN")
                );
            }
        }
    }
}
