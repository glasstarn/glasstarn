//! The Microsoft Dynamics 365 packs on made-up extracts whose contents are
//! known by construction.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Exported, Pack, build_recipe, build_recipe_with, check};
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
            value: Some("+01:00".into()),
            confirmed_by: "Dynamics administrator".into(),
            at: "2026-10-14T00:00:00Z".into(),
            question: String::new(),
            carries: Vec::new(),
        },
    )]);
    let report = check(pack, files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe_with(
        pack,
        &hashes,
        &answers,
        &Exported::from_report(&report.tables),
    )
    .unwrap();
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
    let dir = std::env::temp_dir().join(format!("pm-d365-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn finance_operations_yields_every_planted_step() {
    use pm_testgen::dynamics::{Flow, Settings, write_finance_operations};
    for (flow, id, p) in [
        (
            Flow::Buying,
            "d365-finance-operations-purchase-to-pay",
            "d365_p_",
        ),
        (
            Flow::Selling,
            "d365-finance-operations-order-to-cash",
            "d365_s_",
        ),
    ] {
        let dir = dir(id);
        let truth = write_finance_operations(&dir, flow, &Settings::default()).unwrap();
        let tables = flow.tables();
        let emitted = run(&pack(id), &files(&dir, &tables));
        let e = |rule: &str| emitted[&format!("{p}{rule}")];
        assert_eq!(e("order_created"), truth.orders);
        assert_eq!(e("order_has_line"), truth.lines);
        assert_eq!(e("workflow"), truth.workflow_steps);
        assert_eq!(e("order_confirmed"), truth.confirmations);
        assert_eq!(
            e("order_created_lines") + e("workflow_lines") + e("order_confirmed_lines"),
            truth.order_line_steps
        );
        assert_eq!(e("goods_moved"), truth.receipt_lines);
        assert_eq!(e("invoice_posted"), truth.invoices);
        assert_eq!(e("invoice_for_line"), truth.invoice_lines);
        assert_eq!(e("invoice_settled"), truth.settled);
        assert_eq!(
            e("invoice_posted_lines") + e("invoice_settled_lines"),
            truth.invoice_line_steps
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn finance_operations_invoice_only_variants_need_no_order_exports() {
    use pm_testgen::dynamics::{Flow, Settings, write_finance_operations};
    for (flow, id, variant, prefix, tables) in [
        (
            Flow::Buying,
            "d365-finance-operations-purchase-to-pay",
            "accounts-payable",
            "d365_p_",
            ["VendInvoiceJour", "VendInvoiceTrans", "VendTrans"],
        ),
        (
            Flow::Selling,
            "d365-finance-operations-order-to-cash",
            "accounts-receivable",
            "d365_s_",
            ["CustInvoiceJour", "CustInvoiceTrans", "CustTrans"],
        ),
    ] {
        let dir = dir(variant);
        let truth = write_finance_operations(&dir, flow, &Settings::default()).unwrap();
        assert!(truth.settled > 0 && truth.settled < truth.invoices);
        let narrowed = pack(id).with_variant(variant).unwrap();
        let emitted = run(&narrowed, &files(&dir, &tables));
        assert_eq!(
            emitted,
            BTreeMap::from([
                (format!("{prefix}invoice"), truth.invoices),
                (format!("{prefix}invoice_posted"), truth.invoices),
                (format!("{prefix}invoice_settled"), truth.settled),
            ])
        );
        // X++ represents a null date as 1900-01-01. An open item exported
        // with that value must remain open, just like the blank fixture.
        let transaction_file = dir.join(format!("{}.csv", tables[2]));
        let mut reader = csv::Reader::from_path(&transaction_file).unwrap();
        let headers = reader.headers().unwrap().clone();
        let closed = headers
            .iter()
            .position(|column| column == "Closed")
            .unwrap();
        let rows: Vec<Vec<String>> = reader
            .records()
            .map(|row| {
                let mut row: Vec<String> = row.unwrap().iter().map(str::to_owned).collect();
                if row[closed].is_empty() {
                    row[closed] = "1900-01-01".into();
                }
                row
            })
            .collect();
        drop(reader);
        let mut writer = csv::Writer::from_path(&transaction_file).unwrap();
        writer.write_record(&headers).unwrap();
        for row in rows {
            writer.write_record(row).unwrap();
        }
        writer.flush().unwrap();
        drop(writer);
        assert_eq!(run(&narrowed, &files(&dir, &tables)), emitted);
        let full = run(&pack(id), &files(&dir, &flow.tables()));
        assert_eq!(full[&format!("{prefix}invoice_settled")], truth.settled);
        // Closed records full settlement, which can be against a credit
        // note. It does not establish that a cash payment occurred.
        let hashes = tables
            .iter()
            .map(|t| (t.to_string(), "0".repeat(64)))
            .collect();
        let answers = BTreeMap::from([(
            "setting:offset".to_owned(),
            Answer {
                choice: "value".into(),
                value: Some("+01:00".into()),
                confirmed_by: "Dynamics administrator".into(),
                at: "2026-10-14T00:00:00Z".into(),
                question: String::new(),
                carries: Vec::new(),
            },
        )]);
        let built = build_recipe(&narrowed, &hashes, &answers).unwrap();
        let outcome = common::prepare_both(&built.recipe, &files(&dir, &tables));
        let log = serde_json::to_value(outcome.log.unwrap()).unwrap();
        let label = if flow == Flow::Buying {
            "Settle vendor invoice"
        } else {
            "Settle customer invoice"
        };
        let events = log["events"].as_array().unwrap();
        assert_eq!(
            events.iter().filter(|e| e["type"] == label).count() as u64,
            truth.settled
        );
        assert!(
            events
                .iter()
                .all(|e| !e["type"].as_str().unwrap().contains("paid"))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn business_central_yields_every_planted_step() {
    use pm_testgen::dynamics::{Flow, Settings, business_central_tables, write_business_central};
    for (flow, id, p) in [
        (
            Flow::Buying,
            "d365-business-central-purchase-to-pay",
            "bc_p_",
        ),
        (
            Flow::Selling,
            "d365-business-central-order-to-cash",
            "bc_s_",
        ),
    ] {
        let dir = dir(id);
        let truth = write_business_central(&dir, flow, &Settings::default()).unwrap();
        let emitted = run(&pack(id), &files(&dir, &business_central_tables(flow)));
        let e = |rule: &str| emitted[&format!("{p}{rule}")];
        assert_eq!(e("order_created"), truth.orders);
        assert_eq!(e("order_has_line"), truth.lines);
        assert_eq!(e("approval_sent"), truth.sent);
        assert_eq!(e("approved"), truth.approved);
        assert_eq!(e("rejected"), truth.rejected);
        assert_eq!(
            ["order_created", "approval_sent", "approved", "rejected"]
                .iter()
                .map(|r| e(&format!("{r}_lines")))
                .sum::<u64>(),
            truth.order_line_steps
        );
        assert_eq!(e("goods_moved"), truth.posted_lines);
        assert_eq!(e("invoice_posted"), truth.invoices);
        assert_eq!(e("invoice_for_line"), truth.invoice_lines);
        assert_eq!(e("invoice_paid"), truth.paid);
        assert_eq!(
            e("invoice_posted_lines") + e("invoice_paid_lines"),
            truth.invoice_line_steps
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn sales_and_customer_service_yield_every_planted_step() {
    use pm_testgen::dynamics::{
        SALES_FILES, SERVICE_FILES, Settings, write_customer_service, write_sales,
    };
    let sales_dir = dir("sales");
    let truth = write_sales(&sales_dir, &Settings::default()).unwrap();
    let emitted = run(&pack("d365-sales"), &files(&sales_dir, &SALES_FILES));
    assert_eq!(emitted["dv_lead_created"], truth.leads);
    assert_eq!(emitted["dv_lead_disqualified"], truth.disqualified);
    assert_eq!(emitted["dv_opportunity_created"], truth.opportunities);
    assert_eq!(emitted["dv_opportunity_won"], truth.won);
    assert_eq!(emitted["dv_opportunity_lost"], truth.lost);
    assert_eq!(emitted["dv_activity"], truth.activities);
    std::fs::remove_dir_all(sales_dir).unwrap();

    let service_dir = dir("service");
    let truth = write_customer_service(&service_dir, &Settings::default()).unwrap();
    let emitted = run(
        &pack("d365-customer-service"),
        &files(&service_dir, &SERVICE_FILES),
    );
    assert_eq!(emitted["dv_case_created"], truth.cases);
    assert_eq!(emitted["dv_case_routed"], truth.routed);
    assert_eq!(emitted["dv_case_escalated"], truth.escalated);
    assert_eq!(emitted["dv_activity"], truth.activities);
    assert_eq!(emitted["dv_case_resolved"], truth.resolved);
    assert_eq!(emitted["dv_case_cancelled"], truth.cancelled);
    std::fs::remove_dir_all(service_dir).unwrap();
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
        w.write_record(headers.iter().map(|h| r.get(h).map_or("", String::as_str)))
            .unwrap();
    }
    w.flush().unwrap();
}

/// Approval Entry is shared by orders, quotes and invoices, including documents
/// with the same number. Only order approvals belong in an order process.
#[test]
fn business_central_ignores_approvals_for_other_document_types() {
    use pm_testgen::dynamics::{Flow, Settings, business_central_tables, write_business_central};
    for (flow, id, prefix) in [
        (
            Flow::Buying,
            "d365-business-central-purchase-to-pay",
            "bc_p_",
        ),
        (
            Flow::Selling,
            "d365-business-central-order-to-cash",
            "bc_s_",
        ),
    ] {
        let dir = dir(&format!("{id}-mixed-approvals"));
        let truth = write_business_central(&dir, flow, &Settings::default()).unwrap();
        let files = files(&dir, &business_central_tables(flow));
        let (headers, mut rows) = read(&files["Approval Entry"]);
        let template = rows[0].clone();
        for (index, (document_type, status)) in [
            ("Quote", "Approved"),
            ("Invoice", "Rejected"),
            ("Credit Memo", "Approved"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut row = template.clone();
            row.insert("Entry No.".into(), (900_000 + index).to_string());
            row.insert("Document Type".into(), document_type.into());
            row.insert("Status".into(), status.into());
            rows.push(row.clone());
            row.insert("Entry No.".into(), (910_000 + index).to_string());
            row.insert("Document No.".into(), format!("NONORDER-{index}"));
            rows.push(row);
        }
        write(&files["Approval Entry"], &headers, &rows);
        let report = check(&pack(id), &files).unwrap();
        assert!(report.gaps.is_empty(), "{:?}", report.gaps);
        assert!(
            !report
                .questions
                .iter()
                .any(|q| q.id == "link:approvals-have-orders")
        );
        let emitted = run(&pack(id), &files);
        assert_eq!(emitted[&format!("{prefix}approval_sent")], truth.sent);
        assert_eq!(emitted[&format!("{prefix}approved")], truth.approved);
        assert_eq!(emitted[&format!("{prefix}rejected")], truth.rejected);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// What Business Central's standard configuration-package export gives,
/// as found on a real CRONUS company: no SystemCreatedAt, quotes and
/// invoices beside the orders in the header and line tables, and fully
/// invoiced orders gone because order archiving is off.
#[test]
fn business_central_builds_from_a_standard_export() {
    use pm_testgen::dynamics::{Flow, Settings, business_central_tables, write_business_central};
    for (flow, id, p, header, line, posted, invoice_header) in [
        (
            Flow::Buying,
            "d365-business-central-purchase-to-pay",
            "bc_p_",
            "Purchase Header",
            "Purchase Line",
            "Purch. Rcpt. Line",
            "Purch. Inv. Header",
        ),
        (
            Flow::Selling,
            "d365-business-central-order-to-cash",
            "bc_s_",
            "Sales Header",
            "Sales Line",
            "Sales Shipment Line",
            "Sales Invoice Header",
        ),
    ] {
        let dir = dir(&format!("{id}-standard"));
        let truth = write_business_central(&dir, flow, &Settings::default()).unwrap();
        let files = files(&dir, &business_central_tables(flow));
        // No SystemCreatedAt in the three tables whose times the pack needs.
        // Nor the calculated Amount, an optional field.
        for table in [header, posted, invoice_header] {
            let (mut headers, rows) = read(&files[table]);
            headers.retain(|h| h != "SystemCreatedAt" && h != "Amount");
            write(&files[table], &headers, &rows);
        }
        // Orders whose receipts or shipments were posted: the first three are
        // deleted, as Business Central does once they are fully invoiced.
        let (posted_headers, posted_rows) = read(&files[posted]);
        let _ = posted_headers;
        let mut gone = posted_rows
            .iter()
            .map(|r| r["Order No."].clone())
            .collect::<Vec<_>>();
        gone.dedup();
        gone.truncate(3);
        let (headers, mut rows) = read(&files[header]);
        rows.retain(|r| !gone.contains(&r["No."]));
        // A quote and an invoice share numbers with the remaining orders.
        let mut quote = rows[0].clone();
        quote.insert("Document Type".into(), "Quote".into());
        let mut draft = rows[1].clone();
        draft.insert("Document Type".into(), "Invoice".into());
        rows.push(quote);
        rows.push(draft);
        write(&files[header], &headers, &rows);
        let (headers, mut lines) = read(&files[line]);
        lines.retain(|r| !gone.contains(&r["Document No."]));
        let mut quote_line = lines[0].clone();
        quote_line.insert("Document Type".into(), "Quote".into());
        lines.push(quote_line);
        write(&files[line], &headers, &lines);

        let pack = pack(id);
        let report = check(&pack, &files).unwrap();
        assert_eq!(report.status, "ready", "{:?}", report.gaps);
        assert!(
            report.data_problems.is_empty(),
            "{:?}",
            report.data_problems
        );
        let used = report
            .tables
            .iter()
            .filter(|t| !t.fallbacks_used.is_empty())
            .map(|t| (t.table.clone(), t.fallbacks_used.clone()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(used[header]["SystemCreatedAt"], "Order Date");
        assert_eq!(used[posted]["SystemCreatedAt"], "Posting Date");
        assert_eq!(used[invoice_header]["SystemCreatedAt"], "Posting Date");
        // The person is asked about the deleted orders; following them is
        // recommended.
        let question = report
            .questions
            .iter()
            .find(|q| q.id == "link:posted-lines-have-order-lines")
            .unwrap();
        assert!(question.text.contains("archiving"));
        assert_eq!(question.recommended.as_deref(), Some("keep"));

        let answers = BTreeMap::from([(
            "setting:offset".to_owned(),
            Answer {
                choice: "value".into(),
                value: Some("+01:00".into()),
                confirmed_by: "Business Central administrator".into(),
                at: "2026-10-16T00:00:00Z".into(),
                question: String::new(),
                carries: Vec::new(),
            },
        )]);
        let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
        let built = pm_ingest::pack::build_recipe_with(
            &pack,
            &hashes,
            &answers,
            &pm_ingest::pack::Exported::from_report(&report.tables),
        )
        .unwrap();
        assert!(
            built
                .changes
                .iter()
                .any(|c| c.contains("Order Date is used instead (to the day)"))
        );
        let outcome = common::prepare_both(&built.recipe, &files);
        assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
        let e = |rule: &str| outcome.report.rules[&format!("{p}{rule}")].emitted;
        // Every posted line is still followed; the deleted orders' lines come
        // from their posted documents; quotes and invoices are not orders.
        assert_eq!(e("goods_moved"), truth.posted_lines);
        assert_eq!(e("invoice_posted"), truth.invoices);
        assert!(e("line_from_posted") > 0);
        assert_eq!(e("order_created"), truth.orders - gone.len() as u64);
        let log = serde_json::to_value(outcome.log.as_ref().unwrap()).unwrap();
        let created = log["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|ev| ev["id"].as_str().unwrap().contains("order_created"))
            .unwrap();
        // A date-only time is midnight in the company's time zone.
        assert!(
            created["time"]
                .as_str()
                .unwrap()
                .ends_with("T00:00:00+01:00"),
            "{}",
            created["time"]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
