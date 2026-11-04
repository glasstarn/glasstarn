//! A pack check after a fresh extract uses the extract's files, even when an
//! earlier inventory or extract left files with the same table names. Found
//! in the first real Salesforce test, where a second pull into the same
//! project made every table "ambiguous".
#![cfg(unix)] // runs a stand-in shell script for the CLI
use pm_project::extract;
use pm_project::workspace::WorkspaceFolder;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

/// A stand-in for the Salesforce CLI: one case with one history row.
const SF: &str = r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
  case "$1" in --output-file) shift; output="$1" ;; --query) shift; query="$1" ;; esac
  shift
done
case "$query" in
  "SELECT Id FROM Case "*) printf 'Id\n500NEW\n' > "$output" ;;
  "SELECT Id,CaseId FROM CaseHistory "*) printf 'Id,CaseId\nH1,500NEW\n' > "$output" ;;
  *"FROM Case "*) printf 'Id,CreatedDate,Status\n500NEW,2026-01-05T09:00:00.000Z,New\n' > "$output" ;;
  *) closed=Closed; [ -f "$(dirname "$0")/second" ] && closed=Resolved
     printf 'Id,CaseId,Field,OldValue,NewValue,CreatedDate,DataType\nH1,500NEW,Status,New,%s,2026-01-06T09:00:00.000Z,Text\n' "$closed" > "$output" ;;
esac
printf '{"status":0,"result":{"totalSize":1}}'
"#;

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(args)
        .arg("--workspace-dir")
        .arg(root.join("projects"))
        .output()
        .unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn pull(folder_path: &Path, sf: &Path, end: &str) {
    let spec = json!({"version":2,"system":"salesforce","target_org":"approved-org","api_version":66,
        "start":"2026-01-01T00:00:00Z","end":end,"objects":[
        {"name":"Case","fields":["Id","CreatedDate","Status"],"time_field":"CreatedDate","kind":"records","history_gaps":[]},
        {"name":"CaseHistory","fields":["Id","CaseId","Field","OldValue","NewValue","CreatedDate","DataType"],
         "time_field":"CreatedDate","kind":"history","item_field":"CaseId","parent":"Case","history_gaps":[]}]})
    .to_string();
    let reviewed = extract::review(spec.as_bytes()).unwrap();
    extract::request(folder_path, &reviewed).unwrap();
    extract::approve(folder_path, &reviewed.sha256, "Test approver").unwrap();
    let staged = extract::download(folder_path, &reviewed.sha256, sf).unwrap();
    let mut folder = WorkspaceFolder::open(folder_path).unwrap();
    extract::register(&mut folder, staged).unwrap();
}

#[test]
fn a_later_pull_wins_over_earlier_files_with_the_same_table_names() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = success(cli(root, &["workspace", "create", "--name", "Support"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let folder = Path::new(created["project"]["path"].as_str().unwrap()).to_owned();

    // An earlier, hand-made export of three cases.
    let old = root.join("old");
    fs::create_dir_all(&old).unwrap();
    fs::write(
        old.join("Case.csv"),
        "Id,CreatedDate,Status\nA,2025-12-01T09:00:00.000Z,New\nB,2025-12-01T09:00:00.000Z,New\nC,2025-12-01T09:00:00.000Z,New\n",
    )
    .unwrap();
    fs::write(
        old.join("CaseHistory.csv"),
        "Id,CaseId,Field,OldValue,NewValue,CreatedDate,DataType\nH0,A,Status,New,Closed,2025-12-02T09:00:00.000Z,Text\n",
    )
    .unwrap();
    success(cli(
        root,
        &[
            "workspace",
            "inventory",
            &id,
            old.join("Case.csv").to_str().unwrap(),
            old.join("CaseHistory.csv").to_str().unwrap(),
        ],
    ));

    let tools = root.join("tools");
    fs::create_dir_all(&tools).unwrap();
    let sf = tools.join("sf");
    fs::write(&sf, SF).unwrap();
    fs::set_permissions(&sf, fs::Permissions::from_mode(0o700)).unwrap();
    pull(&folder, &sf, "2026-02-01T00:00:00Z");

    // The made-up export lacks some fields, so the check reports gaps; what
    // matters here is which files it read.
    let check = checked(root, &id);
    let tables = check["tables"].to_string();
    // The pulled case, not the three from the old export.
    assert!(tables.contains("Case: 1 rows"), "{tables}");

    // A second pull of a wider window, with a changed history row: its files
    // win over the first pull's.
    fs::write(tools.join("second"), "").unwrap();
    pull(&folder, &sf, "2026-03-01T00:00:00Z");
    let check = checked(root, &id);
    assert!(check["tables"].to_string().contains("Case: 1 rows"));
}

fn checked(root: &Path, id: &str) -> Value {
    let output = cli(
        root,
        &[
            "workspace",
            "pack",
            "check",
            id,
            "--pack",
            "salesforce-case-handling",
        ],
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}
