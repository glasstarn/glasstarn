//! Small things in real exports that must not stop a build: Excel's
//! byte-order mark, a time without its zone where the source is UTC, and a
//! column name repeated where nothing reads it.
use pm_ingest::prepare_ocel::{Recipe, prepare};
use serde_json::json;
use std::collections::BTreeMap;

fn run(csv: &[u8]) -> Result<pm_ingest::prepare_ocel::Outcome, String> {
    let dir = std::env::temp_dir().join(format!("pm-quirks-{}-{}", std::process::id(), csv.len()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steps.csv"), csv).unwrap();
    let recipe: Recipe = serde_json::from_value(json!({"version":1,"format":"ocel",
        "inputs":[{"id":"steps","sha256":"a".repeat(64)}],
        "objects":[{"id":"case","input":"steps","namespace":"t","object_type":"case","key":["ID"],"existing_ok":true}],
        "events":[{"id":"step","input":"steps","namespace":"t","key":["ID","STEP"],"activity":{"column":"STEP"},
          "timestamp":{"date":"AT","offset":"+00:00"},
          "objects":[{"object":"case","columns":["ID"],"qualifier":"case"}]}]}))
    .unwrap();
    let files = BTreeMap::from([("steps".to_owned(), dir.join("steps.csv"))]);
    let out = prepare(&recipe, &files).map_err(|e| e.to_string());
    std::fs::remove_dir_all(dir).unwrap();
    out
}

#[test]
fn small_export_quirks_do_not_stop_a_build() {
    // Excel's "CSV UTF-8" starts with a byte-order mark.
    let built = run(b"\xef\xbb\xbfID,STEP,AT\n1,Open,2026-01-05T14:30:00Z\n").unwrap();
    assert_eq!(built.report.issue_count, 0, "{:?}", built.report.issues);
    // A UTC time written without its zone, and a date alone.
    let built = run(b"ID,STEP,AT\n1,Open,2026-01-05 14:30:00\n1,Close,2026-01-06\n").unwrap();
    assert_eq!(built.report.issue_count, 0, "{:?}", built.report.issues);
    let log = serde_json::to_value(built.log.as_ref().unwrap()).unwrap();
    assert_eq!(log["events"][0]["time"], "2026-01-05T14:30:00+00:00");
    // A repeated caption nobody reads is fine; a repeated column the build
    // reads is ambiguous and named.
    assert!(run(b"ID,STEP,AT,Note,Note\n1,Open,2026-01-05T14:30:00Z,a,b\n").is_ok());
    let error = run(b"ID,STEP,AT,STEP\n1,Open,2026-01-05T14:30:00Z,x\n")
        .err()
        .unwrap();
    assert!(error.contains("'STEP' appears more than once"), "{error}");
}
