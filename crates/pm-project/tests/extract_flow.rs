//! Request → approval in the app → download → registration, against an
//! isolated stand-in for the Salesforce CLI. No tenant or credentials.
#![cfg(unix)]
use pm_project::workspace::{InvestigationPurpose, WorkspaceFolder};
use pm_project::{audit, extract};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Case 500A changed in December and January; the window is January.
const CLI: &str = r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
  case "$1" in --output-file) shift; output="$1" ;; --query) shift; query="$1" ;;
    # Like the real CLI: a version must read like 66.0.
    --api-version) shift; case "$1" in *.0) ;; *) exit 2 ;; esac ;; esac
  shift
done
printf '%s\n' "$query" >> "$(dirname "$0")/queries.log"
case "$query" in
  "SELECT Id,CaseId FROM"*) printf 'Id,CaseId\nH1,500A\n' > "$output"; n=1 ;;
  *) printf 'Id,CaseId,CreatedDate,Field\nH0,500A,2025-12-20T00:00:00.000Z,Status\nPRIVATE_ROW,500A,2026-01-02T00:00:00.000Z,Status\n' > "$output"; n=2 ;;
esac
[ -f "$(dirname "$0")/fail" ] && { printf 'PRIVATE_VENDOR_ERROR'; exit 1; }
[ -f "$(dirname "$0")/short" ] && n=3
printf '{"status":0,"result":{"totalSize":%s}}' "$n"
"#;

fn spec() -> Vec<u8> {
    json!({"version":2,"system":"salesforce","target_org":"approved-org","api_version":66,
        "start":"2026-01-01T00:00:00Z","end":"2026-02-01T00:00:00Z","objects":[
        {"name":"CaseHistory","fields":["Id","CaseId","CreatedDate","Field"],"time_field":"CreatedDate",
         "kind":"history","item_field":"CaseId","history_gaps":[]}]})
    .to_string()
    .into_bytes()
}

fn fixture() -> (tempfile::TempDir, WorkspaceFolder, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let folder = WorkspaceFolder::create_named(
        &dir.path().join("projects"),
        "Support delays",
        InvestigationPurpose::Improvement,
    )
    .unwrap();
    let tools = dir.path().join("tools");
    fs::create_dir_all(&tools).unwrap();
    let sf = tools.join("sf");
    fs::write(&sf, CLI).unwrap();
    fs::set_permissions(&sf, fs::Permissions::from_mode(0o700)).unwrap();
    (dir, folder, sf)
}

fn actions(root: &Path) -> Vec<String> {
    fs::read_to_string(audit::path(root))
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["action"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn nothing_downloads_until_a_person_approves_then_everything_is_audited() {
    let (_dir, mut folder, sf) = fixture();
    let root = folder.root.clone();
    let reviewed = extract::review(&spec()).unwrap();
    let request = extract::request(&root, &reviewed).unwrap();
    assert_eq!(request["status"], "pending");
    // Filing again does not duplicate the request.
    extract::request(&root, &reviewed).unwrap();
    assert_eq!(extract::requests(&root).unwrap().len(), 1);

    // Not approved: the runner refuses and the CLI is never called.
    assert!(
        extract::download(&root, &reviewed.sha256, &sf)
            .unwrap_err()
            .contains("approved")
    );
    assert!(!sf.with_file_name("queries.log").exists());

    assert!(extract::approve(&root, &reviewed.sha256, "  ").is_err());
    let approved = extract::approve(&root, &reviewed.sha256, "Pat Lee").unwrap();
    assert_eq!(approved["status"], "running");
    assert!(
        extract::approve(&root, &reviewed.sha256, "Pat Lee").is_err(),
        "one approval, one run"
    );

    let staged = extract::download(&root, &reviewed.sha256, &sf).unwrap();
    let result = extract::register(&mut folder, staged).unwrap();
    assert_eq!(result["row_counts"][0]["rows"], 2);
    assert_eq!(result["row_counts"][0]["rows_before_window"], 1);
    assert!(!result.to_string().contains("PRIVATE_ROW"));

    let state = folder.state().unwrap();
    assert_eq!(state.preparation_sources.len(), 1);
    assert_eq!(
        state.observation_window.unwrap().start_micros,
        1_767_225_600_000_000
    );
    let report: Value =
        serde_json::from_slice(&fs::read(root.join(result["report"].as_str().unwrap())).unwrap())
            .unwrap();
    assert_eq!(report["approved_by"], "Pat Lee");
    assert_eq!(extract::requests(&root).unwrap()[0]["status"], "complete");

    assert_eq!(
        actions(&root),
        ["extract_requested", "extract_approved", "extract_completed"]
    );
    assert_eq!(audit::verify(&root).unwrap(), 3);
    let log = fs::read_to_string(audit::path(&root)).unwrap();
    assert!(
        log.contains(&state.preparation_sources[0].sha256),
        "output hashes are logged"
    );
    assert!(!log.contains("PRIVATE_ROW"));
}

#[test]
fn failures_and_declines_register_nothing_and_are_audited() {
    for marker in ["fail", "short"] {
        let (_dir, folder, sf) = fixture();
        let root = folder.root.clone();
        fs::write(sf.with_file_name(marker), "").unwrap();
        let reviewed = extract::review(&spec()).unwrap();
        extract::request(&root, &reviewed).unwrap();
        extract::approve(&root, &reviewed.sha256, "Pat Lee").unwrap();
        let error = extract::download(&root, &reviewed.sha256, &sf).unwrap_err();
        assert!(!error.contains("PRIVATE_VENDOR_ERROR"));
        extract::fail(&root, &reviewed.sha256, &error).unwrap();
        assert!(folder.state().unwrap().preparation_sources.is_empty());
        assert_eq!(
            actions(&root),
            ["extract_requested", "extract_approved", "extract_failed"]
        );
        // A failed run needs a new request and a new approval.
        assert_eq!(
            extract::request(&root, &reviewed).unwrap()["status"],
            "pending"
        );
    }
    let (_dir, folder, sf) = fixture();
    let root = folder.root.clone();
    let reviewed = extract::review(&spec()).unwrap();
    extract::request(&root, &reviewed).unwrap();
    extract::decline(&root, &reviewed.sha256, "Pat Lee").unwrap();
    assert!(extract::download(&root, &reviewed.sha256, &sf).is_err());
    assert!(extract::approve(&root, &reviewed.sha256, "Pat Lee").is_err());
    assert_eq!(actions(&root), ["extract_requested", "extract_declined"]);
}

#[test]
fn an_edited_request_cannot_be_approved() {
    let (_dir, folder, _sf) = fixture();
    let root = folder.root.clone();
    let reviewed = extract::review(&spec()).unwrap();
    extract::request(&root, &reviewed).unwrap();
    let saved = root
        .join("extraction/requests")
        .join(&reviewed.sha256)
        .join("spec.json");
    let widened = String::from_utf8(spec())
        .unwrap()
        .replace("2026-01-01", "2020-01-01");
    fs::write(saved, widened).unwrap();
    assert!(
        extract::approve(&root, &reviewed.sha256, "Pat Lee")
            .unwrap_err()
            .contains("changed")
    );
    assert!(extract::approve(&root, "../../etc", "Pat Lee").is_err());
}
