//! An opportunity created directly has no lead qualification to report.
mod common;
use pm_ingest::pack::{BUILT_IN, Pack, build_recipe, check};
use std::collections::BTreeMap;

#[test]
fn direct_opportunity_creation_does_not_invent_lead_qualification() {
    let dir = std::env::temp_dir().join(format!("pm-d365-direct-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("opportunity.csv");
    std::fs::write(
        &source,
        "opportunityid,createdon,originatingleadid,statecode,actualclosedate,estimatedvalue,actualvalue,statuscode,stepname\n\
         direct-opportunity,2026-10-16T12:00:00Z,,Open,,1000,,In Progress,\n",
    )
    .unwrap();
    let pack = Pack::parse(
        BUILT_IN
            .iter()
            .find(|(id, _)| *id == "d365-sales")
            .unwrap()
            .1,
    )
    .unwrap();
    let files = BTreeMap::from([("opportunity".into(), source)]);
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = BTreeMap::from([("opportunity".into(), "0".repeat(64))]);
    let built = build_recipe(&pack, &hashes, &BTreeMap::new()).unwrap();
    let outcome = common::prepare_both(&built.recipe, &files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    let log = serde_json::to_value(outcome.log.unwrap()).unwrap();
    let events = log["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "Create opportunity");
    assert_eq!(events[0]["time"], "2026-10-16T12:00:00+00:00");
    std::fs::remove_dir_all(dir).unwrap();
}
