//! `glasstarn doctor` says what is wrong and how to fix it; `workspace
//! export` writes the browser's exports into the project folder.
use std::process::Command;

fn glasstarn(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(args)
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("GLASSTARN_NO_UPDATE_CHECK", "1")
        .env("GLASSTARN_NO_BROWSER", "1")
        .output()
        .unwrap()
}

#[test]
fn doctor_names_each_problem_with_its_fix() {
    let home = tempfile::tempdir().unwrap();
    let out = glasstarn(home.path(), &["doctor", "--json"]);
    // The command is not on PATH here, so there is one thing to fix.
    assert_eq!(out.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let checks = report["checks"].as_array().unwrap();
    let path = checks
        .iter()
        .find(|c| c["check"].as_str().unwrap().contains("PATH"))
        .unwrap();
    assert_eq!(path["ok"], false);
    assert_eq!(path["fix"], "run `npx glasstarn setup`");
    assert!(checks.iter().any(|c| {
        c["ok"] == true
            && c["check"]
                .as_str()
                .unwrap()
                .starts_with("Projects can be saved in")
    }));
    assert!(
        checks
            .iter()
            .all(|c| c["ok"] == true || c["fix"].is_string()),
        "every failure carries a fix"
    );
    let text = glasstarn(home.path(), &["doctor"]);
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.contains("✗  The glasstarn command is not on your PATH")
            && text.contains("Fix: run `npx glasstarn setup`"),
        "{text}"
    );
}

#[test]
fn export_writes_the_map_items_and_variants_for_the_current_filters() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("work").join("glasstarn");
    let root_text = root.to_str().unwrap();
    let made = glasstarn(
        home.path(),
        &[
            "demo",
            "purchase-to-pay",
            "--workspace-dir",
            root_text,
            "--json",
        ],
    );
    assert!(made.status.success());
    let id = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    let out = glasstarn(
        home.path(),
        &["workspace", "export", &id, "--workspace-dir", root_text],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["items"], 1500);
    let files: Vec<std::path::PathBuf> = result["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().into())
        .collect();
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|f| {
        f.parent()
            .unwrap()
            .ends_with(std::path::Path::new(&id).join("exports"))
            && f.is_file()
    }));
    let bpmn = std::fs::read_to_string(&files[0]).unwrap();
    assert!(bpmn.contains("bpmn:definitions") && bpmn.contains("Approve Requisition"));
    let items = std::fs::read_to_string(&files[1]).unwrap();
    assert_eq!(items.lines().count(), 1501, "a header and one row per item");
    let variants = std::fs::read_to_string(&files[2]).unwrap();
    let counted: usize = variants
        .lines()
        .skip(1)
        .map(|line| line.split(',').nth(1).unwrap().parse::<usize>().unwrap())
        .sum();
    assert_eq!(counted, 1500, "every item is on exactly one route");
    assert!(variants.lines().nth(1).unwrap().starts_with("V1,1059,"));
}

#[test]
fn export_works_for_linked_records_by_item_type() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("work").join("glasstarn");
    let root_text = root.to_str().unwrap();
    assert!(
        glasstarn(
            home.path(),
            &["demo", "connected", "--workspace-dir", root_text, "--json"]
        )
        .status
        .success()
    );
    let id = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    // Without a type, the error names the types to choose from.
    let asked = glasstarn(
        home.path(),
        &["workspace", "export", &id, "--workspace-dir", root_text],
    );
    assert!(!asked.status.success());
    let reason = String::from_utf8_lossy(&asked.stderr);
    assert!(
        reason.contains("--type") && reason.contains("Invoice") && reason.contains("Order"),
        "{reason}"
    );
    // "invoices" means the Invoice type.
    let out = glasstarn(
        home.path(),
        &[
            "workspace",
            "export",
            &id,
            "--type",
            "invoices",
            "--workspace-dir",
            root_text,
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["item_type"], "Invoice");
    let files: Vec<std::path::PathBuf> = result["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().into())
        .collect();
    assert!(files.iter().all(|f| {
        f.is_file()
            && f.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("-invoice-")
    }));
    let items = std::fs::read_to_string(&files[1]).unwrap();
    assert_eq!(
        items.lines().count() as u64,
        result["items"].as_u64().unwrap() + 1
    );
    let bpmn = std::fs::read_to_string(&files[0]).unwrap();
    assert!(bpmn.contains("Send Invoice"));
}
