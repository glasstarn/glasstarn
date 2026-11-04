//! Records that do not fit are set aside, with the person's approval, rather
//! than blocking the whole build: each is left out, anything that depended
//! on it follows, and every one is listed.
mod common;
use pm_ingest::prepare_ocel::Recipe;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

#[test]
fn records_that_do_not_fit_are_set_aside_and_listed() {
    let dir = std::env::temp_dir().join(format!("pm-set-aside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Order O2 is listed twice.
    std::fs::write(dir.join("orders.csv"), "ORDER\nO1\nO2\nO2\n").unwrap();
    // s3 names an order that is not in the export, s4 has no time and s5 no
    // step name.
    std::fs::write(
        dir.join("steps.csv"),
        "ID,ORDER,AT,STEP\n\
         s1,O1,2026-01-05T09:00:00Z,Order\n\
         s2,O2,2026-01-06T09:00:00Z,Order\n\
         s3,O9,2026-01-07T09:00:00Z,Order\n\
         s4,O1,,Ship\n\
         s5,O1,2026-01-08T09:00:00Z,\n",
    )
    .unwrap();
    // Extra links from steps to orders: the second names s3, which is set
    // aside, so it can only be found once s3 has gone.
    std::fs::write(dir.join("links.csv"), "ID,ORDER\ns1,O2\ns3,O1\n").unwrap();
    let mut value = json!({"version":1,"format":"ocel",
        "inputs":[{"id":"orders","sha256":"a".repeat(64)},{"id":"steps","sha256":"b".repeat(64)},{"id":"links","sha256":"c".repeat(64)}],
        "objects":[{"id":"order","input":"orders","namespace":"t","object_type":"order","key":["ORDER"]}],
        "events":[{"id":"step","input":"steps","namespace":"t","key":["ID"],"activity":{"column":"STEP"},
          "timestamp":"AT","objects":[{"object":"order","columns":["ORDER"],"qualifier":"order"}]}],
        "event_links":[{"id":"extra","input":"links","event":"step","event_key":["ID"],
          "target":{"object":"order","columns":["ORDER"],"qualifier":"also"}}]});
    let files: BTreeMap<String, PathBuf> = ["orders", "steps", "links"]
        .into_iter()
        .map(|t| (t.into(), dir.join(format!("{t}.csv"))))
        .collect();
    let run = |value: &serde_json::Value| {
        let recipe: Recipe = serde_json::from_value(value.clone()).unwrap();
        common::prepare_both(&recipe, &files)
    };

    // Without approval nothing is published.
    let strict = run(&value);
    assert!(strict.log.is_none());
    assert!(strict.report.issue_count >= 4);
    assert_eq!(strict.report.set_aside_count, 0);

    value["set_aside"] = json!(true);
    let outcome = run(&value);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    let log = serde_json::to_value(outcome.log.as_ref().unwrap()).unwrap();
    let mut ids = log["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(ids.len(), 2);
    assert!(ids[0].contains("s1") && ids[1].contains("s2"));
    assert_eq!(log["objects"].as_array().unwrap().len(), 2);

    // Every row set aside is listed with its rule, row and reason; the link
    // that named s3 follows it.
    let found = outcome
        .set_aside
        .iter()
        .map(|i| (i.rule.as_str(), i.row, i.code.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        found,
        [
            ("order", 3, "duplicate_object_key"),
            ("step", 3, "unmatched_object_reference"),
            ("step", 4, "invalid_or_missing_timestamp"),
            ("step", 5, "blank_or_invalid_activity"),
            ("extra", 2, "unmatched_or_blank_event_reference"),
        ]
    );
    assert_eq!(outcome.report.set_aside_count, 5);
    assert_eq!(
        outcome.report.set_aside_by_reason["step: unmatched_object_reference"],
        1
    );
    assert_eq!(outcome.report.rules["order"].set_aside, 1);
    assert_eq!(outcome.report.rules["step"].set_aside, 3);
    assert_eq!(outcome.report.rules["extra"].set_aside, 1);
    // The kept link from s1 to O2 is still made.
    assert_eq!(outcome.report.rules["extra"].emitted, 1);

    // A clean export sets nothing aside.
    std::fs::write(dir.join("orders.csv"), "ORDER\nO1\nO2\nO9\n").unwrap();
    std::fs::write(
        dir.join("steps.csv"),
        "ID,ORDER,AT,STEP\ns1,O1,2026-01-05T09:00:00Z,Order\ns3,O9,2026-01-07T09:00:00Z,Order\n",
    )
    .unwrap();
    let clean = run(&value);
    assert!(clean.log.is_some());
    assert!(clean.set_aside.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}
