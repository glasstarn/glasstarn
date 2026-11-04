//! Made-up purchase-to-pay tables in the layout of standard SAP ECC and
//! S/4HANA tables, as a company's SAP administrator would export them with
//! the data browser: one CSV per table, SAP field names as headers, dates as
//! `YYYYMMDD`, times as `HHMMSS`, document numbers with leading zeros.
//!
//! The table and field names follow public knowledge of the standard SAP
//! data model; nothing is copied from SAP documentation or code. Every
//! business story below is invented, and the counts in [`Truth`] are known by
//! construction, so tests can check the pack and the engine against them.
//!
//! Stories included on purpose:
//! - orders with and without a purchase requisition;
//! - partial deliveries, and goods receipts reversed and posted again;
//! - invoices recorded before the goods arrived;
//! - the same invoice number in two fiscal years, and the same accounting
//!   document number in two company codes;
//! - invoices not yet paid, and vendor invoices posted with no order;
//! - deleted order lines;
//! - a custom approver field (`ZZAPPR_BY`, `ZZAPPR_DAT`) on some order
//!   headers, filled one to three days after the order was created.

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
    pub deleted_items: u64,
    pub requisition_items: u64,
    pub requisitions_released: u64,
    pub goods_receipts: u64,
    pub goods_receipt_reversals: u64,
    /// Supplier invoices recorded against orders (RBKP rows).
    pub invoices: u64,
    /// Distinct MM invoice and credit-memo source items.
    pub invoice_items: u64,
    pub invoices_paid: u64,
    /// Vendor invoices posted straight in accounting, with no order.
    pub invoices_without_order: u64,
    pub invoices_without_order_paid: u64,
    /// Order items whose invoice was recorded before their first goods receipt.
    pub items_invoiced_before_receipt: u64,
    pub orders_with_custom_approver: u64,
    /// Invoice numbers used in both fiscal years.
    pub invoice_numbers_in_two_years: u64,
    /// Release steps recorded in the change log (FRGZU), and final releases.
    pub release_steps: u64,
    pub releases: u64,
    pub price_changes: u64,
    pub quantity_changes: u64,
    pub delivery_date_changes: u64,
    /// Goods sent back to the vendor (movement type 122).
    pub goods_returns: u64,
    /// Vendor credit memos for returned goods.
    pub credit_memos: u64,
    /// Payment blocks set on invoices, and those released again.
    pub payment_blocks: u64,
    pub payment_block_releases: u64,
}

/// Settings for one generated export.
#[derive(Debug, Clone)]
pub struct Settings {
    pub orders: u64,
    pub seed: u64,
    /// Company codes; the first gets most orders.
    pub company_codes: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            orders: 400,
            seed: 7,
            company_codes: vec!["1000".into(), "2000".into()],
        }
    }
}

/// The tables in the order an export checklist lists them.
pub const TABLES: [&str; 12] = [
    "LFA1", "EBAN", "EKKO", "EKPO", "EKBE", "RBKP", "RSEG", "BKPF", "BSAK", "CDHDR", "CDPOS",
    "T16FB",
];

struct Table {
    header: &'static str,
    rows: Vec<String>,
}

impl Table {
    fn new(header: &'static str) -> Self {
        Self {
            header,
            rows: Vec::new(),
        }
    }
    fn push(&mut self, values: &[&str]) {
        debug_assert_eq!(values.len(), self.header.split(',').count());
        self.rows.push(
            values
                .iter()
                .map(|value| csv_field(value))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn date(day: NaiveDate) -> String {
    day.format("%Y%m%d").to_string()
}

fn number(value: u64, width: usize) -> String {
    format!("{value:0width$}")
}

const VENDORS: [(&str, &str); 8] = [
    ("Northwind Components", "DE"),
    ("Blue Harbor Supplies", "NL"),
    ("Carver Industrial", "US"),
    ("Delta Office Goods", "DE"),
    ("Evergreen Packaging", "FR"),
    ("Falcon Logistics", "DE"),
    ("Granite Tools", "PL"),
    ("Helios Electrics", "IT"),
];
const MATERIALS: [(&str, &str, u64); 6] = [
    ("MAT-100", "Steel bracket", 1250),
    ("MAT-200", "Packing carton", 85),
    ("MAT-300", "Copy paper", 420),
    ("MAT-400", "Cable set", 2300),
    ("MAT-500", "Safety gloves", 640),
    ("MAT-600", "Pallet wrap", 310),
];
const BUYERS: [&str; 4] = ["MKOCH", "JSILVA", "ANOWAK", "PDUBOIS"];
const APPROVERS: [&str; 3] = ["LHEAD", "CFOAPPR", "RMEYER"];
const AP_CLERKS: [&str; 3] = ["APCLERK1", "APCLERK2", "APBATCH"];

/// Per company code and fiscal year document counters, as SAP number ranges
/// restart each year (and accounting ranges are per company code).
#[derive(Default)]
struct Ranges(BTreeMap<(String, i32, &'static str), u64>);
impl Ranges {
    fn next(&mut self, company: &str, year: i32, range: &'static str, start: u64) -> String {
        let counter = self
            .0
            .entry((company.to_owned(), year, range))
            .or_insert(start);
        *counter += 1;
        number(*counter, 10)
    }
}

/// Writes one CSV per table into `dir` and returns what they contain.
pub fn write(dir: &Path, settings: &Settings) -> io::Result<Truth> {
    let (tables, _, truth) = generate(settings);
    save(dir, &tables)?;
    Ok(truth)
}

fn save(dir: &Path, tables: &[(&'static str, Table)]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, table) in tables {
        let mut text = String::with_capacity(table.rows.len() * 80);
        text.push_str(table.header);
        text.push('\n');
        for row in &table.rows {
            text.push_str(row);
            text.push('\n');
        }
        std::fs::write(dir.join(format!("{name}.csv")), text)?;
    }
    Ok(())
}

/// The files of the S/4HANA Public Cloud layout, as downloads of SAP's
/// released data views would name them.
pub const CLOUD_TABLES: [&str; 10] = [
    "Supplier",
    "PurchaseRequisitionItem",
    "PurchaseOrder",
    "PurchaseOrderItem",
    "MaterialDocumentItem",
    "SupplierInvoice",
    "SupplierInvoiceItemPurOrdRef",
    "JournalEntryItem",
    "ChangeDocument",
    "ChangeDocumentItem",
];

/// Writes the same made-up business as [`write`], in the layout of SAP
/// S/4HANA Public Cloud: no raw tables, only SAP's released views with
/// readable field names, ISO dates, and company fields named `YY1_…`.
/// Goods receipts and invoices carry posting dates only.
pub fn write_cloud(dir: &Path, settings: &Settings) -> io::Result<Truth> {
    let (tables, journal, truth) = generate(settings);
    save(dir, &cloud_tables(&tables, &journal))?;
    Ok(truth)
}

type Tables = Vec<(&'static str, Table)>;

fn cloud_tables(tables: &Tables, journal: &Table) -> Tables {
    let find = |name: &str| &tables.iter().find(|(n, _)| *n == name).unwrap().1;
    let iso = |value: &str| {
        NaiveDate::parse_from_str(value, "%Y%m%d").map_or(String::new(), |d| d.to_string())
    };
    // (source column, cloud field, is a date)
    let pick = |table: &Table, header: &'static str, fields: &[(&str, bool)], keep: Keep| {
        let columns = table.header.split(',').collect::<Vec<_>>();
        let mut out = Table::new(header);
        for row in &table.rows {
            assert!(!row.contains('"'), "generated values contain no quotes");
            let values = row.split(',').collect::<Vec<_>>();
            let get = |name: &str| {
                values[columns.iter().position(|c| *c == name).expect(name)].to_owned()
            };
            if !keep(&get) {
                continue;
            }
            let picked = fields
                .iter()
                .map(|(column, is_date)| {
                    let value = get(column);
                    if *is_date { iso(&value) } else { value }
                })
                .collect::<Vec<_>>();
            out.push(&picked.iter().map(String::as_str).collect::<Vec<_>>());
        }
        out
    };
    let all = |_: &dyn Fn(&str) -> String| true;
    let cloud = vec![
        (
            "Supplier",
            pick(
                find("LFA1"),
                "Supplier,SupplierName,Country",
                &[("LIFNR", false), ("NAME1", false), ("LAND1", false)],
                &all,
            ),
        ),
        (
            "PurchaseRequisitionItem",
            pick(
                find("EBAN"),
                "PurchaseRequisition,PurchaseRequisitionItem,CreationDate,Material,RequestedQuantity,PurchaseOrder,PurchaseOrderItem",
                &[
                    ("BANFN", false),
                    ("BNFPO", false),
                    ("BADAT", true),
                    ("MATNR", false),
                    ("MENGE", false),
                    ("EBELN", false),
                    ("EBELP", false),
                ],
                &all,
            ),
        ),
        (
            "PurchaseOrder",
            pick(
                find("EKKO"),
                "PurchaseOrder,CompanyCode,PurchaseOrderType,Supplier,CreationDate,CreatedByUser,PurchasingOrganization,PurchasingGroup,DocumentCurrency,YY1_ApprovedBy_PDH,YY1_ApprovalDate_PDH",
                &[
                    ("EBELN", false),
                    ("BUKRS", false),
                    ("BSART", false),
                    ("LIFNR", false),
                    ("AEDAT", true),
                    ("ERNAM", false),
                    ("EKORG", false),
                    ("EKGRP", false),
                    ("WAERS", false),
                    ("ZZAPPR_BY", false),
                    ("ZZAPPR_DAT", true),
                ],
                &all,
            ),
        ),
        (
            "PurchaseOrderItem",
            pick(
                find("EKPO"),
                "PurchaseOrder,PurchaseOrderItem,PurchasingDocumentDeletionCode,Material,PurchaseOrderItemText,Plant,OrderQuantity,NetPriceAmount,NetAmount",
                &[
                    ("EBELN", false),
                    ("EBELP", false),
                    ("LOEKZ", false),
                    ("MATNR", false),
                    ("TXZ01", false),
                    ("WERKS", false),
                    ("MENGE", false),
                    ("NETPR", false),
                    ("NETWR", false),
                ],
                &all,
            ),
        ),
        (
            "MaterialDocumentItem",
            pick(
                find("EKBE"),
                "MaterialDocumentYear,MaterialDocument,MaterialDocumentItem,GoodsMovementType,PurchaseOrder,PurchaseOrderItem,PostingDate,QuantityInEntryUnit",
                &[
                    ("GJAHR", false),
                    ("BELNR", false),
                    ("BUZEI", false),
                    ("BWART", false),
                    ("EBELN", false),
                    ("EBELP", false),
                    ("BUDAT", true),
                    ("MENGE", false),
                ],
                &|get: &dyn Fn(&str) -> String| get("VGABE") == "1",
            ),
        ),
        (
            "SupplierInvoice",
            pick(
                find("RBKP"),
                "SupplierInvoice,FiscalYear,CompanyCode,InvoicingParty,DocumentDate,PostingDate,InvoiceGrossAmount,DocumentCurrency,SupplierInvoiceIDByInvcgParty,IsInvoice",
                &[
                    ("BELNR", false),
                    ("GJAHR", false),
                    ("BUKRS", false),
                    ("LIFNR", false),
                    ("BLDAT", true),
                    ("BUDAT", true),
                    ("RMWWR", false),
                    ("WAERS", false),
                    ("XBLNR", false),
                    ("XRECH", false),
                ],
                &all,
            ),
        ),
        (
            "SupplierInvoiceItemPurOrdRef",
            pick(
                find("RSEG"),
                "SupplierInvoice,FiscalYear,SupplierInvoiceItem,PurchaseOrder,PurchaseOrderItem,QuantityInPurchaseOrderUnit,SupplierInvoiceItemAmount",
                &[
                    ("BELNR", false),
                    ("GJAHR", false),
                    ("BUZEI", false),
                    ("EBELN", false),
                    ("EBELP", false),
                    ("MENGE", false),
                    ("WRBTR", false),
                ],
                &all,
            ),
        ),
    ];
    let mut cloud = cloud;
    let columns = journal.header.split(',').collect::<Vec<_>>();
    let mut entries = Table::new(
        "CompanyCode,AccountingDocument,FiscalYear,AccountingDocumentItem,AccountingDocumentType,FinancialAccountType,DebitCreditCode,Supplier,PostingDate,ReferenceDocumentType,OriginalReferenceDocument,AmountInCompanyCodeCurrency,ClearingDate,ClearingJournalEntry",
    );
    for row in &journal.rows {
        let values = row.split(',').collect::<Vec<_>>();
        let get = |name: &str| values[columns.iter().position(|c| *c == name).unwrap()];
        entries.push(&[
            get("BUKRS"),
            get("BELNR"),
            get("GJAHR"),
            "001",
            get("BLART"),
            "K",
            "H",
            get("LIFNR"),
            &iso(get("BUDAT")),
            get("AWTYP"),
            get("AWKEY"),
            get("DMBTR"),
            &iso(get("AUGDT")),
            get("AUGBL"),
        ]);
    }
    cloud.push(("JournalEntryItem", entries));
    // Invoice or credit memo, as the view's true / false.
    for (name, table) in cloud.iter_mut() {
        if *name == "SupplierInvoice" {
            for row in table.rows.iter_mut() {
                let flag = if row.ends_with(",X") { "true" } else { "false" };
                let cut = row.rfind(',').unwrap();
                row.truncate(cut + 1);
                row.push_str(flag);
            }
        }
    }
    // The change log as change documents and their items.
    let clock = |value: &str| format!("{}:{}:{}", &value[0..2], &value[2..4], &value[4..6]);
    let mut documents = Table::new(
        "ChangeDocObjectClass,ChangeDocObject,ChangeDocument,CreatedByUser,CreationDate,CreationTime,ChangeTransactionCode",
    );
    for row in records(find("CDHDR")) {
        documents.push(&[
            &row["OBJECTCLAS"],
            &row["OBJECTID"],
            &row["CHANGENR"],
            &row["USERNAME"],
            &iso(&row["UDATE"]),
            &clock(&row["UTIME"]),
            &row["TCODE"],
        ]);
    }
    let mut items = Table::new(
        "ChangeDocObjectClass,ChangeDocObject,ChangeDocument,DatabaseTable,ChangeDocTableKey,ChangeDocDatabaseTableField,ChangeDocItemChangeType,ChangeDocNewFieldValue,ChangeDocPreviousFieldValue",
    );
    for row in records(find("CDPOS")) {
        items.push(&[
            &row["OBJECTCLAS"],
            &row["OBJECTID"],
            &row["CHANGENR"],
            &row["TABNAME"],
            &row["TABKEY"],
            &row["FNAME"],
            &row["CHNGIND"],
            &row["VALUE_NEW"],
            &row["VALUE_OLD"],
        ]);
    }
    cloud.push(("ChangeDocument", documents));
    cloud.push(("ChangeDocumentItem", items));
    cloud
}

/// Decides from a row's values (read by column name) whether to keep it.
type Keep<'a> = &'a dyn Fn(&dyn Fn(&str) -> String) -> bool;

/// The generated ECC tables, every accounting vendor line, and the truth.
type Generated = (Vec<(&'static str, Table)>, Table, Truth);

fn generate(settings: &Settings) -> Generated {
    let mut rng = Rng::new(settings.seed);
    let mut truth = Truth::default();
    let mut ranges = Ranges::default();
    let mut lfa1 = Table::new("LIFNR,NAME1,LAND1,KTOKK");
    let mut eban = Table::new("BANFN,BNFPO,BSART,BADAT,ERNAM,MATNR,MENGE,FRGDT,EBELN,EBELP");
    let mut ekko = Table::new(
        "EBELN,BUKRS,BSART,LIFNR,AEDAT,ERNAM,EKORG,EKGRP,WAERS,FRGKE,ZZAPPR_BY,ZZAPPR_DAT",
    );
    let mut ekpo = Table::new("EBELN,EBELP,LOEKZ,MATNR,TXZ01,WERKS,MENGE,MEINS,NETPR,NETWR,ELIKZ");
    let mut ekbe = Table::new(
        "EBELN,EBELP,ZEKKN,VGABE,GJAHR,BELNR,BUZEI,BEWTP,BWART,BUDAT,MENGE,DMBTR,WAERS,CPUDT,CPUTM",
    );
    let mut rbkp = Table::new(
        "BELNR,GJAHR,BUKRS,BLART,BLDAT,BUDAT,CPUDT,CPUTM,USNAM,LIFNR,RMWWR,WAERS,XBLNR,XRECH",
    );
    let mut rseg = Table::new("BELNR,GJAHR,BUZEI,EBELN,EBELP,MENGE,WRBTR");
    let mut bkpf = Table::new("BUKRS,BELNR,GJAHR,BLART,BLDAT,BUDAT,CPUDT,CPUTM,USNAM,AWTYP,AWKEY");
    let mut bsak = Table::new(
        "BUKRS,LIFNR,AUGDT,AUGBL,GJAHR,BELNR,BUZEI,BLART,SHKZG,BUDAT,ZFBDT,ZBD1T,DMBTR,WAERS",
    );

    for (index, (name, country)) in VENDORS.iter().enumerate() {
        lfa1.push(&[&number(100001 + index as u64, 10), name, country, "LIEF"]);
    }
    let vendor = |index: usize| number(100001 + index as u64, 10);
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
    let span = 120;
    let mut invoice_numbers = BTreeMap::<String, Vec<i32>>::new();

    // Pays one accounting document's vendor line, or leaves it open.
    let pay = |rng: &mut Rng,
               ranges: &mut Ranges,
               bsak: &mut Table,
               company: &str,
               lifnr: &str,
               document: (&str, i32),
               blart: &str,
               posted: NaiveDate,
               amount: &str,
               share: f64|
     -> Option<(NaiveDate, String)> {
        if !rng.chance(share) {
            return None;
        }
        let terms = 30;
        let cleared = posted + Duration::days(terms - 10 + rng.below(36) as i64);
        let year = cleared.year();
        let clearing = ranges.next(company, year, "payment", 1_500_000_000);
        let (belnr, gjahr) = document;
        let gjahr = gjahr.to_string();
        let terms = terms.to_string();
        bsak.push(&[
            company,
            lifnr,
            &date(cleared),
            &clearing,
            &gjahr,
            belnr,
            "001",
            blart,
            "H",
            &date(posted),
            &date(posted),
            &terms,
            amount,
            "EUR",
        ]);
        bsak.push(&[
            company,
            lifnr,
            &date(cleared),
            &clearing,
            &year.to_string(),
            &clearing,
            "001",
            "KZ",
            "S",
            &date(cleared),
            &date(cleared),
            "0",
            amount,
            "EUR",
        ]);
        Some((cleared, clearing))
    };
    // Every accounting vendor line, paid or not, for the cloud layout.
    let mut journal =
        Table::new("BUKRS,BELNR,GJAHR,LIFNR,BLART,AWTYP,AWKEY,BUDAT,DMBTR,AUGDT,AUGBL");
    let note = |journal: &mut Table,
                company: &str,
                document: &str,
                year: i32,
                lifnr: &str,
                kind: (&str, &str, &str),
                posted: NaiveDate,
                amount: &str,
                paid: &Option<(NaiveDate, String)>| {
        let (blart, awtyp, awkey) = kind;
        journal.push(&[
            company,
            document,
            &year.to_string(),
            lifnr,
            blart,
            awtyp,
            awkey,
            &date(posted),
            amount,
            &paid.as_ref().map_or(String::new(), |p| date(p.0)),
            &paid.as_ref().map_or(String::new(), |p| p.1.clone()),
        ]);
    };

    for order in 0..settings.orders {
        truth.orders += 1;
        let company = if settings.company_codes.len() > 1 && rng.chance(0.2) {
            settings.company_codes[1].clone()
        } else {
            settings.company_codes[0].clone()
        };
        let vendor_index = rng.below(VENDORS.len() as u64) as usize;
        let lifnr = vendor(vendor_index);
        let created = first + Duration::days(rng.below(span) as i64);
        let ebeln = number(4_500_000_001 + order, 10);
        let buyer = BUYERS[rng.below(BUYERS.len() as u64) as usize];
        let (approver, approved) = if rng.chance(0.3) {
            truth.orders_with_custom_approver += 1;
            (
                APPROVERS[rng.below(APPROVERS.len() as u64) as usize].to_owned(),
                date(created + Duration::days(1 + rng.below(3) as i64)),
            )
        } else {
            (String::new(), "00000000".to_owned())
        };
        ekko.push(&[
            &ebeln,
            &company,
            "NB",
            &lifnr,
            &date(created),
            buyer,
            "1000",
            "001",
            "EUR",
            "R",
            &approver,
            &approved,
        ]);
        let with_requisition = rng.chance(0.75);
        let banfn = number(10_000_001 + order, 10);
        let requested = created - Duration::days(1 + rng.below(5) as i64);
        let released = (rng.chance(0.85)).then(|| {
            requested
                + Duration::days(rng.below((created - requested).num_days() as u64 + 1) as i64)
        });

        let items = 1 + rng.below(4);
        let mut invoice_lines = Vec::new();
        let mut last_receipt = created;
        for item in 0..items {
            truth.order_items += 1;
            let ebelp = number((item + 1) * 10, 5);
            let (matnr, text, price) = MATERIALS[rng.below(MATERIALS.len() as u64) as usize];
            let quantity = 1 + rng.below(20);
            let value = price * quantity;
            let amount = format!("{}.{:02}", value / 100, value % 100);
            let deleted = rng.chance(0.03);
            ekpo.push(&[
                &ebeln,
                &ebelp,
                if deleted { "L" } else { "" },
                matnr,
                text,
                "1100",
                &quantity.to_string(),
                "EA",
                &format!("{}.{:02}", price / 100, price % 100),
                &amount,
                "",
            ]);
            if with_requisition {
                truth.requisition_items += 1;
                if released.is_some() {
                    truth.requisitions_released += 1;
                }
                eban.push(&[
                    &banfn,
                    &ebelp,
                    "NB",
                    &date(requested),
                    "REQUESTER",
                    matnr,
                    &quantity.to_string(),
                    &released.map_or("00000000".to_owned(), date),
                    &ebeln,
                    &ebelp,
                ]);
            }
            if deleted {
                truth.deleted_items += 1;
                continue;
            }
            if !rng.chance(0.95) {
                continue;
            }
            // Goods receipts: sometimes split, sometimes reversed and reposted.
            let mut receipts = vec![(created + Duration::days(3 + rng.below(18) as i64), "101")];
            if rng.chance(0.1) {
                let later = receipts[0].0 + Duration::days(2 + rng.below(9) as i64);
                receipts.push((later, "101"));
            }
            if rng.chance(0.04) {
                let at = receipts[0].0 + Duration::days(1);
                receipts.insert(1, (at, "102"));
                receipts.insert(2, (at + Duration::days(1), "101"));
            }
            let mut receipt_line = 0;
            for (day, movement) in &receipts {
                receipt_line += 1;
                let year = day.year();
                let mblnr = ranges.next("all", year, "material", 5_000_000_000);
                if *movement == "102" {
                    truth.goods_receipt_reversals += 1;
                } else {
                    truth.goods_receipts += 1;
                }
                ekbe.push(&[
                    &ebeln,
                    &ebelp,
                    "00",
                    "1",
                    &year.to_string(),
                    &mblnr,
                    &number(receipt_line, 4),
                    "E",
                    movement,
                    &date(*day),
                    &quantity.to_string(),
                    &amount,
                    "EUR",
                    &date(*day),
                    &time(&mut rng),
                ]);
                last_receipt = last_receipt.max(*day);
            }
            invoice_lines.push((ebelp, quantity, amount, receipts[0].0));
        }
        // One invoice per order covering every received line.
        if invoice_lines.is_empty() || !rng.chance(0.9) {
            continue;
        }
        let early = rng.chance(0.12);
        let recorded = if early {
            invoice_lines.iter().map(|line| line.3).min().unwrap()
                - Duration::days(1 + rng.below(3) as i64)
        } else {
            last_receipt + Duration::days(1 + rng.below(15) as i64)
        };
        let year = recorded.year();
        let belnr = ranges.next("all", year, "invoice", 5_105_600_000);
        invoice_numbers.entry(belnr.clone()).or_default().push(year);
        let gjahr = year.to_string();
        let at = time(&mut rng);
        let total: u64 = invoice_lines
            .iter()
            .map(|line| {
                let (whole, cents) = line.2.split_once('.').unwrap();
                whole.parse::<u64>().unwrap() * 100 + cents.parse::<u64>().unwrap()
            })
            .sum();
        let total = format!("{}.{:02}", total / 100, total % 100);
        let clerk = AP_CLERKS[rng.below(AP_CLERKS.len() as u64) as usize];
        truth.invoices += 1;
        rbkp.push(&[
            &belnr,
            &gjahr,
            &company,
            "RE",
            &date(recorded),
            &date(recorded),
            &date(recorded),
            &at,
            clerk,
            &lifnr,
            &total,
            "EUR",
            &format!("INV-{}", 70000 + order),
            "X",
        ]);
        for (line, (ebelp, quantity, amount, first_receipt)) in invoice_lines.iter().enumerate() {
            truth.invoice_items += 1;
            if recorded < *first_receipt {
                truth.items_invoiced_before_receipt += 1;
            }
            rseg.push(&[
                &belnr,
                &gjahr,
                &number(line as u64 + 1, 6),
                &ebeln,
                ebelp,
                &quantity.to_string(),
                amount,
            ]);
            ekbe.push(&[
                &ebeln,
                ebelp,
                "00",
                "2",
                &gjahr,
                &belnr,
                &number(line as u64 + 1, 4),
                "Q",
                "",
                &date(recorded),
                &quantity.to_string(),
                amount,
                "EUR",
                &date(recorded),
                &at,
            ]);
        }
        let document = ranges.next(&company, year, "accounting", 1_900_000_000);
        bkpf.push(&[
            &company,
            &document,
            &gjahr,
            "RE",
            &date(recorded),
            &date(recorded),
            &date(recorded),
            &at,
            clerk,
            "RMRP",
            &format!("{belnr}{gjahr}"),
        ]);
        let paid = pay(
            &mut rng,
            &mut ranges,
            &mut bsak,
            &company,
            &lifnr,
            (&document, year),
            "RE",
            recorded,
            &total,
            0.88,
        );
        if paid.is_some() {
            truth.invoices_paid += 1;
        }
        note(
            &mut journal,
            &company,
            &document,
            year,
            &lifnr,
            ("RE", "RMRP", &format!("{belnr}{gjahr}")),
            recorded,
            &total,
            &paid,
        );
    }
    // Vendor invoices posted straight in accounting, with no order behind them.
    for _ in 0..settings.orders.div_ceil(12) {
        let company = settings.company_codes[0].clone();
        let lifnr = vendor(rng.below(VENDORS.len() as u64) as usize);
        let posted = first + Duration::days(rng.below(span) as i64);
        let year = posted.year();
        let document = ranges.next(&company, year, "accounting", 1_900_000_000);
        let amount = format!("{}.00", 200 + rng.below(5000));
        truth.invoices_without_order += 1;
        bkpf.push(&[
            &company,
            &document,
            &year.to_string(),
            "KR",
            &date(posted),
            &date(posted),
            &date(posted),
            &time(&mut rng),
            "APCLERK1",
            "BKPF",
            &format!("{document}{company}{year}"),
        ]);
        let paid = pay(
            &mut rng,
            &mut ranges,
            &mut bsak,
            &company,
            &lifnr,
            (&document, year),
            "KR",
            posted,
            &amount,
            0.8,
        );
        if paid.is_some() {
            truth.invoices_without_order_paid += 1;
        }
        note(
            &mut journal,
            &company,
            &document,
            year,
            &lifnr,
            ("KR", "BKPF", &format!("{document}{company}{year}")),
            posted,
            &amount,
            &paid,
        );
    }
    truth.invoice_numbers_in_two_years = invoice_numbers
        .values()
        .filter(|years| years.len() > 1)
        .count() as u64;
    let mut tables = vec![
        ("LFA1", lfa1),
        ("EBAN", eban),
        ("EKKO", ekko),
        ("EKPO", ekpo),
        ("EKBE", ekbe),
        ("RBKP", rbkp),
        ("RSEG", rseg),
        ("BKPF", bkpf),
        ("BSAK", bsak),
    ];
    later_events(&mut tables, &mut journal, &mut truth, settings);
    // Normal posted invoices in this constructed scenario. Origin/status are
    // exported explicitly so maintenance documents cannot masquerade as credit memos.
    let invoices = &mut tables
        .iter_mut()
        .find(|(name, _)| *name == "RBKP")
        .unwrap()
        .1;
    invoices.header = "BELNR,GJAHR,BUKRS,BLART,BLDAT,BUDAT,CPUDT,CPUTM,USNAM,LIFNR,RMWWR,WAERS,XBLNR,XRECH,IVTYP,RBSTAT,STBLG,STJAH";
    for row in &mut invoices.rows {
        row.push_str(",,5,,");
    }
    let orders = &mut tables
        .iter_mut()
        .find(|(name, _)| *name == "EKKO")
        .unwrap()
        .1;
    orders.header =
        "EBELN,BUKRS,BSART,LIFNR,AEDAT,ERNAM,EKORG,EKGRP,WAERS,FRGKE,ZZAPPR_BY,ZZAPPR_DAT,BSTYP";
    for row in &mut orders.rows {
        row.push_str(",F");
    }
    truth.invoice_items += truth.credit_memos;
    let mut release_config = Table::new("FRGKE,KZFRE");
    release_config.push(&["R", "X"]);
    release_config.push(&["B", ""]);
    tables.push(("T16FB", release_config));
    (tables, journal, truth)
}

/// Rows of a generated table as name → value maps.
fn records(table: &Table) -> Vec<BTreeMap<String, String>> {
    let columns = table.header.split(',').collect::<Vec<_>>();
    table
        .rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|c| (*c).to_owned())
                .zip(row.split(',').map(str::to_owned))
                .collect()
        })
        .collect()
}

/// What happens after the documents are created: the change log (order
/// releases, price, quantity and date changes, deleted lines, invoice payment
/// blocks), goods returned to the vendor and the vendor's credit memos.
fn later_events(tables: &mut Tables, journal: &mut Table, truth: &mut Truth, settings: &Settings) {
    let mut rng = Rng::new(settings.seed ^ 0xC4A5);
    let get =
        |tables: &Tables, name: &str| records(&tables.iter().find(|(n, _)| *n == name).unwrap().1);
    let (ekko, ekpo, ekbe, rbkp, rseg, bkpf) = (
        get(tables, "EKKO"),
        get(tables, "EKPO"),
        get(tables, "EKBE"),
        get(tables, "RBKP"),
        get(tables, "RSEG"),
        get(tables, "BKPF"),
    );
    let day = |value: &str| NaiveDate::parse_from_str(value, "%Y%m%d").unwrap();
    let clock = |rng: &mut Rng| {
        let seconds = 7 * 3600 + rng.below(10 * 3600);
        format!(
            "{:02}{:02}{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    };
    let mut cdhdr = Table::new("OBJECTCLAS,OBJECTID,CHANGENR,USERNAME,UDATE,UTIME,TCODE");
    let mut cdpos =
        Table::new("OBJECTCLAS,OBJECTID,CHANGENR,TABNAME,TABKEY,FNAME,CHNGIND,VALUE_NEW,VALUE_OLD");
    let mut change_number = 0u64;
    let mut change = |cdhdr: &mut Table,
                      cdpos: &mut Table,
                      rng: &mut Rng,
                      (class, object): (&str, &str),
                      (user, on, code): (&str, NaiveDate, &str),
                      fields: &[(&str, String, &str, &str, &str)]| {
        change_number += 1;
        let number = number(change_number, 10);
        cdhdr.push(&[class, object, &number, user, &date(on), &clock(rng), code]);
        for (table, key, field, new, old) in fields {
            cdpos.push(&[class, object, &number, table, key, field, "U", new, old]);
        }
    };
    // Order releases: one or two approval steps, then the release itself.
    for order in &ekko {
        if !rng.chance(0.6) {
            continue;
        }
        let created = day(&order["AEDAT"]);
        let object = ("EINKBELEG", order["EBELN"].as_str());
        let key = format!("100{}", order["EBELN"]);
        let steps = 1 + rng.below(2);
        let mut status = String::new();
        for step in 0..steps {
            status.push('X');
            truth.release_steps += 1;
            let on = created + Duration::days((step as i64).min(1));
            let user = ["LHEAD", "CFOAPPR"][step as usize];
            let old = status[1..].to_owned();
            change(
                &mut cdhdr,
                &mut cdpos,
                &mut rng,
                object,
                (user, on, "ME29N"),
                &[("EKKO", key.clone(), "FRGZU", &status, &old)],
            );
        }
        truth.releases += 1;
        change(
            &mut cdhdr,
            &mut cdpos,
            &mut rng,
            object,
            ("CFOAPPR", created + Duration::days(1), "ME29N"),
            &[("EKKO", key.clone(), "FRGKE", "R", "B")],
        );
    }
    // Line changes after the order was created, and deleted lines.
    let created_of = ekko
        .iter()
        .map(|o| (o["EBELN"].clone(), day(&o["AEDAT"])))
        .collect::<BTreeMap<_, _>>();
    for item in &ekpo {
        let created = created_of[&item["EBELN"]];
        let object = ("EINKBELEG", item["EBELN"].as_str());
        let key = format!("100{}{}", item["EBELN"], item["EBELP"]);
        let on = created + Duration::days(rng.below(2) as i64);
        if item["LOEKZ"] == "L" {
            change(
                &mut cdhdr,
                &mut cdpos,
                &mut rng,
                object,
                ("MKOCH", on, "ME22N"),
                &[("EKPO", key.clone(), "LOEKZ", "L", "")],
            );
            continue;
        }
        if rng.chance(0.08) {
            truth.price_changes += 1;
            change(
                &mut cdhdr,
                &mut cdpos,
                &mut rng,
                object,
                ("MKOCH", on, "ME22N"),
                &[("EKPO", key.clone(), "NETPR", &item["NETPR"], "0.00")],
            );
        }
        if rng.chance(0.05) {
            truth.quantity_changes += 1;
            change(
                &mut cdhdr,
                &mut cdpos,
                &mut rng,
                object,
                ("JSILVA", on, "ME22N"),
                &[("EKPO", key.clone(), "MENGE", &item["MENGE"], "1")],
            );
        }
        if rng.chance(0.1) {
            truth.delivery_date_changes += 1;
            let new = date(on + Duration::days(7));
            change(
                &mut cdhdr,
                &mut cdpos,
                &mut rng,
                object,
                ("JSILVA", on, "ME22N"),
                &[(
                    "EKET",
                    format!("{key}0001"),
                    "EINDT",
                    &new,
                    &date(on + Duration::days(3)),
                )],
            );
        }
    }
    // Payment blocks on invoices from purchasing, most released again.
    for posting in bkpf.iter().filter(|b| b["AWTYP"] == "RMRP") {
        if !rng.chance(0.1) {
            continue;
        }
        truth.payment_blocks += 1;
        let posted = day(&posting["CPUDT"]);
        let object_id = format!(
            "100{}{}{}",
            posting["BUKRS"], posting["BELNR"], posting["GJAHR"]
        );
        let object = ("BELEG", object_id.as_str());
        let key = format!(
            "100{}{}{}001",
            posting["BUKRS"], posting["BELNR"], posting["GJAHR"]
        );
        change(
            &mut cdhdr,
            &mut cdpos,
            &mut rng,
            object,
            ("APCLERK1", posted, "FB02"),
            &[("BSEG", key.clone(), "ZLSPR", "A", "")],
        );
        if rng.chance(0.8) {
            truth.payment_block_releases += 1;
            let released = posted + Duration::days(1 + rng.below(10) as i64);
            change(
                &mut cdhdr,
                &mut cdpos,
                &mut rng,
                object,
                ("APLEAD", released, "FB02"),
                &[("BSEG", key, "ZLSPR", "", "A")],
            );
        }
    }
    // Goods sent back to the vendor, and the vendor's credit memo.
    let mut extra_ekbe = Vec::new();
    let mut extra_rbkp = Vec::new();
    let mut extra_rseg = Vec::new();
    let mut extra_bkpf = Vec::new();
    let mut returns = 0u64;
    let invoice_of = rseg
        .iter()
        .map(|line| {
            (
                (line["EBELN"].clone(), line["EBELP"].clone()),
                (line["BELNR"].clone(), line["GJAHR"].clone()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    // Invoice headers by document and year, found in one step each.
    let mut header_of = BTreeMap::new();
    for (index, header) in rbkp.iter().enumerate() {
        header_of
            .entry((header["BELNR"].clone(), header["GJAHR"].clone()))
            .or_insert(index);
    }
    for receipt in ekbe
        .iter()
        .filter(|r| r["VGABE"] == "1" && r["BWART"] == "101")
    {
        if !rng.chance(0.03) {
            continue;
        }
        returns += 1;
        truth.goods_returns += 1;
        let back = day(&receipt["CPUDT"]) + Duration::days(2 + rng.below(5) as i64);
        let year = back.year().to_string();
        let document = number(5_090_000_000 + returns, 10);
        extra_ekbe.push(vec![
            receipt["EBELN"].clone(),
            receipt["EBELP"].clone(),
            "00".into(),
            "1".into(),
            year.clone(),
            document,
            "0001".into(),
            "E".into(),
            "122".into(),
            date(back),
            receipt["MENGE"].clone(),
            receipt["DMBTR"].clone(),
            "EUR".into(),
            date(back),
            clock(&mut rng),
        ]);
        let item = (receipt["EBELN"].clone(), receipt["EBELP"].clone());
        let Some(invoice) = invoice_of.get(&item) else {
            continue;
        };
        let Some(header) = header_of.get(invoice).map(|index| &rbkp[*index]) else {
            continue;
        };
        truth.credit_memos += 1;
        let credited = back + Duration::days(1 + rng.below(5) as i64);
        let year = credited.year().to_string();
        let belnr = number(5_900_000_000 + truth.credit_memos, 10);
        let at = clock(&mut rng);
        extra_rbkp.push(vec![
            belnr.clone(),
            year.clone(),
            header["BUKRS"].clone(),
            "RE".into(),
            date(credited),
            date(credited),
            date(credited),
            at.clone(),
            "APCLERK2".into(),
            header["LIFNR"].clone(),
            receipt["DMBTR"].clone(),
            "EUR".into(),
            format!("CM-{}", 80000 + truth.credit_memos),
            String::new(),
        ]);
        extra_rseg.push(vec![
            belnr.clone(),
            year.clone(),
            "000001".into(),
            item.0.clone(),
            item.1.clone(),
            receipt["MENGE"].clone(),
            receipt["DMBTR"].clone(),
        ]);
        let accounting = number(1_950_000_000 + truth.credit_memos, 10);
        extra_bkpf.push(vec![
            header["BUKRS"].clone(),
            accounting.clone(),
            year.clone(),
            "RE".into(),
            date(credited),
            date(credited),
            date(credited),
            at,
            "APCLERK2".into(),
            "RMRP".into(),
            format!("{belnr}{year}"),
        ]);
        journal.push(&[
            &header["BUKRS"],
            &accounting,
            &year,
            &header["LIFNR"],
            "RE",
            "RMRP",
            &format!("{belnr}{year}"),
            &date(credited),
            &receipt["DMBTR"],
            "",
            "",
        ]);
    }
    for (name, rows) in [
        ("EKBE", extra_ekbe),
        ("RBKP", extra_rbkp),
        ("RSEG", extra_rseg),
        ("BKPF", extra_bkpf),
    ] {
        let table = &mut tables.iter_mut().find(|(n, _)| *n == name).unwrap().1;
        for row in rows {
            table.push(&row.iter().map(String::as_str).collect::<Vec<_>>());
        }
    }
    tables.push(("CDHDR", cdhdr));
    tables.push(("CDPOS", cdpos));
}

/// What the made-up Ariba files contain, known by construction.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AribaTruth {
    pub requisitions: u64,
    pub submitted: u64,
    pub approvals: u64,
    pub denials: u64,
    pub orders: u64,
    /// SAP orders created directly in SAP, with no Ariba order.
    pub orders_outside_ariba: u64,
    /// Supplier invoices sent through Ariba, rejected ones included.
    pub invoices: u64,
    /// Invoices held on an exception that was resolved before approval.
    pub exceptions: u64,
    pub invoices_rejected: u64,
}

/// The files of the SAP Ariba plus S/4HANA layout: Ariba report exports
/// for requisitions, their approval history and orders, and the on-premise
/// S/4HANA tables for everything after the order. Requisitions live in
/// Ariba, so there is no EBAN.
pub const ARIBA_TABLES: [&str; 13] = [
    "AribaRequisition",
    "AribaApproval",
    "AribaOrder",
    "AribaInvoice",
    "LFA1",
    "EKKO",
    "EKPO",
    "EKBE",
    "RBKP",
    "RSEG",
    "BKPF",
    "BSAK",
    "T16FB",
];

/// The files of SAP Ariba with S/4HANA Public Cloud.
pub const ARIBA_CLOUD_TABLES: [&str; 11] = [
    "AribaRequisition",
    "AribaApproval",
    "AribaOrder",
    "AribaInvoice",
    "Supplier",
    "PurchaseOrder",
    "PurchaseOrderItem",
    "MaterialDocumentItem",
    "SupplierInvoice",
    "SupplierInvoiceItemPurOrdRef",
    "JournalEntryItem",
];

/// Writes the same made-up business with requisitions and approvals in
/// SAP Ariba. Ariba report columns are readable labels and dates carry the
/// time in one field (`2026-01-02 09:15:00`).
pub fn write_ariba(dir: &Path, settings: &Settings) -> io::Result<(Truth, AribaTruth)> {
    let (tables, _, truth) = generate(settings);
    let (mut files, ariba) = ariba_tables(&tables, settings);
    files.extend(tables.into_iter().filter(|(name, _)| *name != "EBAN"));
    save(dir, &files)?;
    Ok((truth, ariba))
}

/// The files of SAP Ariba with S/4HANA Public Cloud behind it: the Ariba
/// reports of [`write_ariba`] and the cloud views of [`write_cloud`], without
/// requisitions on the SAP side.
pub fn write_ariba_cloud(dir: &Path, settings: &Settings) -> io::Result<(Truth, AribaTruth)> {
    let (tables, journal, truth) = generate(settings);
    let (mut files, ariba) = ariba_tables(&tables, settings);
    files.extend(
        cloud_tables(&tables, &journal)
            .into_iter()
            .filter(|(name, _)| *name != "PurchaseRequisitionItem"),
    );
    save(dir, &files)?;
    Ok((truth, ariba))
}

fn ariba_tables(tables: &Tables, settings: &Settings) -> (Tables, AribaTruth) {
    let find = |name: &str| &tables.iter().find(|(n, _)| *n == name).unwrap().1;
    let rows = |table: &Table| {
        let columns = table
            .header
            .split(',')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        table
            .rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .cloned()
                    .zip(row.split(',').map(str::to_owned))
                    .collect::<BTreeMap<_, _>>()
            })
            .collect::<Vec<_>>()
    };
    let day = |value: &str| NaiveDate::parse_from_str(value, "%Y%m%d").unwrap();
    let mut rng = Rng::new(settings.seed ^ 0xA21BA);
    let mut ariba = AribaTruth::default();
    let mut requisitions = Table::new(
        "Requisition ID,Title,Requester,Created Date,Submitted Date,ERP Order ID,Amount,Currency",
    );
    let mut approvals =
        Table::new("Requisition ID,Approval Step,Approver,Approval Role,Action,Action Date");
    let mut orders = Table::new("Order ID,ERP Order ID,Requisition ID,Ordered Date,Supplier");
    let at = |date: NaiveDate, minutes: u64| {
        (date.and_hms_opt(0, 0, 0).unwrap() + Duration::minutes(minutes as i64))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    };
    // One requisition per order that had one in the on-premise data.
    let mut requested = BTreeMap::<String, NaiveDate>::new();
    for row in rows(find("EBAN")) {
        requested
            .entry(row["EBELN"].clone())
            .or_insert(day(&row["BADAT"]));
    }
    let roles = [
        ("Manager", "LHEAD"),
        ("Budget owner", "CFOAPPR"),
        ("Procurement", "RMEYER"),
    ];
    for (n, order) in rows(find("EKKO")).iter().enumerate() {
        let ebeln = &order["EBELN"];
        let created = day(&order["AEDAT"]);
        let requisition = requested.get(ebeln).map(|_| format!("PR{}", 100_000 + n));
        if let (Some(id), Some(start)) = (&requisition, requested.get(ebeln)) {
            ariba.requisitions += 1;
            // Minutes from the request day's midnight to the order day's 08:00.
            let window = ((created - *start).num_days().max(0) as u64) * 24 * 60 + 8 * 60;
            let opened = 8 * 60 + rng.below(4 * 60);
            let submitted = (rng.chance(0.97)).then(|| opened + 5 + rng.below(90));
            requisitions.push(&[
                id,
                "Supplies",
                "REQUESTER",
                &at(*start, opened),
                &submitted.map_or(String::new(), |m| at(*start, m)),
                ebeln,
                "100.00",
                "EUR",
            ]);
            if let Some(mut clock) = submitted {
                ariba.submitted += 1;
                let steps = 1 + rng.below(3) as usize;
                let mut sequence = 0;
                for (role, approver) in roles.iter().take(steps) {
                    let mut denied_once = false;
                    loop {
                        sequence += 1;
                        let left = window.saturating_sub(clock + 1).max(1);
                        clock += 1 + rng.below(left.min(24 * 60) / (steps as u64 + 1) + 1);
                        let deny = !denied_once && rng.chance(0.05);
                        approvals.push(&[
                            id,
                            &sequence.to_string(),
                            approver,
                            role,
                            if deny { "Denied" } else { "Approved" },
                            &at(*start, clock.min(window.saturating_sub(1))),
                        ]);
                        if deny {
                            ariba.denials += 1;
                            denied_once = true;
                        } else {
                            ariba.approvals += 1;
                            break;
                        }
                    }
                }
            }
        } else if rng.chance(0.15) {
            // Bought directly in SAP, outside Ariba.
            ariba.orders_outside_ariba += 1;
            continue;
        }
        ariba.orders += 1;
        orders.push(&[
            &format!("EP{}", 200_000 + n),
            ebeln,
            requisition.as_deref().unwrap_or(""),
            &at(created, 9 * 60 + rng.below(8 * 60)),
            &order["LIFNR"],
        ]);
    }
    // Supplier invoices sent through Ariba: received, sometimes held on an
    // exception, approved, then posted in SAP. A few are rejected in Ariba
    // and never reach SAP.
    let mut invoices = Table::new(
        "Invoice ID,Supplier Invoice Number,Order ID,ERP Order ID,ERP Invoice ID,ERP Fiscal Year,Submitted Date,Exception Resolved Date,Approved Date,Rejected Date,Status",
    );
    let mut order_of_invoice = BTreeMap::<(String, String), String>::new();
    for row in rows(find("RSEG")) {
        order_of_invoice
            .entry((row["BELNR"].clone(), row["GJAHR"].clone()))
            .or_insert(row["EBELN"].clone());
    }
    let ariba_order = orders
        .rows
        .iter()
        .map(|row| {
            let mut values = row.split(',');
            let id = values.next().unwrap().to_owned();
            (values.next().unwrap().to_owned(), id)
        })
        .collect::<BTreeMap<_, _>>();
    let mut number = 0;
    for invoice in rows(find("RBKP")) {
        let key = (invoice["BELNR"].clone(), invoice["GJAHR"].clone());
        let Some(ebeln) = order_of_invoice.get(&key) else {
            continue;
        };
        let Some(order_id) = ariba_order.get(ebeln) else {
            continue;
        };
        if !rng.chance(0.75) {
            continue;
        }
        number += 1;
        ariba.invoices += 1;
        let posted = day(&invoice["CPUDT"]);
        let received = posted - Duration::days(1 + rng.below(4) as i64);
        let start = 9 * 60 + rng.below(6 * 60);
        // Minutes from the receiving day's midnight to 06:00 on the posting day.
        let approved = ((posted - received).num_days() as u64) * 24 * 60 + 6 * 60;
        let resolved = rng.chance(0.25).then(|| {
            ariba.exceptions += 1;
            start + 1 + rng.below(approved - start - 1)
        });
        invoices.push(&[
            &format!("INV{}", 300_000 + number),
            &invoice["XBLNR"],
            order_id,
            ebeln,
            &key.0,
            &key.1,
            &at(received, start),
            &resolved.map_or(String::new(), |m| at(received, m)),
            &at(received, approved),
            "",
            "Approved",
        ]);
    }
    for (ebeln, order_id) in &ariba_order {
        if !rng.chance(0.03) {
            continue;
        }
        number += 1;
        ariba.invoices += 1;
        ariba.invoices_rejected += 1;
        let received = first_day(settings) + Duration::days(rng.below(120) as i64);
        let start = 9 * 60 + rng.below(6 * 60);
        invoices.push(&[
            &format!("INV{}", 300_000 + number),
            "DUPLICATE",
            order_id,
            ebeln,
            "",
            "",
            &at(received, start),
            "",
            "",
            &at(received, start + 60 + rng.below(24 * 60)),
            "Rejected",
        ]);
    }
    let files = vec![
        ("AribaRequisition", requisitions),
        ("AribaApproval", approvals),
        ("AribaOrder", orders),
        ("AribaInvoice", invoices),
    ];
    (files, ariba)
}

fn first_day(_: &Settings) -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 11, 1).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_tables_and_every_story_present() {
        let (first, _, truth) = generate(&Settings::default());
        let (second, _, again) = generate(&Settings::default());
        assert_eq!(truth, again);
        for ((_, a), (_, b)) in first.iter().zip(&second) {
            assert_eq!(a.rows, b.rows);
        }
        assert!(truth.goods_receipt_reversals > 0);
        assert!(truth.items_invoiced_before_receipt > 0);
        assert!(truth.invoice_numbers_in_two_years > 0);
        assert!(truth.invoices_paid < truth.invoices);
        assert!(truth.invoices_without_order > 0);
        assert!(truth.deleted_items > 0);
        let share = truth.orders_with_custom_approver as f64 / truth.orders as f64;
        assert!((0.2..0.4).contains(&share), "{share}");
    }
}
