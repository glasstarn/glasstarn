//! Source-fact oracle: resubmission creates another request; a rejection also
//! populates approvedon, and a pending row with that field is not approved.
mod common;
use pm_ingest::pack::{BUILT_IN, Exported, Pack, build_recipe_with, check};
use std::collections::BTreeMap;

#[test]
fn project_time_preserves_requests_and_interprets_decisions_with_stage() {
    let dir = std::env::temp_dir().join(format!("pm-project-time-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let projects = dir.join("msdyn_project.csv");
    let entries = dir.join("msdyn_timeentry.csv");
    let approvals = dir.join("msdyn_projectapproval.csv");
    std::fs::write(&projects,"msdyn_projectid,createdon,msdyn_scheduledstart\nP,2026-10-01T08:00:00Z,2099-01-01\nQ,2026-10-01T09:00:00Z,2099-01-02\n").unwrap();
    std::fs::write(&entries,"msdyn_timeentryid,createdon,msdyn_project,msdyn_entrystatus,msdyn_date,modifiedon\nT,2026-10-01T10:00:00Z,P,Approved,2099-01-03,2099-01-04T00:00:00Z\nU,2026-10-01T11:00:00Z,Q,Draft,2099-01-05,2099-01-06T00:00:00Z\n").unwrap();
    std::fs::write(&approvals,"msdyn_projectapprovalid,createdon,msdyn_project,msdyn_recordstage,msdyn_approvedon\nA,2026-10-01T12:00:00Z,P,1,2026-10-01T13:00:00Z\nB,2026-10-02T12:00:00+01:00,P,Approved,2026-10-02T12:00:00Z\nC,2026-10-03T12:00:00Z,Q,0,2099-01-01T00:00:00Z\n").unwrap();
    let pack = Pack::parse(
        BUILT_IN
            .iter()
            .find(|(id, _)| *id == "d365-project-operations-time")
            .unwrap()
            .1,
    )
    .unwrap();
    let files = BTreeMap::from([
        ("msdyn_project".to_owned(), projects.clone()),
        ("msdyn_timeentry".to_owned(), entries.clone()),
        ("msdyn_projectapproval".to_owned(), approvals.clone()),
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
    assert_eq!((out.report.objects, out.report.events), (7, 9));
    let log = serde_json::to_value(out.log.unwrap()).unwrap();
    let mut actual = Vec::new();
    let mut links = 0;
    for e in log["events"].as_array().unwrap() {
        let rel = e["relationships"].as_array().unwrap();
        links += rel.len();
        let project = rel.iter().find(|r| r["qualifier"] == "project").unwrap();
        let id: serde_json::Value =
            serde_json::from_str(project["objectId"].as_str().unwrap()).unwrap();
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
        ("P", "Create project", "2026-10-01T08:00:00+00:00"),
        ("Q", "Create project", "2026-10-01T09:00:00+00:00"),
        ("P", "Create time entry", "2026-10-01T10:00:00+00:00"),
        ("Q", "Create time entry", "2026-10-01T11:00:00+00:00"),
        (
            "P",
            "Create time approval request",
            "2026-10-01T12:00:00+00:00",
        ),
        (
            "P",
            "Reject time entry (latest decision)",
            "2026-10-01T13:00:00+00:00",
        ),
        (
            "P",
            "Create time approval request",
            "2026-10-02T11:00:00+00:00",
        ),
        (
            "P",
            "Approve time entry (latest decision)",
            "2026-10-02T12:00:00+00:00",
        ),
        (
            "Q",
            "Create time approval request",
            "2026-10-03T12:00:00+00:00",
        ),
    ]
    .into_iter()
    .map(|(a, b, c)| (a.to_owned(), b.to_owned(), c.to_owned()))
    .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    assert_eq!(links, 16);

    // Optional decision/context columns may be absent while request creation stays.
    std::fs::write(
        &approvals,
        "msdyn_projectapprovalid,createdon,msdyn_project\nA,2026-10-01T12:00:00Z,P\n",
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
    assert_eq!((out.report.objects, out.report.events), (5, 5));
    let minimal = BTreeMap::from([("msdyn_project".to_owned(), projects)]);
    let report = check(&pack, &minimal).unwrap();
    assert_eq!(report.status, "ready");
    let hashes = BTreeMap::from([("msdyn_project".to_owned(), "0".repeat(64))]);
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
