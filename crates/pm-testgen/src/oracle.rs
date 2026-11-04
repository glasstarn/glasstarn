//! Made-up Oracle E-Business Suite and Fusion tables in the standard layout:
//! table and column names as in the APPS schema, internal IDs as numbers,
//! dates with their time (`2026-03-14 09:30:00`). Every record is invented;
//! the counts in the truth structs are known by construction.

use crate::Rng;
use crate::csv::{Table, save};
use chrono::{Duration, NaiveDate, NaiveDateTime};
use std::io;
use std::path::Path;

/// Settings for one generated set of tables.
#[derive(Debug, Clone)]
pub struct Settings {
    pub records: u64,
    pub seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            records: 200,
            seed: 29,
        }
    }
}

fn at(time: NaiveDateTime) -> String {
    time.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn start() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap()
}

/// What the purchasing tables contain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PurchasingTruth {
    pub orders: u64,
    pub lines: u64,
    pub requisition_lines: u64,
    pub submitted: u64,
    pub forwarded: u64,
    pub approved: u64,
    pub rejected: u64,
    /// Lines of orders that went through each approval action, counted per
    /// action (the steps that reach order lines).
    pub approval_line_steps: u64,
    pub cancelled_lines: u64,
    pub receipts: u64,
    pub deliveries: u64,
    pub returns: u64,
    pub invoices: u64,
    pub invoice_lines_matched: u64,
    pub cancelled_invoices: u64,
    pub holds: u64,
    pub releases: u64,
    pub payments: u64,
    /// Order lines reached by invoice, hold and payment steps, per step.
    pub invoice_line_steps: u64,
}

pub const PURCHASING_TABLES: [&str; 10] = [
    "PO_HEADERS_ALL",
    "PO_LINES_ALL",
    "PO_ACTION_HISTORY",
    "PO_REQUISITION_LINES_ALL",
    "PO_LINE_LOCATIONS_ALL",
    "RCV_TRANSACTIONS",
    "AP_INVOICES_ALL",
    "AP_INVOICE_LINES_ALL",
    "AP_HOLDS_ALL",
    "AP_INVOICE_PAYMENTS_ALL",
];

/// Writes the ten purchasing, receiving and payables tables.
pub fn write_purchasing(dir: &Path, settings: &Settings) -> io::Result<PurchasingTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x0AC1);
    let mut truth = PurchasingTruth::default();
    let mut headers = Table::new(
        "PO_HEADER_ID,SEGMENT1,VENDOR_ID,ORG_ID,CREATION_DATE,AUTHORIZATION_STATUS,TYPE_LOOKUP_CODE,APPROVED_DATE,CREATED_BY,CURRENCY_CODE,ATTRIBUTE1",
    );
    let mut lines = Table::new(
        "PO_LINE_ID,PO_HEADER_ID,LINE_NUM,QUANTITY,CREATION_DATE,CANCEL_FLAG,CANCEL_DATE,ITEM_ID,UNIT_PRICE,CATEGORY_ID",
    );
    let mut actions =
        Table::new("OBJECT_ID,OBJECT_TYPE_CODE,SEQUENCE_NUM,ACTION_CODE,ACTION_DATE,EMPLOYEE_ID");
    let mut requisitions = Table::new(
        "REQUISITION_LINE_ID,REQUISITION_HEADER_ID,LINE_LOCATION_ID,CREATION_DATE,QUANTITY,ITEM_ID",
    );
    let mut locations =
        Table::new("LINE_LOCATION_ID,PO_LINE_ID,PO_HEADER_ID,NEED_BY_DATE,PROMISED_DATE");
    let mut receiving = Table::new(
        "TRANSACTION_ID,TRANSACTION_TYPE,TRANSACTION_DATE,PO_HEADER_ID,PO_LINE_ID,QUANTITY,CREATION_DATE",
    );
    let mut invoices = Table::new(
        "INVOICE_ID,INVOICE_NUM,VENDOR_ID,CREATION_DATE,INVOICE_AMOUNT,CANCELLED_DATE,INVOICE_DATE,ORG_ID,INVOICE_CURRENCY_CODE",
    );
    let mut invoice_lines =
        Table::new("INVOICE_ID,LINE_NUMBER,LINE_TYPE_LOOKUP_CODE,PO_HEADER_ID,PO_LINE_ID,AMOUNT");
    let mut holds = Table::new(
        "HOLD_ID,INVOICE_ID,HOLD_LOOKUP_CODE,HOLD_DATE,RELEASE_LOOKUP_CODE,LAST_UPDATE_DATE",
    );
    let mut payments = Table::new(
        "INVOICE_PAYMENT_ID,INVOICE_ID,CHECK_ID,CREATION_DATE,REVERSAL_FLAG,AMOUNT,ACCOUNTING_DATE",
    );
    let (mut line_id, mut location_id, mut requisition_id, mut transaction_id) =
        (5_000u64, 8_000u64, 3_000u64, 70_000u64);
    let (mut invoice_id, mut hold_id, mut payment_id) = (90_000u64, 400u64, 600u64);
    for n in 0..settings.records {
        truth.orders += 1;
        let header = (1_000 + n).to_string();
        let created = start()
            + Duration::days(rng.below(80) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let count = 1 + rng.below(3);
        let mut order_lines = Vec::new();
        for number in 1..=count {
            line_id += 1;
            truth.lines += 1;
            order_lines.push(line_id.to_string());
            location_id += 1;
            locations.push(&[
                &location_id.to_string(),
                &line_id.to_string(),
                &header,
                &at(created + Duration::days(14)),
                "",
            ]);
            if n % 3 == 0 {
                requisition_id += 1;
                truth.requisition_lines += 1;
                requisitions.push(&[
                    &requisition_id.to_string(),
                    &(2_000 + n).to_string(),
                    &location_id.to_string(),
                    &at(created - Duration::days(2)),
                    "5",
                    "1001",
                ]);
            }
            lines.push(&[
                &line_id.to_string(),
                &header,
                &number.to_string(),
                "5",
                &at(created),
                "N",
                "",
                "1001",
                "40.00",
                "12",
            ]);
        }
        // Approval: submitted, sometimes forwarded, approved or rejected.
        let mut time = created + Duration::hours(1);
        let mut sequence = 0;
        let mut act = |actions: &mut Table, code: &str, time: NaiveDateTime| {
            actions.push(&[&header, "PO", &sequence.to_string(), code, &at(time), "77"]);
            sequence += 1;
        };
        act(&mut actions, "SUBMIT", time);
        truth.submitted += 1;
        if rng.chance(0.3) {
            time += Duration::hours(4 + rng.below(24) as i64);
            act(&mut actions, "FORWARD", time);
            truth.forwarded += 1;
        }
        time += Duration::hours(2 + rng.below(48) as i64);
        let approved = rng.chance(0.9);
        act(
            &mut actions,
            if approved { "APPROVE" } else { "REJECT" },
            time,
        );
        if approved {
            truth.approved += 1;
        } else {
            truth.rejected += 1;
        }
        let steps = 2 + u64::from(sequence == 3);
        // Creation, and each approval action, reach every line.
        truth.approval_line_steps += (steps + 1) * count;
        // A pending action row with no code yet is not a step.
        actions.push(&[&header, "PO", &sequence.to_string(), "", "", "78"]);
        headers.push(&[
            &header,
            &(100_000 + n).to_string(),
            &(500 + n % 20).to_string(),
            "204",
            &at(created),
            if approved { "APPROVED" } else { "REJECTED" },
            "STANDARD",
            &if approved { at(time) } else { String::new() },
            "1318",
            "USD",
            ["Capex", ""][n as usize % 2],
        ]);
        if !approved {
            continue;
        }
        let mut invoice_lines_for: Vec<String> = Vec::new();
        for line in &order_lines {
            if rng.chance(0.05) {
                truth.cancelled_lines += 1;
                lines_cancel(&mut lines, line, time + Duration::days(1));
                continue;
            }
            let mut received = time + Duration::days(3 + rng.below(14) as i64);
            transaction_id += 1;
            truth.receipts += 1;
            receiving.push(&[
                &transaction_id.to_string(),
                "RECEIVE",
                &at(received),
                &header,
                line,
                "5",
                &at(received),
            ]);
            received += Duration::hours(2);
            transaction_id += 1;
            truth.deliveries += 1;
            receiving.push(&[
                &transaction_id.to_string(),
                "DELIVER",
                &at(received),
                &header,
                line,
                "5",
                &at(received),
            ]);
            if rng.chance(0.05) {
                transaction_id += 1;
                truth.returns += 1;
                receiving.push(&[
                    &transaction_id.to_string(),
                    "RETURN TO VENDOR",
                    &at(received + Duration::days(1)),
                    &header,
                    line,
                    "1",
                    &at(received + Duration::days(1)),
                ]);
            }
            invoice_lines_for.push(line.clone());
        }
        if invoice_lines_for.is_empty() || !rng.chance(0.85) {
            continue;
        }
        invoice_id += 1;
        truth.invoices += 1;
        let invoice = invoice_id.to_string();
        let recorded = time + Duration::days(10 + rng.below(10) as i64);
        let cancelled = rng.chance(0.04);
        truth.cancelled_invoices += u64::from(cancelled);
        invoices.push(&[
            &invoice,
            &format!("INV-{invoice}"),
            &(500 + n % 20).to_string(),
            &at(recorded),
            "200.00",
            &if cancelled {
                at(recorded + Duration::days(1))
            } else {
                String::new()
            },
            &at(recorded),
            "204",
            "USD",
        ]);
        let reached = invoice_lines_for.len() as u64;
        let mut invoice_steps = 1 + u64::from(cancelled);
        for (number, line) in invoice_lines_for.iter().enumerate() {
            truth.invoice_lines_matched += 1;
            invoice_lines.push(&[
                &invoice,
                &(number + 1).to_string(),
                "ITEM",
                &header,
                line,
                "200.00",
            ]);
        }
        // A tax line matched to nothing.
        invoice_lines.push(&[&invoice, "99", "TAX", "", "", "16.00"]);
        if cancelled {
            truth.invoice_line_steps += invoice_steps * reached;
            continue;
        }
        let mut paid_at = recorded + Duration::days(20 + rng.below(25) as i64);
        if rng.chance(0.2) {
            hold_id += 1;
            truth.holds += 1;
            invoice_steps += 1;
            let released = rng.chance(0.8);
            if released {
                truth.releases += 1;
                invoice_steps += 1;
            }
            holds.push(&[
                &hold_id.to_string(),
                &invoice,
                "QTY REC",
                &at(recorded + Duration::hours(1)),
                if released { "VARIANCE CORRECTED" } else { "" },
                &at(recorded + Duration::days(3)),
            ]);
            if !released {
                truth.invoice_line_steps += invoice_steps * reached;
                continue;
            }
            paid_at += Duration::days(3);
        }
        if rng.chance(0.85) {
            payment_id += 1;
            truth.payments += 1;
            invoice_steps += 1;
            payments.push(&[
                &payment_id.to_string(),
                &invoice,
                &(7_000 + payment_id).to_string(),
                &at(paid_at),
                "N",
                "200.00",
                &at(paid_at),
            ]);
        }
        truth.invoice_line_steps += invoice_steps * reached;
    }
    save(
        dir,
        &[
            ("PO_HEADERS_ALL", &headers),
            ("PO_LINES_ALL", &lines),
            ("PO_ACTION_HISTORY", &actions),
            ("PO_REQUISITION_LINES_ALL", &requisitions),
            ("PO_LINE_LOCATIONS_ALL", &locations),
            ("RCV_TRANSACTIONS", &receiving),
            ("AP_INVOICES_ALL", &invoices),
            ("AP_INVOICE_LINES_ALL", &invoice_lines),
            ("AP_HOLDS_ALL", &holds),
            ("AP_INVOICE_PAYMENTS_ALL", &payments),
        ],
    )?;
    Ok(truth)
}

/// Marks an already written order line as cancelled.
fn lines_cancel(lines: &mut Table, line: &str, time: NaiveDateTime) {
    lines.update(|row| {
        if row.split(',').next() == Some(line) {
            let mut fields = row.split(',').map(str::to_owned).collect::<Vec<_>>();
            fields[5] = "Y".into();
            fields[6] = at(time);
            *row = fields.join(",");
        }
    });
}

/// What the order-to-cash tables contain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SellingTruth {
    pub orders: u64,
    pub lines: u64,
    pub booked: u64,
    pub holds: u64,
    pub releases: u64,
    pub shipped: u64,
    pub cancelled: u64,
    pub invoices: u64,
    pub invoice_lines: u64,
    pub payments: u64,
    /// Order lines reached by order-level steps (create, book, holds).
    pub order_line_steps: u64,
    /// Order lines reached by invoice and payment steps.
    pub invoice_line_steps: u64,
}

/// The order-to-cash tables, E-Business Suite or Fusion.
pub fn selling_tables(cloud: bool) -> Vec<&'static str> {
    if cloud {
        vec![
            "DOO_HEADERS_ALL",
            "DOO_FULFILL_LINES_ALL",
            "RA_CUSTOMER_TRX_ALL",
            "RA_CUSTOMER_TRX_LINES_ALL",
            "AR_RECEIVABLE_APPLICATIONS_ALL",
        ]
    } else {
        vec![
            "OE_ORDER_HEADERS_ALL",
            "OE_ORDER_LINES_ALL",
            "OE_ORDER_HOLDS_ALL",
            "OE_HOLD_RELEASES",
            "RA_CUSTOMER_TRX_ALL",
            "RA_CUSTOMER_TRX_LINES_ALL",
            "AR_RECEIVABLE_APPLICATIONS_ALL",
        ]
    }
}

/// Writes the order-to-cash tables; Fusion keeps the billed fulfillment line
/// in INTERFACE_LINE_ATTRIBUTE5, E-Business Suite the order line in
/// INTERFACE_LINE_ATTRIBUTE6.
pub fn write_selling(dir: &Path, cloud: bool, settings: &Settings) -> io::Result<SellingTruth> {
    let mut rng = Rng::new(settings.seed ^ if cloud { 0x0F05 } else { 0x0EB5 });
    let mut truth = SellingTruth::default();
    let mut headers = Table::new(if cloud {
        "HEADER_ID,ORDER_NUMBER,CREATION_DATE,SUBMITTED_DATE,STATUS_CODE,SOLD_TO_PARTY_ID,ORG_ID"
    } else {
        "HEADER_ID,ORDER_NUMBER,ORG_ID,CREATION_DATE,BOOKED_FLAG,BOOKED_DATE,SOLD_TO_ORG_ID,FLOW_STATUS_CODE,ORDER_TYPE_ID"
    });
    let mut lines = Table::new(if cloud {
        "FULFILL_LINE_ID,HEADER_ID,ORDERED_QTY,CREATION_DATE,ACTUAL_SHIP_DATE,CANCELED_FLAG,LAST_UPDATE_DATE,FULFILL_LINE_NUMBER,INVENTORY_ITEM_ID,STATUS_CODE"
    } else {
        "LINE_ID,HEADER_ID,ORDERED_QUANTITY,CREATION_DATE,ACTUAL_SHIPMENT_DATE,CANCELLED_FLAG,LAST_UPDATE_DATE,LINE_NUMBER,INVENTORY_ITEM_ID,FLOW_STATUS_CODE"
    });
    let mut holds = Table::new(
        "ORDER_HOLD_ID,HEADER_ID,LINE_ID,CREATION_DATE,RELEASED_FLAG,HOLD_RELEASE_ID,HOLD_SOURCE_ID",
    );
    let mut releases = Table::new("HOLD_RELEASE_ID,CREATION_DATE,RELEASE_REASON_CODE");
    let mut invoices = Table::new(
        "CUSTOMER_TRX_ID,TRX_NUMBER,CREATION_DATE,TRX_DATE,BILL_TO_CUSTOMER_ID,CUST_TRX_TYPE_ID",
    );
    let mut invoice_lines = Table::new(
        "CUSTOMER_TRX_LINE_ID,CUSTOMER_TRX_ID,LINE_TYPE,INTERFACE_LINE_CONTEXT,INTERFACE_LINE_ATTRIBUTE5,INTERFACE_LINE_ATTRIBUTE6,EXTENDED_AMOUNT",
    );
    let mut applications = Table::new(
        "RECEIVABLE_APPLICATION_ID,APPLIED_CUSTOMER_TRX_ID,STATUS,DISPLAY,CREATION_DATE,CASH_RECEIPT_ID,AMOUNT_APPLIED,APPLY_DATE",
    );
    let (mut line_id, mut hold_id, mut trx_id, mut trx_line_id, mut application_id) =
        (20_000u64, 100u64, 50_000u64, 60_000u64, 1_000u64);
    for n in 0..settings.records {
        truth.orders += 1;
        let header = (3_000 + n).to_string();
        let created = start()
            + Duration::days(rng.below(80) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let booked_at = created + Duration::hours(1 + rng.below(24) as i64);
        let booked = rng.chance(0.95);
        truth.booked += u64::from(booked);
        let count = 1 + rng.below(3);
        let mut order_steps = 1 + u64::from(booked);
        let mut shipped_lines = Vec::new();
        let mut line_rows = Vec::new();
        for number in 1..=count {
            line_id += 1;
            truth.lines += 1;
            let id = line_id.to_string();
            let cancelled = booked && rng.chance(0.04);
            let ship = booked && !cancelled && rng.chance(0.9);
            let ship_at = booked_at + Duration::days(2 + rng.below(10) as i64);
            truth.cancelled += u64::from(cancelled);
            truth.shipped += u64::from(ship);
            if ship {
                shipped_lines.push((id.clone(), ship_at));
            }
            line_rows.push([
                id.clone(),
                header.clone(),
                "3".to_owned(),
                at(created),
                if ship { at(ship_at) } else { String::new() },
                if cancelled {
                    "Y".to_owned()
                } else {
                    "N".to_owned()
                },
                at(booked_at + Duration::days(1)),
                number.to_string(),
                "2001".to_owned(),
                if ship {
                    "SHIPPED".to_owned()
                } else {
                    "AWAITING_SHIPPING".to_owned()
                },
            ]);
        }
        for row in &line_rows {
            lines.push(&row.iter().map(String::as_str).collect::<Vec<_>>());
        }
        if !cloud && booked && rng.chance(0.15) {
            hold_id += 1;
            truth.holds += 1;
            order_steps += 1;
            let released = rng.chance(0.85);
            if released {
                truth.releases += 1;
                order_steps += 1;
                releases.push(&[
                    &(9_000 + hold_id).to_string(),
                    &at(booked_at + Duration::hours(6)),
                    "CREDIT_OK",
                ]);
            }
            holds.push(&[
                &hold_id.to_string(),
                &header,
                "",
                &at(booked_at + Duration::minutes(5)),
                if released { "Y" } else { "N" },
                &if released {
                    (9_000 + hold_id).to_string()
                } else {
                    String::new()
                },
                "1",
            ]);
        }
        truth.order_line_steps += order_steps * count;
        if cloud {
            headers.push(&[
                &header,
                &(56_000 + n).to_string(),
                &at(created),
                &if booked { at(booked_at) } else { String::new() },
                "OPEN",
                "8800",
                "204",
            ]);
        } else {
            headers.push(&[
                &header,
                &(56_000 + n).to_string(),
                "204",
                &at(created),
                if booked { "Y" } else { "N" },
                &if booked { at(booked_at) } else { String::new() },
                "8800",
                "BOOKED",
                "1430",
            ]);
        }
        if shipped_lines.is_empty() || !rng.chance(0.9) {
            continue;
        }
        trx_id += 1;
        truth.invoices += 1;
        let invoice = trx_id.to_string();
        let billed = shipped_lines.iter().map(|l| l.1).max().unwrap() + Duration::days(1);
        invoices.push(&[
            &invoice,
            &format!("{trx_id}"),
            &at(billed),
            &at(billed),
            "8800",
            "1",
        ]);
        for (line, _) in &shipped_lines {
            trx_line_id += 1;
            truth.invoice_lines += 1;
            let (five, six) = if cloud {
                (line.as_str(), "")
            } else {
                ("", line.as_str())
            };
            invoice_lines.push(&[
                &trx_line_id.to_string(),
                &invoice,
                "LINE",
                if cloud { "DOO_ORDER" } else { "ORDER ENTRY" },
                five,
                six,
                "120.00",
            ]);
        }
        // Tax lines name no order line.
        trx_line_id += 1;
        invoice_lines.push(&[
            &trx_line_id.to_string(),
            &invoice,
            "TAX",
            "",
            "",
            "",
            "9.60",
        ]);
        let mut invoice_steps = 1;
        if rng.chance(0.8) {
            application_id += 1;
            truth.payments += 1;
            invoice_steps += 1;
            let paid = billed + Duration::days(15 + rng.below(30) as i64);
            applications.push(&[
                &application_id.to_string(),
                &invoice,
                "APP",
                "Y",
                &at(paid),
                "4400",
                "360.00",
                &at(paid),
            ]);
        }
        // An unapplied row on the same receipt is not a payment step.
        application_id += 1;
        applications.push(&[
            &application_id.to_string(),
            &invoice,
            "UNAPP",
            "Y",
            &at(billed + Duration::days(60)),
            "4400",
            "0.00",
            "",
        ]);
        truth.invoice_line_steps += invoice_steps * shipped_lines.len() as u64;
    }
    let names = selling_tables(cloud);
    let mut tables: Vec<(&str, &Table)> = vec![(names[0], &headers), (names[1], &lines)];
    if !cloud {
        tables.push(("OE_ORDER_HOLDS_ALL", &holds));
        tables.push(("OE_HOLD_RELEASES", &releases));
    }
    tables.push(("RA_CUSTOMER_TRX_ALL", &invoices));
    tables.push(("RA_CUSTOMER_TRX_LINES_ALL", &invoice_lines));
    tables.push(("AR_RECEIVABLE_APPLICATIONS_ALL", &applications));
    save(dir, &tables)?;
    Ok(truth)
}
