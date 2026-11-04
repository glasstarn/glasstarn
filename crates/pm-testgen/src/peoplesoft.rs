//! Made-up PeopleSoft Financials and Supply Chain tables in the standard
//! layout: record names with their PS_ prefix, business-unit keys, dates as
//! `YYYY-MM-DD`. Every record is invented; the counts in the truth struct are
//! known by construction.

use crate::Rng;
use crate::csv::{Table, save};
use chrono::{Duration, NaiveDate};
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
            seed: 37,
        }
    }
}

/// What the tables contain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PurchasingTruth {
    pub requisitions: u64,
    pub requisitions_approved: u64,
    /// Order lines that came from a requisition.
    pub lines_from_requisitions: u64,
    /// Requisition steps that reach order lines.
    pub requisition_line_steps: u64,
    pub orders: u64,
    pub orders_approved: u64,
    pub lines: u64,
    pub order_line_steps: u64,
    pub receipts: u64,
    pub vouchers: u64,
    pub vouchers_approved: u64,
    pub payments: u64,
    pub voucher_line_steps: u64,
}

pub const PURCHASING_TABLES: [&str; 9] = [
    "PS_REQ_HDR",
    "PS_PO_HDR",
    "PS_PO_LINE",
    "PS_PO_LINE_DISTRIB",
    "PS_RECV_LN_SHIP",
    "PS_VOUCHER",
    "PS_VOUCHER_LINE",
    "PS_PYMNT_VCHR_XREF",
    "PS_PAYMENT_TBL",
];

/// Writes the nine requisition, purchasing, receiving and payables tables.
pub fn write_purchasing(dir: &Path, settings: &Settings) -> io::Result<PurchasingTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x9501);
    let mut truth = PurchasingTruth::default();
    let mut requisitions =
        Table::new("BUSINESS_UNIT,REQ_ID,REQ_DT,REQ_STATUS,APPROVAL_DT,REQUESTOR_ID");
    let mut orders = Table::new(
        "BUSINESS_UNIT,PO_ID,PO_DT,PO_STATUS,VENDOR_ID,APPROVAL_DT,BUDGET_HDR_STATUS,BUYER_ID",
    );
    let mut lines =
        Table::new("BUSINESS_UNIT,PO_ID,LINE_NBR,CANCEL_STATUS,INV_ITEM_ID,CATEGORY_ID,QTY_PO");
    let mut distributions = Table::new(
        "BUSINESS_UNIT,PO_ID,LINE_NBR,DISTRIB_LINE_NUM,BUSINESS_UNIT_REQ,REQ_ID,REQ_LINE_NBR",
    );
    let mut receipts = Table::new(
        "BUSINESS_UNIT,RECEIVER_ID,RECV_LN_NBR,RECV_SHIP_SEQ_NBR,BUSINESS_UNIT_PO,PO_ID,LINE_NBR,RECEIPT_DT,QTY_SH_ACCPT",
    );
    let mut vouchers = Table::new(
        "BUSINESS_UNIT,VOUCHER_ID,ENTERED_DT,APPROVAL_DT,MATCH_STATUS_VCHR,ENTRY_STATUS,INVOICE_ID,INVOICE_DT,VENDOR_ID,GROSS_AMT",
    );
    let mut voucher_lines = Table::new(
        "BUSINESS_UNIT,VOUCHER_ID,VOUCHER_LINE_NUM,BUSINESS_UNIT_PO,PO_ID,LINE_NBR,MERCHANDISE_AMT",
    );
    let mut schedules = Table::new(
        "BUSINESS_UNIT,VOUCHER_ID,PYMNT_CNT,PYMNT_SELCT_STATUS,BANK_SETID,BANK_CD,BANK_ACCT_KEY,PYMNT_ID,PAID_AMT",
    );
    let mut payments = Table::new(
        "BANK_SETID,BANK_CD,BANK_ACCT_KEY,PYMNT_ID,PYMNT_DT,PYMNT_STATUS,PYMNT_ID_REF,PYMNT_AMT",
    );
    let start = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let bu = "STATE";
    let (mut receiver, mut voucher_number, mut payment_number) = (0u64, 0u64, 0u64);
    for n in 0..settings.records {
        let day = start + Duration::days(rng.below(80) as i64);
        let requisition = rng.chance(0.6).then(|| {
            truth.requisitions += 1;
            let id = format!("{:010}", 500 + n);
            let approved = rng.chance(0.95);
            truth.requisitions_approved += u64::from(approved);
            requisitions.push(&[
                bu,
                &id,
                &(day - Duration::days(3)).to_string(),
                if approved { "A" } else { "P" },
                &if approved {
                    (day - Duration::days(1)).to_string()
                } else {
                    String::new()
                },
                "EMP42",
            ]);
            (id, approved)
        });
        // An unapproved requisition never became an order.
        if requisition.as_ref().is_some_and(|r| !r.1) {
            continue;
        }
        truth.orders += 1;
        let order = format!("{:010}", 12_000 + n);
        let approved = rng.chance(0.95);
        truth.orders_approved += u64::from(approved);
        orders.push(&[
            bu,
            &order,
            &day.to_string(),
            if approved { "D" } else { "PA" },
            &format!("V{:05}", n % 30),
            &if approved {
                (day + Duration::days(2)).to_string()
            } else {
                String::new()
            },
            "V",
            "BUYER7",
        ]);
        let count = 1 + rng.below(3);
        truth.lines += count;
        truth.order_line_steps += (1 + u64::from(approved)) * count;
        let mut received = Vec::new();
        for line in 1..=count {
            lines.push(&[bu, &order, &line.to_string(), "A", "ITM100", "OFFICE", "4"]);
            let (req_bu, req_id) = match &requisition {
                Some((id, _)) => {
                    truth.lines_from_requisitions += 1;
                    truth.requisition_line_steps += 2;
                    (bu, id.as_str())
                }
                None => ("", ""),
            };
            distributions.push(&[
                bu,
                &order,
                &line.to_string(),
                "1",
                req_bu,
                req_id,
                if req_id.is_empty() { "" } else { "1" },
            ]);
            if approved && rng.chance(0.9) {
                receiver += 1;
                truth.receipts += 1;
                let at = day + Duration::days(5 + rng.below(20) as i64);
                receipts.push(&[
                    bu,
                    &format!("{receiver:010}"),
                    "1",
                    "1",
                    bu,
                    &order,
                    &line.to_string(),
                    &at.to_string(),
                    "4",
                ]);
                received.push((line, at));
            }
        }
        if received.is_empty() || !rng.chance(0.85) {
            continue;
        }
        voucher_number += 1;
        truth.vouchers += 1;
        let voucher = format!("{voucher_number:08}");
        let entered = received.iter().map(|r| r.1).max().unwrap() + Duration::days(4);
        let voucher_approved = rng.chance(0.9);
        truth.vouchers_approved += u64::from(voucher_approved);
        vouchers.push(&[
            bu,
            &voucher,
            &entered.to_string(),
            &if voucher_approved {
                (entered + Duration::days(2)).to_string()
            } else {
                String::new()
            },
            "M",
            "P",
            &format!("INV{voucher_number}"),
            &(entered - Duration::days(3)).to_string(),
            &format!("V{:05}", n % 30),
            "480.00",
        ]);
        for (number, (line, _)) in received.iter().enumerate() {
            voucher_lines.push(&[
                bu,
                &voucher,
                &(number + 1).to_string(),
                bu,
                &order,
                &line.to_string(),
                "160.00",
            ]);
        }
        let paid = voucher_approved && rng.chance(0.85);
        if paid {
            payment_number += 1;
            truth.payments += 1;
            let id = format!("{:010}", 70_000 + payment_number);
            payments.push(&[
                "SHARE",
                "BOA",
                "OPER",
                &id,
                &(entered + Duration::days(25)).to_string(),
                "P",
                &format!("CHK{payment_number}"),
                "480.00",
            ]);
            schedules.push(&[
                bu, &voucher, "1", "P", "SHARE", "BOA", "OPER", &id, "480.00",
            ]);
        } else {
            schedules.push(&[bu, &voucher, "1", "N", "", "", "", "", "0.00"]);
        }
        let steps = 1 + u64::from(voucher_approved) + u64::from(paid);
        truth.voucher_line_steps += steps * received.len() as u64;
    }
    save(
        dir,
        &[
            ("PS_REQ_HDR", &requisitions),
            ("PS_PO_HDR", &orders),
            ("PS_PO_LINE", &lines),
            ("PS_PO_LINE_DISTRIB", &distributions),
            ("PS_RECV_LN_SHIP", &receipts),
            ("PS_VOUCHER", &vouchers),
            ("PS_VOUCHER_LINE", &voucher_lines),
            ("PS_PYMNT_VCHR_XREF", &schedules),
            ("PS_PAYMENT_TBL", &payments),
        ],
    )?;
    Ok(truth)
}
