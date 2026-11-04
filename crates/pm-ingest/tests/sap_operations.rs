//! The SAP production, maintenance and journal-entry packs on made-up tables
//! whose contents are known by construction.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe, check};
use pm_testgen::sap_operations::{
    JOURNAL_TABLES, MAINTENANCE_TABLES, PRODUCTION_TABLES, Settings, write_journal_entries,
    write_maintenance, write_production,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn pack(id: &str) -> Pack {
    Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap()
}
fn answer(choice: &str, value: &str) -> Answer {
    Answer {
        choice: choice.into(),
        value: Some(value.into()),
        confirmed_by: "Operations analyst".into(),
        at: "2026-10-14T00:00:00Z".into(),
        question: String::new(),
        carries: Vec::new(),
    }
}
fn files(dir: &Path, tables: &[&str]) -> BTreeMap<String, PathBuf> {
    tables
        .iter()
        .map(|t| ((*t).to_owned(), dir.join(format!("{t}.csv"))))
        .collect()
}
fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-sap-ops-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Checks the data, answers the clock question, builds both ways and returns
/// the steps each rule made.
fn run(pack: &Pack, files: &BTreeMap<String, PathBuf>) -> BTreeMap<String, u64> {
    run_with_log(pack, files).0
}

/// As [`run`], with the built log as JSON text.
fn run_with_log(pack: &Pack, files: &BTreeMap<String, PathBuf>) -> (BTreeMap<String, u64>, String) {
    let report = check(pack, files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let answers = BTreeMap::from([("setting:offset".to_owned(), answer("value", "+01:00"))]);
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(pack, &hashes, &answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    let log = serde_json::to_string(&outcome.log).unwrap();
    let counts = outcome
        .report
        .rules
        .iter()
        .map(|(rule, counts)| (rule.clone(), counts.emitted))
        .collect();
    (counts, log)
}

#[test]
fn plan_to_produce_yields_every_planted_step() {
    let dir = dir("produce");
    let truth = write_production(&dir, &Settings::default()).unwrap();
    assert!(truth.other_orders > 0 && truth.cancelled_confirmations > 0);
    let pack = pack("sap-plan-to-produce");
    let (emitted, log) = run_with_log(&pack, &files(&dir, &PRODUCTION_TABLES));
    // Each order says whether it came from the planning run or a sales order.
    assert!(truth.from_planning > 0 && truth.made_to_order > 0);
    assert!(log.contains("planned order") && log.contains("0000030000"));
    assert!(log.contains("sales order") && log.contains("0001200"));
    // Maintenance orders in the same table are left out.
    assert_eq!(emitted["production_order_created"], truth.orders);
    assert_eq!(emitted["production_order_released"], truth.released);
    assert_eq!(emitted["components_issued"], truth.component_issues);
    assert_eq!(
        emitted["component_issue_reversed"],
        truth.component_reversals
    );
    assert_eq!(emitted["operation_confirmed"], truth.partial_confirmations);
    assert_eq!(emitted["operation_finished"], truth.final_confirmations);
    assert_eq!(
        emitted["confirmation_cancelled"],
        truth.cancelled_confirmations
    );
    assert_eq!(emitted["finished_goods_received"], truth.goods_receipts);
    assert_eq!(
        emitted["goods_receipt_reversed"],
        truth.goods_receipt_reversals
    );
    assert_eq!(emitted["production_order_completed"], truth.completed);
    assert_eq!(emitted["production_order_closed"], truth.closed);
    assert_eq!(
        emitted["production_order_deletion_flagged"],
        truth.deletion_flags
    );

    // Without the optional goods movements and status changes the order
    // steps still build.
    let mut core = files(&dir, &PRODUCTION_TABLES);
    core.remove("MSEG");
    core.remove("JCDS");
    let emitted = run(&pack, &core);
    assert_eq!(emitted["operation_finished"], truth.final_confirmations);
    assert!(!emitted.contains_key("components_issued"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn maintenance_yields_every_planted_step() {
    let dir = dir("maintain");
    let truth = write_maintenance(&dir, &Settings::default()).unwrap();
    assert!(truth.other_orders > 0 && truth.breakdowns > 0);
    let pack = pack("sap-plant-maintenance");
    let emitted = run(&pack, &files(&dir, &MAINTENANCE_TABLES));
    assert_eq!(emitted["problem_reported"], truth.notifications);
    assert_eq!(emitted["equipment_broke_down"], truth.breakdowns);
    // Production orders in the same table are left out.
    assert_eq!(emitted["maintenance_order_created"], truth.orders);
    assert_eq!(emitted["maintenance_order_released"], truth.released);
    assert_eq!(emitted["spare_parts_issued"], truth.spare_part_issues);
    assert_eq!(emitted["spare_parts_returned"], truth.spare_part_returns);
    assert_eq!(emitted["work_confirmed"], truth.partial_confirmations);
    assert_eq!(emitted["work_finished"], truth.final_confirmations);
    assert_eq!(
        emitted["confirmation_cancelled"],
        truth.cancelled_confirmations
    );
    assert_eq!(emitted["maintenance_order_completed"], truth.completed);
    assert_eq!(emitted["maintenance_order_closed"], truth.closed);
    assert_eq!(
        emitted["notification_in_process"],
        truth.notifications_in_process
    );
    assert_eq!(
        emitted["notification_completed"],
        truth.notifications_completed
    );
    // Orders reach their notification; every order reaches its equipment.
    assert_eq!(
        emitted["order_for_notification"],
        truth.orders_from_notifications
    );
    assert_eq!(emitted["order_on_equipment"], truth.orders);

    // The core tables alone still build the orders.
    let mut core = files(&dir, &MAINTENANCE_TABLES);
    for optional in ["QMIH", "EQUI", "MSEG", "JCDS"] {
        core.remove(optional);
    }
    let emitted = run(&pack, &core);
    assert_eq!(emitted["maintenance_order_created"], truth.orders);
    assert!(!emitted.contains_key("equipment_broke_down"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn record_to_report_yields_every_planted_step() {
    let dir = dir("record");
    let truth = write_journal_entries(&dir, &Settings::default()).unwrap();
    assert!(truth.reversals > 0 && truth.entered_after_period > 0 && truth.still_parked > 0);
    let pack = pack("sap-record-to-report");
    let emitted = run(&pack, &files(&dir, &JOURNAL_TABLES));
    // Reversal documents remain separate objects and also link to the original;
    // entries still parked are not posted yet.
    assert_eq!(emitted["journal_entry_entered"], truth.entries);
    assert_eq!(emitted["journal_entry_parked"], truth.parked);
    assert_eq!(emitted["journal_workflow_completed"], truth.approvals);
    assert_eq!(emitted["journal_entry_reversed"], truth.reversals);
    assert_eq!(emitted["journal_entry_changed"], truth.changes);
    assert_eq!(emitted["item_cleared"], truth.cleared_items);
    let mut core = files(&dir, &JOURNAL_TABLES);
    core.retain(|table, _| table == "BKPF");
    let emitted = run(&pack, &core);
    assert_eq!(emitted["journal_entry_reversed"], truth.reversals);
    assert!(!emitted.contains_key("item_cleared"));
    std::fs::remove_dir_all(dir).unwrap();
}
