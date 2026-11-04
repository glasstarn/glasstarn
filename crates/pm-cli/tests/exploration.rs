use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;
fn cli(root: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(args)
        .arg("--workspace-dir")
        .arg(root.join("projects"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn interval_exploration_is_available_through_the_real_workspace_command() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let csv = root.join("orders.csv");
    std::fs::write(&csv,"case_id,activity,timestamp,vendor\nA,Create,2026-01-01T00:00:00Z,ABC\nA,Ship,2026-01-13T00:00:00Z,ABC\nB,Create,2026-01-01T00:00:00Z,XYZ\nB,Ship,2026-01-04T00:00:00Z,XYZ\nC,Create,2026-01-01T00:00:00Z,ABC\n").unwrap();
    let created = cli(root, &["workspace", "create", "--name", "Interval test"]);
    let id = created["project"]["id"].as_str().unwrap();
    cli(
        root,
        &[
            "workspace",
            "import",
            id,
            csv.to_str().unwrap(),
            "--case",
            "case_id",
            "--activity",
            "activity",
            "--timestamp",
            "timestamp",
            "--case-attribute",
            "vendor",
        ],
    );
    let file = root.join("query.json");
    std::fs::write(&file,json!({"type":"explore","group_by":"vendor","segment":{"from":"Create","to":"Ship","to_occurrence":"last","min_micros":864000000000_i64,"max_micros":1555200000000_i64}}).to_string()).unwrap();
    let result = cli(
        root,
        &[
            "workspace",
            "query",
            id,
            "--request",
            file.to_str().unwrap(),
        ],
    );
    let result = &result["response"]["result"];
    assert_eq!(result["overall"]["items"], 3);
    assert_eq!(result["overall"]["measurable"], 2);
    assert_eq!(result["selected"]["items"], 1);
    assert_eq!(result["selected"]["examples"], json!(["A"]));
}
