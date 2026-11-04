//! Literal request histories: repeated workflow instances stay distinct, while
//! planned absence dates and current status never manufacture events.
mod common;
use pm_ingest::pack::{BUILT_IN, Exported, Pack, build_recipe_with, check};
use std::collections::BTreeMap;

#[test]
fn leave_requests_preserve_workflow_history_and_allow_creation_only() {
    let dir = std::env::temp_dir().join(format!("pm-d365-leave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let requests = dir.join("LeaveRequest.csv");
    let history = dir.join("WorkflowTrackingTable.csv");
    std::fs::write(
        &requests,
        "RecId,CreatedDateTime,Status,StartDate,modifiedDateTime\nA,2026-10-01T08:00:00Z,Completed,2099-01-01,2099-01-02T00:00:00Z\nB,2026-10-01T09:00:00Z,Draft,2099-01-03,2099-01-04T00:00:00Z\n",
    ).unwrap();
    std::fs::write(
        &history,
        "RecId,LeaveRequestRecId,TrackingType,CreatedDateTime\n1,A,Submission,2026-10-01T10:00:00Z\n2,A,Cancellation,2026-10-01T11:00:00Z\n3,A,Submission,2026-10-02T12:00:00+01:00\n4,A,Approval,2026-10-02T12:00:00Z\n5,A,WorkItemCreation,2026-10-02T11:30:00Z\n",
    ).unwrap();
    let pack = Pack::parse(
        BUILT_IN
            .iter()
            .find(|(id, _)| *id == "d365-human-resources-leave")
            .unwrap()
            .1,
    )
    .unwrap();
    let files = BTreeMap::from([
        ("LeaveRequest".to_owned(), requests.clone()),
        ("WorkflowTrackingTable".to_owned(), history),
    ]);
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = files
        .keys()
        .map(|name| (name.clone(), "0".repeat(64)))
        .collect();
    let built = build_recipe_with(
        &pack,
        &hashes,
        &BTreeMap::new(),
        &Exported::from_report(&report.tables),
    )
    .unwrap();
    let outcome = common::prepare_both(&built.recipe, &files);
    assert_eq!(outcome.report.issue_count, 0);
    assert_eq!((outcome.report.objects, outcome.report.events), (2, 6));
    let log = serde_json::to_value(outcome.log.unwrap()).unwrap();
    let mut actual = Vec::new();
    for event in log["events"].as_array().unwrap() {
        let links = event["relationships"].as_array().unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0]["qualifier"], "leave request");
        let object: serde_json::Value =
            serde_json::from_str(links[0]["objectId"].as_str().unwrap()).unwrap();
        assert_eq!(object[1], "leave request");
        let time = chrono::DateTime::parse_from_rfc3339(event["time"].as_str().unwrap())
            .unwrap()
            .with_timezone(&chrono::Utc)
            .to_rfc3339();
        actual.push((
            object[2][0].as_str().unwrap().to_owned(),
            event["type"].as_str().unwrap().to_owned(),
            time,
        ));
    }
    let mut expected: Vec<_> = [
        ("A", "Create leave request", "2026-10-01T08:00:00+00:00"),
        ("B", "Create leave request", "2026-10-01T09:00:00+00:00"),
        (
            "A",
            "Leave workflow: Submission",
            "2026-10-01T10:00:00+00:00",
        ),
        (
            "A",
            "Leave workflow: Cancellation",
            "2026-10-01T11:00:00+00:00",
        ),
        (
            "A",
            "Leave workflow: Submission",
            "2026-10-02T11:00:00+00:00",
        ),
        ("A", "Leave workflow: Approval", "2026-10-02T12:00:00+00:00"),
    ]
    .into_iter()
    .map(|(a, b, c)| (a.to_owned(), b.to_owned(), c.to_owned()))
    .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);

    std::fs::write(
        &requests,
        "RecId,CreatedDateTime\nA,2026-10-01T08:00:00Z\nB,2026-10-01T09:00:00Z\n",
    )
    .unwrap();
    let minimal = BTreeMap::from([("LeaveRequest".to_owned(), requests)]);
    let report = check(&pack, &minimal).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = BTreeMap::from([("LeaveRequest".to_owned(), "0".repeat(64))]);
    let built = build_recipe_with(
        &pack,
        &hashes,
        &BTreeMap::new(),
        &Exported::from_report(&report.tables),
    )
    .unwrap();
    let outcome = common::prepare_both(&built.recipe, &minimal);
    assert_eq!(outcome.report.issue_count, 0);
    assert_eq!((outcome.report.objects, outcome.report.events), (2, 2));
    std::fs::remove_dir_all(dir).unwrap();
}
