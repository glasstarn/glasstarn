//! The SAP Business One buying and selling packs on made-up tables whose
//! contents are known by construction.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe, check};
use pm_testgen::sap_business_one::{Flow, Settings, write};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn pack(id: &str) -> Pack {
    Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap()
}
fn files(dir: &Path, tables: &[&str]) -> BTreeMap<String, PathBuf> {
    tables
        .iter()
        .map(|t| ((*t).to_owned(), dir.join(format!("{t}.csv"))))
        .collect()
}

fn run(pack: &Pack, files: &BTreeMap<String, PathBuf>) -> BTreeMap<String, u64> {
    let report = check(pack, files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let answers = BTreeMap::from([(
        "setting:offset".to_owned(),
        Answer {
            choice: "value".into(),
            value: Some("+01:00".into()),
            confirmed_by: "Controller".into(),
            at: "2026-10-14T00:00:00Z".into(),
            question: String::new(),
            carries: Vec::new(),
        },
    )]);
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(pack, &hashes, &answers).unwrap();
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
    let dir = std::env::temp_dir().join(format!("pm-b1-{id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write(&dir, flow, &Settings::default()).unwrap();
    assert!(truth.invoice_lines_from_orders > 0 && truth.paid_invoices > 0);
    assert!(truth.rejections > 0 && truth.cancelled_orders > 0);
    assert!(truth.cancelled_receipt_lines > 0 && truth.cancelled_invoices > 0);
    let tables = flow.tables();
    let emitted = run(&pack(id), &files(&dir, &tables));
    let e = |rule: &str| emitted[&format!("{p}{rule}")];
    assert_eq!(e("request_created"), truth.requests);
    assert_eq!(e("order_created"), truth.orders);
    assert_eq!(e("order_has_line"), truth.order_lines);
    assert_eq!(e("order_created_lines"), truth.order_lines);
    assert_eq!(e("request_created_lines"), truth.lines_from_requests);
    assert_eq!(e("goods_moved"), truth.receipt_lines);
    assert_eq!(e("receipt_for_line"), truth.receipt_lines);
    assert_eq!(e("invoice_created"), truth.invoices);
    // Invoices reach order lines directly or through the receipt.
    assert_eq!(e("invoice_created_lines"), truth.invoice_lines_from_orders);
    assert_eq!(
        e("invoice_created_receipt_lines"),
        truth.invoice_lines_from_receipts
    );
    assert_eq!(e("invoice_paid"), truth.paid_invoices);
    assert_eq!(
        e("invoice_paid_lines") + e("invoice_paid_receipt_lines"),
        truth.paid_order_lines
    );
    // Approvals reach the order their draft became, and its lines.
    assert_eq!(e("approval_requested"), truth.approval_requests);
    assert_eq!(e("approved"), truth.approvals);
    assert_eq!(e("rejected"), truth.rejections);
    assert_eq!(e("approval_became_order"), truth.approvals);
    assert_eq!(e("approved_order"), truth.approvals);
    assert_eq!(e("approved_lines"), truth.approved_order_lines);
    // Cancellations reach the order lines they undo.
    assert_eq!(e("order_cancelled"), truth.cancelled_orders);
    assert_eq!(e("order_cancelled_lines"), truth.cancelled_order_lines);
    assert_eq!(e("goods_movement_cancelled"), truth.cancelled_receipt_lines);
    assert_eq!(
        e("goods_movement_cancelled_lines"),
        truth.cancelled_receipt_lines
    );
    assert_eq!(e("invoice_cancelled"), truth.cancelled_invoices);
    assert_eq!(
        e("invoice_cancelled_lines") + e("invoice_cancelled_receipt_lines"),
        truth.cancelled_invoice_order_lines
    );

    // Without the optional requests, payments and partners the rest builds.
    let mut core = files(&dir, &tables);
    for optional in [
        tables[0], tables[1], tables[8], tables[9], tables[10], tables[11], tables[12],
    ] {
        core.remove(optional);
    }
    let emitted = run(&pack(id), &core);
    assert_eq!(emitted[&format!("{p}invoice_created")], truth.invoices);
    assert!(!emitted.contains_key(&format!("{p}invoice_paid")));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn business_one_buying_yields_every_planted_step() {
    every_planted_step(Flow::Buying, "sap-business-one-purchase-to-pay", "b1_p_");
}

#[test]
fn business_one_selling_yields_every_planted_step() {
    every_planted_step(Flow::Selling, "sap-business-one-order-to-cash", "b1_s_");
}
