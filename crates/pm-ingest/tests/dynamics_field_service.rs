//! Independent literal visit histories: repeated statuses stay on their own booking
//! and work order. Current status and planned times are not historical events.
mod common;
use pm_ingest::pack::{BUILT_IN, Exported, Pack, build_recipe, build_recipe_with, check};
use std::collections::BTreeMap;

#[test]
fn field_service_preserves_visit_history_and_allows_creation_only_exports() {
    let dir = std::env::temp_dir().join(format!("pm-d365-visits-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut files = BTreeMap::new();
    for (name, data) in [
        (
            "msdyn_workorder",
            "msdyn_workorderid,createdon,msdyn_name,msdyn_systemstatus,msdyn_timeclosed\nA,2026-10-01T08:00:00Z,WO-A,Posted,2026-10-03T14:00:00Z\nB,2026-10-01T08:30:00Z,WO-B,In Progress,\n",
        ),
        (
            "bookableresourcebooking",
            "bookableresourcebookingid,createdon,msdyn_workorder,resource,starttime,endtime\nA1,2026-10-01T09:00:00Z,A,R1,2099-01-01T00:00:00Z,2099-01-02T00:00:00Z\nA2,2026-10-02T09:00:00Z,A,R2,,\nB1,2026-10-01T09:30:00Z,B,R3,,\n",
        ),
        (
            "msdyn_bookingtimestamp",
            "msdyn_bookingtimestampid,msdyn_booking,msdyn_systemstatus,msdyn_timestamptime,msdyn_timestampsource,createdon\n1,A1,Scheduled,2026-10-01T10:00:00Z,Desktop,2026-10-10T00:00:00Z\n2,A1,Traveling,2026-10-01T11:00:00Z,Desktop,2026-10-10T00:00:00Z\n3,A1,In Progress,2026-10-01T12:00:00Z,Mobile,2026-10-10T00:00:00Z\n4,A1,Completed,2026-10-01T13:00:00Z,Mobile,2026-10-10T00:00:00Z\n5,A1,In Progress,2026-10-01T14:00:00Z,Mobile,2026-10-10T00:00:00Z\n6,A1,Completed,2026-10-01T15:00:00Z,Mobile,2026-10-10T00:00:00Z\n7,A2,Scheduled,2026-10-02T10:00:00Z,Desktop,2026-10-10T00:00:00Z\n8,A2,Canceled,2026-10-02T11:00:00Z,Desktop,2026-10-10T00:00:00Z\n9,B1,Scheduled,2026-10-01T10:30:00Z,Desktop,2026-10-10T00:00:00Z\n10,B1,Traveling,2026-10-01T11:30:00Z,Mobile,2026-10-10T00:00:00Z\n11,B1,In Progress,2026-10-01T12:30:00+01:00,Mobile,2026-10-10T00:00:00Z\n",
        ),
    ] {
        let path = dir.join(format!("{name}.csv"));
        std::fs::write(&path, data).unwrap();
        files.insert(name.to_owned(), path);
    }
    let pack = Pack::parse(
        BUILT_IN
            .iter()
            .find(|(id, _)| *id == "d365-field-service")
            .unwrap()
            .1,
    )
    .unwrap();
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = files
        .keys()
        .map(|name| (name.clone(), "0".repeat(64)))
        .collect();
    let built = build_recipe(&pack, &hashes, &BTreeMap::new()).unwrap();
    let outcome = common::prepare_both(&built.recipe, &files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    assert_eq!(outcome.report.objects, 5);
    assert_eq!(outcome.report.events, 17);
    let log = serde_json::to_value(outcome.log.unwrap()).unwrap();
    let mut actual = Vec::new();
    let mut links = 0;
    for event in log["events"].as_array().unwrap() {
        let relationships = event["relationships"].as_array().unwrap();
        links += relationships.len();
        let mut order = None;
        let mut booking = None;
        for link in relationships {
            let obj: serde_json::Value =
                serde_json::from_str(link["objectId"].as_str().unwrap()).unwrap();
            if obj[1] == "work order" {
                order = Some(obj[2][0].as_str().unwrap().to_owned());
            }
            if obj[1] == "booking" {
                booking = Some(obj[2][0].as_str().unwrap().to_owned());
            }
        }
        let time = chrono::DateTime::parse_from_rfc3339(event["time"].as_str().unwrap())
            .unwrap()
            .with_timezone(&chrono::Utc)
            .to_rfc3339();
        actual.push((
            order.unwrap(),
            booking.unwrap_or_default(),
            event["type"].as_str().unwrap().to_owned(),
            time,
        ));
    }
    assert_eq!(links, 31);
    let expected = [
        ("A", "", "Create work order", "2026-10-01T08:00:00+00:00"),
        ("B", "", "Create work order", "2026-10-01T08:30:00+00:00"),
        (
            "A",
            "",
            "Close work order (latest)",
            "2026-10-03T14:00:00+00:00",
        ),
        ("A", "A1", "Create booking", "2026-10-01T09:00:00+00:00"),
        ("A", "A2", "Create booking", "2026-10-02T09:00:00+00:00"),
        ("B", "B1", "Create booking", "2026-10-01T09:30:00+00:00"),
        (
            "A",
            "A1",
            "Booking status: Scheduled",
            "2026-10-01T10:00:00+00:00",
        ),
        (
            "A",
            "A1",
            "Booking status: Traveling",
            "2026-10-01T11:00:00+00:00",
        ),
        (
            "A",
            "A1",
            "Booking status: In Progress",
            "2026-10-01T12:00:00+00:00",
        ),
        (
            "A",
            "A1",
            "Booking status: Completed",
            "2026-10-01T13:00:00+00:00",
        ),
        (
            "A",
            "A1",
            "Booking status: In Progress",
            "2026-10-01T14:00:00+00:00",
        ),
        (
            "A",
            "A1",
            "Booking status: Completed",
            "2026-10-01T15:00:00+00:00",
        ),
        (
            "A",
            "A2",
            "Booking status: Scheduled",
            "2026-10-02T10:00:00+00:00",
        ),
        (
            "A",
            "A2",
            "Booking status: Canceled",
            "2026-10-02T11:00:00+00:00",
        ),
        (
            "B",
            "B1",
            "Booking status: Scheduled",
            "2026-10-01T10:30:00+00:00",
        ),
        (
            "B",
            "B1",
            "Booking status: Traveling",
            "2026-10-01T11:30:00+00:00",
        ),
        (
            "B",
            "B1",
            "Booking status: In Progress",
            "2026-10-01T11:30:00+00:00",
        ),
    ];
    let mut expected: Vec<_> = expected
        .into_iter()
        .map(|(a, b, c, d)| (a.to_owned(), b.to_owned(), c.to_owned(), d.to_owned()))
        .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);

    // Only case ID and creation time: missing context and later steps cannot stop a build.
    let minimal = dir.join("minimal.csv");
    std::fs::write(
        &minimal,
        "msdyn_workorderid,createdon\nA,2026-10-01T08:00:00Z\nB,2026-10-01T08:30:00Z\n",
    )
    .unwrap();
    let only = BTreeMap::from([("msdyn_workorder".to_owned(), minimal)]);
    let report = check(&pack, &only).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = BTreeMap::from([("msdyn_workorder".to_owned(), "0".repeat(64))]);
    let built = build_recipe_with(
        &pack,
        &hashes,
        &BTreeMap::new(),
        &Exported::from_report(&report.tables),
    )
    .unwrap();
    let outcome = common::prepare_both(&built.recipe, &only);
    assert_eq!(outcome.report.issue_count, 0);
    assert_eq!(outcome.report.objects, 2);
    assert_eq!(outcome.report.events, 2);
    std::fs::remove_dir_all(dir).unwrap();
}
