//! Made-up Microsoft Dynamics 365 Finance and Operations tables: orders,
//! lines linked by their lot ID (InventTransId), workflow history,
//! confirmations, receipts or packing slips, posted invoices and the open or
//! settled items. Date-times in UTC (`2026-01-05T14:30:00Z`), dates as
//! `YYYY-MM-DD`. Every record is invented; the counts in the truth struct are
//! known by construction.

use crate::Rng;
use crate::csv::{Table, save};
use chrono::{Duration, NaiveDate, NaiveDateTime};
use std::io;
use std::path::Path;

/// Buying (purchase to pay) or selling (order to cash).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Buying,
    Selling,
}

impl Flow {
    /// Order, lines, workflow, confirmations, receipts, invoices, invoice
    /// lines, open and settled items.
    pub fn tables(self) -> [&'static str; 8] {
        match self {
            Self::Buying => [
                "PurchTable",
                "PurchLine",
                "WorkflowTrackingTable",
                "VendPurchOrderJour",
                "VendPackingSlipTrans",
                "VendInvoiceJour",
                "VendInvoiceTrans",
                "VendTrans",
            ],
            Self::Selling => [
                "SalesTable",
                "SalesLine",
                "WorkflowTrackingTable",
                "CustConfirmJour",
                "CustPackingSlipTrans",
                "CustInvoiceJour",
                "CustInvoiceTrans",
                "CustTrans",
            ],
        }
    }
}

/// Settings for one generated extract.
#[derive(Debug, Clone)]
pub struct Settings {
    pub records: u64,
    pub seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            records: 200,
            seed: 43,
        }
    }
}

/// What the extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Truth {
    pub orders: u64,
    pub lines: u64,
    pub workflow_steps: u64,
    pub confirmations: u64,
    /// Order lines reached by order-level steps.
    pub order_line_steps: u64,
    pub receipt_lines: u64,
    pub invoices: u64,
    pub invoice_lines: u64,
    pub settled: u64,
    /// Order lines reached by invoice and settlement steps.
    pub invoice_line_steps: u64,
}

fn utc(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Writes the eight tables of `flow`.
pub fn write_finance_operations(dir: &Path, flow: Flow, settings: &Settings) -> io::Result<Truth> {
    let buying = flow == Flow::Buying;
    let mut rng = Rng::new(settings.seed ^ if buying { 0xD365 } else { 0xD366 });
    let mut truth = Truth::default();
    let mut orders = Table::new(if buying {
        "PurchId,OrderAccount,DataAreaId,CreatedDateTime,PurchStatus,DocumentState"
    } else {
        "SalesId,CustAccount,DataAreaId,CreatedDateTime,SalesStatus,SalesType"
    });
    let mut lines = Table::new(if buying {
        "PurchId,LineNumber,InventTransId,ItemId,PurchQty,LineAmount,CreatedDateTime"
    } else {
        "SalesId,LineNumber,InventTransId,ItemId,SalesQty,LineAmount,CreatedDateTime"
    });
    let mut workflow = Table::new(if buying {
        "RecId,PurchId,TrackingType,CreatedDateTime,TrackingContext,User"
    } else {
        "RecId,SalesId,TrackingType,CreatedDateTime,TrackingContext,User"
    });
    let mut confirmations = Table::new(if buying {
        "PurchId,CreatedDateTime,PurchOrderDocNum"
    } else {
        "SalesId,CreatedDateTime,ConfirmDocNum"
    });
    let mut receipts =
        Table::new("RecId,PackingSlipId,InventTransId,CreatedDateTime,Qty,DeliveryDate");
    let mut invoices = Table::new(if buying {
        "InternalInvoiceId,InvoiceId,LedgerVoucher,CreatedDateTime,InvoiceDate,PurchId,InvoiceAmount"
    } else {
        "InvoiceId,LedgerVoucher,CreatedDateTime,InvoiceDate,SalesId,InvoiceAmount"
    });
    let mut invoice_lines = Table::new(if buying {
        "InternalInvoiceId,LineNum,InventTransId,Qty,LineAmount"
    } else {
        "InvoiceId,LineNum,InventTransId,Qty,LineAmount"
    });
    let mut items = Table::new("RecId,Voucher,AccountNum,Closed,Invoice,TransDate,AmountCur");
    let start = NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap();
    let prefix = if buying { "PO" } else { "SO" };
    let (mut rec, mut lot) = (5_637_000_000u64, 0u64);
    for n in 0..settings.records {
        truth.orders += 1;
        let order = format!("{prefix}-{:06}", 100 + n);
        let created = start
            + Duration::days(rng.below(80) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let partner = format!("{}{:04}", if buying { "V" } else { "C" }, n % 20);
        let count = 1 + rng.below(3);
        let mut order_lots = Vec::new();
        for number in 1..=count {
            lot += 1;
            truth.lines += 1;
            let id = format!("LOT{lot:07}");
            lines.push(&[
                &order,
                &number.to_string(),
                &id,
                "A0001",
                "3",
                "300.00",
                &utc(created),
            ]);
            order_lots.push(id);
        }
        let mut at = created;
        let mut steps = 1;
        let mut track = |workflow: &mut Table, kind: &str, at: NaiveDateTime| {
            rec += 1;
            workflow.push(&[
                &rec.to_string(),
                &order,
                kind,
                &utc(at),
                "PurchTableApproval",
                "anna",
            ]);
        };
        // Creation of the workflow is not a step; submission and the
        // decision are.
        track(&mut workflow, "Creation", at);
        at += Duration::hours(1);
        track(&mut workflow, "Submission", at);
        at += Duration::hours(2 + rng.below(48) as i64);
        let approved = rng.chance(0.9);
        track(
            &mut workflow,
            if approved { "Approval" } else { "Rejection" },
            at,
        );
        truth.workflow_steps += 2;
        steps += 2;
        if approved {
            at += Duration::hours(1 + rng.below(8) as i64);
            truth.confirmations += 1;
            steps += 1;
            confirmations.push(&[&order, &utc(at), &format!("C-{n:05}")]);
        }
        truth.order_line_steps += steps * count;
        orders.push(&[
            &order,
            &partner,
            "usmf",
            &utc(created),
            if approved { "Backorder" } else { "None" },
            if approved { "Confirmed" } else { "Rejected" },
        ]);
        if !approved || !rng.chance(0.9) {
            continue;
        }
        let received = at + Duration::days(2 + rng.below(10) as i64);
        for id in &order_lots {
            rec += 1;
            truth.receipt_lines += 1;
            receipts.push(&[
                &rec.to_string(),
                &format!("PS-{n:05}"),
                id,
                &utc(received),
                "3",
                &received.date().to_string(),
            ]);
        }
        if !rng.chance(0.85) {
            continue;
        }
        truth.invoices += 1;
        let invoiced = received + Duration::days(1 + rng.below(7) as i64);
        let internal = format!("II-{n:05}");
        let voucher = format!("APINV{n:06}");
        if buying {
            invoices.push(&[
                &internal,
                &format!("VINV{n}"),
                &voucher,
                &utc(invoiced),
                &invoiced.date().to_string(),
                &order,
                "900.00",
            ]);
        } else {
            invoices.push(&[
                &internal,
                &voucher,
                &utc(invoiced),
                &invoiced.date().to_string(),
                &order,
                "900.00",
            ]);
        }
        for (number, id) in order_lots.iter().enumerate() {
            truth.invoice_lines += 1;
            invoice_lines.push(&[&internal, &(number + 1).to_string(), id, "3", "300.00"]);
        }
        let settled = rng.chance(0.8);
        rec += 1;
        items.push(&[
            &rec.to_string(),
            &voucher,
            &partner,
            &if settled {
                (invoiced + Duration::days(20 + rng.below(20) as i64))
                    .date()
                    .to_string()
            } else {
                String::new()
            },
            &format!("VINV{n}"),
            &invoiced.date().to_string(),
            "900.00",
        ]);
        truth.settled += u64::from(settled);
        truth.invoice_line_steps += (1 + u64::from(settled)) * count;
    }
    let names = flow.tables();
    save(
        dir,
        &[
            (names[0], &orders),
            (names[1], &lines),
            (names[2], &workflow),
            (names[3], &confirmations),
            (names[4], &receipts),
            (names[5], &invoices),
            (names[6], &invoice_lines),
            (names[7], &items),
        ],
    )?;
    Ok(truth)
}

/// The Business Central tables of `flow`: orders, lines, approval entries,
/// posted receipt or shipment lines, posted invoices and their lines, and
/// ledger entries.
pub fn business_central_tables(flow: Flow) -> [&'static str; 7] {
    match flow {
        Flow::Buying => [
            "Purchase Header",
            "Purchase Line",
            "Approval Entry",
            "Purch. Rcpt. Line",
            "Purch. Inv. Header",
            "Purch. Inv. Line",
            "Vendor Ledger Entry",
        ],
        Flow::Selling => [
            "Sales Header",
            "Sales Line",
            "Approval Entry",
            "Sales Shipment Line",
            "Sales Invoice Header",
            "Sales Invoice Line",
            "Cust. Ledger Entry",
        ],
    }
}

/// What the Business Central extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CentralTruth {
    pub orders: u64,
    pub lines: u64,
    pub sent: u64,
    pub approved: u64,
    pub rejected: u64,
    pub order_line_steps: u64,
    pub posted_lines: u64,
    pub invoices: u64,
    pub invoice_lines: u64,
    pub paid: u64,
    pub invoice_line_steps: u64,
}

/// Writes the seven Business Central tables of `flow`.
pub fn write_business_central(
    dir: &Path,
    flow: Flow,
    settings: &Settings,
) -> io::Result<CentralTruth> {
    let buying = flow == Flow::Buying;
    let mut rng = Rng::new(settings.seed ^ if buying { 0xBC1 } else { 0xBC2 });
    let mut truth = CentralTruth::default();
    let mut orders = Table::new(if buying {
        "No.,Document Type,Buy-from Vendor No.,SystemCreatedAt,Order Date,Status,Amount"
    } else {
        "No.,Document Type,Sell-to Customer No.,SystemCreatedAt,Order Date,Status,Amount"
    });
    let mut lines = Table::new(
        "Document Type,Document No.,Line No.,No.,Quantity,Type,Line Amount,SystemCreatedAt",
    );
    let mut approvals = Table::new(
        "Entry No.,Table ID,Document Type,Document No.,Status,Date-Time Sent for Approval,Last Date-Time Modified,Sequence No.,Approver ID",
    );
    let mut posted = Table::new(
        "Document No.,Line No.,Order No.,Order Line No.,SystemCreatedAt,Quantity,Posting Date",
    );
    let mut invoices = Table::new(if buying {
        "No.,Order No.,SystemCreatedAt,Posting Date,Amount,Vendor Invoice No."
    } else {
        "No.,Order No.,SystemCreatedAt,Posting Date,Amount,External Document No."
    });
    let mut invoice_lines =
        Table::new("Document No.,Line No.,Order No.,Order Line No.,Quantity,Amount");
    let mut ledger =
        Table::new("Entry No.,Document Type,Document No.,Open,Closed at Date,Amount,Due Date");
    let table_id = if buying { "38" } else { "36" };
    let start = NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap();
    let mut entry = 0u64;
    for n in 0..settings.records {
        truth.orders += 1;
        let order = (106_000 + n).to_string();
        let created = start
            + Duration::days(rng.below(80) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let count = 1 + rng.below(3);
        truth.lines += count;
        for line in 1..=count {
            lines.push(&[
                "Order",
                &order,
                &(line * 10_000).to_string(),
                "1000",
                "2",
                "Item",
                "200.00",
                &utc(created),
            ]);
        }
        let mut steps = 1;
        let mut approved = true;
        let mut at = created;
        if rng.chance(0.5) {
            entry += 1;
            truth.sent += 1;
            steps += 2;
            at += Duration::hours(1);
            let decided = at + Duration::hours(2 + rng.below(48) as i64);
            approved = rng.chance(0.85);
            if approved {
                truth.approved += 1;
            } else {
                truth.rejected += 1;
            }
            approvals.push(&[
                &entry.to_string(),
                table_id,
                "Order",
                &order,
                if approved { "Approved" } else { "Rejected" },
                &utc(at),
                &utc(decided),
                "1",
                "ALICE",
            ]);
            at = decided;
        }
        truth.order_line_steps += steps * count;
        orders.push(&[
            &order,
            "Order",
            &format!("P{:05}", n % 20),
            &utc(created),
            &created.date().to_string(),
            if approved { "Released" } else { "Open" },
            "400.00",
        ]);
        if !approved || !rng.chance(0.9) {
            continue;
        }
        let moved = at + Duration::days(2 + rng.below(10) as i64);
        for line in 1..=count {
            truth.posted_lines += 1;
            posted.push(&[
                &format!("R{n:05}"),
                &(line * 10_000).to_string(),
                &order,
                &(line * 10_000).to_string(),
                &utc(moved),
                "2",
                &moved.date().to_string(),
            ]);
        }
        if !rng.chance(0.85) {
            continue;
        }
        truth.invoices += 1;
        let invoice = format!("PI{n:05}");
        let invoiced = moved + Duration::days(1 + rng.below(7) as i64);
        invoices.push(&[
            &invoice,
            &order,
            &utc(invoiced),
            &invoiced.date().to_string(),
            "400.00",
            &format!("EXT{n}"),
        ]);
        for line in 1..=count {
            truth.invoice_lines += 1;
            invoice_lines.push(&[
                &invoice,
                &(line * 10_000).to_string(),
                &order,
                &(line * 10_000).to_string(),
                "2",
                "200.00",
            ]);
        }
        let paid = rng.chance(0.8);
        entry += 1;
        ledger.push(&[
            &entry.to_string(),
            "Invoice",
            &invoice,
            if paid { "false" } else { "true" },
            &if paid {
                (invoiced + Duration::days(25)).date().to_string()
            } else {
                "0001-01-01".to_owned()
            },
            "400.00",
            &(invoiced + Duration::days(30)).date().to_string(),
        ]);
        truth.paid += u64::from(paid);
        truth.invoice_line_steps += (1 + u64::from(paid)) * count;
    }
    let names = business_central_tables(flow);
    save(
        dir,
        &[
            (names[0], &orders),
            (names[1], &lines),
            (names[2], &approvals),
            (names[3], &posted),
            (names[4], &invoices),
            (names[5], &invoice_lines),
            (names[6], &ledger),
        ],
    )?;
    Ok(truth)
}

/// What the Dynamics 365 Sales extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SalesTruth {
    pub leads: u64,
    pub disqualified: u64,
    pub opportunities: u64,
    pub won: u64,
    pub lost: u64,
    pub activities: u64,
}

pub const SALES_FILES: [&str; 3] = ["lead", "opportunity", "activitypointer"];

/// Writes lead, opportunity and activitypointer.
pub fn write_sales(dir: &Path, settings: &Settings) -> io::Result<SalesTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x5A1E);
    let mut truth = SalesTruth::default();
    let mut leads = Table::new("leadid,createdon,statecode,statuscode,leadsourcecode,modifiedon");
    let mut opportunities = Table::new(
        "opportunityid,createdon,originatingleadid,statecode,actualclosedate,statuscode,estimatedvalue,actualvalue,stepname",
    );
    let mut activities = Table::new(
        "activityid,regardingobjectid,activitytypecode,createdon,actualend,statecode,ownerid",
    );
    let start = NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap();
    let mut activity = 0u64;
    for n in 0..settings.records {
        truth.leads += 1;
        let lead = format!("1e000000-0000-0000-0000-{n:012}");
        let created =
            start + Duration::days(rng.below(80) as i64) + Duration::minutes(rng.below(480) as i64);
        let qualified = rng.chance(0.6);
        let changed = created + Duration::days(1 + rng.below(10) as i64);
        if !qualified {
            truth.disqualified += 1;
        }
        leads.push(&[
            &lead,
            &utc(created),
            if qualified {
                "Qualified"
            } else {
                "Disqualified"
            },
            if qualified {
                "Qualified"
            } else {
                "No Longer Interested"
            },
            "Web",
            &utc(changed),
        ]);
        if !qualified {
            continue;
        }
        truth.opportunities += 1;
        let opportunity = format!("0a000000-0000-0000-0000-{n:012}");
        let mut at = changed;
        for kind in ["Phone Call", "Email", "Appointment"]
            .iter()
            .take(1 + rng.below(3) as usize)
        {
            activity += 1;
            truth.activities += 1;
            at += Duration::days(1 + rng.below(7) as i64);
            activities.push(&[
                &format!("ac{activity:010}"),
                &opportunity,
                kind,
                &utc(at),
                &utc(at + Duration::hours(1)),
                "Completed",
                "owner1",
            ]);
        }
        let (state, closed) = match rng.below(10) {
            0..=4 => {
                truth.won += 1;
                ("Won", Some(at + Duration::days(5)))
            }
            5..=7 => {
                truth.lost += 1;
                ("Lost", Some(at + Duration::days(5)))
            }
            _ => ("Open", None),
        };
        opportunities.push(&[
            &opportunity,
            &utc(changed),
            &lead,
            state,
            &closed.map(utc).unwrap_or_default(),
            state,
            "25000",
            if state == "Won" { "24000" } else { "" },
            "3-Propose",
        ]);
    }
    save(
        dir,
        &[
            ("lead", &leads),
            ("opportunity", &opportunities),
            ("activitypointer", &activities),
        ],
    )?;
    Ok(truth)
}

/// What the Dynamics 365 Customer Service extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ServiceTruth {
    pub cases: u64,
    pub routed: u64,
    pub escalated: u64,
    pub activities: u64,
    pub resolved: u64,
    pub cancelled: u64,
}

pub const SERVICE_FILES: [&str; 4] = [
    "incident",
    "queueitem",
    "incidentresolution",
    "activitypointer",
];

/// Writes incident, queueitem, incidentresolution and activitypointer.
pub fn write_customer_service(dir: &Path, settings: &Settings) -> io::Result<ServiceTruth> {
    let mut rng = Rng::new(settings.seed ^ 0xCA5E5);
    let mut truth = ServiceTruth::default();
    let mut cases = Table::new(
        "incidentid,createdon,statecode,isescalated,escalatedon,modifiedon,ticketnumber,statuscode,prioritycode,caseorigincode",
    );
    let mut queue = Table::new("queueitemid,objectid,queueid,enteredon,workerid");
    let mut resolutions = Table::new("activityid,incidentid,createdon,timespent");
    let mut activities = Table::new(
        "activityid,regardingobjectid,activitytypecode,createdon,actualend,statecode,ownerid",
    );
    let start = NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap();
    let (mut item, mut activity) = (0u64, 0u64);
    for n in 0..settings.records {
        truth.cases += 1;
        let case = format!("ca000000-0000-0000-0000-{n:012}");
        let created =
            start + Duration::days(rng.below(80) as i64) + Duration::minutes(rng.below(480) as i64);
        let mut at = created;
        for name in ["Tier 1", "Tier 2"]
            .iter()
            .take(1 + usize::from(rng.chance(0.3)))
        {
            item += 1;
            truth.routed += 1;
            at += Duration::minutes(5 + rng.below(240) as i64);
            queue.push(&[&format!("q{item:08}"), &case, name, &utc(at), "agent1"]);
        }
        for _ in 0..1 + rng.below(3) {
            activity += 1;
            truth.activities += 1;
            at += Duration::hours(1 + rng.below(24) as i64);
            activities.push(&[
                &format!("ac{activity:010}"),
                &case,
                ["Email", "Phone Call"][rng.below(2) as usize],
                &utc(at),
                &utc(at),
                "Completed",
                "agent1",
            ]);
        }
        let escalated = rng.chance(0.15);
        let escalated_at = at + Duration::hours(2);
        if escalated {
            truth.escalated += 1;
            at = escalated_at;
        }
        let state = match rng.below(10) {
            0 => {
                truth.cancelled += 1;
                at += Duration::hours(3);
                "Cancelled"
            }
            1 | 2 => "Active",
            _ => {
                truth.resolved += 1;
                at += Duration::hours(3);
                resolutions.push(&[&format!("res{n:08}"), &case, &utc(at), "45"]);
                "Resolved"
            }
        };
        cases.push(&[
            &case,
            &utc(created),
            state,
            if escalated { "true" } else { "false" },
            &if escalated {
                utc(escalated_at)
            } else {
                String::new()
            },
            &utc(at),
            &format!("CAS-{n:05}"),
            state,
            "Normal",
            "Email",
        ]);
    }
    save(
        dir,
        &[
            ("incident", &cases),
            ("queueitem", &queue),
            ("incidentresolution", &resolutions),
            ("activitypointer", &activities),
        ],
    )?;
    Ok(truth)
}
