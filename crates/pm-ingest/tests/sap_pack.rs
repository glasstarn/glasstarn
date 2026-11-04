//! The SAP purchase-to-pay pack against made-up SAP tables whose contents
//! are known by construction: the data check finds what was planted, and the
//! recipe built from the pack and the answers yields exactly the planted steps.
mod common;
use pm_ingest::pack::{Answer, Pack, build_recipe, check};
use pm_testgen::sap_p2p::{Settings, TABLES, Truth, write};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn pack() -> Pack {
    Pack::parse(include_str!("../../../packs/sap-purchase-to-pay/pack.json")).unwrap()
}

fn export(name: &str) -> (PathBuf, BTreeMap<String, PathBuf>, Truth) {
    let dir = std::env::temp_dir().join(format!("pm-sap-pack-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write(&dir, &Settings::default()).unwrap();
    let files = TABLES
        .iter()
        .map(|t| ((*t).to_owned(), dir.join(format!("{t}.csv"))))
        .collect();
    (dir, files, truth)
}

fn answer(choice: &str, value: &str, carries: &[&str]) -> Answer {
    Answer {
        choice: choice.into(),
        value: Some(value.into()),
        confirmed_by: "Purchasing analyst".into(),
        at: "2026-10-12T00:00:00Z".into(),
        question: String::new(),
        carries: carries.iter().map(|c| (*c).to_owned()).collect(),
    }
}

#[test]
fn data_check_finds_what_was_planted() {
    let pack = pack();
    let (dir, files, truth) = export("check");
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    assert!(report.unique.iter().all(|u| u.repeated == 0));

    // Vendor invoices posted with no order: asked about, with exact counts.
    let paid = truth.invoices_paid + truth.invoices_without_order_paid;
    let link = report
        .links
        .iter()
        .find(|l| l.id == "paid-invoices-came-through-purchasing")
        .unwrap();
    assert_eq!(
        (link.rows, link.missing),
        (paid, truth.invoices_without_order_paid)
    );
    let question = report
        .questions
        .iter()
        .find(|q| q.id == "link:paid-invoices-came-through-purchasing")
        .unwrap();
    assert!(question.text.starts_with(&format!(
        "{} of {paid} paid vendor invoices",
        truth.invoices_without_order_paid
    )));

    // The custom approver: one question about the date, carrying the name.
    let custom = report
        .questions
        .iter()
        .filter(|q| q.kind == "custom_field")
        .collect::<Vec<_>>();
    assert_eq!(custom.len(), 1, "{custom:?}");
    assert_eq!(custom[0].id, "custom:EKKO.ZZAPPR_DAT");
    assert_eq!(custom[0].recommended.as_deref(), Some("step"));
    assert_eq!(custom[0].carries, ["ZZAPPR_BY"]);
    let ekko = report.tables.iter().find(|t| t.table == "EKKO").unwrap();
    let date = ekko
        .custom_fields
        .iter()
        .find(|f| f.column == "ZZAPPR_DAT")
        .unwrap();
    assert_eq!(date.filled, truth.orders_with_custom_approver);
    let timing = date.timing.as_ref().unwrap();
    assert!((1..=3).contains(&timing.median_days));
    assert_eq!(timing.before_share, 0.0);
    assert!(report.questions.iter().any(|q| q.id == "setting:offset"));
    // Both company codes are visible, so the agent can confirm the scope.
    assert_eq!(ekko.scope.len(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_built_recipe_yields_exactly_the_planted_steps() {
    let pack = pack();
    let (dir, files, truth) = export("build");
    let hashes = TABLES
        .iter()
        .map(|t| ((*t).to_owned(), format!("{:064}", 0)))
        .collect::<BTreeMap<_, _>>();
    let answers = BTreeMap::from([
        ("setting:offset".to_owned(), answer("value", "+01:00", &[])),
        (
            "custom:EKKO.ZZAPPR_DAT".to_owned(),
            answer("step", "Approve purchase order", &["ZZAPPR_BY"]),
        ),
    ]);
    let built = build_recipe(&pack, &hashes, &answers).unwrap();
    assert_eq!(built.changes.len(), 1);
    let outcome = common::prepare_both(&built.recipe, &files);
    let report = &outcome.report;
    assert_eq!(report.issue_count, 0, "{:?}", report.issues);
    let emitted = |rule: &str| report.rules[rule].emitted;
    assert_eq!(emitted("requisition_created"), truth.requisition_items);
    assert_eq!(emitted("requisition_released"), truth.requisitions_released);
    assert_eq!(emitted("order_created"), truth.orders);
    assert_eq!(emitted("order_created_lines"), truth.order_items);
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
    // Unpaid invoices have no payment step; invoices with no order are
    // left out of the invoice link by its condition.
    assert_eq!(
        report.rules["payment_of_invoice"].skipped,
        truth.invoices - truth.invoices_paid + truth.credit_memos
    );
    assert_eq!(
        report.rules["payment_of_invoice"].filtered,
        truth.invoices_without_order
    );
    assert_eq!(
        emitted("custom_ekko_zzappr_dat"),
        truth.orders_with_custom_approver
    );
    assert!(emitted("custom_ekko_zzappr_dat_lines_0") > truth.orders_with_custom_approver);
    // Every payment of an order invoice also reaches the order lines it paid,
    // so order lines show the payment step.
    let log = serde_json::to_value(outcome.log.as_ref().unwrap()).unwrap();
    let payments = log["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "Clear vendor item")
        .map(|e| {
            let qualifiers = e["relationships"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["qualifier"].as_str().unwrap())
                .collect::<Vec<_>>();
            (
                qualifiers.contains(&"paid invoice"),
                qualifiers.contains(&"paid order line"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        payments.iter().filter(|(invoice, _)| *invoice).count() as u64,
        truth.invoices_paid
    );
    assert!(payments.iter().all(|(invoice, line)| invoice == line));
    assert_eq!(report.rules["payment_order_lines"].skipped, 0);
    // Same invoice number in two fiscal years stays two invoices.
    assert!(truth.invoice_numbers_in_two_years > 0);
    // Invoices and credit memos are supplier invoices; every accounting
    // document is a payable posting.
    let invoices = truth.invoices + truth.credit_memos;
    assert_eq!(
        report.objects as u64,
        8 + truth.requisition_items
            + truth.orders
            + truth.order_items
            + invoices
            + invoices
            + truth.invoices_without_order
            + truth.invoice_items
    );
    // Returns to the vendor, credit memos and the change log.
    assert_eq!(emitted("goods_returned"), truth.goods_returns);
    assert_eq!(emitted("credit_memo_recorded"), truth.credit_memos);
    assert_eq!(emitted("credit_memo_lines"), truth.credit_memos);
    assert_eq!(emitted("order_release_step"), truth.release_steps);
    assert_eq!(emitted("order_released"), truth.releases);
    assert_eq!(emitted("price_changed"), truth.price_changes);
    assert_eq!(emitted("quantity_changed"), truth.quantity_changes);
    assert_eq!(
        emitted("delivery_date_changed"),
        truth.delivery_date_changes
    );
    assert_eq!(emitted("order_line_deleted"), truth.deleted_items);
    assert_eq!(emitted("payment_blocked"), truth.payment_blocks);
    assert_eq!(
        emitted("payment_block_released"),
        truth.payment_block_releases
    );
    // Payment blocks reach their invoice; releases reach every order line.
    assert_eq!(emitted("payment_blocked_invoice"), truth.payment_blocks);
    assert!(emitted("payment_blocked_lines") >= truth.payment_blocks);
    assert!(emitted("payment_block_released_lines") >= truth.payment_block_releases);
    assert!(emitted("order_released_lines") >= truth.releases);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_missing_table_or_field_leaves_out_steps_and_only_a_missing_case_stops() {
    let pack = pack();
    let (dir, mut files, _) = export("gaps");
    files.remove("RSEG");
    let ekpo = std::fs::read_to_string(dir.join("EKPO.csv")).unwrap();
    std::fs::write(dir.join("EKPO.csv"), ekpo.replacen("WERKS", "PLANT", 1)).unwrap();
    // Order lines, their steps and times are still there: the check goes
    // ahead and names what is lost.
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let rseg = report.left_out.iter().find(|l| l.contains("RSEG")).unwrap();
    assert!(rseg.contains("goes ahead without"), "{rseg}");
    assert!(report.left_out.iter().any(|l| l.contains("WERKS")));
    // Without the order lines themselves there is no case to follow.
    files.remove("EKPO");
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "export_incomplete");
    assert!(report.gaps.iter().any(|g| g.contains("EKPO")));
    std::fs::remove_dir_all(dir).unwrap();
}
