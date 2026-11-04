//! Made-up SAP Business One tables for buying or selling, in the database
//! layout: one header table and one line table per document type, lines
//! linked to their base document through BaseType, BaseEntry and BaseLine,
//! dates as `YYYY-MM-DD`, creation times as whole numbers (`93005` is
//! 09:30:05). Every record is invented; the counts in the truth struct are
//! known by construction.

use crate::Rng;
use crate::csv::{Table, save};
use chrono::{Duration, NaiveDate, NaiveDateTime, Timelike};
use std::io;
use std::path::Path;

/// Buying (purchase to pay) or selling (order to cash).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Buying,
    Selling,
}

impl Flow {
    /// Header and line tables: request, order, receipt or delivery,
    /// invoice, payment.
    pub fn tables(self) -> [&'static str; 13] {
        match self {
            Self::Buying => [
                "OPRQ", "PRQ1", "OPOR", "POR1", "OPDN", "PDN1", "OPCH", "PCH1", "OVPM", "VPM2",
                "OWDD", "WDD1", "OCRD",
            ],
            Self::Selling => [
                "OQUT", "QUT1", "ORDR", "RDR1", "ODLN", "DLN1", "OINV", "INV1", "ORCT", "RCT2",
                "OWDD", "WDD1", "OCRD",
            ],
        }
    }
    /// Object types of request, order, receipt or delivery, and invoice.
    fn types(self) -> [&'static str; 4] {
        match self {
            Self::Buying => ["1470000113", "22", "20", "18"],
            Self::Selling => ["23", "17", "15", "13"],
        }
    }
}

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
            seed: 17,
        }
    }
}

/// What the tables contain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Truth {
    pub requests: u64,
    pub orders: u64,
    pub order_lines: u64,
    /// Order lines that came from a request line.
    pub lines_from_requests: u64,
    pub receipt_lines: u64,
    pub invoices: u64,
    /// Invoice lines based on a receipt or delivery line.
    pub invoice_lines_from_receipts: u64,
    /// Invoice lines based on an order line directly.
    pub invoice_lines_from_orders: u64,
    /// Rows of payments, one per invoice paid.
    pub paid_invoices: u64,
    /// Order lines a payment reaches, directly or through a receipt line.
    pub paid_order_lines: u64,
    pub approval_requests: u64,
    pub approvals: u64,
    pub rejections: u64,
    /// Lines of orders that went through approval.
    pub approved_order_lines: u64,
    pub cancelled_orders: u64,
    pub cancelled_order_lines: u64,
    /// Lines of cancellation documents for receipts or deliveries.
    pub cancelled_receipt_lines: u64,
    pub cancelled_invoices: u64,
    /// Order lines a cancelled invoice reaches.
    pub cancelled_invoice_order_lines: u64,
}

fn date(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%d").to_string()
}

/// The creation time as Business One stores it: a whole number.
fn stamp(at: NaiveDateTime) -> String {
    (at.hour() * 10_000 + at.minute() * 100 + at.second()).to_string()
}

struct Documents {
    headers: Table,
    lines: Table,
    next: u64,
}

impl Documents {
    fn new(start: u64) -> Self {
        Self {
            headers: Table::new(
                "DocEntry,DocNum,CardCode,DocDate,CreateDate,CreateTS,CANCELED,DocTotal,UserSign,DocStatus,DocDueDate,draftKey,CancelDate",
            ),
            lines: Table::new(
                "DocEntry,LineNum,ItemCode,Quantity,BaseType,BaseEntry,BaseLine,Price,LineTotal,LineStatus",
            ),
            next: start,
        }
    }
    /// Adds a document; each line is (item, quantity, base type, base entry,
    /// base line). Returns the document's entry.
    fn add(
        &mut self,
        partner: &str,
        at: NaiveDateTime,
        lines: &[(String, u64, &str, String, String)],
    ) -> String {
        self.add_with(partner, at, lines, "", "N", "")
    }
    /// As [`Self::add`], with the draft it came from, the cancellation flag
    /// (`N`, `Y` cancelled, `C` a cancellation document) and date.
    fn add_with(
        &mut self,
        partner: &str,
        at: NaiveDateTime,
        lines: &[(String, u64, &str, String, String)],
        draft: &str,
        cancelled: &str,
        cancel_date: &str,
    ) -> String {
        self.next += 1;
        let entry = self.next.to_string();
        let total: u64 = lines.iter().map(|l| l.1 * 100).sum();
        self.headers.push(&[
            &entry,
            &(10_000 + self.next).to_string(),
            partner,
            &date(at),
            &format!("{} 00:00:00.000", date(at)),
            &stamp(at),
            cancelled,
            &format!("{total}.00"),
            "1",
            "O",
            &date(at + Duration::days(30)),
            draft,
            cancel_date,
        ]);
        for (number, (item, quantity, base_type, base_entry, base_line)) in lines.iter().enumerate()
        {
            self.lines.push(&[
                &entry,
                &number.to_string(),
                item,
                &quantity.to_string(),
                base_type,
                base_entry,
                base_line,
                "100.00",
                &format!("{}.00", quantity * 100),
                "O",
            ]);
        }
        entry
    }
}

/// Writes the thirteen tables of `flow`.
pub fn write(dir: &Path, flow: Flow, settings: &Settings) -> io::Result<Truth> {
    let mut rng = Rng::new(settings.seed ^ if flow == Flow::Buying { 0xB1 } else { 0xB2 });
    let mut truth = Truth::default();
    let [request_type, order_type, receipt_type, _] = flow.types();
    let mut requests = Documents::new(100);
    let mut orders = Documents::new(1_000);
    let mut receipts = Documents::new(5_000);
    let mut invoices = Documents::new(9_000);
    let mut payments = Table::new(
        "DocEntry,DocNum,CardCode,DocDate,CreateDate,CreateTS,Canceled,DocTotal,UserSign",
    );
    let mut paid = Table::new("DocNum,InvoiceId,DocEntry,InvType,SumApplied");
    let mut approvals = Table::new("WddCode,ObjType,DocEntry,Status,CreateDate,CreateTime,OwnerID");
    let mut decisions = Table::new("WddCode,StepCode,UserID,Status,UpdateDate,UpdateTime");
    let mut approval_number = 0u64;
    let invoice_type = flow.types()[3];
    let mut partners = Table::new("CardCode,CardType,GroupCode,Country,U_Segment");
    let kind = if flow == Flow::Buying { "S" } else { "C" };
    let prefix = if flow == Flow::Buying { "V" } else { "C" };
    for p in 0..10 {
        partners.push(&[
            &format!("{prefix}{p:04}"),
            kind,
            "100",
            "DE",
            ["A", ""][p % 2],
        ]);
    }
    let start = NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap();
    // Invoices waiting for payment: (entry, partner, order lines reached).
    let mut unpaid: Vec<(String, String, u64, NaiveDateTime)> = Vec::new();
    let mut payment_number = 0u64;
    for _ in 0..settings.records {
        let partner = format!("{prefix}{:04}", rng.below(10));
        let mut at = start
            + Duration::days(rng.below(80) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let items: Vec<(String, u64)> = (0..1 + rng.below(3))
            .map(|i| (format!("ITEM-{i:03}"), 1 + rng.below(20)))
            .collect();
        let request = rng.chance(0.4).then(|| {
            truth.requests += 1;
            let lines: Vec<_> = items
                .iter()
                .map(|(item, quantity)| {
                    (item.clone(), *quantity, "-1", String::new(), String::new())
                })
                .collect();
            let entry = requests.add(&partner, at, &lines);
            at += Duration::hours(2 + rng.below(48) as i64);
            entry
        });
        // Large orders need approval: the draft is approved or rejected.
        let mut draft = String::new();
        if rng.chance(0.3) {
            approval_number += 1;
            truth.approval_requests += 1;
            draft = (700 + approval_number).to_string();
            let approved = rng.chance(0.85);
            let decided = at + Duration::hours(1 + rng.below(30) as i64);
            approvals.push(&[
                &(40 + approval_number).to_string(),
                order_type,
                &draft,
                if approved { "Y" } else { "N" },
                &date(at),
                &stamp(at),
                "1",
            ]);
            decisions.push(&[
                &(40 + approval_number).to_string(),
                "1",
                "5",
                if approved { "Y" } else { "N" },
                &date(decided),
                &stamp(decided),
            ]);
            at = decided + Duration::minutes(10);
            if approved {
                truth.approvals += 1;
                truth.approved_order_lines += items.len() as u64;
            } else {
                truth.rejections += 1;
                continue;
            }
        }
        let cancel_order = rng.chance(0.04);
        truth.orders += 1;
        let lines: Vec<_> = items
            .iter()
            .enumerate()
            .map(|(n, (item, quantity))| match &request {
                Some(entry) => {
                    truth.lines_from_requests += 1;
                    (
                        item.clone(),
                        *quantity,
                        request_type,
                        entry.clone(),
                        n.to_string(),
                    )
                }
                None => (item.clone(), *quantity, "-1", String::new(), String::new()),
            })
            .collect();
        truth.order_lines += lines.len() as u64;
        let order = orders.add_with(
            &partner,
            at,
            &lines,
            &draft,
            if cancel_order { "Y" } else { "N" },
            &if cancel_order {
                date(at + Duration::days(2))
            } else {
                String::new()
            },
        );
        let order_lines = lines.len() as u64;
        if cancel_order {
            truth.cancelled_orders += 1;
            truth.cancelled_order_lines += order_lines;
            continue;
        }
        // Services are invoiced straight from the order; goods are received
        // or delivered first.
        let invoice_lines: Vec<_> = if rng.chance(0.1) {
            truth.invoice_lines_from_orders += order_lines;
            items
                .iter()
                .enumerate()
                .map(|(n, (item, quantity))| {
                    (
                        item.clone(),
                        *quantity,
                        order_type,
                        order.clone(),
                        n.to_string(),
                    )
                })
                .collect()
        } else {
            if !rng.chance(0.9) {
                continue;
            }
            at += Duration::days(1 + rng.below(14) as i64);
            truth.receipt_lines += order_lines;
            let lines: Vec<_> = items
                .iter()
                .enumerate()
                .map(|(n, (item, quantity))| {
                    (
                        item.clone(),
                        *quantity,
                        order_type,
                        order.clone(),
                        n.to_string(),
                    )
                })
                .collect();
            let cancel_receipt = rng.chance(0.03);
            let receipt = receipts.add_with(
                &partner,
                at,
                &lines,
                "",
                if cancel_receipt { "Y" } else { "N" },
                "",
            );
            if cancel_receipt {
                // A cancellation document whose lines point at the receipt.
                truth.cancelled_receipt_lines += order_lines;
                let back: Vec<_> = items
                    .iter()
                    .enumerate()
                    .map(|(n, (item, quantity))| {
                        (
                            item.clone(),
                            *quantity,
                            receipt_type,
                            receipt.clone(),
                            n.to_string(),
                        )
                    })
                    .collect();
                receipts.add_with(&partner, at + Duration::hours(3), &back, "", "C", "");
                continue;
            }
            if !rng.chance(0.85) {
                continue;
            }
            truth.invoice_lines_from_receipts += order_lines;
            items
                .iter()
                .enumerate()
                .map(|(n, (item, quantity))| {
                    (
                        item.clone(),
                        *quantity,
                        receipt_type,
                        receipt.clone(),
                        n.to_string(),
                    )
                })
                .collect()
        };
        at += Duration::days(1 + rng.below(10) as i64);
        truth.invoices += 1;
        let cancel_invoice = rng.chance(0.04);
        let invoice = invoices.add_with(
            &partner,
            at,
            &invoice_lines,
            "",
            if cancel_invoice { "Y" } else { "N" },
            "",
        );
        if cancel_invoice {
            truth.cancelled_invoices += 1;
            truth.cancelled_invoice_order_lines += order_lines;
            let back: Vec<_> = items
                .iter()
                .enumerate()
                .map(|(n, (item, quantity))| {
                    (
                        item.clone(),
                        *quantity,
                        invoice_type,
                        invoice.clone(),
                        n.to_string(),
                    )
                })
                .collect();
            invoices.add_with(&partner, at + Duration::hours(5), &back, "", "C", "");
            continue;
        }
        if rng.chance(0.8) {
            unpaid.push((invoice, partner, order_lines, at));
        }
    }
    // Payments, sometimes two invoices of one partner at once.
    unpaid.sort_by(|a, b| a.1.cmp(&b.1).then(a.3.cmp(&b.3)));
    let mut index = 0;
    while index < unpaid.len() {
        let together = if index + 1 < unpaid.len()
            && unpaid[index + 1].1 == unpaid[index].1
            && rng.chance(0.3)
        {
            2
        } else {
            1
        };
        payment_number += 1;
        let batch = &unpaid[index..index + together];
        let at =
            batch.iter().map(|b| b.3).max().unwrap() + Duration::days(5 + rng.below(40) as i64);
        let entry = (20_000 + payment_number).to_string();
        payments.push(&[
            &entry,
            &(30_000 + payment_number).to_string(),
            &batch[0].1,
            &date(at),
            &date(at),
            &stamp(at),
            "N",
            "1000.00",
            "1",
        ]);
        for (row, (invoice, _, lines, _)) in batch.iter().enumerate() {
            truth.paid_invoices += 1;
            truth.paid_order_lines += lines;
            paid.push(&[&entry, &row.to_string(), invoice, flow.types()[3], "500.00"]);
        }
        index += together;
    }
    let names = flow.tables();
    save(
        dir,
        &[
            (names[0], &requests.headers),
            (names[1], &requests.lines),
            (names[2], &orders.headers),
            (names[3], &orders.lines),
            (names[4], &receipts.headers),
            (names[5], &receipts.lines),
            (names[6], &invoices.headers),
            (names[7], &invoices.lines),
            (names[8], &payments),
            (names[9], &paid),
            (names[10], &approvals),
            (names[11], &decisions),
            (names[12], &partners),
        ],
    )?;
    Ok(truth)
}
