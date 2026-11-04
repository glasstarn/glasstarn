//! The command line files extracts for approval in the app and reports their
//! state. It cannot approve or download, so no stand-in CLI is needed here;
//! the approved run is covered in pm-project and pm-server.
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(args)
        .arg("--workspace-dir")
        .arg(root.join("projects"))
        // An empty PATH proves no external tool is started.
        .env("PATH", "")
        .output()
        .unwrap()
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn fixture() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("spec.json"), json!({"version":2,"system":"salesforce","target_org":"approved-org","api_version":66,
        "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z","objects":[
        {"name":"Opportunity","fields":["Id","CreatedDate","ERP_Order_ID__c"],"time_field":"CreatedDate","kind":"records","history_gaps":[]},
        {"name":"OpportunityFieldHistory","fields":["Id","OpportunityId","CreatedDate","Field"],"time_field":"CreatedDate","kind":"history","item_field":"OpportunityId","parent":"Opportunity","history_gaps":[]}]}).to_string()).unwrap();
    let created = success(cli(
        root,
        &[
            "workspace",
            "create",
            "--name",
            "Deals",
            "--purpose",
            "improvement",
        ],
    ));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    (dir, id)
}
fn request(root: &Path, id: &str) -> Output {
    cli(
        root,
        &[
            "extract",
            "salesforce",
            id,
            "--spec",
            root.join("spec.json").to_str().unwrap(),
        ],
    )
}

#[test]
fn filing_an_extract_waits_for_approval_in_the_app_and_downloads_nothing() {
    let (dir, id) = fixture();
    let root = dir.path();
    // The app holds the project's write lock while it is open; filing still works.
    let open_in_app =
        pm_project::workspace::WorkspaceFolder::open(&root.join("projects").join(&id)).unwrap();
    let filed = success(request(root, &id));
    assert_eq!(filed["status"], "awaiting_approval");
    assert_eq!(filed["request_status"], "pending");
    assert!(
        filed["preview"]["exports"][0]["soql"]
            .as_str()
            .unwrap()
            .ends_with("WHERE Id IN (<items active in the window>)")
    );
    assert!(filed["next"].as_str().unwrap().contains("Approve and run"));

    let status = success(cli(root, &["extract", "status", &id]));
    assert_eq!(status["requests"][0]["status"], "pending");
    assert!(status["requests"][0].get("preview").is_none());
    let log = fs::read_to_string(open_in_app.root.join("audit/log.jsonl")).unwrap();
    assert!(log.contains("\"extract_requested\""));
    assert!(open_in_app.state().unwrap().preparation_sources.is_empty());
}

#[test]
fn the_command_line_cannot_approve() {
    let (dir, id) = fixture();
    let root = dir.path();
    let filed = success(request(root, &id));
    let attempt = cli(
        root,
        &[
            "extract",
            "salesforce",
            &id,
            "--spec",
            root.join("spec.json").to_str().unwrap(),
            "--approve",
            filed["spec_sha256"].as_str().unwrap(),
        ],
    );
    assert!(!attempt.status.success());
    assert!(String::from_utf8_lossy(&attempt.stderr).contains("approval is given in the app"));
    let status = success(cli(root, &["extract", "status", &id]));
    assert_eq!(status["requests"][0]["status"], "pending");
}

#[test]
fn invalid_specs_file_nothing() {
    let (dir, id) = fixture();
    let root = dir.path();
    let mut spec: Value =
        serde_json::from_slice(&fs::read(root.join("spec.json")).unwrap()).unwrap();
    spec["access_token"] = json!("secret");
    fs::write(root.join("spec.json"), spec.to_string()).unwrap();
    assert!(!request(root, &id).status.success());
    let status = success(cli(root, &["extract", "status", &id]));
    assert!(status["requests"].as_array().unwrap().is_empty());
}
