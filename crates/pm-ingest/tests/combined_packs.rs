//! Two packs combined with one link: Appian requests that name an SAP
//! purchase order, built into one connected log with SAP purchase to pay.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe, check};
use pm_testgen::sap_p2p::{Settings, TABLES, write};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn pack(id: &str) -> Pack {
    Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap()
}
fn answer(value: &str) -> Answer {
    Answer {
        choice: "value".into(),
        value: Some(value.into()),
        confirmed_by: "Procurement analyst".into(),
        at: "2026-10-14T00:00:00Z".into(),
        question: String::new(),
        carries: Vec::new(),
    }
}

/// The SAP export, plus Appian requests: most name an existing purchase
/// order, some name none, and one names an order without its leading zeros.
struct Fixture {
    dir: PathBuf,
    files: BTreeMap<String, PathBuf>,
    linked_requests: u64,
    linked_events: u64,
    /// Order items reached by the linked requests' events.
    linked_item_steps: u64,
    unmatched: u64,
}

fn fixture(name: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pm-combined-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write(&dir, &Settings::default()).unwrap();
    let column = |table: &str, name: &str| -> Vec<String> {
        let mut reader = csv::Reader::from_path(dir.join(format!("{table}.csv"))).unwrap();
        let index = reader
            .headers()
            .unwrap()
            .iter()
            .position(|h| h == name)
            .unwrap();
        reader
            .records()
            .map(|r| r.unwrap()[index].to_owned())
            .collect()
    };
    let orders = column("EKKO", "EBELN");
    let mut items = BTreeMap::<String, u64>::new();
    for order in column("EKPO", "EBELN") {
        *items.entry(order).or_default() += 1;
    }
    let mut records = String::from("id,createdOn,status,SAP_PO_Number\n");
    let mut events = String::from("id,recordId,eventTypeId,timestamp,user\n");
    let types = "id,value\n1,Request submitted\n2,Request approved\n";
    let (mut linked_requests, mut linked_events, mut linked_item_steps, mut unmatched) =
        (0, 0, 0, 0);
    let mut event = 0;
    for n in 0..40u64 {
        let order = match n % 10 {
            // No order named yet.
            0 => String::new(),
            // Saved without leading zeros: names no order in the export.
            1 if n == 1 => {
                unmatched += 1;
                orders[0].trim_start_matches('0').to_owned() + "9"
            }
            _ => orders[n as usize % orders.len()].clone(),
        };
        let day = 1 + n % 20;
        records.push_str(&format!(
            "{id},2026-01-{day:02}T09:00:00Z,Approved,{order}\n",
            id = 100 + n
        ));
        for (kind, hour) in [(1, 9), (2, 15)] {
            event += 1;
            events.push_str(&format!(
                "{event},{id},{kind},2026-01-{day:02}T{hour:02}:00:00Z,buyer1\n",
                id = 100 + n
            ));
        }
        if let Some(count) = items.get(&order) {
            linked_requests += 1;
            linked_events += 2;
            linked_item_steps += 2 * count;
        }
    }
    std::fs::write(dir.join("Record.csv"), records).unwrap();
    std::fs::write(dir.join("RecordEvent.csv"), events).unwrap();
    std::fs::write(dir.join("RecordEventType.csv"), types).unwrap();
    let files = TABLES
        .iter()
        .copied()
        .chain(["Record", "RecordEvent", "RecordEventType"])
        .map(|t| (t.to_owned(), dir.join(format!("{t}.csv"))))
        .collect();
    Fixture {
        dir,
        files,
        linked_requests,
        linked_events,
        linked_item_steps,
        unmatched,
    }
}

#[test]
fn appian_requests_reach_the_sap_orders_they_name() {
    let combined = pack("appian-record-events")
        .combine(&pack("sap-purchase-to-pay"), "Record", "purchase_order")
        .unwrap();
    assert_eq!(combined.id, "appian-record-events+sap-purchase-to-pay");
    assert_eq!(combined.case_object_type, "purchase order item");
    let f = fixture("appian-sap");

    // Until the company says which field holds the number, the link check
    // waits and the question is asked.
    let report = check(&combined, &f.files).unwrap();
    assert!(
        report
            .questions
            .iter()
            .any(|q| q.id == "setting:link_field")
    );

    let answers = BTreeMap::from([
        ("setting:offset".to_owned(), answer("+01:00")),
        ("setting:link_field".to_owned(), answer("SAP_PO_Number")),
    ]);
    let ready = combined.with_settings(&answers).unwrap();
    let report = check(&ready, &f.files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let link = report
        .links
        .iter()
        .find(|l| l.id == "link-matches")
        .unwrap();
    assert_eq!(link.matched, f.linked_requests);
    assert_eq!(link.missing, f.unmatched);

    let hashes = f
        .files
        .keys()
        .map(|t| (t.clone(), "0".repeat(64)))
        .collect();
    let built = build_recipe(&combined, &hashes, &answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, &f.files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    let emitted = |rule: &str| outcome.report.rules[rule].emitted;
    // Each request links to its order; its steps reach the order and the
    // order's items, which SAP's own steps follow.
    assert_eq!(emitted("link_purchase_order"), f.linked_requests);
    assert_eq!(emitted("appian_event_to_purchase_order"), f.linked_events);
    assert_eq!(
        emitted("appian_event_to_purchase_order_order_lines"),
        f.linked_item_steps
    );
    assert!(emitted("order_created") > 0);
    std::fs::remove_dir_all(&f.dir).unwrap();
}

#[test]
fn combining_explains_what_cannot_be_joined() {
    let appian = pack("appian-record-events");
    let sap = pack("sap-purchase-to-pay");
    let error = appian.combine(&sap, "Record", "invoice").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("offers no link target 'invoice'")
    );
    assert!(error.to_string().contains("purchase_order"));
    let error = appian
        .combine(&sap, "RecordEvent", "purchase_order")
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("not a table whose rows are items")
    );
    // Two packs that share a table name cannot be combined yet.
    let error = pack("salesforce-quote-approval")
        .combine(&pack("salesforce-contract-renewal"), "Quote", "opportunity")
        .unwrap_err();
    assert!(error.to_string().contains("both packs have the table"));
}

/// When people write the number they see (Oracle's purchase order number)
/// rather than the key the system links by (its internal header ID), the
/// number is linked to the order first.
#[test]
fn a_link_by_the_visible_number_reaches_the_internal_item() {
    use pm_testgen::oracle::{PURCHASING_TABLES, Settings, write_purchasing};
    let dir = std::env::temp_dir().join(format!("pm-combined-oracle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write_purchasing(&dir, &Settings::default()).unwrap();
    let read = |table: &str| -> Vec<csv::StringRecord> {
        csv::Reader::from_path(dir.join(format!("{table}.csv")))
            .unwrap()
            .records()
            .map(Result::unwrap)
            .collect()
    };
    // Header ID, then the order number people see.
    let orders = read("PO_HEADERS_ALL")
        .iter()
        .map(|r| (r[0].to_owned(), r[1].to_owned()))
        .collect::<Vec<_>>();
    let mut lines = BTreeMap::<String, u64>::new();
    for line in read("PO_LINES_ALL") {
        *lines.entry(line[1].to_owned()).or_default() += 1;
    }
    let mut records = String::from("id,createdOn,status,PO_Number\n");
    let mut events = String::from("id,recordId,eventTypeId,timestamp,user\n");
    let (mut linked, mut line_steps) = (0, 0);
    for n in 0..20u64 {
        let (header, number) = &orders[n as usize];
        records.push_str(&format!(
            "{},2026-01-02T09:00:00Z,Approved,{number}\n",
            100 + n
        ));
        events.push_str(&format!("{n},{},1,2026-01-02T09:00:00Z,buyer1\n", 100 + n));
        linked += 1;
        line_steps += lines[header];
    }
    std::fs::write(dir.join("Record.csv"), records).unwrap();
    std::fs::write(dir.join("RecordEvent.csv"), events).unwrap();
    std::fs::write(
        dir.join("RecordEventType.csv"),
        "id,value\n1,Request approved\n",
    )
    .unwrap();
    let files = PURCHASING_TABLES
        .iter()
        .copied()
        .chain(["Record", "RecordEvent", "RecordEventType"])
        .map(|t| (t.to_owned(), dir.join(format!("{t}.csv"))))
        .collect::<BTreeMap<_, _>>();
    let combined = pack("appian-record-events")
        .combine(
            &pack("oracle-ebs-purchase-to-pay"),
            "Record",
            "purchase_order",
        )
        .unwrap();
    let answers = BTreeMap::from([
        ("setting:offset".to_owned(), answer("-05:00")),
        ("setting:link_field".to_owned(), answer("PO_Number")),
    ]);
    let report = check(&combined.with_settings(&answers).unwrap(), &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let link = report
        .links
        .iter()
        .find(|l| l.id == "link-matches")
        .unwrap();
    assert_eq!((link.matched, link.missing), (linked, 0));
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(&combined, &hashes, &answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, &files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    let emitted = |rule: &str| outcome.report.rules[rule].emitted;
    assert_eq!(emitted("link_purchase_order"), linked);
    assert_eq!(
        emitted("link_number_of_purchase_order"),
        orders.len() as u64
    );
    assert_eq!(emitted("appian_event_to_purchase_order_item"), linked);
    assert_eq!(
        emitted("appian_event_to_purchase_order_ora_po_has_line"),
        line_steps
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
