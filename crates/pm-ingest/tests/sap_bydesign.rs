//! The SAP Business ByDesign buying and selling packs on made-up extracts
//! whose contents are known by construction.
mod common;
use pm_ingest::pack::{BUILT_IN, Pack, build_recipe, check};
use pm_testgen::sap_bydesign::{Flow, Settings, write};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn pack(id: &str) -> Pack {
    Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap()
}
fn files(dir: &Path, names: &[String]) -> BTreeMap<String, PathBuf> {
    names
        .iter()
        .map(|t| (t.clone(), dir.join(format!("{t}.csv"))))
        .collect()
}
fn run(pack: &Pack, files: &BTreeMap<String, PathBuf>) -> BTreeMap<String, u64> {
    let report = check(pack, files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(pack, &hashes, &BTreeMap::new()).unwrap();
    let outcome = common::prepare_both(&built.recipe, files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    outcome
        .report
        .rules
        .iter()
        .map(|(rule, counts)| (rule.clone(), counts.emitted))
        .collect()
}

fn every_planted_step(flow: Flow, id: &str, p: &str) {
    let dir = std::env::temp_dir().join(format!("pm-byd-{id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write(&dir, flow, &Settings::default()).unwrap();
    let names = flow.files();
    let emitted = run(&pack(id), &files(&dir, &names));
    let e = |rule: &str| emitted[&format!("{p}{rule}")];
    assert_eq!(e("request_created"), truth.requests);
    assert_eq!(e("request_created_lines"), truth.lines_from_requests);
    assert_eq!(e("order_created"), truth.orders);
    assert_eq!(e("order_created_lines"), truth.order_lines);
    assert_eq!(e("goods_moved"), truth.receipt_lines);
    assert_eq!(e("invoice_created"), truth.invoices);
    assert_eq!(e("invoice_created_lines"), truth.invoice_lines);
    assert_eq!(e("invoice_paid"), truth.allocations);
    assert_eq!(e("invoice_paid_lines"), truth.paid_order_lines);

    let mut core = files(&dir, &names);
    for optional in [0, 1, 8, 9] {
        core.remove(&names[optional]);
    }
    let emitted = run(&pack(id), &core);
    assert_eq!(emitted[&format!("{p}invoice_created")], truth.invoices);
    assert!(!emitted.contains_key(&format!("{p}invoice_paid")));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bydesign_buying_yields_every_planted_step() {
    every_planted_step(Flow::Buying, "sap-bydesign-purchase-to-pay", "byd_p_");
}

#[test]
fn bydesign_selling_yields_every_planted_step() {
    every_planted_step(Flow::Selling, "sap-bydesign-order-to-cash", "byd_s_");
}
