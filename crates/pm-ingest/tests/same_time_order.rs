//! Steps recorded at the same moment (dates without times, for example)
//! keep the order the preparation wrote them in, which is the pack's process
//! order, not the alphabetical order of their names.
use pm_ingest::prepare_ocel::{Recipe, prepare};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn same_time_steps_keep_the_order_they_were_written_in() {
    let dir = std::env::temp_dir().join(format!("pm-same-time-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("orders.csv"), "ID,DAY\nO1,2026-04-04\n").unwrap();
    // "Receive goods" happens first but sorts after "Pay invoice" by name.
    let recipe: Recipe = serde_json::from_value(json!({"version":1,"format":"ocel",
        "inputs":[{"id":"orders","sha256":"a".repeat(64)}],
        "objects":[{"id":"order","input":"orders","namespace":"t","object_type":"order","key":["ID"]}],
        "events":[
          {"id":"received","input":"orders","namespace":"t","key":["ID"],"activity":{"value":"Receive goods"},
           "timestamp":{"date":"DAY","offset":"+00:00"},"objects":[{"object":"order","columns":["ID"],"qualifier":"order"}]},
          {"id":"paid","input":"orders","namespace":"t","key":["ID"],"activity":{"value":"Pay invoice"},
           "timestamp":{"date":"DAY","offset":"+00:00"},"objects":[{"object":"order","columns":["ID"],"qualifier":"order"}]}]}))
    .unwrap();
    let files = BTreeMap::from([("orders".to_owned(), dir.join("orders.csv"))]);
    let outcome = prepare(&recipe, &files).unwrap();
    let path = dir.join("log.sqlite");
    outcome.log.unwrap().write_sqlite(&path).unwrap();
    let names = |log: &pm_core::OcelLog| {
        log.events
            .iter()
            .map(|e| log.event_types[e.event_type.0 as usize].name.clone())
            .collect::<Vec<_>>()
    };
    let loaded = pm_ingest::read_ocel_sqlite(&path).unwrap();
    assert_eq!(names(&loaded), ["Receive goods", "Pay invoice"]);
    std::fs::remove_dir_all(dir).unwrap();
}
