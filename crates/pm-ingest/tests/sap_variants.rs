//! The S/4HANA Public Cloud pack and the accounts-payable variant, on made-up
//! data whose contents are known by construction.
mod common;
use pm_ingest::pack::{Answer, Pack, build_recipe, check};
use pm_testgen::sap_p2p::{CLOUD_TABLES, Settings, TABLES, Truth, write, write_cloud};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn answer(choice: &str, value: &str) -> Answer {
    Answer {
        choice: choice.into(),
        value: Some(value.into()),
        confirmed_by: "Purchasing analyst".into(),
        at: "2026-10-12T00:00:00Z".into(),
        question: String::new(),
        carries: Vec::new(),
    }
}

fn files(dir: &std::path::Path, tables: &[&str]) -> BTreeMap<String, PathBuf> {
    tables
        .iter()
        .map(|t| ((*t).to_owned(), dir.join(format!("{t}.csv"))))
        .collect()
}

fn fake_hashes(files: &BTreeMap<String, PathBuf>) -> BTreeMap<String, String> {
    files.keys().map(|t| (t.clone(), "0".repeat(64))).collect()
}

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-sap-variant-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn public_cloud_files_give_the_same_business_as_the_on_premise_tables() {
    let pack = Pack::parse(include_str!(
        "../../../packs/sap-s4-cloud-purchase-to-pay/pack.json"
    ))
    .unwrap();
    let dir = dir("cloud");
    let truth: Truth = write_cloud(&dir, &Settings::default()).unwrap();
    let files = files(&dir, &CLOUD_TABLES);
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let custom = report
        .questions
        .iter()
        .find(|q| q.id == "custom:PurchaseOrder.YY1_ApprovalDate_PDH")
        .unwrap();
    assert_eq!(custom.carries, ["YY1_ApprovedBy_PDH"]);
    assert!(custom.text.starts_with(&format!(
        "35% of purchase orders ({} of {})",
        truth.orders_with_custom_approver, truth.orders
    )));
    assert!(
        report
            .questions
            .iter()
            .any(|q| q.id == "link:paid-invoices-came-through-purchasing")
    );

    let answers = BTreeMap::from([
        ("setting:offset".to_owned(), answer("value", "+01:00")),
        (
            "custom:PurchaseOrder.YY1_ApprovalDate_PDH".to_owned(),
            answer("step", "Approve purchase order"),
        ),
    ]);
    let built = build_recipe(&pack, &fake_hashes(&files), &answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, &files);
    let report = &outcome.report;
    assert_eq!(report.issue_count, 0, "{:?}", report.issues);
    let emitted = |rule: &str| report.rules[rule].emitted;
    assert_eq!(emitted("requisition_created"), truth.requisition_items);
    assert_eq!(emitted("order_created"), truth.orders);
    assert_eq!(emitted("goods_received"), truth.goods_receipts);
    assert_eq!(
        emitted("goods_receipt_reversed"),
        truth.goods_receipt_reversals
    );
    assert_eq!(emitted("invoice_recorded"), truth.invoices);
    assert_eq!(
        emitted("invoice_paid"),
        truth.invoices_paid + truth.invoices_without_order_paid
    );
    assert_eq!(emitted("payment_of_invoice"), truth.invoices_paid);
    assert_eq!(
        emitted("custom_purchaseorder_yy1_approvaldate_pdh"),
        truth.orders_with_custom_approver
    );
    assert!(emitted("payment_order_lines") > truth.invoices_paid);
    // The change documents and returns read the same as on-premise.
    assert_eq!(emitted("order_release_step"), truth.release_steps);
    assert_eq!(emitted("price_changed"), truth.price_changes);
    assert_eq!(emitted("payment_blocked"), truth.payment_blocks);
    assert_eq!(emitted("goods_returned"), truth.goods_returns);
    assert_eq!(emitted("credit_memo_recorded"), truth.credit_memos);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn accounts_payable_runs_on_invoices_and_payments_alone() {
    let full = Pack::parse(include_str!("../../../packs/sap-purchase-to-pay/pack.json")).unwrap();
    let pack = full.with_variant("accounts-payable").unwrap();
    assert_eq!(pack.case_object_type, "invoice verification document");
    assert!(full.with_variant("nope").is_err());
    let dir = dir("ap");
    let truth = write(&dir, &Settings::default()).unwrap();
    // Only the four tables the variant needs are exported.
    let files = files(&dir, &["LFA1", "RBKP", "BKPF", "BSAK"]);
    assert!(TABLES.len() > files.len());
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    assert!(report.questions.iter().all(|q| !q.id.contains("EKKO")));

    let answers = BTreeMap::from([("setting:offset".to_owned(), answer("value", "+01:00"))]);
    let built = build_recipe(&pack, &fake_hashes(&files), &answers).unwrap();
    assert_eq!(built.recipe.inputs.len(), 4);
    let outcome = common::prepare_both(&built.recipe, &files);
    let report = &outcome.report;
    assert_eq!(report.issue_count, 0, "{:?}", report.issues);
    assert_eq!(report.rules["invoice_recorded"].emitted, truth.invoices);
    assert_eq!(
        report.rules["payment_of_invoice"].emitted,
        truth.invoices_paid
    );
    let invoices = truth.invoices + truth.credit_memos;
    assert_eq!(
        report.objects as u64,
        8 + invoices + invoices + truth.invoices_without_order
    );
    assert!(!report.rules.contains_key("order_created"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ariba_requisitions_and_approvals_join_the_sap_orders() {
    let pack = Pack::parse(include_str!(
        "../../../packs/sap-ariba-purchase-to-pay/pack.json"
    ))
    .unwrap();
    let dir = dir("ariba");
    let (truth, ariba) = pm_testgen::sap_p2p::write_ariba(&dir, &Settings::default()).unwrap();
    assert!(ariba.denials > 0 && ariba.approvals > ariba.requisitions);
    let files = files(&dir, &pm_testgen::sap_p2p::ARIBA_TABLES);
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let ids = report
        .questions
        .iter()
        .map(|q| q.id.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"setting:ariba_offset"));
    assert!(ids.contains(&"custom:EKKO.ZZAPPR_DAT"));
    // Orders bought directly in SAP are counted and asked about.
    assert!(ariba.orders_outside_ariba > 0);
    let outside = report
        .questions
        .iter()
        .find(|q| q.id == "link:sap-orders-came-through-ariba")
        .unwrap();
    assert!(outside.text.starts_with(&format!(
        "{} of {} SAP orders",
        ariba.orders_outside_ariba, truth.orders
    )));

    let answers = BTreeMap::from([
        ("setting:offset".to_owned(), answer("value", "+01:00")),
        ("setting:ariba_offset".to_owned(), answer("value", "+01:00")),
    ]);
    let built = build_recipe(&pack, &fake_hashes(&files), &answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, &files);
    let report = &outcome.report;
    assert_eq!(report.issue_count, 0, "{:?}", report.issues);
    let emitted = |rule: &str| report.rules[rule].emitted;
    assert_eq!(emitted("ariba_requisition_created"), ariba.requisitions);
    assert_eq!(emitted("ariba_requisition_submitted"), ariba.submitted);
    assert_eq!(emitted("ariba_requisition_approved"), ariba.approvals);
    assert_eq!(emitted("ariba_requisition_denied"), ariba.denials);
    assert_eq!(emitted("ariba_order_sent"), ariba.orders);
    assert_eq!(ariba.orders + ariba.orders_outside_ariba, truth.orders);
    assert_eq!(emitted("order_created"), truth.orders);
    assert_eq!(emitted("invoice_recorded"), truth.invoices);
    // Invoices received in Ariba, with exceptions and rejections.
    assert_eq!(emitted("ariba_invoice_received"), ariba.invoices);
    assert_eq!(
        emitted("ariba_invoice_exception_resolved"),
        ariba.exceptions
    );
    assert_eq!(
        emitted("ariba_invoice_approved"),
        ariba.invoices - ariba.invoices_rejected
    );
    assert_eq!(emitted("ariba_invoice_rejected"), ariba.invoices_rejected);
    // Approved Ariba invoices reach their SAP invoice and the order lines it
    // charges; rejected ones reach only their order.
    assert_eq!(
        emitted("ariba_invoice_approved_sap_invoice"),
        ariba.invoices - ariba.invoices_rejected
    );
    assert!(
        emitted("ariba_invoice_approved_lines") >= emitted("ariba_invoice_approved_sap_invoice")
    );
    assert_eq!(emitted("ariba_invoice_rejected_sap_invoice"), 0);
    assert_eq!(
        emitted("ariba_invoice_rejected_order"),
        ariba.invoices_rejected
    );
    // Approvals reach the SAP order and each of its lines.
    assert_eq!(emitted("ariba_requisition_approved_order"), ariba.approvals);
    assert!(emitted("ariba_requisition_approved_lines") > ariba.approvals);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ariba_with_public_cloud_gives_the_same_counts() {
    let pack = Pack::parse(
        pm_ingest::pack::BUILT_IN
            .iter()
            .find(|(id, _)| *id == "sap-ariba-s4-cloud-purchase-to-pay")
            .unwrap()
            .1,
    )
    .unwrap();
    let dir = dir("ariba-cloud");
    let (truth, ariba) =
        pm_testgen::sap_p2p::write_ariba_cloud(&dir, &Settings::default()).unwrap();
    let files = files(&dir, &pm_testgen::sap_p2p::ARIBA_CLOUD_TABLES);
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    assert!(
        report
            .questions
            .iter()
            .any(|q| q.id == "link:sap-orders-came-through-ariba")
    );
    let answers = BTreeMap::from([
        ("setting:offset".to_owned(), answer("value", "+01:00")),
        ("setting:ariba_offset".to_owned(), answer("value", "+01:00")),
    ]);
    let built = build_recipe(&pack, &fake_hashes(&files), &answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, &files);
    let report = &outcome.report;
    assert_eq!(report.issue_count, 0, "{:?}", report.issues);
    let emitted = |rule: &str| report.rules[rule].emitted;
    assert_eq!(emitted("ariba_requisition_approved"), ariba.approvals);
    assert_eq!(emitted("ariba_order_sent"), ariba.orders);
    assert_eq!(emitted("ariba_invoice_received"), ariba.invoices);
    assert_eq!(emitted("order_created"), truth.orders);
    assert_eq!(emitted("goods_received"), truth.goods_receipts);
    assert_eq!(emitted("invoice_recorded"), truth.invoices);
    assert_eq!(emitted("payment_of_invoice"), truth.invoices_paid);
    std::fs::remove_dir_all(dir).unwrap();
}
