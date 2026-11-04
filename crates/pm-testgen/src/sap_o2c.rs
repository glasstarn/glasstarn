//! Made-up order-to-cash tables in the layout of standard SAP ECC and
//! S/4HANA tables, and the same business in the layout of S/4HANA Public
//! Cloud data views. Table and field names follow public knowledge of the
//! standard SAP data model; nothing is copied from SAP documentation or code.
//! The counts in [`Truth`] are known by construction.
//!
//! Stories included on purpose:
//! - orders with one to four lines, some lines rejected and never delivered;
//! - an order confirmation sent to the customer, kept in the company's own
//!   fields (`ZZCONF_DAT`, `ZZCONF_BY`) up to two days after the order;
//! - the change log: credit blocks set and released, delivery and billing
//!   blocks set and removed, rejected lines, price and quantity changes;
//! - returns: a return order, the returned goods received, a credit memo;
//!   and credit memo requests that end in a credit memo;
//! - deliveries split in two, and deliveries not yet shipped;
//! - one invoice per shipped delivery, some cancelled and billed again;
//! - customer payments, some late, some invoices still open, and invoices
//!   cleared by their cancellation rather than by a payment;
//! - the same accounting document number in two company codes.

use crate::Rng;
use chrono::{Datelike, Duration, NaiveDate};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// What the generated tables contain, known by construction.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Truth {
    pub orders: u64,
    pub order_items: u64,
    pub rejected_items: u64,
    pub credit_blocked_orders: u64,
    pub deliveries: u64,
    pub deliveries_shipped: u64,
    /// Order items delivered in two deliveries.
    pub split_items: u64,
    /// Invoices (billing documents of type F2), rebills included.
    pub invoices: u64,
    pub cancellations: u64,
    /// Payments received; each clears one invoice.
    pub payments: u64,
    /// Invoices neither paid nor cancelled.
    pub open_invoices: u64,
    pub confirmations: u64,
    pub credit_releases: u64,
    pub delivery_blocks: u64,
    pub billing_blocks: u64,
    pub price_changes: u64,
    pub quantity_changes: u64,
    pub return_orders: u64,
    pub return_items: u64,
    /// Return deliveries created, and those whose goods were received.
    pub return_deliveries: u64,
    pub returns_received: u64,
    /// Credit memos for returns (RE) and for credit memo requests (G2).
    pub return_credit_memos: u64,
    pub credit_requests: u64,
    pub credit_memos: u64,
}

/// Settings for one generated export.
#[derive(Debug, Clone)]
pub struct Settings {
    pub orders: u64,
    pub seed: u64,
    pub company_codes: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            orders: 400,
            seed: 5,
            company_codes: vec!["1000".into(), "2000".into()],
        }
    }
}

/// The on-premise tables.
pub const TABLES: [&str; 11] = [
    "KNA1", "VBAK", "VBAP", "LIKP", "LIPS", "VBRK", "VBRP", "BKPF", "BSAD", "CDHDR", "CDPOS",
];

/// The S/4HANA Public Cloud files.
pub const CLOUD_TABLES: [&str; 10] = [
    "Customer",
    "SalesOrder",
    "SalesOrderItem",
    "OutboundDelivery",
    "OutboundDeliveryItem",
    "BillingDocument",
    "BillingDocumentItem",
    "JournalEntryItem",
    "ChangeDocument",
    "ChangeDocumentItem",
];

struct Table {
    header: String,
    rows: Vec<Vec<String>>,
}

impl Table {
    fn new(header: &'static str) -> Self {
        Self {
            header: header.into(),
            rows: Vec::new(),
        }
    }
    fn push(&mut self, values: &[&str]) {
        assert_eq!(
            values.len(),
            self.header.split(',').count(),
            "{}",
            self.header
        );
        self.rows
            .push(values.iter().map(|v| (*v).to_owned()).collect());
    }
    /// The column's values, by name, for one row.
    fn get<'a>(&self, row: &'a [String], column: &str) -> &'a str {
        let at = self
            .header
            .split(',')
            .position(|c| c == column)
            .expect(column);
        &row[at]
    }
}

fn date(day: NaiveDate) -> String {
    day.format("%Y%m%d").to_string()
}
fn number(value: u64, width: usize) -> String {
    format!("{value:0width$}")
}
fn money(cents: u64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

const CUSTOMERS: [(&str, &str); 8] = [
    ("Alder Retail", "DE"),
    ("Beacon Hospitals", "NL"),
    ("Cobalt Motors", "US"),
    ("Dune Hotels", "ES"),
    ("Ember Foods", "FR"),
    ("Fjord Shipping", "NO"),
    ("Gale Electronics", "DE"),
    ("Harbor Clinics", "IT"),
];
const MATERIALS: [(&str, &str, u64); 5] = [
    ("FG-100", "Pump unit", 48_000),
    ("FG-200", "Valve set", 12_500),
    ("FG-300", "Control board", 27_000),
    ("FG-400", "Filter pack", 3_900),
    ("FG-500", "Service kit", 8_800),
];

/// Billing and accounting tables with their number ranges.
struct Books {
    vbrk: Table,
    vbrp: Table,
    bkpf: Table,
    counters: BTreeMap<(String, i32, &'static str), u64>,
    billing_number: u64,
}

/// One delivered order line: item number, material, quantity, value in cents.
type Line = (String, &'static str, u64, u64);

impl Books {
    fn next(&mut self, company: &str, year: i32, range: &'static str, start: u64) -> String {
        let counter = self
            .counters
            .entry((company.to_owned(), year, range))
            .or_insert(start);
        *counter += 1;
        number(*counter, 10)
    }

    /// Bills one delivery; returns the billing document and its accounting
    /// document and fiscal year.
    #[allow(clippy::too_many_arguments)]
    fn bill(
        &mut self,
        at: String,
        kind: &str,
        day: NaiveDate,
        cancels: &str,
        (company, customer): (&str, &str),
        (order, delivery): (&str, &str),
        delivered: &[Line],
    ) -> (String, String, i32) {
        self.billing_number += 1;
        let billing = number(self.billing_number, 10);
        let amount: u64 = delivered.iter().map(|line| line.3).sum();
        self.vbrk.push(&[
            &billing,
            kind,
            &date(day),
            &date(day),
            &at,
            company,
            customer,
            &money(amount),
            "EUR",
            "",
            cancels,
        ]);
        for (n, (posnr, _, quantity, value)) in delivered.iter().enumerate() {
            self.vbrp.push(&[
                &billing,
                &number(n as u64 + 1, 6),
                order,
                posnr,
                delivery,
                posnr,
                &quantity.to_string(),
                &money(*value),
            ]);
        }
        let year = day.year();
        let document = self.next(company, year, "accounting", 1_400_000_000);
        self.bkpf.push(&[
            company,
            &document,
            &year.to_string(),
            "RV",
            &date(day),
            &date(day),
            &at,
            "BILLING",
            "VBRK",
            &billing,
        ]);
        (billing, document, year)
    }
}

struct Generated {
    tables: Vec<(&'static str, Table)>,
    truth: Truth,
}

fn generate(settings: &Settings) -> Generated {
    let mut rng = Rng::new(settings.seed);
    let mut truth = Truth::default();
    let mut kna1 = Table::new("KUNNR,NAME1,LAND1,KTOKD");
    let mut vbak = Table::new(
        "VBELN,AUART,VKORG,VTWEG,SPART,KUNNR,ERDAT,ERZET,ERNAM,NETWR,WAERK,BUKRS_VF,CMGST,VGBEL,ZZCONF_BY,ZZCONF_DAT",
    );
    let mut vbap =
        Table::new("VBELN,POSNR,MATNR,ARKTX,KWMENG,VRKME,NETWR,WERKS,ABGRU,ERDAT,VGBEL,VGPOS");
    let mut likp = Table::new("VBELN,LFART,ERDAT,ERZET,LFDAT,WADAT_IST,KUNNR,VSTEL");
    // (object, user, day, fixed time, [(table, key, field, new, old)])
    let mut changes = Vec::<(String, &str, NaiveDate, Option<String>, Vec<[String; 5]>)>::new();
    let header_key = |order: &str| format!("100{order}");
    let item_key = |order: &str, item: &str| format!("100{order}{item}");
    let mut return_number = 6_000_000u64;
    let mut lips = Table::new("VBELN,POSNR,VGBEL,VGPOS,MATNR,LFIMG");
    let mut books = Books {
        vbrk: Table::new("VBELN,FKART,FKDAT,ERDAT,ERZET,BUKRS,KUNAG,NETWR,WAERK,FKSTO,SFAKN"),
        vbrp: Table::new("VBELN,POSNR,AUBEL,AUPOS,VGBEL,VGPOS,FKIMG,NETWR"),
        bkpf: Table::new("BUKRS,BELNR,GJAHR,BLART,BUDAT,CPUDT,CPUTM,USNAM,AWTYP,AWKEY"),
        counters: BTreeMap::new(),
        billing_number: 90_000_000,
    };
    let mut bsad = Table::new(
        "BUKRS,KUNNR,AUGDT,AUGBL,AUGGJ,GJAHR,BELNR,BUZEI,BLART,SHKZG,BUDAT,ZFBDT,ZBD1T,DMBTR,WAERS",
    );
    for (index, (name, country)) in CUSTOMERS.iter().enumerate() {
        kna1.push(&[&number(200_001 + index as u64, 10), name, country, "KUNA"]);
    }
    let time = |rng: &mut Rng| {
        let seconds = 7 * 3600 + rng.below(10 * 3600);
        format!(
            "{:02}{:02}{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    };
    let first = NaiveDate::from_ymd_opt(2025, 11, 1).unwrap();
    let mut delivery_number = 80_000_000u64;

    for order in 0..settings.orders {
        truth.orders += 1;
        let company = if settings.company_codes.len() > 1 && rng.chance(0.25) {
            settings.company_codes[1].clone()
        } else {
            settings.company_codes[0].clone()
        };
        let kunnr = number(200_001 + rng.below(CUSTOMERS.len() as u64), 10);
        let created = first + Duration::days(rng.below(120) as i64);
        let vbeln = number(1_000_000 + order, 10);
        let created_at = time(&mut rng);
        let blocked = rng.chance(0.15);
        let released = blocked.then(|| created + Duration::days(1 + rng.below(4) as i64));
        let hdr = header_key(&vbeln);
        let field = |table: &str, key: &str, name: &str, new: &str, old: &str| {
            [
                table.to_owned(),
                key.to_owned(),
                name.to_owned(),
                new.to_owned(),
                old.to_owned(),
            ]
        };
        // The credit check blocks the order as it is saved, a minute later.
        let just_after = format!(
            "{}{:02}",
            &created_at[..2],
            (created_at[2..4].parse::<u32>().unwrap() + 1).min(59)
        ) + &created_at[4..];
        if let Some(released) = released {
            truth.credit_blocked_orders += 1;
            truth.credit_releases += 1;
            changes.push((
                vbeln.clone(),
                "SALES01",
                created,
                Some(just_after.clone()),
                vec![field("VBAK", &hdr, "CMGST", "B", "")],
            ));
            changes.push((
                vbeln.clone(),
                "CREDITMGR",
                released,
                None,
                vec![field("VBAK", &hdr, "CMGST", "D", "B")],
            ));
        }
        let confirmed = rng
            .chance(0.4)
            .then(|| created + Duration::days(rng.below(3) as i64));
        if confirmed.is_some() {
            truth.confirmations += 1;
        }
        let items = 1 + rng.below(4);
        let mut lines: Vec<Line> = Vec::new();
        let mut total = 0;
        for item in 0..items {
            truth.order_items += 1;
            let posnr = number((item + 1) * 10, 6);
            let (matnr, text, price) = MATERIALS[rng.below(MATERIALS.len() as u64) as usize];
            let quantity = 1 + rng.below(10);
            let value = price * quantity;
            total += value;
            let rejected = rng.chance(0.05);
            let key = item_key(&vbeln, &posnr);
            if rejected {
                truth.rejected_items += 1;
                changes.push((
                    vbeln.clone(),
                    "SALES01",
                    created + Duration::days(1 + rng.below(3) as i64),
                    None,
                    vec![field("VBAP", &key, "ABGRU", "02", "")],
                ));
            } else if rng.chance(0.06) {
                truth.price_changes += 1;
                changes.push((
                    vbeln.clone(),
                    "SALES01",
                    created,
                    None,
                    vec![field("VBAP", &key, "NETWR", &money(value), "0.00")],
                ));
            } else if rng.chance(0.05) {
                truth.quantity_changes += 1;
                changes.push((
                    vbeln.clone(),
                    "SALES02",
                    created,
                    None,
                    vec![field("VBAP", &key, "KWMENG", &quantity.to_string(), "1")],
                ));
            }
            vbap.push(&[
                &vbeln,
                &posnr,
                matnr,
                text,
                &quantity.to_string(),
                "EA",
                &money(value),
                "1100",
                if rejected { "02" } else { "" },
                &date(created),
                "",
                "",
            ]);
            if !rejected {
                lines.push((posnr, matnr, quantity, value));
            }
        }
        vbak.push(&[
            &vbeln,
            "OR",
            "1000",
            "10",
            "00",
            &kunnr,
            &date(created),
            &created_at,
            "SALES01",
            &money(total),
            "EUR",
            &company,
            if blocked { "D" } else { "" },
            "",
            if confirmed.is_some() {
                ["CSR01", "CSR02"][order as usize % 2]
            } else {
                ""
            },
            &confirmed.map_or("00000000".to_owned(), date),
        ]);
        if lines.is_empty() {
            continue;
        }
        // Deliveries: usually one; sometimes the first line's quantity is
        // split over a second delivery a few days later.
        let ready = released.unwrap_or(created) + Duration::days(1 + rng.below(6) as i64);
        if rng.chance(0.08) {
            // A delivery block, lifted the day before delivery.
            truth.delivery_blocks += 1;
            changes.push((
                vbeln.clone(),
                "SALES01",
                created,
                Some(just_after.clone()),
                vec![field("VBAK", &hdr, "LIFSK", "01", "")],
            ));
            changes.push((
                vbeln.clone(),
                "SALES02",
                ready - Duration::days(1),
                None,
                vec![field("VBAK", &hdr, "LIFSK", "", "01")],
            ));
        }
        let billing_block = rng.chance(0.06);
        let mut block_lifted: Option<NaiveDate> = None;
        if billing_block {
            truth.billing_blocks += 1;
            changes.push((
                vbeln.clone(),
                "SALES01",
                created,
                Some(just_after.clone()),
                vec![field("VBAK", &hdr, "FAKSK", "02", "")],
            ));
        }
        let mut deliveries = vec![(ready, lines.clone(), false)];
        if rng.chance(0.1) {
            let (posnr, matnr, quantity, value) = lines[0].clone();
            if quantity > 1 {
                truth.split_items += 1;
                let later = ready + Duration::days(2 + rng.below(8) as i64);
                deliveries[0].1[0].2 = quantity - quantity / 2;
                deliveries[0].1[0].3 = value - value / 2;
                deliveries.push((later, vec![(posnr, matnr, quantity / 2, value / 2)], true));
            }
        }
        for (made, delivered, _) in deliveries {
            truth.deliveries += 1;
            delivery_number += 1;
            let delivery = number(delivery_number, 10);
            let shipped = rng
                .chance(0.95)
                .then(|| made + Duration::days(1 + rng.below(3) as i64));
            likp.push(&[
                &delivery,
                "LF",
                &date(made),
                &time(&mut rng),
                &date(made + Duration::days(2)),
                &shipped.map_or("00000000".to_owned(), date),
                &kunnr,
                "1000",
            ]);
            for (posnr, matnr, quantity, _) in &delivered {
                lips.push(&[
                    &delivery,
                    posnr,
                    &vbeln,
                    posnr,
                    matnr,
                    &quantity.to_string(),
                ]);
            }
            let Some(shipped) = shipped else {
                continue;
            };
            truth.deliveries_shipped += 1;
            // One invoice per shipped delivery; some are cancelled and billed again.
            let amount: u64 = delivered.iter().map(|line| line.3).sum();
            let mut billed = shipped + Duration::days(rng.below(3) as i64);
            if billing_block {
                // Billing waits for the block to be lifted.
                let lifted = *block_lifted.get_or_insert_with(|| {
                    let lifted = shipped + Duration::days(1 + rng.below(3) as i64);
                    changes.push((
                        vbeln.clone(),
                        "BILLING",
                        lifted,
                        None,
                        vec![field("VBAK", &hdr, "FAKSK", "", "02")],
                    ));
                    lifted
                });
                billed = billed.max(lifted + Duration::days(1));
            }
            let parties = (company.as_str(), kunnr.as_str());
            let documents = (vbeln.as_str(), delivery.as_str());
            let (invoice, mut document, mut year) = books.bill(
                time(&mut rng),
                "F2",
                billed,
                "",
                parties,
                documents,
                &delivered,
            );
            truth.invoices += 1;
            if rng.chance(0.04) {
                // Cancelled: the cancellation clears the invoice, then a new one.
                let cancelled_on = billed + Duration::days(1 + rng.below(5) as i64);
                let (_, cancel_document, cancel_year) = books.bill(
                    time(&mut rng),
                    "S1",
                    cancelled_on,
                    &invoice,
                    parties,
                    documents,
                    &delivered,
                );
                truth.cancellations += 1;
                let row = books
                    .vbrk
                    .rows
                    .iter_mut()
                    .find(|r| r[0] == invoice)
                    .unwrap();
                row[9] = "X".into();
                bsad.push(&[
                    &company,
                    &kunnr,
                    &date(cancelled_on),
                    &cancel_document,
                    &cancel_year.to_string(),
                    &year.to_string(),
                    &document,
                    "001",
                    "RV",
                    "S",
                    &date(billed),
                    &date(billed),
                    "30",
                    &money(amount),
                    "EUR",
                ]);
                bsad.push(&[
                    &company,
                    &kunnr,
                    &date(cancelled_on),
                    &cancel_document,
                    &cancel_year.to_string(),
                    &cancel_year.to_string(),
                    &cancel_document,
                    "001",
                    "RV",
                    "H",
                    &date(cancelled_on),
                    &date(cancelled_on),
                    "0",
                    &money(amount),
                    "EUR",
                ]);
                billed = cancelled_on + Duration::days(1);
                (_, document, year) = books.bill(
                    time(&mut rng),
                    "F2",
                    billed,
                    "",
                    parties,
                    documents,
                    &delivered,
                );
                truth.invoices += 1;
            }
            if rng.chance(0.85) {
                truth.payments += 1;
                let paid = billed + Duration::days(15 + rng.below(40) as i64);
                let paid_year = paid.year();
                let payment = books.next(&company, paid_year, "payment", 1_600_000_000);
                bsad.push(&[
                    &company,
                    &kunnr,
                    &date(paid),
                    &payment,
                    &paid_year.to_string(),
                    &year.to_string(),
                    &document,
                    "001",
                    "RV",
                    "S",
                    &date(billed),
                    &date(billed),
                    "30",
                    &money(amount),
                    "EUR",
                ]);
                bsad.push(&[
                    &company,
                    &kunnr,
                    &date(paid),
                    &payment,
                    &paid_year.to_string(),
                    &paid_year.to_string(),
                    &payment,
                    "001",
                    "DZ",
                    "H",
                    &date(paid),
                    &date(paid),
                    "0",
                    &money(amount),
                    "EUR",
                ]);
                let at = time(&mut rng);
                books.bkpf.push(&[
                    &company,
                    &payment,
                    &paid_year.to_string(),
                    "DZ",
                    &date(paid),
                    &date(paid),
                    &at,
                    "CASHAPP",
                    "BKPF",
                    &format!("{payment}{company}{paid_year}"),
                ]);
            } else {
                truth.open_invoices += 1;
            }
            // A credit memo request: a price adjustment credited later.
            if rng.chance(0.03) {
                truth.credit_requests += 1;
                truth.credit_memos += 1;
                return_number += 1;
                let request = number(return_number, 10);
                let asked = billed + Duration::days(3 + rng.below(10) as i64);
                let (posnr, matnr, quantity, value) = delivered[0].clone();
                let credit = value / 10;
                vbak.push(&[
                    &request,
                    "CR",
                    "1000",
                    "10",
                    "00",
                    &kunnr,
                    &date(asked),
                    &time(&mut rng),
                    "SALES02",
                    &money(credit),
                    "EUR",
                    &company,
                    "",
                    &vbeln,
                    "",
                    "00000000",
                ]);
                vbap.push(&[
                    &request,
                    "000010",
                    matnr,
                    "Price adjustment",
                    &quantity.to_string(),
                    "EA",
                    &money(credit),
                    "1100",
                    "",
                    &date(asked),
                    &vbeln,
                    &posnr,
                ]);
                let lines = vec![("000010".to_owned(), matnr, quantity, credit)];
                books.bill(
                    time(&mut rng),
                    "G2",
                    asked + Duration::days(1 + rng.below(4) as i64),
                    "",
                    parties,
                    (request.as_str(), ""),
                    &lines,
                );
            }
            // A return: return order, returned goods received, credit memo.
            if rng.chance(0.05) {
                truth.return_orders += 1;
                return_number += 1;
                let returned = number(return_number, 10);
                let opened = shipped + Duration::days(5 + rng.below(20) as i64);
                let mut lines = Vec::new();
                for (n, (posnr, matnr, quantity, value)) in delivered.iter().enumerate() {
                    truth.return_items += 1;
                    let item = number((n as u64 + 1) * 10, 6);
                    vbap.push(&[
                        &returned,
                        &item,
                        matnr,
                        "Returned goods",
                        &quantity.to_string(),
                        "EA",
                        &money(*value),
                        "1100",
                        "",
                        &date(opened),
                        &vbeln,
                        posnr,
                    ]);
                    lines.push((item, *matnr, *quantity, *value));
                }
                vbak.push(&[
                    &returned,
                    "RE",
                    "1000",
                    "10",
                    "00",
                    &kunnr,
                    &date(opened),
                    &time(&mut rng),
                    "SALES02",
                    &money(amount),
                    "EUR",
                    &company,
                    "",
                    &vbeln,
                    "",
                    "00000000",
                ]);
                truth.return_deliveries += 1;
                delivery_number += 1;
                let back = number(delivery_number, 10);
                let made = opened + Duration::days(1 + rng.below(3) as i64);
                let received = rng
                    .chance(0.9)
                    .then(|| made + Duration::days(1 + rng.below(4) as i64));
                likp.push(&[
                    &back,
                    "LR",
                    &date(made),
                    &time(&mut rng),
                    &date(made + Duration::days(2)),
                    &received.map_or("00000000".to_owned(), date),
                    &kunnr,
                    "1000",
                ]);
                for (item, matnr, quantity, _) in &lines {
                    lips.push(&[&back, item, &returned, item, matnr, &quantity.to_string()]);
                }
                if let Some(received) = received {
                    truth.returns_received += 1;
                    truth.return_credit_memos += 1;
                    truth.credit_memos += 1;
                    books.bill(
                        time(&mut rng),
                        "RE",
                        received + Duration::days(1 + rng.below(3) as i64),
                        "",
                        parties,
                        (returned.as_str(), back.as_str()),
                        &lines,
                    );
                }
            }
        }
    }
    // The change log, one change header per change, numbered in order.
    let mut cdhdr = Table::new("OBJECTCLAS,OBJECTID,CHANGENR,USERNAME,UDATE,UTIME,TCODE");
    let mut cdpos =
        Table::new("OBJECTCLAS,OBJECTID,CHANGENR,TABNAME,TABKEY,FNAME,CHNGIND,VALUE_NEW,VALUE_OLD");
    for (n, (object, user, day, at, fields)) in changes.into_iter().enumerate() {
        let change = number(n as u64 + 1, 10);
        let at = at.unwrap_or_else(|| time(&mut rng));
        cdhdr.push(&["VERKBELEG", &object, &change, user, &date(day), &at, "VA02"]);
        for [table, key, name, new, old] in fields {
            cdpos.push(&[
                "VERKBELEG",
                &object,
                &change,
                &table,
                &key,
                &name,
                "U",
                &new,
                &old,
            ]);
        }
    }
    // Standard categories are independent of the configurable document type.
    for (table, column, mappings, default) in [
        (
            &mut vbak,
            "AUART",
            &[("RE", "H"), ("CR", "K"), ("DR", "L")][..],
            "C",
        ),
        (&mut likp, "LFART", &[("LR", "T")][..], "J"),
        (
            &mut books.vbrk,
            "FKART",
            &[("S1", "N"), ("RE", "O"), ("G2", "O")][..],
            "M",
        ),
    ] {
        let at = table.header.split(',').position(|c| c == column).unwrap();
        table.header.push_str(",VBTYP");
        for row in &mut table.rows {
            let category = mappings
                .iter()
                .find(|(kind, _)| *kind == row[at])
                .map_or(default, |(_, category)| category);
            row.push(category.to_string());
        }
    }
    Generated {
        tables: vec![
            ("KNA1", kna1),
            ("VBAK", vbak),
            ("VBAP", vbap),
            ("LIKP", likp),
            ("LIPS", lips),
            ("VBRK", books.vbrk),
            ("VBRP", books.vbrp),
            ("BKPF", books.bkpf),
            ("BSAD", bsad),
            ("CDHDR", cdhdr),
            ("CDPOS", cdpos),
        ],
        truth,
    }
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn save(dir: &Path, tables: &[(&'static str, Table)]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, table) in tables {
        let mut text = table.header.clone();
        text.push('\n');
        for row in &table.rows {
            text.push_str(
                &row.iter()
                    .map(|v| csv_field(v))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            text.push('\n');
        }
        std::fs::write(dir.join(format!("{name}.csv")), text)?;
    }
    Ok(())
}

/// Writes one CSV per on-premise table into `dir`.
pub fn write(dir: &Path, settings: &Settings) -> io::Result<Truth> {
    let generated = generate(settings);
    save(dir, &generated.tables)?;
    Ok(generated.truth)
}

/// Writes the same business in the S/4HANA Public Cloud layout: readable
/// field names, ISO dates, company fields named `YY1_…`, and customer
/// journal entry items in place of BKPF and BSAD.
pub fn write_cloud(dir: &Path, settings: &Settings) -> io::Result<Truth> {
    let generated = generate(settings);
    save(dir, &cloud_files(&generated))?;
    Ok(generated.truth)
}

fn cloud_files(generated: &Generated) -> Vec<(&'static str, Table)> {
    let find = |name: &str| &generated.tables.iter().find(|(n, _)| *n == name).unwrap().1;
    let iso = |value: &str| {
        NaiveDate::parse_from_str(value, "%Y%m%d").map_or(String::new(), |d| d.to_string())
    };
    let clock = |value: &str| format!("{}:{}:{}", &value[0..2], &value[2..4], &value[4..6]);
    let map = |source: &Table, header: &'static str, fields: &[(&str, char)]| {
        let mut out = Table::new(header);
        for row in &source.rows {
            let values = fields
                .iter()
                .map(|(column, kind)| {
                    let value = source.get(row, column);
                    match kind {
                        'd' => iso(value),
                        't' => clock(value),
                        _ => value.to_owned(),
                    }
                })
                .collect::<Vec<_>>();
            out.push(&values.iter().map(String::as_str).collect::<Vec<_>>());
        }
        out
    };
    let mut files = vec![
        (
            "Customer",
            map(
                find("KNA1"),
                "Customer,CustomerName,Country",
                &[("KUNNR", 's'), ("NAME1", 's'), ("LAND1", 's')],
            ),
        ),
        (
            "SalesOrder",
            map(
                find("VBAK"),
                "SalesOrder,SalesOrderType,SalesOrganization,SoldToParty,CreationDate,CreationTime,CreatedByUser,TotalNetAmount,TransactionCurrency,CompanyCode,ReferenceSDDocument,YY1_ConfirmedBy_SDH,YY1_ConfirmationSentOn_SDH",
                &[
                    ("VBELN", 's'),
                    ("AUART", 's'),
                    ("VKORG", 's'),
                    ("KUNNR", 's'),
                    ("ERDAT", 'd'),
                    ("ERZET", 't'),
                    ("ERNAM", 's'),
                    ("NETWR", 's'),
                    ("WAERK", 's'),
                    ("BUKRS_VF", 's'),
                    ("VGBEL", 's'),
                    ("ZZCONF_BY", 's'),
                    ("ZZCONF_DAT", 'd'),
                ],
            ),
        ),
        (
            "SalesOrderItem",
            map(
                find("VBAP"),
                "SalesOrder,SalesOrderItem,Material,SalesOrderItemText,RequestedQuantity,NetAmount,Plant,SalesDocumentRjcnReason,ReferenceSDDocument,ReferenceSDDocumentItem",
                &[
                    ("VBELN", 's'),
                    ("POSNR", 's'),
                    ("MATNR", 's'),
                    ("ARKTX", 's'),
                    ("KWMENG", 's'),
                    ("NETWR", 's'),
                    ("WERKS", 's'),
                    ("ABGRU", 's'),
                    ("VGBEL", 's'),
                    ("VGPOS", 's'),
                ],
            ),
        ),
        (
            "OutboundDelivery",
            map(
                find("LIKP"),
                "OutboundDelivery,DeliveryDocumentType,CreationDate,CreationTime,PlannedGoodsIssueDate,ActualGoodsMovementDate,SoldToParty,ShippingPoint",
                &[
                    ("VBELN", 's'),
                    ("LFART", 's'),
                    ("ERDAT", 'd'),
                    ("ERZET", 't'),
                    ("LFDAT", 'd'),
                    ("WADAT_IST", 'd'),
                    ("KUNNR", 's'),
                    ("VSTEL", 's'),
                ],
            ),
        ),
        (
            "OutboundDeliveryItem",
            map(
                find("LIPS"),
                "OutboundDelivery,OutboundDeliveryItem,ReferenceSDDocument,ReferenceSDDocumentItem,Material,ActualDeliveryQuantity",
                &[
                    ("VBELN", 's'),
                    ("POSNR", 's'),
                    ("VGBEL", 's'),
                    ("VGPOS", 's'),
                    ("MATNR", 's'),
                    ("LFIMG", 's'),
                ],
            ),
        ),
        (
            "BillingDocument",
            map(
                find("VBRK"),
                "BillingDocument,BillingDocumentType,BillingDocumentDate,CreationDate,CreationTime,CompanyCode,SoldToParty,TotalNetAmount,TransactionCurrency,BillingDocumentIsCancelled,CancelledBillingDocument",
                &[
                    ("VBELN", 's'),
                    ("FKART", 's'),
                    ("FKDAT", 'd'),
                    ("ERDAT", 'd'),
                    ("ERZET", 't'),
                    ("BUKRS", 's'),
                    ("KUNAG", 's'),
                    ("NETWR", 's'),
                    ("WAERK", 's'),
                    ("FKSTO", 's'),
                    ("SFAKN", 's'),
                ],
            ),
        ),
        (
            "BillingDocumentItem",
            map(
                find("VBRP"),
                "BillingDocument,BillingDocumentItem,SalesDocument,SalesDocumentItem,ReferenceSDDocument,ReferenceSDDocumentItem,BillingQuantity,NetAmount",
                &[
                    ("VBELN", 's'),
                    ("POSNR", 's'),
                    ("AUBEL", 's'),
                    ("AUPOS", 's'),
                    ("VGBEL", 's'),
                    ("VGPOS", 's'),
                    ("FKIMG", 's'),
                    ("NETWR", 's'),
                ],
            ),
        ),
    ];
    // Customer journal entry items: every BSAD line, plus open invoice lines.
    let bkpf = find("BKPF");
    let bsad = find("BSAD");
    let mut journal = Table::new(
        "CompanyCode,AccountingDocument,FiscalYear,AccountingDocumentItem,AccountingDocumentType,FinancialAccountType,DebitCreditCode,Customer,PostingDate,ReferenceDocumentType,OriginalReferenceDocument,AmountInCompanyCodeCurrency,ClearingDate,ClearingJournalEntry,ClearingJournalEntryFiscalYear",
    );
    let reference = |company: &str, document: &str, year: &str| {
        bkpf.rows
            .iter()
            .find(|r| {
                bkpf.get(r, "BUKRS") == company
                    && bkpf.get(r, "BELNR") == document
                    && bkpf.get(r, "GJAHR") == year
            })
            .map(|r| {
                (
                    bkpf.get(r, "AWTYP").to_owned(),
                    bkpf.get(r, "AWKEY").to_owned(),
                )
            })
            .unwrap_or_default()
    };
    let mut cleared = std::collections::BTreeSet::new();
    for row in &bsad.rows {
        let (company, document, year) = (
            bsad.get(row, "BUKRS"),
            bsad.get(row, "BELNR"),
            bsad.get(row, "GJAHR"),
        );
        cleared.insert((company.to_owned(), document.to_owned(), year.to_owned()));
        let (kind, key) = reference(company, document, year);
        journal.push(&[
            company,
            document,
            year,
            bsad.get(row, "BUZEI"),
            bsad.get(row, "BLART"),
            "D",
            bsad.get(row, "SHKZG"),
            bsad.get(row, "KUNNR"),
            &iso(bsad.get(row, "BUDAT")),
            &kind,
            &key,
            bsad.get(row, "DMBTR"),
            &iso(bsad.get(row, "AUGDT")),
            bsad.get(row, "AUGBL"),
            bsad.get(row, "AUGGJ"),
        ]);
    }
    let vbrk = find("VBRK");
    for row in &bkpf.rows {
        let key = (
            bkpf.get(row, "BUKRS").to_owned(),
            bkpf.get(row, "BELNR").to_owned(),
            bkpf.get(row, "GJAHR").to_owned(),
        );
        if bkpf.get(row, "AWTYP") != "VBRK" || cleared.contains(&key) {
            continue;
        }
        let billing = bkpf.get(row, "AWKEY");
        let bill = vbrk
            .rows
            .iter()
            .find(|r| vbrk.get(r, "VBELN") == billing)
            .unwrap();
        let credit = ["S1", "RE", "G2"].contains(&vbrk.get(bill, "FKART"));
        journal.push(&[
            &key.0,
            &key.1,
            &key.2,
            "001",
            "RV",
            "D",
            if credit { "H" } else { "S" },
            vbrk.get(bill, "KUNAG"),
            &iso(bkpf.get(row, "BUDAT")),
            "VBRK",
            billing,
            vbrk.get(bill, "NETWR"),
            "",
            "",
            "",
        ]);
    }
    files.push(("JournalEntryItem", journal));
    files.push((
        "ChangeDocument",
        map(
            find("CDHDR"),
            "ChangeDocObjectClass,ChangeDocObject,ChangeDocument,CreatedByUser,CreationDate,CreationTime,ChangeTransactionCode",
            &[("OBJECTCLAS", 's'), ("OBJECTID", 's'), ("CHANGENR", 's'), ("USERNAME", 's'), ("UDATE", 'd'), ("UTIME", 't'), ("TCODE", 's')],
        ),
    ));
    files.push((
        "ChangeDocumentItem",
        map(
            find("CDPOS"),
            "ChangeDocObjectClass,ChangeDocObject,ChangeDocument,DatabaseTable,ChangeDocTableKey,ChangeDocDatabaseTableField,ChangeDocItemChangeType,ChangeDocNewFieldValue,ChangeDocPreviousFieldValue",
            &[("OBJECTCLAS", 's'), ("OBJECTID", 's'), ("CHANGENR", 's'), ("TABNAME", 's'), ("TABKEY", 's'), ("FNAME", 's'), ("CHNGIND", 's'), ("VALUE_NEW", 's'), ("VALUE_OLD", 's')],
        ),
    ));
    files
}

/// What the made-up front-end files contain, known by construction.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FrontTruth {
    pub opportunities: u64,
    pub won: u64,
    pub lost: u64,
    pub quotes: u64,
    /// Quotes that went through approval.
    pub quotes_approved: u64,
    pub quotes_sent: u64,
    pub quotes_accepted: u64,
    /// Stage changes recorded in the opportunity field history (Salesforce).
    pub stage_changes: u64,
    /// SAP orders that came from a won opportunity, and those that did not.
    pub orders_from_front: u64,
    pub orders_without_front: u64,
}

/// Which front-end system sits before the SAP order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Front {
    /// SAP Sales Cloud: opportunity and quote reports, readable column labels,
    /// local times in one field.
    SalesCloud,
    /// Salesforce: Opportunity, OpportunityFieldHistory and Quote as the extract
    /// writes them, times in UTC, the SAP order number in a custom field.
    Salesforce,
}

/// The front-end files for each layout.
pub const SALES_CLOUD_FILES: [&str; 2] = ["SalesCloudOpportunity", "SalesCloudQuote"];
pub const SALESFORCE_FILES: [&str; 3] = ["Opportunity", "OpportunityFieldHistory", "Quote"];

/// Writes the order-to-cash business with opportunities and quotes in a front
/// end before the SAP orders; `cloud` picks the S/4HANA Public Cloud layout
/// for the SAP side.
pub fn write_front(
    dir: &Path,
    settings: &Settings,
    front: Front,
    cloud: bool,
) -> io::Result<(Truth, FrontTruth)> {
    let generated = generate(settings);
    let mut files = if cloud {
        cloud_files(&generated)
    } else {
        generated
            .tables
            .iter()
            .map(|(name, table)| {
                (
                    *name,
                    Table {
                        header: table.header.clone(),
                        rows: table.rows.clone(),
                    },
                )
            })
            .collect()
    };
    let vbak = &generated
        .tables
        .iter()
        .find(|(n, _)| *n == "VBAK")
        .unwrap()
        .1;
    let mut rng = Rng::new(settings.seed ^ 0xF407);
    let mut truth = FrontTruth::default();
    let day = |value: &str| NaiveDate::parse_from_str(value, "%Y%m%d").unwrap();
    let local = |day: NaiveDate, minutes: i64| {
        (day.and_hms_opt(0, 0, 0).unwrap() + Duration::minutes(minutes))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    };
    // Salesforce writes UTC; the made-up local times are one hour ahead.
    let utc = |day: NaiveDate, minutes: i64| {
        (day.and_hms_opt(0, 0, 0).unwrap() + Duration::minutes(minutes - 60))
            .format("%Y-%m-%dT%H:%M:%S.000Z")
            .to_string()
    };
    let mut opportunities = match front {
        Front::SalesCloud => {
            Table::new("Opportunity ID,Name,Account ID,Created On,Status,Closed On")
        }
        Front::Salesforce => Table::new(
            "Id,Name,AccountId,CreatedDate,StageName,IsClosed,IsWon,CloseDate,Amount,SAP_Order_Number__c",
        ),
    };
    let mut quotes = match front {
        Front::SalesCloud => Table::new(
            "Quote ID,Opportunity ID,Created On,Submitted On,Approved On,Sent On,Accepted On,ERP Sales Order ID,Status",
        ),
        Front::Salesforce => Table::new("Id,OpportunityId,Name,CreatedDate,Status"),
    };
    let mut history = Table::new("Id,OpportunityId,Field,OldValue,NewValue,CreatedDate,DataType");
    let stages = [
        "Prospecting",
        "Qualification",
        "Proposal/Price Quote",
        "Negotiation/Review",
    ];
    let mut opportunity_number = 0u64;
    let mut history_number = 0u64;
    let mut write_opportunity = |rng: &mut Rng,
                                 truth: &mut FrontTruth,
                                 opportunities: &mut Table,
                                 quotes: &mut Table,
                                 history: &mut Table,
                                 customer: &str,
                                 start: NaiveDate,
                                 end: NaiveDate,
                                 order: Option<&str>| {
        opportunity_number += 1;
        truth.opportunities += 1;
        let won = order.is_some();
        if won {
            truth.won += 1;
        } else {
            truth.lost += 1;
        }
        let id = match front {
            Front::SalesCloud => format!("{}", 50_000 + opportunity_number),
            Front::Salesforce => format!("0065g00000{:08}", opportunity_number),
        };
        let opened = 9 * 60 + rng.below(8 * 60) as i64;
        let closed = 16 * 60;
        let span = (end - start).num_days();
        let quoted = start + Duration::days(2 + rng.below((span / 2 - 1).max(1) as u64) as i64);
        match front {
            Front::SalesCloud => opportunities.push(&[
                &id,
                "Equipment deal",
                customer,
                &local(start, opened),
                if won { "Won" } else { "Lost" },
                &local(end, closed),
            ]),
            Front::Salesforce => {
                opportunities.push(&[
                    &id,
                    "Equipment deal",
                    customer,
                    &utc(start, opened),
                    if won { "Closed Won" } else { "Closed Lost" },
                    "true",
                    if won { "true" } else { "false" },
                    &end.to_string(),
                    "25000.00",
                    order.unwrap_or(""),
                ]);
                // Field history: a "created" row, then each stage change.
                history_number += 1;
                history.push(&[
                    &format!("0175g00000{:08}", history_number),
                    &id,
                    "created",
                    "",
                    "",
                    &utc(start, opened),
                    "Text",
                ]);
                let reached = 2 + rng.below(3) as usize;
                let mut previous = stages[0];
                for (n, stage) in stages.iter().take(reached).enumerate().skip(1) {
                    history_number += 1;
                    truth.stage_changes += 1;
                    let at = utc(
                        start + Duration::days(n as i64 * (span / 5).max(1)),
                        opened + 30,
                    );
                    history.push(&[
                        &format!("0175g00000{:08}", history_number),
                        &id,
                        "StageName",
                        previous,
                        stage,
                        &at,
                        "Picklist",
                    ]);
                    previous = stage;
                }
                history_number += 1;
                truth.stage_changes += 1;
                history.push(&[
                    &format!("0175g00000{:08}", history_number),
                    &id,
                    "StageName",
                    previous,
                    if won { "Closed Won" } else { "Closed Lost" },
                    &utc(end, closed),
                    "Picklist",
                ]);
            }
        }
        // One quote: approved for some, sent, then accepted when won.
        truth.quotes += 1;
        let quote = match front {
            Front::SalesCloud => format!("{}", 70_000 + opportunity_number),
            Front::Salesforce => format!("0Q05g00000{:08}", opportunity_number),
        };
        let approval = rng.chance(0.6);
        let submitted = quoted + Duration::days(1);
        let approved = approval.then(|| quoted + Duration::days(2));
        let sent = quoted + Duration::days(3);
        truth.quotes_sent += 1;
        if approval {
            truth.quotes_approved += 1;
        }
        if won {
            truth.quotes_accepted += 1;
        }
        match front {
            Front::SalesCloud => quotes.push(&[
                &quote,
                &id,
                &local(quoted, opened + 15),
                &if approval {
                    local(submitted, 10 * 60)
                } else {
                    String::new()
                },
                &approved.map_or(String::new(), |d| local(d, 11 * 60)),
                &local(sent, 12 * 60),
                &if won {
                    local(end - Duration::days(1), 15 * 60)
                } else {
                    String::new()
                },
                order.unwrap_or(""),
                if won { "Accepted" } else { "Rejected" },
            ]),
            Front::Salesforce => quotes.push(&[
                &quote,
                &id,
                "Equipment quote",
                &utc(quoted, opened + 15),
                if won { "Accepted" } else { "Rejected" },
            ]),
        }
    };
    for order in &vbak.rows {
        if vbak.get(order, "AUART") != "OR" {
            continue;
        }
        let created = day(vbak.get(order, "ERDAT"));
        let customer = vbak.get(order, "KUNNR");
        if !rng.chance(0.85) {
            truth.orders_without_front += 1;
            continue;
        }
        truth.orders_from_front += 1;
        let start = created - Duration::days(12 + rng.below(48) as i64);
        write_opportunity(
            &mut rng,
            &mut truth,
            &mut opportunities,
            &mut quotes,
            &mut history,
            customer,
            start,
            created - Duration::days(1),
            Some(vbak.get(order, "VBELN")),
        );
    }
    let first = NaiveDate::from_ymd_opt(2025, 10, 1).unwrap();
    for n in 0..settings.orders / 5 {
        let start = first + Duration::days(rng.below(120) as i64);
        let customer = number(200_001 + n % CUSTOMERS.len() as u64, 10);
        let end = start + Duration::days(14 + rng.below(30) as i64);
        write_opportunity(
            &mut rng,
            &mut truth,
            &mut opportunities,
            &mut quotes,
            &mut history,
            &customer,
            start,
            end,
            None,
        );
    }
    match front {
        Front::SalesCloud => {
            files.push(("SalesCloudOpportunity", opportunities));
            files.push(("SalesCloudQuote", quotes));
        }
        Front::Salesforce => {
            files.push(("Opportunity", opportunities));
            files.push(("OpportunityFieldHistory", history));
            files.push(("Quote", quotes));
        }
    }
    save(dir, &files)?;
    Ok((generated.truth, truth))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_tables_and_every_story_present() {
        let first = generate(&Settings::default());
        let again = generate(&Settings::default());
        assert_eq!(first.truth, again.truth);
        let truth = first.truth;
        assert!(truth.rejected_items > 0 && truth.split_items > 0);
        assert!(truth.cancellations > 0 && truth.open_invoices > 0);
        assert!(truth.deliveries_shipped < truth.deliveries);
        assert_eq!(
            truth.payments + truth.open_invoices + truth.cancellations,
            truth.invoices
        );
        let share = truth.credit_blocked_orders as f64 / truth.orders as f64;
        assert!((0.08..0.25).contains(&share), "{share}");
    }
}
