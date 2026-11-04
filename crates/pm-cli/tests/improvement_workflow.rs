//! The measured facts behind an improvement conversation: confirmed rules and
//! system accounts shape what the engine reports, while suggestions about
//! where a step belongs stay in the chat and are refused here.
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

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
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Forty invoices: received by a batch job, approved by a person, paid by a
/// batch job; every tenth is paid before it is approved.
fn invoices(dir: &Path) -> std::path::PathBuf {
    let mut csv = String::from("case_id,activity,timestamp,resource,amount\n");
    for n in 0..40 {
        let day = 1 + n % 25;
        let ((second, by2), (third, by3)) = if n % 10 == 0 {
            (("Pay invoice", "batch_02"), ("Approve invoice", "anna"))
        } else {
            (("Approve invoice", "anna"), ("Pay invoice", "batch_02"))
        };
        csv.push_str(&format!(
            "INV-{n},Receive invoice,2026-03-{day:02} 09:00:00,batch_01,100\n\
             INV-{n},{second},2026-03-{day:02} 10:00:00,{by2},100\n\
             INV-{n},{third},2026-03-{day:02} 11:00:00,{by3},100\n"
        ));
    }
    let path = dir.join("invoices.csv");
    std::fs::write(&path, csv).unwrap();
    path
}

#[test]
fn decisions_shape_the_measured_facts_and_suggestions_stay_in_chat() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source = invoices(root);
    let created = success(cli(root, &["workspace", "create", "--name", "Payables"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let folder = std::path::PathBuf::from(created["project"]["path"].as_str().unwrap());
    success(cli(
        root,
        &[
            "workspace",
            "import",
            &id,
            source.to_str().unwrap(),
            "--case",
            "case_id",
            "--activity",
            "activity",
            "--timestamp",
            "timestamp",
            "--resource",
            "resource",
            "--case-attribute",
            "amount",
        ],
    ));
    let decide = |args: &[&str]| {
        let mut all = vec!["workspace", "decide", id.as_str()];
        all.extend_from_slice(args);
        success(cli(root, &all))
    };
    decide(&[
        "performer",
        "--name",
        "batch_01",
        "--kind",
        "system",
        "--status",
        "confirmed",
        "--by",
        "AP lead",
    ]);
    decide(&[
        "performer",
        "--name",
        "batch_02",
        "--kind",
        "system",
        "--status",
        "suggested",
        "--by",
        "agent",
    ]);
    decide(&[
        "rule",
        "--template",
        "precedence",
        "--a",
        "Approve invoice",
        "--b",
        "Pay invoice",
        "--status",
        "confirmed",
        "--by",
        "AP lead",
        "--reason",
        "company policy",
    ]);
    // Where a step belongs, and what it costs, are suggestions for the chat.
    for kind in ["bucket", "effort"] {
        let refused = cli(
            root,
            &[
                "workspace",
                "decide",
                &id,
                kind,
                "--step",
                "Approve invoice",
                "--status",
                "suggested",
                "--by",
                "agent",
            ],
        );
        assert!(!refused.status.success());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("conversation"));
    }
    let saved = decide(&["value-field", "--field", "amount"]);
    assert_eq!(saved["revision"], 4);

    let request = root.join("q.json");
    std::fs::write(&request, r#"{"type":"improvement"}"#).unwrap();
    let report = success(cli(
        root,
        &[
            "workspace",
            "query",
            &id,
            "--request",
            request.to_str().unwrap(),
            "--full",
        ],
    ));
    let result = &report["response"]["result"];
    assert_eq!(result["revision"], 4);
    let rule = &result["rules"][0];
    assert_eq!(rule["status"], "confirmed");
    assert_eq!(
        (rule["applies_to"].as_u64(), rule["broken"].as_u64()),
        (Some(40), Some(4))
    );
    assert_eq!(rule["broken_value"], 400.0);
    let steps = result["steps"].as_array().unwrap();
    let approve = steps
        .iter()
        .find(|s| s["activity"] == "Approve invoice")
        .unwrap();
    assert!(
        approve["bucket"]["status"] != "suggested",
        "no saved suggestion"
    );
    let receive = steps
        .iter()
        .find(|s| s["activity"] == "Receive invoice")
        .unwrap();
    assert_eq!(receive["by_systems"], 40);
    // A suggested performer is not yet a decision.
    let pay = steps
        .iter()
        .find(|s| s["activity"] == "Pay invoice")
        .unwrap();
    assert_eq!(pay["not_classified"], 40);
    assert_eq!(result["performers"]["look_like_systems"][0], "batch_02");
    // Every decision is in the audit log.
    let project = created["project"]["path"].as_str().unwrap();
    let log = std::fs::read_to_string(Path::new(project).join("audit/log.jsonl")).unwrap();
    assert_eq!(log.matches("decide_").count(), 4);

    // Bad input is refused with the reason.
    let refused = cli(
        root,
        &[
            "workspace",
            "decide",
            &id,
            "rule",
            "--template",
            "response",
            "--a",
            "X",
            "--status",
            "confirmed",
            "--by",
            "AP lead",
        ],
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("give --a and --b"));

    // A person confirms in chat while the project is open in the app, which
    // holds the project's lock.
    let open = pm_project::workspace::WorkspaceFolder::open(&folder).unwrap();
    let saved = decide(&[
        "performer",
        "--name",
        "batch_02",
        "--kind",
        "system",
        "--status",
        "confirmed",
        "--by",
        "AP lead",
    ]);
    assert_eq!(saved["revision"], 5);
    drop(open);
}
