//! The SAP order-to-cash packs, on-premise and Public Cloud, and the accounts
//! receivable variant, on made-up data whose contents are known by construction.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe, check};
use pm_testgen::sap_o2c::{CLOUD_TABLES, Settings, TABLES, Truth, write, write_cloud};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn pack(id: &str) -> Pack {
    Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap()
}
fn answer(choice: &str, value: &str) -> Answer {
    Answer {
        choice: choice.into(),
        value: Some(value.into()),
        confirmed_by: "Order-to-cash analyst".into(),
        at: "2026-10-12T00:00:00Z".into(),
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
    let dir = std::env::temp_dir().join(format!("pm-sap-o2c-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Checks the data, answers as the analyst, builds and returns the report.
fn run(
    pack: &Pack,
    files: &BTreeMap<String, PathBuf>,
    custom: Option<&str>,
    truth: &Truth,
) -> pm_ingest::prepare_ocel::Report {
    let report = check(pack, files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let mut answers = BTreeMap::from([("setting:offset".to_owned(), answer("value", "+01:00"))]);
    if let Some(custom) = custom {
        let question = report.questions.iter().find(|q| q.id == custom).unwrap();
        assert_eq!(question.recommended.as_deref(), Some("step"));
        assert!(
            question
                .text
                .contains(&format!("({} of ", truth.confirmations))
        );
        answers.insert(custom.to_owned(), answer("step", "Send order confirmation"));
    }
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(pack, &hashes, &answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    outcome.report
}

fn assert_full_flow(report: &pm_ingest::prepare_ocel::Report, truth: &Truth, custom: &str) {
    let emitted = |rule: &str| report.rules[rule].emitted;
    assert_eq!(emitted("sales_order_created"), truth.orders);
    assert_eq!(emitted("sales_order_created_lines"), truth.order_items);
    assert_eq!(emitted(custom), truth.confirmations);
    // The change log.
    assert_eq!(emitted("credit_blocked"), truth.credit_blocked_orders);
    assert_eq!(emitted("credit_released"), truth.credit_releases);
    assert_eq!(emitted("delivery_blocked"), truth.delivery_blocks);
    assert_eq!(emitted("delivery_block_removed"), truth.delivery_blocks);
    assert_eq!(emitted("billing_blocked"), truth.billing_blocks);
    assert!(emitted("billing_block_removed") <= truth.billing_blocks);
    assert_eq!(emitted("order_line_rejected"), truth.rejected_items);
    assert_eq!(emitted("price_changed"), truth.price_changes);
    assert_eq!(emitted("quantity_changed"), truth.quantity_changes);
    // Returns and credit memos, carried back to the original order lines.
    assert_eq!(emitted("return_order_created"), truth.return_orders);
    assert_eq!(emitted("return_order_created_lines"), truth.return_items);
    assert_eq!(emitted("return_order_created_original"), truth.return_items);
    assert_eq!(emitted("return_delivery_created"), truth.return_deliveries);
    assert_eq!(emitted("returned_goods_received"), truth.returns_received);
    assert_eq!(emitted("credit_request_created"), truth.credit_requests);
    assert_eq!(emitted("credit_memo_created"), truth.credit_memos);
    assert!(emitted("credit_memo_created_original") >= truth.credit_memos);
    assert_eq!(emitted("delivery_created"), truth.deliveries);
    assert_eq!(emitted("goods_issued"), truth.deliveries_shipped);
    assert_eq!(emitted("invoice_created"), truth.invoices);
    assert_eq!(emitted("invoice_cancelled"), truth.cancellations);
    assert_eq!(emitted("payment_received"), truth.payments);
    // Each payment reaches its invoice; invoices cleared by their
    // cancellation get no payment.
    assert_eq!(emitted("payment_invoice"), truth.payments);
    assert_eq!(emitted("payment_of_receivable"), truth.payments);
    assert!(emitted("payment_lines") >= truth.payments);
    // Rejected order lines are never delivered.
    assert!(emitted("delivery_created_lines") < truth.order_items + truth.split_items);
}

#[test]
fn on_premise_order_to_cash_yields_every_planted_step() {
    let dir = dir("ecc");
    let truth = write(&dir, &Settings::default()).unwrap();
    let report = run(
        &pack("sap-order-to-cash"),
        &files(&dir, &TABLES),
        Some("custom:VBAK.ZZCONF_DAT"),
        &truth,
    );
    assert_full_flow(&report, &truth, "custom_vbak_zzconf_dat");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn public_cloud_order_to_cash_gives_the_same_counts() {
    let dir = dir("cloud");
    let truth = write_cloud(&dir, &Settings::default()).unwrap();
    let report = run(
        &pack("sap-s4-cloud-order-to-cash"),
        &files(&dir, &CLOUD_TABLES),
        Some("custom:SalesOrder.YY1_ConfirmationSentOn_SDH"),
        &truth,
    );
    assert_full_flow(
        &report,
        &truth,
        "custom_salesorder_yy1_confirmationsenton_sdh",
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn accounts_receivable_runs_on_invoices_and_payments_alone() {
    for (id, layout_cloud, tables) in [
        (
            "sap-order-to-cash",
            false,
            &["KNA1", "VBRK", "BKPF", "BSAD"][..],
        ),
        (
            "sap-s4-cloud-order-to-cash",
            true,
            &["Customer", "BillingDocument", "JournalEntryItem"][..],
        ),
    ] {
        let dir = dir(&format!("ar-{layout_cloud}"));
        let truth = if layout_cloud {
            write_cloud(&dir, &Settings::default()).unwrap()
        } else {
            write(&dir, &Settings::default()).unwrap()
        };
        let pack = pack(id).with_variant("accounts-receivable").unwrap();
        let report = run(&pack, &files(&dir, tables), None, &truth);
        assert_eq!(report.rules["invoice_created"].emitted, truth.invoices);
        assert_eq!(report.rules["payment_invoice"].emitted, truth.payments);
        assert!(!report.rules.contains_key("sales_order_created"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
