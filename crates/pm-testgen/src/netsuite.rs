//! Made-up Oracle NetSuite extracts as SuiteQL returns them: transactions,
//! their lines, the links between transaction lines, and system notes. Times
//! in UTC (`2026-01-05T14:30:00Z`). Every record is invented; the counts in
//! the truth struct are known by construction.

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
    /// Transaction types: order, receipt or shipment, bill, payment.
    fn types(self) -> [&'static str; 4] {
        match self {
            Self::Buying => ["PurchOrd", "ItemRcpt", "VendBill", "VendPymt"],
            Self::Selling => ["SalesOrd", "ItemShip", "CustInvc", "CustPymt"],
        }
    }
}

pub const FILES: [&str; 4] = [
    "Transaction",
    "TransactionLine",
    "PreviousTransactionLineLink",
    "SystemNote",
];

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
            seed: 41,
        }
    }
}

/// What the extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Truth {
    pub orders: u64,
    pub lines: u64,
    pub status_changes: u64,
    /// Order lines reached by order steps (create and status changes).
    pub order_line_steps: u64,
    pub moved_lines: u64,
    pub bills: u64,
    pub billed_lines: u64,
    pub payments: u64,
    pub paid_lines: u64,
}

fn utc(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Writes the four files of `flow`.
pub fn write(dir: &Path, flow: Flow, settings: &Settings) -> io::Result<Truth> {
    let mut rng = Rng::new(settings.seed ^ if flow == Flow::Buying { 0x5501 } else { 0x5502 });
    let mut truth = Truth::default();
    let [order_type, move_type, bill_type, pay_type] = flow.types();
    let mut transactions =
        Table::new("id,tranid,type,createddate,entity,trandate,status,approvalstatus,foreigntotal");
    let mut lines = Table::new("transaction,id,type,item,quantity,rate,amount");
    let mut links = Table::new(
        "previousdoc,previousline,previoustype,nextdoc,nextline,nexttype,linktype,foreignamount",
    );
    let mut notes = Table::new("id,recordid,field,oldvalue,newvalue,date,name,context");
    let start = NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap();
    let mut id = 1_000u64;
    let mut note = 0u64;
    let mut next_id = || {
        id += 1;
        id.to_string()
    };
    for n in 0..settings.records {
        truth.orders += 1;
        let order = next_id();
        let created = start
            + Duration::days(rng.below(80) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let partner = (300 + n % 25).to_string();
        transactions.push(&[
            &order,
            &format!("{}{:04}", order_type[..2].to_uppercase(), n),
            order_type,
            &utc(created),
            &partner,
            &created.date().to_string(),
            "B",
            "2",
            "500.00",
        ]);
        let count = 1 + rng.below(3);
        truth.lines += count;
        for line in 1..=count {
            lines.push(&[
                &order,
                &line.to_string(),
                order_type,
                "77",
                "2",
                "250.00",
                "500.00",
            ]);
        }
        // Approval, then a later status, from the system notes.
        let mut at = created;
        let mut statuses = 0;
        for (old, new) in [
            ("Pending Approval", "Pending Receipt"),
            ("Pending Receipt", "Pending Billing"),
        ] {
            if statuses == 1 && !rng.chance(0.8) {
                break;
            }
            at += Duration::hours(2 + rng.below(48) as i64);
            note += 1;
            notes.push(&[
                &note.to_string(),
                &order,
                "TRANDOC.KSTATUS",
                old,
                new,
                &utc(at),
                "Jane Buyer",
                "UI",
            ]);
            statuses += 1;
        }
        // A change to another field is not a status step.
        note += 1;
        notes.push(&[
            &note.to_string(),
            &order,
            "TRANDOC.MEMO",
            "",
            "Rush",
            &utc(created),
            "Jane Buyer",
            "UI",
        ]);
        truth.status_changes += statuses;
        truth.order_line_steps += (1 + statuses) * count;
        if !rng.chance(0.9) {
            continue;
        }
        let moved = next_id();
        let moved_at = at + Duration::days(2 + rng.below(10) as i64);
        transactions.push(&[
            &moved,
            &format!("M{n:05}"),
            move_type,
            &utc(moved_at),
            &partner,
            &moved_at.date().to_string(),
            "",
            "",
            "",
        ]);
        for line in 1..=count {
            truth.moved_lines += 1;
            links.push(&[
                &order,
                &line.to_string(),
                order_type,
                &moved,
                &line.to_string(),
                move_type,
                "ShipRcpt",
                "",
            ]);
        }
        if !rng.chance(0.85) {
            continue;
        }
        let bill = next_id();
        truth.bills += 1;
        truth.billed_lines += count;
        let billed_at = moved_at + Duration::days(1 + rng.below(10) as i64);
        transactions.push(&[
            &bill,
            &format!("B{n:05}"),
            bill_type,
            &utc(billed_at),
            &partner,
            &billed_at.date().to_string(),
            "A",
            "",
            "500.00",
        ]);
        for line in 1..=count {
            links.push(&[
                &order,
                &line.to_string(),
                order_type,
                &bill,
                &line.to_string(),
                bill_type,
                "OrdBill",
                "",
            ]);
        }
        if rng.chance(0.8) {
            let payment = next_id();
            truth.payments += 1;
            truth.paid_lines += count;
            let paid_at = billed_at + Duration::days(10 + rng.below(30) as i64);
            transactions.push(&[
                &payment,
                &format!("P{n:05}"),
                pay_type,
                &utc(paid_at),
                &partner,
                &paid_at.date().to_string(),
                "",
                "",
                "500.00",
            ]);
            links.push(&[
                &bill, "0", bill_type, &payment, "0", pay_type, "Payment", "500.00",
            ]);
        }
    }
    save(
        dir,
        &[
            ("Transaction", &transactions),
            ("TransactionLine", &lines),
            ("PreviousTransactionLineLink", &links),
            ("SystemNote", &notes),
        ],
    )?;
    Ok(truth)
}
