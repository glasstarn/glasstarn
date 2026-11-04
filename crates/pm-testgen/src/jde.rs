//! Made-up JD Edwards EnterpriseOne tables in the standard layout: table and
//! column names as in the database, dates as Julian `CYYDDD` (`126014` is 14
//! January 2026), times as `HHMMSS`. Every record is invented; the counts in
//! the truth structs are known by construction.

use crate::Rng;
use crate::csv::{Table, save};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};
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
            seed: 31,
        }
    }
}

/// A Julian date as JD Edwards stores it.
fn julian(at: NaiveDateTime) -> String {
    ((at.year() - 1900) * 1000 + at.ordinal() as i32).to_string()
}

fn hms(at: NaiveDateTime) -> String {
    (at.hour() * 10_000 + at.minute() * 100 + at.second()).to_string()
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
    pub status_changes: u64,
    pub receipts: u64,
    pub matches: u64,
    pub vouchers: u64,
    pub payments: u64,
    /// Order lines reached by voucher and payment steps.
    pub voucher_line_steps: u64,
}

pub const PURCHASING_TABLES: [&str; 7] = [
    "F4301", "F4311", "F43199", "F43121", "F0411", "F0414", "F0413",
];

/// Writes the purchasing, voucher and payment tables.
pub fn write_purchasing(dir: &Path, settings: &Settings) -> io::Result<PurchasingTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x1DE);
    let mut truth = PurchasingTruth::default();
    let mut headers = Table::new("PHDOCO,PHDCTO,PHKCOO,PHAN8,PHTRDJ,PHUSER,PHMCU");
    let mut lines = Table::new("PDDOCO,PDDCTO,PDKCOO,PDLNID,PDUORG,PDNXTR,PDLTTR,PDLITM,PDPRRC");
    let mut ledger = Table::new("OLDOCO,OLDCTO,OLKCOO,OLLNID,OLLTTR,OLUPMJ,OLTDAY,OLNXTR,OLUSER");
    let mut matches = Table::new(
        "PRMATC,PRDOCO,PRDCTO,PRKCOO,PRLNID,PRDOC,PRDCT,PRKCO,PRRCDJ,PRUPMJ,PRUREC,PRTDAY",
    );
    let mut vouchers =
        Table::new("RPDOC,RPDCT,RPKCO,RPSFX,RPPO,RPPDCT,RPPKCO,RPDGJ,RPPST,RPDIVJ,RPAG,RPAN8");
    let mut details = Table::new("RNPYID,RNRC5,RNDOC,RNDCT,RNKCO,RNSFX,RNPAAP");
    let mut payments = Table::new("RMPYID,RMDOCM,RMDMTJ,RMVDGJ,RMPYIN");
    let (mut receipt_number, mut voucher_number, mut payment_number) =
        (7_000u64, 30_000u64, 900u64);
    for n in 0..settings.records {
        truth.orders += 1;
        let order = (4_000 + n).to_string();
        let entered = start()
            + Duration::days(rng.below(80) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        headers.push(&[
            &order,
            "OP",
            "00200",
            &(5_000 + n % 15).to_string(),
            &julian(entered),
            "JDOE",
            "M30",
        ]);
        let count = 1 + rng.below(3);
        let mut received_lines = Vec::new();
        for number in 1..=count {
            truth.lines += 1;
            let line = (number * 1000).to_string();
            let status = |ledger: &mut Table,
                          truth: &mut PurchasingTruth,
                          last: &str,
                          next: &str,
                          at: NaiveDateTime| {
                truth.status_changes += 1;
                ledger.push(&[
                    &order,
                    "OP",
                    "00200",
                    &line,
                    last,
                    &julian(at),
                    &hms(at),
                    next,
                    "JDOE",
                ]);
            };
            status(&mut ledger, &mut truth, "220", "280", entered);
            let approved = entered + Duration::hours(4 + rng.below(48) as i64);
            status(&mut ledger, &mut truth, "280", "400", approved);
            let received = rng.chance(0.9);
            let (last, next) = if received {
                ("400", "999")
            } else {
                ("280", "400")
            };
            lines.push(&[
                &order, "OP", "00200", &line, "10", next, last, "ITEM-1", "12.50",
            ]);
            if received {
                let at = approved + Duration::days(3 + rng.below(14) as i64);
                receipt_number += 1;
                truth.receipts += 1;
                matches.push(&[
                    "1",
                    &order,
                    "OP",
                    "00200",
                    &line,
                    &receipt_number.to_string(),
                    "OV",
                    "00200",
                    &julian(at),
                    &julian(at),
                    "10",
                    &hms(at),
                ]);
                status(&mut ledger, &mut truth, "400", "999", at);
                received_lines.push((line.clone(), at));
            }
        }
        if received_lines.is_empty() || !rng.chance(0.85) {
            continue;
        }
        voucher_number += 1;
        truth.vouchers += 1;
        let voucher = voucher_number.to_string();
        let entered_at = received_lines.iter().map(|l| l.1).max().unwrap() + Duration::days(5);
        for (line, _) in &received_lines {
            truth.matches += 1;
            matches.push(&[
                "2",
                &order,
                "OP",
                "00200",
                line,
                &voucher,
                "PV",
                "00200",
                &julian(entered_at),
                &julian(entered_at),
                "10",
                &hms(entered_at),
            ]);
        }
        let paid = rng.chance(0.8);
        // Every line of the order is reached through the order.
        truth.voucher_line_steps += (1 + u64::from(paid)) * count;
        for suffix in ["001", "002"] {
            vouchers.push(&[
                &voucher,
                "PV",
                "00200",
                suffix,
                &order,
                "OP",
                "00200",
                &julian(entered_at),
                if paid { "P" } else { "A" },
                &julian(entered_at - Duration::days(2)),
                "125.00",
                "5001",
            ]);
        }
        if paid {
            payment_number += 1;
            truth.payments += 1;
            let at = entered_at + Duration::days(20 + rng.below(20) as i64);
            payments.push(&[
                &payment_number.to_string(),
                &(80_000 + payment_number).to_string(),
                &julian(at),
                "0",
                "T",
            ]);
            details.push(&[
                &payment_number.to_string(),
                "1",
                &voucher,
                "PV",
                "00200",
                "001",
                "125.00",
            ]);
        }
    }
    save(
        dir,
        &[
            ("F4301", &headers),
            ("F4311", &lines),
            ("F43199", &ledger),
            ("F43121", &matches),
            ("F0411", &vouchers),
            ("F0414", &details),
            ("F0413", &payments),
        ],
    )?;
    Ok(truth)
}

/// What the order-to-cash tables contain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SellingTruth {
    pub orders: u64,
    pub lines: u64,
    pub status_changes: u64,
    pub shipped: u64,
    pub invoices: u64,
    pub invoiced_lines: u64,
    pub payments: u64,
    pub paid_lines: u64,
}

pub const SELLING_TABLES: [&str; 6] = ["F4201", "F4211", "F42199", "F03B11", "F03B14", "F03B13"];

/// Writes the sales order, invoice and receipt tables.
pub fn write_selling(dir: &Path, settings: &Settings) -> io::Result<SellingTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x2DE);
    let mut truth = SellingTruth::default();
    let mut headers = Table::new("SHDOCO,SHDCTO,SHKCOO,SHAN8,SHTRDJ,SHUSER,SHMCU");
    let mut lines = Table::new(
        "SDDOCO,SDDCTO,SDKCOO,SDLNID,SDUORG,SDNXTR,SDLTTR,SDADDJ,SDDOC,SDDCT,SDKCO,SDLITM,SDUPRC",
    );
    let mut ledger = Table::new("SLDOCO,SLDCTO,SLKCOO,SLLNID,SLLTTR,SLUPMJ,SLTDAY,SLNXTR,SLUSER");
    let mut invoices = Table::new("RPDOC,RPDCT,RPKCO,RPSFX,RPDIVJ,RPPST,RPAN8,RPAG");
    let mut details = Table::new("RZPYID,RZRC5,RZDOC,RZDCT,RZKCO,RZSFX,RZPAAP");
    let mut receipts = Table::new("RYPYID,RYCKNU,RYDMTJ,RYVDGJ");
    let (mut invoice_number, mut receipt_number) = (60_000u64, 300u64);
    for n in 0..settings.records {
        truth.orders += 1;
        let order = (7_000 + n).to_string();
        let entered = start()
            + Duration::days(rng.below(80) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        headers.push(&[
            &order,
            "SO",
            "00200",
            &(9_000 + n % 15).to_string(),
            &julian(entered),
            "ASMITH",
            "M30",
        ]);
        let count = 1 + rng.below(3);
        let invoice = rng.chance(0.85).then(|| {
            invoice_number += 1;
            invoice_number.to_string()
        });
        let shipped_at = entered + Duration::days(2 + rng.below(8) as i64);
        let mut shipped_lines = 0;
        for number in 1..=count {
            truth.lines += 1;
            let line = (number * 1000).to_string();
            let status = |ledger: &mut Table,
                          truth: &mut SellingTruth,
                          last: &str,
                          next: &str,
                          at: NaiveDateTime| {
                truth.status_changes += 1;
                ledger.push(&[
                    &order,
                    "SO",
                    "00200",
                    &line,
                    last,
                    &julian(at),
                    &hms(at),
                    next,
                    "ASMITH",
                ]);
            };
            status(&mut ledger, &mut truth, "520", "540", entered);
            let shipped = rng.chance(0.92);
            if shipped {
                truth.shipped += 1;
                shipped_lines += 1;
                status(&mut ledger, &mut truth, "560", "580", shipped_at);
            }
            let billed = shipped && invoice.is_some();
            if billed {
                status(
                    &mut ledger,
                    &mut truth,
                    "620",
                    "999",
                    shipped_at + Duration::days(1),
                );
            }
            lines.push(&[
                &order,
                "SO",
                "00200",
                &line,
                "4",
                if billed {
                    "999"
                } else if shipped {
                    "620"
                } else {
                    "560"
                },
                if billed {
                    "620"
                } else if shipped {
                    "580"
                } else {
                    "540"
                },
                &if shipped {
                    julian(shipped_at)
                } else {
                    "0".to_owned()
                },
                &if billed {
                    invoice.clone().unwrap()
                } else {
                    "0".to_owned()
                },
                if billed { "RI" } else { "" },
                if billed { "00200" } else { "" },
                "ITEM-9",
                "30.00",
            ]);
        }
        let Some(invoice) = invoice.filter(|_| shipped_lines > 0) else {
            continue;
        };
        truth.invoices += 1;
        truth.invoiced_lines += shipped_lines;
        let billed = shipped_at + Duration::days(1);
        let paid = rng.chance(0.8);
        for suffix in ["001", "002"] {
            invoices.push(&[
                &invoice,
                "RI",
                "00200",
                suffix,
                &julian(billed),
                if paid { "P" } else { "A" },
                "9001",
                "120.00",
            ]);
        }
        if paid {
            receipt_number += 1;
            truth.payments += 1;
            truth.paid_lines += shipped_lines;
            let at = billed + Duration::days(15 + rng.below(30) as i64);
            receipts.push(&[
                &receipt_number.to_string(),
                &format!("CHK{receipt_number}"),
                &julian(at),
                "0",
            ]);
            details.push(&[
                &receipt_number.to_string(),
                "1",
                &invoice,
                "RI",
                "00200",
                "001",
                "120.00",
            ]);
        }
    }
    save(
        dir,
        &[
            ("F4201", &headers),
            ("F4211", &lines),
            ("F42199", &ledger),
            ("F03B11", &invoices),
            ("F03B14", &details),
            ("F03B13", &receipts),
        ],
    )?;
    Ok(truth)
}
