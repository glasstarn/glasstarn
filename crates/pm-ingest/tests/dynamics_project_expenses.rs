//! Literal expense approval history, independent of recipe-derived expectations.
mod common;
use pm_ingest::pack::{BUILT_IN, Exported, Pack, build_recipe_with, check};
use std::collections::BTreeMap;
#[test]
fn project_expenses_preserve_requests_and_allow_creation_only() {
    let dir = std::env::temp_dir().join(format!("pm-project-expenses-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let expense = dir.join("msdyn_expense.csv");
    let approval = dir.join("msdyn_projectapproval.csv");
    std::fs::write(&expense,"msdyn_expenseid,createdon,msdyn_expensestatus,msdyn_transactiondate,modifiedon\nE,2026-10-01T08:00:00Z,Approved,2099-01-01,2099-01-02T00:00:00Z\nF,2026-10-01T09:00:00Z,Draft,2099-01-03,2099-01-04T00:00:00Z\n").unwrap();
    std::fs::write(&approval,"msdyn_projectapprovalid,createdon,msdyn_expenseentry,msdyn_recordstage,msdyn_approvedon\nA,2026-10-01T10:00:00Z,E,Rejected,2026-10-01T11:00:00Z\nB,2026-10-02T12:00:00+01:00,E,2,2026-10-02T12:00:00Z\nC,2026-10-03T12:00:00Z,F,0,2099-01-01T00:00:00Z\n").unwrap();
    let pack = Pack::parse(
        BUILT_IN
            .iter()
            .find(|(id, _)| *id == "d365-project-operations-expenses")
            .unwrap()
            .1,
    )
    .unwrap();
    let files = BTreeMap::from([
        ("msdyn_expense".to_owned(), expense.clone()),
        ("msdyn_projectapproval".to_owned(), approval.clone()),
    ]);
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = files.keys().map(|s| (s.clone(), "0".repeat(64))).collect();
    let built = build_recipe_with(
        &pack,
        &hashes,
        &BTreeMap::new(),
        &Exported::from_report(&report.tables),
    )
    .unwrap();
    let out = common::prepare_both(&built.recipe, &files);
    assert_eq!(out.report.issue_count, 0);
    assert_eq!((out.report.objects, out.report.events), (5, 7));
    let log = serde_json::to_value(out.log.unwrap()).unwrap();
    let mut actual = Vec::new();
    let mut links = 0;
    for e in log["events"].as_array().unwrap() {
        let rel = e["relationships"].as_array().unwrap();
        links += rel.len();
        let x = rel.iter().find(|r| r["qualifier"] == "expense").unwrap();
        let id: serde_json::Value = serde_json::from_str(x["objectId"].as_str().unwrap()).unwrap();
        let time = chrono::DateTime::parse_from_rfc3339(e["time"].as_str().unwrap())
            .unwrap()
            .with_timezone(&chrono::Utc)
            .to_rfc3339();
        actual.push((
            id[2][0].as_str().unwrap().to_owned(),
            e["type"].as_str().unwrap().to_owned(),
            time,
        ));
    }
    let mut expected: Vec<_> = [
        ("E", "Create project expense", "2026-10-01T08:00:00+00:00"),
        ("F", "Create project expense", "2026-10-01T09:00:00+00:00"),
        (
            "E",
            "Create expense approval request",
            "2026-10-01T10:00:00+00:00",
        ),
        (
            "E",
            "Reject expense (latest decision)",
            "2026-10-01T11:00:00+00:00",
        ),
        (
            "E",
            "Create expense approval request",
            "2026-10-02T11:00:00+00:00",
        ),
        (
            "E",
            "Approve expense (latest decision)",
            "2026-10-02T12:00:00+00:00",
        ),
        (
            "F",
            "Create expense approval request",
            "2026-10-03T12:00:00+00:00",
        ),
    ]
    .into_iter()
    .map(|(a, b, c)| (a.to_owned(), b.to_owned(), c.to_owned()))
    .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    assert_eq!(links, 12);
    std::fs::write(
        &approval,
        "msdyn_projectapprovalid,createdon,msdyn_expenseentry\nA,2026-10-01T10:00:00Z,E\n",
    )
    .unwrap();
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready");
    let built = build_recipe_with(
        &pack,
        &hashes,
        &BTreeMap::new(),
        &Exported::from_report(&report.tables),
    )
    .unwrap();
    let out = common::prepare_both(&built.recipe, &files);
    assert_eq!(out.report.issue_count, 0);
    assert_eq!((out.report.objects, out.report.events), (3, 3));
    let minimal = BTreeMap::from([("msdyn_expense".to_owned(), expense)]);
    let report = check(&pack, &minimal).unwrap();
    assert_eq!(report.status, "ready");
    let hashes = BTreeMap::from([("msdyn_expense".to_owned(), "0".repeat(64))]);
    let built = build_recipe_with(
        &pack,
        &hashes,
        &BTreeMap::new(),
        &Exported::from_report(&report.tables),
    )
    .unwrap();
    let out = common::prepare_both(&built.recipe, &minimal);
    assert_eq!(out.report.issue_count, 0);
    assert_eq!((out.report.objects, out.report.events), (2, 2));
    std::fs::remove_dir_all(dir).unwrap();
}
