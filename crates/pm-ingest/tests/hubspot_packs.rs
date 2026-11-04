//! The HubSpot packs on made-up extracts whose contents are known by
//! construction.
mod common;
use pm_ingest::pack::{BUILT_IN, Pack, build_recipe, check};
use pm_testgen::hubspot::{CONTACTS, DEALS, Kind, QUOTES, Settings, TICKETS, write};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn run(
    id: &str,
    kind: Kind,
) -> (
    pm_testgen::hubspot::Truth,
    BTreeMap<String, u64>,
    Vec<String>,
) {
    let dir = std::env::temp_dir().join(format!("pm-hs-{id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write(&dir, &Settings::default(), kind).unwrap();
    let pack = Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap();
    let files: BTreeMap<String, PathBuf> = [kind.file, "HubSpotHistory"]
        .iter()
        .map(|t| ((*t).to_owned(), dir.join(format!("{t}.csv"))))
        .collect();
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(&pack, &hashes, &BTreeMap::new()).unwrap();
    let outcome = common::prepare_both(&built.recipe, &files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    let emitted = outcome
        .report
        .rules
        .iter()
        .map(|(rule, counts)| (rule.clone(), counts.emitted))
        .collect();
    let log = serde_json::to_value(outcome.log.as_ref().unwrap()).unwrap();
    let names = log["eventTypes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    std::fs::remove_dir_all(dir).unwrap();
    (truth, emitted, names)
}

#[test]
fn lead_lifecycle_yields_every_planted_step_and_leaves_out_first_values() {
    let (truth, emitted, names) = run("hubspot-lead-lifecycle", CONTACTS);
    assert_eq!(emitted["contact_created"], truth.records);
    assert_eq!(emitted["lifecycle_changed"], truth.stage_changes);
    assert_eq!(emitted["lead_status_changed"], truth.other_changes);
    assert_eq!(emitted["owner_changed"], truth.owner_changes);
    for step in [
        "Create contact",
        "Lifecycle: Marketing Qualified Lead",
        "Lifecycle: Customer",
        "Lead status: IN_PROGRESS",
        "Reassign contact",
    ] {
        assert!(
            names.contains(&step.to_owned()),
            "{step} missing from {names:?}"
        );
    }
    // The first value of each property is the creation, not a step.
    assert!(!names.contains(&"Lifecycle: Subscriber".to_owned()));
    assert!(truth.sent_back > 0);
}

#[test]
fn deal_pipeline_yields_every_planted_step() {
    let (truth, emitted, names) = run("hubspot-deal-pipeline", DEALS);
    assert_eq!(emitted["deal_created"], truth.records);
    assert_eq!(emitted["stage_changed"], truth.stage_changes);
    assert_eq!(emitted["amount_changed"], truth.other_changes);
    assert_eq!(emitted["owner_changed"], truth.owner_changes);
    assert_eq!(emitted["close_date_moved"], 0);
    for step in [
        "Create deal",
        "Stage: Qualified to buy",
        "Stage: Closed won",
        "Stage: Closed lost",
        "Change amount",
    ] {
        assert!(
            names.contains(&step.to_owned()),
            "{step} missing from {names:?}"
        );
    }
}

#[test]
fn ticket_pipeline_yields_every_planted_step() {
    let (truth, emitted, names) = run("hubspot-ticket-pipeline", TICKETS);
    assert_eq!(emitted["ticket_created"], truth.records);
    assert_eq!(emitted["status_changed"], truth.stage_changes);
    assert_eq!(emitted["priority_changed"], truth.other_changes);
    assert_eq!(emitted["owner_changed"], truth.owner_changes);
    for step in [
        "Create ticket",
        "Status: Waiting on contact",
        "Status: Closed",
        "Change priority",
    ] {
        assert!(
            names.contains(&step.to_owned()),
            "{step} missing from {names:?}"
        );
    }
}

#[test]
fn quote_approval_yields_every_planted_step() {
    let (truth, emitted, names) = run("hubspot-quote-approval", QUOTES);
    assert_eq!(emitted["quote_created"], truth.records);
    assert_eq!(emitted["status_changed"], truth.stage_changes);
    assert_eq!(emitted["expiry_moved"], truth.other_changes);
    for step in [
        "Create quote",
        "Quote: Pending approval",
        "Quote: Approved",
        "Quote: Rejected",
        "Move expiry date",
    ] {
        assert!(
            names.contains(&step.to_owned()),
            "{step} missing from {names:?}"
        );
    }
}
