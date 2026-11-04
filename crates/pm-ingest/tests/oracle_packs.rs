//! The Oracle packs (E-Business Suite, Fusion, PeopleSoft, NetSuite) on made-up tables
//! whose contents are known by construction.
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
fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-oracle-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}
fn run(pack: &Pack, files: &BTreeMap<String, PathBuf>, offset: &str) -> BTreeMap<String, u64> {
    run_with(pack, files, offset, &[])
}
fn run_with(
    pack: &Pack,
    files: &BTreeMap<String, PathBuf>,
    offset: &str,
    more: &[(&str, &str)],
) -> BTreeMap<String, u64> {
    let mut answers = BTreeMap::from([(
        "setting:offset".to_owned(),
        Answer {
            choice: "value".into(),
            value: Some(offset.into()),
            confirmed_by: "Oracle application administrator".into(),
            at: "2026-10-14T00:00:00Z".into(),
            question: String::new(),
            carries: Vec::new(),
        },
    )]);
    for (question, value) in more {
        let mut answer = answers["setting:offset"].clone();
        answer.value = Some((*value).into());
        answers.insert((*question).into(), answer);
    }
    let ready = pack.with_settings(&answers).unwrap();
    let report = check(&ready, files).unwrap();
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

#[test]
fn oracle_purchase_to_pay_yields_every_planted_step() {
    use pm_testgen::oracle::{PURCHASING_TABLES, Settings, write_purchasing};
    let dir = dir("p2p");
    let truth = write_purchasing(&dir, &Settings::default()).unwrap();
    assert!(truth.rejected > 0 && truth.holds > 0 && truth.cancelled_invoices > 0);
    for (id, offset) in [
        ("oracle-ebs-purchase-to-pay", "-05:00"),
        ("oracle-fusion-purchase-to-pay", "Z"),
    ] {
        let emitted = run(&pack(id), &files(&dir, &PURCHASING_TABLES), offset);
        assert_eq!(emitted["ora_po_created"], truth.orders);
        assert_eq!(emitted["ora_requisition_created"], truth.requisition_lines);
        assert_eq!(emitted["ora_po_submitted"], truth.submitted);
        assert_eq!(emitted["ora_po_forwarded"], truth.forwarded);
        assert_eq!(emitted["ora_po_approved"], truth.approved);
        assert_eq!(emitted["ora_po_rejected"], truth.rejected);
        let carried = |events: &[&str]| {
            events
                .iter()
                .map(|e| emitted[&format!("{e}_lines")])
                .sum::<u64>()
        };
        assert_eq!(
            carried(&[
                "ora_po_created",
                "ora_po_submitted",
                "ora_po_forwarded",
                "ora_po_approved",
                "ora_po_rejected"
            ]),
            truth.approval_line_steps
        );
        assert_eq!(emitted["ora_po_line_cancelled"], truth.cancelled_lines);
        assert_eq!(emitted["ora_goods_received"], truth.receipts);
        assert_eq!(emitted["ora_goods_delivered"], truth.deliveries);
        assert_eq!(emitted["ora_goods_returned"], truth.returns);
        assert_eq!(emitted["ora_invoice_created"], truth.invoices);
        assert_eq!(emitted["ora_invoice_for_line"], truth.invoice_lines_matched);
        assert_eq!(emitted["ora_invoice_cancelled"], truth.cancelled_invoices);
        assert_eq!(emitted["ora_invoice_held"], truth.holds);
        assert_eq!(emitted["ora_hold_released"], truth.releases);
        assert_eq!(emitted["ora_invoice_paid"], truth.payments);
        assert_eq!(
            carried(&[
                "ora_invoice_created",
                "ora_invoice_cancelled",
                "ora_invoice_held",
                "ora_hold_released",
                "ora_invoice_paid"
            ]),
            truth.invoice_line_steps
        );
    }
    // Accounts payable alone.
    let ap = pack("oracle-ebs-purchase-to-pay")
        .with_variant("accounts-payable")
        .unwrap();
    let emitted = run(&ap, &files(&dir, &PURCHASING_TABLES[6..]), "-05:00");
    assert_eq!(emitted["ora_invoice_paid"], truth.payments);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn oracle_order_to_cash_yields_every_planted_step() {
    use pm_testgen::oracle::{Settings, selling_tables, write_selling};
    for cloud in [false, true] {
        let dir = dir(if cloud { "o2c-fusion" } else { "o2c-ebs" });
        let truth = write_selling(&dir, cloud, &Settings::default()).unwrap();
        let (id, offset, more): (&str, &str, &[(&str, &str)]) = if cloud {
            (
                "oracle-fusion-order-to-cash",
                "Z",
                &[("setting:invoice_line_field", "INTERFACE_LINE_ATTRIBUTE5")],
            )
        } else {
            ("oracle-ebs-order-to-cash", "-05:00", &[])
        };
        let emitted = run_with(
            &pack(id),
            &files(&dir, &selling_tables(cloud)),
            offset,
            more,
        );
        assert_eq!(emitted["ora_so_created"], truth.orders);
        assert_eq!(emitted["ora_so_booked"], truth.booked);
        assert_eq!(emitted["ora_line_shipped"], truth.shipped);
        assert_eq!(emitted["ora_line_cancelled"], truth.cancelled);
        assert_eq!(emitted["ora_trx_created"], truth.invoices);
        assert_eq!(emitted["ora_trx_for_line"], truth.invoice_lines);
        assert_eq!(emitted["ora_payment_applied"], truth.payments);
        let order_steps = [
            "ora_so_created",
            "ora_so_booked",
            "ora_hold_applied",
            "ora_hold_released",
        ]
        .iter()
        .filter_map(|e| emitted.get(&format!("{e}_lines")))
        .sum::<u64>();
        assert_eq!(order_steps, truth.order_line_steps);
        assert_eq!(
            emitted["ora_trx_created_lines"] + emitted["ora_payment_applied_lines"],
            truth.invoice_line_steps
        );
        if !cloud {
            assert!(truth.holds > 0);
            assert_eq!(emitted["ora_hold_applied"], truth.holds);
            assert_eq!(emitted["ora_hold_released"], truth.releases);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn peoplesoft_purchase_to_pay_yields_every_planted_step() {
    use pm_testgen::peoplesoft::{PURCHASING_TABLES, Settings, write_purchasing};
    let dir = dir("peoplesoft");
    let truth = write_purchasing(&dir, &Settings::default()).unwrap();
    let emitted = run(
        &pack("peoplesoft-purchase-to-pay"),
        &files(&dir, &PURCHASING_TABLES),
        "-05:00",
    );
    assert_eq!(emitted["ps_req_created"], truth.requisitions);
    assert_eq!(emitted["ps_req_approved"], truth.requisitions_approved);
    assert_eq!(emitted["ps_req_became_line"], truth.lines_from_requisitions);
    assert_eq!(
        emitted["ps_req_created_lines"] + emitted["ps_req_approved_lines"],
        truth.requisition_line_steps
    );
    assert_eq!(emitted["ps_po_created"], truth.orders);
    assert_eq!(emitted["ps_po_approved"], truth.orders_approved);
    assert_eq!(
        emitted["ps_po_created_lines"] + emitted["ps_po_approved_lines"],
        truth.order_line_steps
    );
    assert_eq!(emitted["ps_goods_received"], truth.receipts);
    assert_eq!(emitted["ps_voucher_entered"], truth.vouchers);
    assert_eq!(emitted["ps_voucher_approved"], truth.vouchers_approved);
    assert_eq!(emitted["ps_voucher_paid"], truth.payments);
    assert_eq!(
        emitted["ps_voucher_entered_lines"]
            + emitted["ps_voucher_approved_lines"]
            + emitted["ps_voucher_paid_lines"],
        truth.voucher_line_steps
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn netsuite_yields_every_planted_step() {
    use pm_testgen::netsuite::{FILES, Flow, Settings, write};
    for (flow, id, p) in [
        (Flow::Buying, "netsuite-purchase-to-pay", "ns_p_"),
        (Flow::Selling, "netsuite-order-to-cash", "ns_s_"),
    ] {
        let dir = dir(id);
        let truth = write(&dir, flow, &Settings::default()).unwrap();
        let emitted = run_with(
            &pack(id),
            &files(&dir, &FILES),
            "Z",
            &[("setting:status_field", "TRANDOC.KSTATUS")],
        );
        let e = |rule: &str| emitted[&format!("{p}{rule}")];
        assert_eq!(e("order_created"), truth.orders);
        assert_eq!(e("order_has_line"), truth.lines);
        assert_eq!(e("order_status"), truth.status_changes);
        assert_eq!(
            e("order_created_lines") + e("order_status_lines"),
            truth.order_line_steps
        );
        assert_eq!(e("goods_moved"), truth.moved_lines);
        assert_eq!(e("bill_created"), truth.bills);
        assert_eq!(e("bill_for_line"), truth.billed_lines);
        assert_eq!(e("bill_created_lines"), truth.billed_lines);
        assert_eq!(e("bill_paid"), truth.payments);
        assert_eq!(e("bill_paid_lines"), truth.paid_lines);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
