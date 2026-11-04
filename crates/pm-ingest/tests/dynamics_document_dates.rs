//! Standard posted-document tables can expose dates without creation timestamps.
//! Two distinct confirmations on one day must survive with their own identities.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Exported, Pack, build_recipe_with, check};
use std::collections::BTreeMap;

#[test]
fn document_date_fallback_preserves_same_day_confirmations_and_line_links() {
    for with_row_id in [true, false] {
        for buying in [true, false] {
            let (
                id,
                order,
                line,
                confirmation,
                confirmation_date,
                movement,
                invoice,
                invoice_line,
                order_key,
                invoice_key,
            ) = if buying {
                (
                    "d365-finance-operations-purchase-to-pay",
                    "PurchTable",
                    "PurchLine",
                    "VendPurchOrderJour",
                    "PurchOrderDate",
                    "VendPackingSlipTrans",
                    "VendInvoiceJour",
                    "VendInvoiceTrans",
                    "PurchId",
                    "InternalInvoiceId",
                )
            } else {
                (
                    "d365-finance-operations-order-to-cash",
                    "SalesTable",
                    "SalesLine",
                    "CustConfirmJour",
                    "ConfirmDate",
                    "CustPackingSlipTrans",
                    "CustInvoiceJour",
                    "CustInvoiceTrans",
                    "SalesId",
                    "InvoiceId",
                )
            };
            let dir = std::env::temp_dir().join(format!(
                "pm-d365-document-dates-{buying}-{with_row_id}-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let mut files = BTreeMap::new();
            for (table, content) in [
            (
                order,
                format!(
                    "{order_key},DataAreaId,CreatedDateTime\nORDER-1,usmf,2026-10-05T09:00:00Z\n"
                ),
            ),
            (
                line,
                format!("{order_key},LineNumber,InventTransId\nORDER-1,1,LOT-A\nORDER-1,2,LOT-B\n"),
            ),
            (
                confirmation,
                if with_row_id { format!(
                    "RecId,{order_key},{confirmation_date}\n101,ORDER-1,2026-10-06\n102,ORDER-1,2026-10-06\n"
                ) } else {
                    let document = if buying { "PurchOrderDocNum" } else { "ConfirmDocNum" };
                    format!("{document},{order_key},{confirmation_date}\nDOC-1,ORDER-1,2026-10-06\nDOC-2,ORDER-1,2026-10-06\n")
                },
            ),
            (
                movement,
                "RecId,PackingSlipId,InventTransId,DeliveryDate\n501,RECEIPT-1,LOT-A,2026-10-07\n"
                    .into(),
            ),
            (
                invoice,
                if buying {
                    "InternalInvoiceId,InvoiceId,LedgerVoucher,InvoiceDate\nINV-1,VENDOR-1,VOUCHER-1,2026-10-08\n".into()
                } else {
                    "InvoiceId,LedgerVoucher,InvoiceDate\nINV-1,VOUCHER-1,2026-10-08\n".into()
                },
            ),
            (
                invoice_line,
                format!("{invoice_key},LineNum,InventTransId\nINV-1,1,LOT-A\nINV-1,2,LOT-B\n"),
            ),
        ] {
            let path = dir.join(format!("{table}.csv"));
            std::fs::write(&path, content).unwrap();
            files.insert(table.to_owned(), path);
        }
            let pack =
                Pack::parse(BUILT_IN.iter().find(|(name, _)| *name == id).unwrap().1).unwrap();
            let report = check(&pack, &files).unwrap();
            assert_eq!(report.status, "ready", "{:?}", report.gaps);
            for table in [confirmation, movement, invoice] {
                let checked = report.tables.iter().find(|t| t.table == table).unwrap();
                assert!(checked.fallbacks_used.contains_key("CreatedDateTime"));
            }
            let hashes = files
                .keys()
                .map(|name| (name.clone(), "0".repeat(64)))
                .collect();
            let answers = BTreeMap::from([(
                "setting:offset".into(),
                Answer {
                    choice: "value".into(),
                    value: Some("-04:00".into()),
                    confirmed_by: "Declared fixture timezone".into(),
                    at: "2026-10-17T00:00:00Z".into(),
                    question: String::new(),
                    carries: Vec::new(),
                },
            )]);
            let built = build_recipe_with(
                &pack,
                &hashes,
                &answers,
                &Exported::from_report(&report.tables),
            )
            .unwrap();
            let outcome = common::prepare_both(&built.recipe, &files);
            assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
            assert_eq!(outcome.report.events, 5);
            assert_eq!(outcome.report.objects, 4);
            let log = serde_json::to_value(outcome.log.unwrap()).unwrap();
            let events = log["events"].as_array().unwrap();
            let utc_time = |event: &serde_json::Value| {
                chrono::DateTime::parse_from_rfc3339(event["time"].as_str().unwrap())
                    .unwrap()
                    .with_timezone(&chrono::Utc)
                    .to_rfc3339()
            };
            let confirm = if buying {
                "Confirm purchase order"
            } else {
                "Confirm sales order"
            };
            let confirmations: Vec<_> = events.iter().filter(|e| e["type"] == confirm).collect();
            assert_eq!(confirmations.len(), 2);
            assert_ne!(confirmations[0]["id"], confirmations[1]["id"]);
            for e in &confirmations {
                assert_eq!(utc_time(e), "2026-10-06T04:00:00+00:00");
                assert_eq!(e["relationships"].as_array().unwrap().len(), 3);
            }
            let movement_name = if buying {
                "Receive goods (product receipt)"
            } else {
                "Ship goods (packing slip)"
            };
            let moved = events.iter().find(|e| e["type"] == movement_name).unwrap();
            assert_eq!(utc_time(moved), "2026-10-07T04:00:00+00:00");
            assert_eq!(moved["relationships"].as_array().unwrap().len(), 1);
            let post = if buying {
                "Post vendor invoice"
            } else {
                "Post customer invoice"
            };
            let posted = events.iter().find(|e| e["type"] == post).unwrap();
            assert_eq!(utc_time(posted), "2026-10-08T04:00:00+00:00");
            assert_eq!(posted["relationships"].as_array().unwrap().len(), 3);
            if !with_row_id {
                // Profiling must still detect duplicate fallback identities.
                let path = &files[confirmation];
                let source = std::fs::read_to_string(path)
                    .unwrap()
                    .replace("DOC-2", "DOC-1");
                std::fs::write(path, source).unwrap();
                let duplicated = check(&pack, &files).unwrap();
                assert_eq!(
                    duplicated
                        .unique
                        .iter()
                        .find(|u| u.id.contains(&confirmation.to_ascii_lowercase()))
                        .unwrap()
                        .repeated,
                    1
                );
            }
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
