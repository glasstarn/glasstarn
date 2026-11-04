//! A first read names what a consultant would: the longest single wait, a
//! hand-off slow only for one group, and a usual step skipped by items that
//! share one person, all in the headline an agent reads first.
use std::{fmt::Write, process::Command};

#[test]
fn the_headline_names_slow_groups_long_waits_and_skipped_approvals() {
    let dir = tempfile::tempdir().unwrap();
    let mut csv = String::from("Claim ID,Activity,Timestamp,Region,Adjuster\n");
    let day = 86_400i64;
    for n in 0..300i64 {
        let region = ["North", "South", "East", "West"][(n % 4) as usize];
        let adjuster = if n % 30 == 7 {
            "Pat"
        } else {
            ["Ann", "Bo", "Cy"][(n % 3) as usize]
        };
        let mut at = 1_767_225_600 + n * 3_600;
        let mut step = |name: &str, after: i64, csv: &mut String| {
            at += after;
            let time = chrono::DateTime::from_timestamp(at, 0)
                .unwrap()
                .format("%Y-%m-%dT%H:%M:%SZ");
            writeln!(csv, "C{n},{name},{time},{region},{adjuster}").unwrap();
        };
        step("Submitted", 0, &mut csv);
        // South starts work late.
        step("Assigned", day / 4, &mut csv);
        step(
            "Assessed",
            if region == "South" { 6 * day } else { 2 * day },
            &mut csv,
        );
        // A rarer, very long wait for documents.
        if n % 5 == 0 {
            step("Documents requested", day / 2, &mut csv);
            step("Documents received", 9 * day, &mut csv);
        }
        // Pat's claims are paid without approval.
        if adjuster != "Pat" {
            step("Payment approved", day, &mut csv);
        }
        step("Paid", day / 2, &mut csv);
        step("Closed", day / 4, &mut csv);
    }
    let file = dir.path().join("claims.csv");
    std::fs::write(&file, csv).unwrap();
    let root = dir.path().join("projects");
    let glasstarn = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(args)
            .arg("--workspace-dir")
            .arg(&root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let created = glasstarn(&["workspace", "create", "--name", "Claims"]);
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let imported = glasstarn(&["workspace", "import", &id, file.to_str().unwrap()]);
    let titles = imported["headline"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            format!(
                "{} | {}",
                f["title"].as_str().unwrap(),
                f["detail"].as_str().unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        titles.contains("Documents requested → Documents received is the longest wait"),
        "{titles}"
    );
    assert!(
        titles.contains("Assigned → Assessed takes 6 days for cases with Region = South"),
        "{titles}"
    );
    assert!(
        titles.contains("10 cases reached Paid without Payment approved"),
        "{titles}"
    );
    assert!(titles.contains("All 10 have Adjuster = Pat"), "{titles}");
    // A closing step reached by every route is not reported as skipping anything.
    assert!(!titles.contains("reached Closed without"), "{titles}");
    // Headline filters keep only the conditions they set.
    let first = &imported["headline"]["findings"][0]["filter"];
    assert!(
        first
            .as_object()
            .is_none_or(|f| f.values().all(|v| !v.is_null())),
        "{first}"
    );
}

#[test]
fn an_sop_written_as_steps_is_checked_and_deviations_are_counted_by_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut csv = String::from("Claim ID,Activity,Timestamp\n");
    for n in 0..40 {
        let mut steps = vec![
            "Submitted",
            "Assessed",
            "Payment approved",
            "Paid",
            "Closed",
        ];
        if n % 10 == 0 {
            steps.retain(|s| *s != "Payment approved");
        }
        for (i, step) in steps.iter().enumerate() {
            writeln!(csv, "C{n},{step},2026-01-{:02}T09:00:00Z", 1 + i).unwrap();
        }
    }
    let file = dir.path().join("claims.csv");
    std::fs::write(&file, csv).unwrap();
    let model = dir.path().join("sop.json");
    std::fs::write(
        &model,
        r#"{"source":"SOP","tree":{"kind":"sequence","children":[
        {"kind":"activity","activity":"Submitted"},{"kind":"activity","activity":"Assessed"},
        {"kind":"activity","activity":"Payment approved"},{"kind":"activity","activity":"Paid"},
        {"kind":"activity","activity":"Closed"}]}}"#,
    )
    .unwrap();
    let root = dir.path().join("projects");
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(args)
            .arg("--workspace-dir")
            .arg(&root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let id = run(&["workspace", "create", "--name", "SOP check"])["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    run(&["workspace", "import", &id, file.to_str().unwrap()]);
    let request = dir.path().join("check.json");
    std::fs::write(
        &request,
        format!(
            r#"{{"type":"conformance","model":"{}","algorithm":"token","limit":5}}"#,
            model.display()
        ),
    )
    .unwrap();
    let response = run(&[
        "workspace",
        "query",
        &id,
        "--request",
        request.to_str().unwrap(),
    ]);
    let result = &response["response"]["result"];
    assert_eq!(result["deviating_cases"], 4, "{result}");
    let paid = result["by_step"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["activity"] == "Paid")
        .unwrap();
    assert_eq!(paid["cases"], 4);
    assert_eq!(paid["kind"], "out_of_order");
}
