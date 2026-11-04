//! Made-up SAP Business ByDesign extracts for buying or selling, shaped like
//! the business objects' OData collections: one file per document and one per
//! its items, items carrying the IDs of the order item they belong to, times
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
    /// The documents: request, order, receipt or delivery, invoice, payment.
    fn documents(self) -> [&'static str; 5] {
        match self {
            Self::Buying => [
                "PurchaseRequest",
                "PurchaseOrder",
                "GoodsAndServiceAcknowledgement",
                "SupplierInvoice",
                "SupplierPayment",
            ],
            Self::Selling => [
                "SalesQuote",
                "SalesOrder",
                "OutboundDelivery",
                "CustomerInvoice",
                "IncomingPayment",
            ],
        }
    }
    /// Every file, documents followed by their items (and the payment
    /// allocations).
    pub fn files(self) -> Vec<String> {
        let [request, order, receipt, invoice, payment] = self.documents();
        vec![
            request.to_owned(),
            format!("{request}Item"),
            order.to_owned(),
            format!("{order}Item"),
            receipt.to_owned(),
            format!("{receipt}Item"),
            invoice.to_owned(),
            format!("{invoice}Item"),
            payment.to_owned(),
            format!("{payment}Allocation"),
        ]
    }
    fn partner(self) -> &'static str {
        match self {
            Self::Buying => "SellerPartyID",
            Self::Selling => "BuyerPartyID",
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
            seed: 19,
        }
    }
}

/// What the extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Truth {
    pub requests: u64,
    pub orders: u64,
    pub order_lines: u64,
    pub lines_from_requests: u64,
    pub receipt_lines: u64,
    pub invoices: u64,
    pub invoice_lines: u64,
    pub allocations: u64,
    pub paid_order_lines: u64,
}

fn utc(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

struct Documents {
    headers: Table,
    items: Table,
    next: u64,
}

impl Documents {
    fn new(header: &'static str, items: &'static str, start: u64) -> Self {
        Self {
            headers: Table::new(header),
            items: Table::new(items),
            next: start,
        }
    }
    fn header(&mut self, partner: &str, at: NaiveDateTime) -> String {
        self.next += 1;
        let id = self.next.to_string();
        self.headers
            .push(&[&id, &utc(at), partner, "3", "EUR", "1000.00", "A"]);
        id
    }
}

/// Writes the ten files of `flow`.
pub fn write(dir: &Path, flow: Flow, settings: &Settings) -> io::Result<Truth> {
    let mut rng = Rng::new(settings.seed ^ if flow == Flow::Buying { 0xBD1 } else { 0xBD2 });
    let mut truth = Truth::default();
    let header: &'static str = match flow {
        Flow::Buying => {
            "ID,CreationDateTime,SellerPartyID,LifeCycleStatusCode,CurrencyCode,TotalNetAmount,Region_KUT"
        }
        Flow::Selling => {
            "ID,CreationDateTime,BuyerPartyID,LifeCycleStatusCode,CurrencyCode,TotalNetAmount,Region_KUT"
        }
    };
    let (request_items, order_items, receipt_items, invoice_items): (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) = match flow {
        Flow::Buying => (
            "PurchaseRequestID,ID,ProductID,Quantity,NetAmount",
            "PurchaseOrderID,ID,ProductID,Quantity,PurchaseRequestID,PurchaseRequestItemID,NetAmount",
            "GoodsAndServiceAcknowledgementID,ID,ProductID,Quantity,PurchaseOrderID,PurchaseOrderItemID,NetAmount",
            "SupplierInvoiceID,ID,ProductID,Quantity,PurchaseOrderID,PurchaseOrderItemID,NetAmount",
        ),
        Flow::Selling => (
            "SalesQuoteID,ID,ProductID,Quantity,NetAmount",
            "SalesOrderID,ID,ProductID,Quantity,SalesQuoteID,SalesQuoteItemID,NetAmount",
            "OutboundDeliveryID,ID,ProductID,Quantity,SalesOrderID,SalesOrderItemID,NetAmount",
            "CustomerInvoiceID,ID,ProductID,Quantity,SalesOrderID,SalesOrderItemID,NetAmount",
        ),
    };
    let mut requests = Documents::new(header, request_items, 100);
    let mut orders = Documents::new(header, order_items, 4_500_000);
    let mut receipts = Documents::new(header, receipt_items, 7_000);
    let mut invoices = Documents::new(header, invoice_items, 9_000);
    let mut payments = Table::new(match flow {
        Flow::Buying => "ID,CreationDateTime,SellerPartyID,TotalAmount",
        Flow::Selling => "ID,CreationDateTime,BuyerPartyID,TotalAmount",
    });
    let mut allocations = Table::new(match flow {
        Flow::Buying => "SupplierPaymentID,ID,SupplierInvoiceID,Amount",
        Flow::Selling => "IncomingPaymentID,ID,CustomerInvoiceID,Amount",
    });
    let start = NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap();
    let mut payment_number = 0u64;
    for _ in 0..settings.records {
        let partner = format!("{}{:05}", &flow.partner()[..1], rng.below(12));
        let mut at = start
            + Duration::days(rng.below(80) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let lines = 1 + rng.below(3);
        let request = rng.chance(0.4).then(|| {
            truth.requests += 1;
            let id = requests.header(&partner, at);
            for n in 1..=lines {
                requests
                    .items
                    .push(&[&id, &(n * 10).to_string(), "P-100", "5", "500.00"]);
            }
            at += Duration::hours(2 + rng.below(48) as i64);
            id
        });
        truth.orders += 1;
        let order = orders.header(&partner, at);
        for n in 1..=lines {
            truth.order_lines += 1;
            let item = (n * 10).to_string();
            let (from, from_item) = match &request {
                Some(id) => {
                    truth.lines_from_requests += 1;
                    (id.as_str(), item.as_str())
                }
                None => ("", ""),
            };
            orders
                .items
                .push(&[&order, &item, "P-100", "5", from, from_item, "500.00"]);
        }
        if !rng.chance(0.9) {
            continue;
        }
        at += Duration::days(1 + rng.below(14) as i64);
        let receipt = receipts.header(&partner, at);
        for n in 1..=lines {
            truth.receipt_lines += 1;
            receipts.items.push(&[
                &receipt,
                &n.to_string(),
                "P-100",
                "5",
                &order,
                &(n * 10).to_string(),
                "500.00",
            ]);
        }
        if !rng.chance(0.85) {
            continue;
        }
        at += Duration::days(1 + rng.below(10) as i64);
        truth.invoices += 1;
        let invoice = invoices.header(&partner, at);
        for n in 1..=lines {
            truth.invoice_lines += 1;
            invoices.items.push(&[
                &invoice,
                &n.to_string(),
                "P-100",
                "5",
                &order,
                &(n * 10).to_string(),
                "500.00",
            ]);
        }
        if rng.chance(0.8) {
            payment_number += 1;
            truth.allocations += 1;
            truth.paid_order_lines += lines;
            let paid = at + Duration::days(5 + rng.below(40) as i64);
            let payment = (60_000 + payment_number).to_string();
            payments.push(&[&payment, &utc(paid), &partner, "1500.00"]);
            allocations.push(&[&payment, "1", &invoice, "1500.00"]);
        }
    }
    let files = flow.files();
    save(
        dir,
        &[
            (files[0].as_str(), &requests.headers),
            (files[1].as_str(), &requests.items),
            (files[2].as_str(), &orders.headers),
            (files[3].as_str(), &orders.items),
            (files[4].as_str(), &receipts.headers),
            (files[5].as_str(), &receipts.items),
            (files[6].as_str(), &invoices.headers),
            (files[7].as_str(), &invoices.items),
            (files[8].as_str(), &payments),
            (files[9].as_str(), &allocations),
        ],
    )?;
    Ok(truth)
}
