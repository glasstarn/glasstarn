//! The JD Edwards packs on made-up tables whose contents are known by
//! construction, with Julian dates.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe, check};
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
    let answers = BTreeMap::from([(
        "setting:offset".to_owned(),
        Answer {
            choice: "value".into(),
            value: Some("-06:00".into()),
            confirmed_by: "JD Edwards administrator".into(),
            at: "2026-10-14T00:00:00Z".into(),
            question: String::new(),
            carries: Vec::new(),
        },
    )]);
    let report = check(pack, files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
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
fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-jde-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn jde_purchase_to_pay_yields_every_planted_step() {
    use pm_testgen::jde::{PURCHASING_TABLES, Settings, write_purchasing};
    let dir = dir("p2p");
    let truth = write_purchasing(&dir, &Settings::default()).unwrap();
    let emitted = run(
        &pack("jde-purchase-to-pay"),
        &files(&dir, &PURCHASING_TABLES),
    );
    assert_eq!(emitted["jde_po_entered"], truth.orders);
    assert_eq!(emitted["jde_po_entered_lines"], truth.lines);
    assert_eq!(emitted["jde_po_status"], truth.status_changes);
    assert_eq!(emitted["jde_goods_received"], truth.receipts);
    assert_eq!(emitted["jde_voucher_matched"], truth.matches);
    assert_eq!(emitted["jde_voucher_entered"], truth.vouchers);
    assert_eq!(emitted["jde_voucher_paid"], truth.payments);
    assert_eq!(
        emitted["jde_voucher_entered_lines"] + emitted["jde_voucher_paid_lines"],
        truth.voucher_line_steps
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn jde_order_to_cash_yields_every_planted_step() {
    use pm_testgen::jde::{SELLING_TABLES, Settings, write_selling};
    let dir = dir("o2c");
    let truth = write_selling(&dir, &Settings::default()).unwrap();
    let emitted = run(&pack("jde-order-to-cash"), &files(&dir, &SELLING_TABLES));
    assert_eq!(emitted["jde_so_entered"], truth.orders);
    assert_eq!(emitted["jde_so_entered_lines"], truth.lines);
    assert_eq!(emitted["jde_so_status"], truth.status_changes);
    assert_eq!(emitted["jde_so_shipped"], truth.shipped);
    assert_eq!(emitted["jde_invoice_created"], truth.invoices);
    assert_eq!(emitted["jde_invoice_for_line"], truth.invoiced_lines);
    assert_eq!(emitted["jde_invoice_created_lines"], truth.invoiced_lines);
    assert_eq!(emitted["jde_payment_received"], truth.payments);
    assert_eq!(emitted["jde_payment_received_lines"], truth.paid_lines);
    std::fs::remove_dir_all(dir).unwrap();
}
