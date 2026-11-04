//! Literal source facts: two order lines, partial goods movements, two invoices,
//! and only the first invoice settled. Expectations precede the recipe output.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Exported, Pack, build_recipe_with, check};
use std::collections::BTreeMap;

#[test]
fn partial_fulfilment_and_split_invoices_reach_only_their_order_lines() {
    for with_context in [true, false] {
        for (buying, id, order, line, goods, invoice, invoice_line, transaction) in [
            (
                true,
                "d365-finance-operations-purchase-to-pay",
                "PurchTable",
                "PurchLine",
                "VendPackingSlipTrans",
                "VendInvoiceJour",
                "VendInvoiceTrans",
                "VendTrans",
            ),
            (
                false,
                "d365-finance-operations-order-to-cash",
                "SalesTable",
                "SalesLine",
                "CustPackingSlipTrans",
                "CustInvoiceJour",
                "CustInvoiceTrans",
                "CustTrans",
            ),
        ] {
            let dir = std::env::temp_dir().join(format!(
                "pm-d365-split-{buying}-{with_context}-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let order_key = if buying { "PurchId" } else { "SalesId" };
            let partner_key = if buying {
                "OrderAccount"
            } else {
                "CustAccount"
            };
            let invoice_key = if buying {
                "InternalInvoiceId"
            } else {
                "InvoiceId"
            };
            let quantity_key = if buying { "PurchQty" } else { "SalesQty" };
            let mut files = BTreeMap::new();
            for (table, content) in [
            (order, format!("{order_key},{partner_key},DataAreaId,CreatedDateTime\nORDER-1,PARTNER-1,usmf,2026-10-05T09:00:00Z\n")),
            (line, format!("{order_key},LineNumber,InventTransId,ItemId,{quantity_key},LineAmount\nORDER-1,1,LOT-A,ITEM-A,3,300\nORDER-1,2,LOT-B,ITEM-B,3,300\n")),
            (goods, "RecId,PackingSlipId,InventTransId,CreatedDateTime,Qty\n1,RECEIPT-A,LOT-A,2026-10-06T10:00:00Z,1\n2,RECEIPT-B,LOT-A,2026-10-07T11:00:00Z,2\n3,RECEIPT-B,LOT-B,2026-10-07T11:30:00Z,3\n".into()),
            (invoice, if buying {
                "InternalInvoiceId,InvoiceId,LedgerVoucher,CreatedDateTime,InvoiceAmount\nINV-A,VENDOR-A,VOUCHER-A,2026-10-08T12:00:00Z,100\nINV-B,VENDOR-B,VOUCHER-B,2026-10-09T13:00:00Z,500\n".into()
            } else {
                "InvoiceId,LedgerVoucher,CreatedDateTime,InvoiceAmount\nINV-A,VOUCHER-A,2026-10-08T12:00:00Z,100\nINV-B,VOUCHER-B,2026-10-09T13:00:00Z,500\n".into()
            }),
            (invoice_line, format!("{invoice_key},LineNum,InventTransId,Qty\nINV-A,1,LOT-A,1\nINV-B,1,LOT-A,2\nINV-B,2,LOT-B,3\n")),
            (transaction, "RecId,Voucher,AccountNum,Closed\n10,VOUCHER-A,PARTNER-1,2026-10-16\n11,VOUCHER-B,PARTNER-1,1900-01-01\n".into()),
        ] {
            let content = if with_context { content } else {
                let mut rows = content.lines();
                let headers: Vec<_> = rows.next().unwrap().split(',').collect();
                let keep: Vec<_> = headers.iter().enumerate().filter_map(|(i, name)|
                    (!matches!(*name, "OrderAccount" | "CustAccount" | "ItemId" | "AccountNum" | "LineNumber" | "PackingSlipId")
                        && !(buying && *name == "InvoiceId")).then_some(i)
                ).collect();
                std::iter::once(headers.join(",")).chain(rows.map(str::to_owned))
                    .map(|row| { let cells: Vec<_> = row.split(',').collect(); keep.iter().map(|i| cells[*i]).collect::<Vec<_>>().join(",") })
                    .collect::<Vec<_>>().join("\n") + "\n"
            };
            let path = dir.join(format!("{table}.csv"));
            std::fs::write(&path, content).unwrap();
            files.insert(table.to_string(), path);
        }
            let pack =
                Pack::parse(BUILT_IN.iter().find(|(name, _)| *name == id).unwrap().1).unwrap();
            let checked = check(&pack, &files).unwrap();
            assert_eq!(checked.status, "ready", "{:?}", checked.gaps);
            let hashes = files
                .keys()
                .map(|name| (name.clone(), "0".repeat(64)))
                .collect();
            let answers = BTreeMap::from([(
                "setting:offset".to_string(),
                Answer {
                    choice: "value".into(),
                    value: Some("+01:00".into()),
                    confirmed_by: "Dynamics administrator".into(),
                    at: "2026-10-17T00:00:00Z".into(),
                    question: String::new(),
                    carries: Vec::new(),
                },
            )]);
            let built = build_recipe_with(
                &pack,
                &hashes,
                &answers,
                &Exported::from_report(&checked.tables),
            )
            .unwrap();
            let outcome = common::prepare_both(&built.recipe, &files);
            assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
            assert_eq!(outcome.report.objects, 5);
            assert_eq!(outcome.report.events, 7);
            let log = serde_json::to_value(outcome.log.unwrap()).unwrap();
            let events = log["events"].as_array().unwrap();
            let trace = |lot: &str| {
                let mut steps: Vec<(String, String)> = events
                    .iter()
                    .filter(|event| {
                        event["relationships"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|link| {
                                let object: serde_json::Value =
                                    serde_json::from_str(link["objectId"].as_str().unwrap())
                                        .unwrap();
                                object[2][0] == lot
                            })
                    })
                    .map(|event| {
                        (
                            event["type"].as_str().unwrap().to_owned(),
                            chrono::DateTime::parse_from_rfc3339(event["time"].as_str().unwrap())
                                .unwrap()
                                .with_timezone(&chrono::Utc)
                                .to_rfc3339(),
                        )
                    })
                    .collect();
                steps.sort_by(|a, b| a.1.cmp(&b.1));
                steps
            };
            let create = if buying {
                "Create purchase order"
            } else {
                "Create sales order"
            };
            let movement = if buying {
                "Receive goods (product receipt)"
            } else {
                "Ship goods (packing slip)"
            };
            let post = if buying {
                "Post vendor invoice"
            } else {
                "Post customer invoice"
            };
            let settle = if buying {
                "Settle vendor invoice"
            } else {
                "Settle customer invoice"
            };
            let expected = |steps: Vec<(&str, &str)>| {
                steps
                    .into_iter()
                    .map(|(step, time)| (step.to_owned(), time.to_owned()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                trace("LOT-A"),
                expected(vec![
                    (create, "2026-10-05T09:00:00+00:00"),
                    (movement, "2026-10-06T10:00:00+00:00"),
                    (movement, "2026-10-07T11:00:00+00:00"),
                    (post, "2026-10-08T12:00:00+00:00"),
                    (post, "2026-10-09T13:00:00+00:00"),
                    (settle, "2026-10-15T23:00:00+00:00"),
                ])
            );
            assert_eq!(
                trace("LOT-B"),
                expected(vec![
                    (create, "2026-10-05T09:00:00+00:00"),
                    (movement, "2026-10-07T11:30:00+00:00"),
                    (post, "2026-10-09T13:00:00+00:00"),
                ])
            );
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
