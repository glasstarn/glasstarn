# SAP purchase to pay (tested on an SAP trial system)

Purchase requisitions, orders, goods receipts and invoice verification on SAP
ECC and S/4HANA on-premise or private cloud, followed by purchase order item.
The pack consumes local exports. Run
`glasstarn workspace pack checklist sap-purchase-to-pay --full` for the current
technical fields and export filters.

| Tables | Contents |
|---|---|
| LFA1 | Vendors |
| EBAN | Purchase requisition lines |
| EKKO, EKPO | Purchase order headers (`BSTYP = F`) and items |
| EKBE | History of the selected order items |
| RBKP, RSEG | Invoice verification headers and physical invoice items |
| BKPF, BSAK | Accounting headers and cleared vendor items |
| CDHDR, CDPOS | Optional document changes |
| T16FB | Optional purchasing release indicators and their `KZFRE` setting |

Repeated invoice lines against one order item are supported. Each physical
RSEG item keeps its own identity and source lineage; shared invoice events
reach an order item once. Duplicate physical keys still stop the build.

## Steps and interpretation

Requisition creation and release, order creation, goods receipts and reversals,
returns, invoices and credit memos, invoice reversals, vendor-item clearing,
and supported document changes are included. Held and parked invoices and GR/IR
account maintenance have separate activities. They are not classified as posted
credit memos. Export RBKP origin, status and reversal fields even when testing
the accounts payable variant.

Final order release uses the exported T16FB configuration. Without it, the
pack omits final-release events and retains raw release-status changes when
change logs are present. No particular release-indicator letter is assumed.
Clearing is labeled **Clear vendor item**: a clearing date alone does not prove
a bank payment.

## Accounts payable

`--variant accounts-payable` follows each **invoice verification document**
using LFA1, RBKP, BKPF and BSAK, with optional CDHDR/CDPOS changes. Order items
and T16FB are unnecessary for this variant. Invoice documents without an order
remain visible here.

## Scope and limits

- Export purchase orders in category F, then their linked items/history and
  change records. Contracts, quotations and scheduling agreements require
  separate interpretation; mixed-category headers fail the check.
- Blank purchase-order vendors are allowed, including stock transfers. A
  nonblank vendor reference must resolve. Normal posted invoices and credit
  memos require a vendor; special maintenance, held and parked records may not.
- History and invoice references outside the selected order cohort are counted
  and disclosed. They do not invent missing order objects.
- Several credit vendor items in one accounting document remain unsupported:
  the existing BSAK cardinality check stops the build. This trial did not
  exercise that pattern; no allocation or aggregation is inferred.
- Date-only fields retain day precision. System times use the confirmed fixed
  offset; daylight-saving changes are not inferred.
- Document numbers are scoped by fiscal year where applicable and company code
  for accounting documents. Keep one SAP system/client per export set.

## Live validation

On a scoped SAP S/4HANA 2025 FPS01 trial extract, `pack check` and `pack build`
both passed for the full pack and for `--variant accounts-payable`, using the
same source exports. Five source-record timelines matched for each: common
and slow cases plus split invoice items, goods-receipt reversal and configured
release for the full pack; account maintenance, held documents and reversal
for accounts payable. Original broad exports were retained. The tested cohort
was selected by the documented document categories and linked keys, without
deduplicating rows or disabling validation.

This tests that trial scope. Other releases, customer configuration and
business branches absent from the extract still need validation. Invented
regression fixtures cover the observed source patterns without distributing
trial data.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
