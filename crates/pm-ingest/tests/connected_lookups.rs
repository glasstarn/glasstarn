//! Literal independent oracle for filtered, ordered many-to-one joins.
mod common;
use pm_ingest::prepare_ocel::Recipe;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

#[test]
fn lookup_indexes_are_independent_and_later_joins_can_use_earlier_values() {
    let dir = std::env::temp_dir().join(format!("pm-ordered-lookups-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("facts.csv"), "ID,K\n0001,x\n").unwrap();
    std::fs::write(dir.join("dates.csv"), "K,JOIN,D\nx,x,20261015\n").unwrap();
    std::fs::write(dir.join("times.csv"), "K,T,USE\nx,12:34:56,X\nx,invalid,\n").unwrap();
    let mut value = json!({"version":1,"format":"ocel",
        "inputs":[{"id":"facts","sha256":"a".repeat(64)},{"id":"dates","sha256":"b".repeat(64)},{"id":"times","sha256":"c".repeat(64)}],
        "objects":[{"id":"item","input":"facts","namespace":"test","object_type":"item","key":["ID"]}],
        "events":[{"id":"seen","input":"facts","namespace":"test","key":["ID"],"activity":{"value":"Observed"},
          "timestamp":{"date":"D","time":"T","offset":"+00:00"},"objects":[{"object":"item","columns":["ID"],"qualifier":"item"}],
          "lookup":{"input":"dates","key":["K"],"matching":["K"],"columns":["JOIN","D"]},
          "lookups":[{"input":"times","key":["K"],"matching":["JOIN"],"columns":["T"],"where":[{"column":"USE","in":["X"]}]}]}]});
    let files: BTreeMap<String, PathBuf> = ["facts", "dates", "times"]
        .into_iter()
        .map(|t| (t.into(), dir.join(format!("{t}.csv"))))
        .collect();
    let run = |value: &serde_json::Value| {
        let recipe: Recipe = serde_json::from_value(value.clone()).unwrap();
        common::prepare_both(&recipe, &files)
    };
    let outcome = run(&value);
    assert_eq!(outcome.report.issue_count, 0);
    let log = serde_json::to_value(outcome.log.as_ref().unwrap()).unwrap();
    assert_eq!(log["events"].as_array().unwrap().len(), 1);
    assert_eq!(log["events"][0]["time"], "2026-10-15T12:34:56+00:00");
    // Same-key source rows cannot overwrite each other silently.
    std::fs::write(
        dir.join("times.csv"),
        "K,T,USE\nx,12:34:56,X\nx,13:00:00,X\n",
    )
    .unwrap();
    let duplicate = run(&value);
    assert!(duplicate.log.is_none());
    assert!(
        duplicate
            .report
            .issues
            .iter()
            .any(|i| i.code == "duplicate_lookup_key")
    );
    std::fs::write(dir.join("times.csv"), "K,T,USE\nx,12:34:56,\n").unwrap();
    assert!(run(&value).log.is_none());
    value["events"][0]["lookups"][0]["optional"] = json!(true);
    let optional = run(&value);
    // Skipping the only action still fails the existing empty-log guard.
    assert_eq!(optional.report.issue_count, 1);
    assert_eq!(optional.report.issues[0].code, "empty_objects_or_events");
    assert_eq!(optional.report.rules["seen"].skipped, 1);
    assert_eq!(optional.report.events, 0);
    std::fs::remove_dir_all(dir).unwrap();
}
