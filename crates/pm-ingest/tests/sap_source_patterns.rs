//! Invented source rows target patterns seen in real extracts. No tenant data.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, SET_ASIDE, build_recipe, check};
use pm_ingest::prepare_ocel::Outcome;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn pack(id: &str) -> Pack {
    Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap()
}
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-sap-pattern-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn inputs(dir: &Path, tables: &[&str]) -> BTreeMap<String, PathBuf> {
    tables
        .iter()
        .map(|t| (t.to_string(), dir.join(format!("{t}.csv"))))
        .collect()
}
fn read(path: &Path) -> (Vec<String>, Vec<BTreeMap<String, String>>) {
    let mut csv = csv::Reader::from_path(path).unwrap();
    let headers = csv
        .headers()
        .unwrap()
        .iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let rows = csv
        .records()
        .map(|r| {
            headers
                .iter()
                .cloned()
                .zip(r.unwrap().iter().map(str::to_owned))
                .collect()
        })
        .collect();
    (headers, rows)
}
fn write(path: &Path, headers: &[String], rows: &[BTreeMap<String, String>]) {
    let mut w = csv::Writer::from_path(path).unwrap();
    w.write_record(headers).unwrap();
    for r in rows {
        w.write_record(headers.iter().map(|h| &r[h])).unwrap();
    }
    w.flush().unwrap();
}
fn build(pack: &Pack, files: &BTreeMap<String, PathBuf>) -> Outcome {
    build_with(pack, files, false)
}
/// With `set_aside`, the person approved leaving out records that do not fit.
fn build_with(pack: &Pack, files: &BTreeMap<String, PathBuf>, set_aside: bool) -> Outcome {
    let mut answers = BTreeMap::from([(
        "setting:offset".into(),
        Answer {
            choice: "value".into(),
            value: Some("+02:00".into()),
            confirmed_by: "Fixture author".into(),
            at: "2026-01-01T00:00:00Z".into(),
            question: String::new(),
            carries: vec![],
        },
    )]);
    if set_aside {
        answers.insert(
            SET_ASIDE.into(),
            Answer {
                choice: "set_aside".into(),
                value: None,
                confirmed_by: "Fixture author".into(),
                at: "2026-01-01T00:00:00Z".into(),
                question: String::new(),
                carries: vec![],
            },
        );
    }
    let hashes = files.keys().map(|t| (t.clone(), "a".repeat(64))).collect();
    let recipe = build_recipe(pack, &hashes, &answers).unwrap().recipe;
    common::prepare_both(&recipe, files)
}

#[test]
fn separate_invoice_items_for_one_order_item_keep_lineage_but_not_duplicate_events() {
    let dir = folder("invoice-items");
    pm_testgen::sap_p2p::write(&dir, &Default::default()).unwrap();
    let pack = pack("sap-purchase-to-pay");
    let files = inputs(&dir, &pm_testgen::sap_p2p::TABLES);
    let original = build(&pack, &files);
    assert_eq!(original.report.issue_count, 0);
    let path = &files["RSEG"];
    let (headers, mut rows) = read(path);
    let mut extra = rows[0].clone();
    extra.insert("BUZEI".into(), "999999".into());
    rows.push(extra.clone());
    write(path, &headers, &rows);
    assert_eq!(check(&pack, &files).unwrap().status, "ready");
    let split = build(&pack, &files);
    assert_eq!(split.report.issue_count, 0);
    assert_eq!(split.report.objects, original.report.objects + 1);
    assert_eq!(split.report.events, original.report.events);
    assert_eq!(
        split.report.rules["invoice_order_lines"].emitted,
        original.report.rules["invoice_order_lines"].emitted + 1
    );
    assert_eq!(
        split.report.rules["invoice_lines_orders"].emitted,
        original.report.rules["invoice_lines_orders"].emitted
    );
    let evidence = serde_json::to_value(&split.relationship_lineage).unwrap();
    assert!(
        evidence
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["rule"] == "invoice_order_lines" && l["row"] == rows.len() as u64)
    );
    // The same physical item appearing twice remains an error, not a split.
    rows.push(extra);
    write(path, &headers, &rows);
    assert_eq!(
        check(&pack, &files)
            .unwrap()
            .unique
            .iter()
            .find(|u| u.id == "one-invoice-item")
            .unwrap()
            .repeated,
        1
    );
    assert!(build(&pack, &files).log.is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn account_maintenance_is_not_a_credit_memo_and_posted_credit_memos_require_a_vendor() {
    let dir = folder("maintenance");
    let truth = pm_testgen::sap_p2p::write(&dir, &Default::default()).unwrap();
    let pack = pack("sap-purchase-to-pay")
        .with_variant("accounts-payable")
        .unwrap();
    let files = inputs(&dir, &["LFA1", "RBKP", "BKPF", "BSAK"]);
    let path = &files["RBKP"];
    let (headers, mut rows) = read(path);
    assert_eq!(rows[0]["XRECH"], "X");
    for (k, v) in [("IVTYP", "C"), ("LIFNR", ""), ("XRECH", "")] {
        rows[0].insert(k.into(), v.into());
    }
    write(path, &headers, &rows);
    assert_eq!(check(&pack, &files).unwrap().status, "ready");
    let result = build(&pack, &files);
    assert_eq!(result.report.issue_count, 0);
    assert_eq!(
        result.report.rules["account_maintenance_recorded"].emitted,
        1
    );
    assert_eq!(
        result.report.rules["credit_memo_recorded"].emitted,
        truth.credit_memos
    );
    assert_eq!(
        result.report.rules["invoice_recorded"].emitted,
        truth.invoices - 1
    );
    rows[0].insert("IVTYP".into(), String::new());
    write(path, &headers, &rows);
    let report = check(&pack, &files).unwrap();
    let vendors = report
        .links
        .iter()
        .find(|l| l.id == "invoices-have-vendors")
        .unwrap();
    assert_eq!(vendors.missing, 1);
    // One posted credit memo without a vendor among many is a record that
    // does not fit, not a broken export: the person decides.
    assert_eq!(report.status, "ready");
    assert_eq!(report.data_problems.len(), 1);
    assert_eq!(report.questions[0].id, SET_ASIDE);
    assert!(build(&pack, &files).log.is_none());
    // Set aside, the rest builds and the memo is listed.
    let kept = build_with(&pack, &files, true);
    assert!(kept.log.is_some(), "{:?}", kept.report.issues);
    assert!(
        kept.set_aside
            .iter()
            .any(|i| i.rule == "credit_memo_recorded")
    );
    // The former account maintenance row became the memo without a vendor;
    // the real credit memos all remain.
    assert_eq!(
        kept.report.rules["credit_memo_recorded"].emitted,
        truth.credit_memos
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn billing_splits_and_multiline_payments_do_not_multiply_invoice_steps() {
    let dir = folder("customer-lines");
    let truth = pm_testgen::sap_o2c::write(&dir, &Default::default()).unwrap();
    let pack = pack("sap-order-to-cash");
    let files = inputs(&dir, &pm_testgen::sap_o2c::TABLES);
    let original = build(&pack, &files);
    assert_eq!(original.report.issue_count, 0);
    let path = &files["VBRP"];
    let (headers, mut rows) = read(path);
    let mut extra = rows[0].clone();
    extra.insert("POSNR".into(), "999999".into());
    rows.push(extra);
    write(path, &headers, &rows);
    let path = &files["BSAD"];
    let (headers, mut rows) = read(path);
    // One payment with a second customer posting line, and one invoice
    // posting cleared in two items: four distinct facts, one cash document.
    let mut payment = rows
        .iter()
        .find(|r| r["BLART"] == "DZ" && r["SHKZG"] == "H")
        .unwrap()
        .clone();
    payment.insert("BUZEI".into(), "999".into());
    rows.push(payment);
    let mut invoice = rows.iter().find(|r| r["SHKZG"] == "S").unwrap().clone();
    invoice.insert("BUZEI".into(), "998".into());
    rows.push(invoice.clone());
    write(path, &headers, &rows);
    assert_eq!(check(&pack, &files).unwrap().status, "ready");
    let result = build(&pack, &files);
    assert_eq!(result.report.issue_count, 0);
    assert_eq!(result.report.events, original.report.events);
    assert_eq!(result.report.objects, original.report.objects + 3);
    assert_eq!(
        result.report.rules["payment_received"].emitted,
        truth.payments
    );
    assert_eq!(
        result.report.rules["payment_invoice"].emitted,
        truth.payments
    );
    rows.push(invoice);
    write(path, &headers, &rows);
    assert!(
        check(&pack, &files)
            .unwrap()
            .unique
            .iter()
            .any(|u| u.id == "one-customer-item" && u.repeated == 1)
    );
    assert!(build(&pack, &files).log.is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reversal_documents_keep_their_own_clearing_and_change_history() {
    let dir = folder("journal-reversal");
    pm_testgen::sap_operations::write_journal_entries(&dir, &Default::default()).unwrap();
    let pack = pack("sap-record-to-report");
    let files = inputs(&dir, &pm_testgen::sap_operations::JOURNAL_TABLES);
    let (_, headers) = read(&files["BKPF"]);
    let reverse = headers.iter().find(|r| r["XREVERSAL"] == "2").unwrap();
    let path = &files["BSAS"];
    let (columns, mut cleared) = read(path);
    let mut item = cleared[0].clone();
    for k in ["BUKRS", "BELNR", "GJAHR"] {
        item.insert(k.into(), reverse[k].clone());
    }
    item.insert("BUZEI".into(), "999".into());
    cleared.push(item);
    write(path, &columns, &cleared);
    let original = build(&pack, &files);
    assert_eq!(original.report.issue_count, 0);
    assert!(original.log.is_some());
    // An unrelated workflow object on the same work item cannot override FIPP.
    let path = &files["SWW_CONTOB"];
    let (columns, mut links) = read(path);
    let mut other = links[0].clone();
    other.insert("OBJTYPE".into(), "BUS2081".into());
    other.insert("OBJKEY".into(), "unrelated".into());
    links.push(other);
    write(path, &columns, &links);
    assert_eq!(check(&pack, &files).unwrap().status, "ready");
    let outcome = build(&pack, &files);
    assert_eq!(outcome.report.issue_count, 0);
    assert_eq!(outcome.report.events, original.report.events);
    let log = serde_json::to_value(outcome.log.as_ref().unwrap()).unwrap();
    let completion = log["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "Complete journal workflow task")
        .unwrap();
    assert!(completion["time"].as_str().unwrap().ends_with("+00:00"));
    // Completion alone never becomes an approval assertion.
    assert!(
        !log["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "Approve journal entry")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn document_categories_gate_scope_and_configured_invoice_types_do_not_hide_invoices() {
    let dir = folder("categories");
    let truth = pm_testgen::sap_o2c::write(&dir, &Default::default()).unwrap();
    let p = pack("sap-order-to-cash");
    let files = inputs(&dir, &pm_testgen::sap_o2c::TABLES);
    let path = &files["VBRK"];
    let (headers, mut rows) = read(path);
    let invoice = rows.iter_mut().find(|r| r["VBTYP"] == "M").unwrap();
    invoice.insert("FKART".into(), "F1".into());
    write(path, &headers, &rows);
    assert_eq!(
        build(&p, &files).report.rules["invoice_created"].emitted,
        truth.invoices
    );
    let path = &files["VBAK"];
    let (headers, mut rows) = read(path);
    rows[0].insert("VBTYP".into(), "B".into());
    write(path, &headers, &rows);
    let check = check(&p, &files).unwrap();
    assert_eq!(check.status, "export_incomplete");
    assert_eq!(
        check
            .links
            .iter()
            .find(|l| l.id == "vbak-document-scope")
            .unwrap()
            .missing,
        1
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_purchase_order_can_omit_its_vendor_but_a_nonblank_broken_reference_still_blocks() {
    let dir = folder("stock-transfer");
    pm_testgen::sap_p2p::write(&dir, &Default::default()).unwrap();
    let p = pack("sap-purchase-to-pay");
    let files = inputs(&dir, &pm_testgen::sap_p2p::TABLES);
    let path = &files["EKKO"];
    let (headers, mut rows) = read(path);
    rows[0].insert("LIFNR".into(), String::new());
    rows[0].insert("BSART".into(), "UB".into());
    write(path, &headers, &rows);
    let result = build(&p, &files);
    assert_eq!(result.report.issue_count, 0);
    assert_eq!(result.report.rules["order_created"].omitted_blank_links, 1);
    rows[0].insert("LIFNR".into(), "NOT-IN-EXPORT".into());
    write(path, &headers, &rows);
    assert!(build(&p, &files).log.is_none());
    rows[0].insert("LIFNR".into(), String::new());
    rows[0].insert("BSTYP".into(), "K".into());
    write(path, &headers, &rows);
    let report = check(&p, &files).unwrap();
    assert_eq!(report.status, "export_incomplete");
    assert_eq!(
        report
            .links
            .iter()
            .find(|l| l.id == "ekko-document-scope")
            .unwrap()
            .missing,
        1
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reference_free_items_and_missing_historical_items_keep_their_document_evidence() {
    let dir = folder("reference-free");
    let truth = pm_testgen::sap_o2c::write(&dir, &Default::default()).unwrap();
    let p = pack("sap-order-to-cash");
    let files = inputs(&dir, &pm_testgen::sap_o2c::TABLES);
    let path = &files["VBRP"];
    let (headers, mut rows) = read(path);
    rows[0].insert("AUBEL".into(), String::new());
    write(path, &headers, &rows);
    let path = &files["LIPS"];
    let (headers, mut rows) = read(path);
    rows[0].insert("VGBEL".into(), String::new());
    write(path, &headers, &rows);
    let path = &files["CDPOS"];
    let (headers, mut rows) = read(path);
    let mut historical = rows
        .iter()
        .find(|r| r["TABNAME"] == "VBAP" && r["FNAME"] == "ABGRU" && !r["VALUE_NEW"].is_empty())
        .unwrap()
        .clone();
    historical.insert(
        "TABKEY".into(),
        format!("{}999999", &historical["TABKEY"][..13]),
    );
    rows.push(historical);
    write(path, &headers, &rows);
    let report = check(&p, &files).unwrap();
    assert!(
        report
            .questions
            .iter()
            .any(|q| q.id == "link:rejection-history-items-present")
    );
    let result = build(&p, &files);
    assert_eq!(result.report.issue_count, 0);
    assert_eq!(
        result.report.rules["order_line_rejected"].emitted,
        truth.rejected_items + 1
    );
    assert_eq!(result.report.rules["order_line_rejected"].unmatched, 1);
    assert_eq!(
        result.report.rules["billing_order_lines"].omitted_blank_links,
        1
    );
    assert_eq!(
        result.report.rules["delivery_order_lines"].omitted_blank_links,
        1
    );
    assert!(result.log.is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn final_release_uses_the_exported_configuration_and_needs_both_lookup_sources() {
    let dir = folder("release-config");
    let truth = pm_testgen::sap_p2p::write(&dir, &Default::default()).unwrap();
    let p = pack("sap-purchase-to-pay");
    let mut files = inputs(&dir, &pm_testgen::sap_p2p::TABLES);
    let path = &files["T16FB"];
    let (headers, mut config) = read(path);
    config.push(BTreeMap::from([
        ("FRGKE".into(), "G".into()),
        ("KZFRE".into(), "X".into()),
    ]));
    write(path, &headers, &config);
    let path = &files["CDPOS"];
    let (headers, mut changes) = read(path);
    let release = changes.iter_mut().find(|r| r["FNAME"] == "FRGKE").unwrap();
    release.insert("VALUE_NEW".into(), "G".into());
    write(path, &headers, &changes);
    assert_eq!(check(&p, &files).unwrap().status, "ready");
    let report = build(&p, &files).report;
    assert_eq!(report.issue_count, 0);
    assert_eq!(report.rules["order_released"].emitted, truth.releases);
    // The exact same change ceases to be a release when the source's flag is off.
    config
        .last_mut()
        .unwrap()
        .insert("KZFRE".into(), String::new());
    write(&files["T16FB"], &["FRGKE".into(), "KZFRE".into()], &config);
    assert_eq!(
        build(&p, &files).report.rules["order_released"].emitted,
        truth.releases - 1
    );
    files.remove("T16FB");
    let outcome = build(&p, &files);
    assert_eq!(outcome.report.issue_count, 0);
    assert!(!outcome.report.rules.contains_key("order_released"));
    assert!(outcome.report.rules.contains_key("order_release_step"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_reversal_without_its_own_items_still_reaches_the_original_order_items() {
    let dir = folder("invoice-reversal");
    pm_testgen::sap_p2p::write(&dir, &Default::default()).unwrap();
    let p = pack("sap-purchase-to-pay");
    let files = inputs(&dir, &pm_testgen::sap_p2p::TABLES);
    let path = &files["RBKP"];
    let (headers, mut invoices) = read(path);
    let original = invoices[0].clone();
    let mut reversal = original.clone();
    for (k, v) in [
        ("BELNR", "9999999999"),
        ("IVTYP", "5"),
        ("XRECH", ""),
        ("LIFNR", ""),
    ] {
        reversal.insert(k.into(), v.into());
    }
    reversal.insert("STBLG".into(), original["BELNR"].clone());
    reversal.insert("STJAH".into(), original["GJAHR"].clone());
    invoices.push(reversal);
    write(path, &headers, &invoices);
    let (_, lines) = read(&files["RSEG"]);
    let expected = lines
        .iter()
        .filter(|r| r["BELNR"] == original["BELNR"] && r["GJAHR"] == original["GJAHR"])
        .map(|r| (&r["EBELN"], &r["EBELP"]))
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u64;
    assert!(expected > 0);
    let outcome = build(&p, &files);
    assert_eq!(outcome.report.issue_count, 0);
    assert_eq!(outcome.report.rules["invoice_reversal_recorded"].emitted, 1);
    assert_eq!(
        outcome.report.rules["invoice_reversal_recorded_orders"].emitted,
        expected
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn debit_requests_and_credit_cancellations_reach_the_original_sales_item() {
    let dir = folder("memo-cancellations");
    pm_testgen::sap_o2c::write(&dir, &Default::default()).unwrap();
    let p = pack("sap-order-to-cash");
    let files = inputs(&dir, &pm_testgen::sap_o2c::TABLES);
    let (_, items) = read(&files["VBAP"]);
    let by_item = items
        .iter()
        .map(|r| ((r["VBELN"].clone(), r["POSNR"].clone()), r))
        .collect::<BTreeMap<_, _>>();
    let (_, bill_items) = read(&files["VBRP"]);
    let path = &files["VBRK"];
    let (headers, mut bills) = read(path);
    let memo = bills
        .iter()
        .find(|h| {
            h["VBTYP"] == "O"
                && bill_items.iter().any(|i| {
                    i["VBELN"] == h["VBELN"]
                        && by_item
                            .get(&(i["AUBEL"].clone(), i["AUPOS"].clone()))
                            .is_some_and(|r| !r["VGBEL"].is_empty())
                })
        })
        .unwrap()
        .clone();
    let mut cancellation = memo.clone();
    cancellation.insert("VBELN".into(), "9999999999".into());
    cancellation.insert("VBTYP".into(), "S".into());
    cancellation.insert("FKART".into(), "S2".into());
    cancellation.insert("SFAKN".into(), memo["VBELN"].clone());
    bills.push(cancellation);
    write(path, &headers, &bills);
    let outcome = build(&p, &files);
    assert_eq!(outcome.report.issue_count, 0);
    assert_eq!(outcome.report.rules["credit_memo_cancelled"].emitted, 1);
    assert!(outcome.report.rules["credit_memo_cancelled_original"].emitted > 0);
    let path = &files["VBAK"];
    let (headers, mut sales) = read(path);
    let request = sales.iter_mut().find(|r| r["VBTYP"] == "K").unwrap();
    request.insert("VBTYP".into(), "L".into());
    request.insert("AUART".into(), "DR".into());
    write(path, &headers, &sales);
    let outcome = build(&p, &files);
    assert_eq!(outcome.report.issue_count, 0);
    assert_eq!(outcome.report.rules["debit_request_created"].emitted, 1);
    assert!(outcome.report.rules["debit_request_created_original"].emitted > 0);
    std::fs::remove_dir_all(dir).unwrap();
}
