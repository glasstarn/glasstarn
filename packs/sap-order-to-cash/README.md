# SAP order to cash (tested on an SAP trial system)

Sales orders through customer payment postings on SAP ECC and S/4HANA
on-premise or private cloud, followed by sales order item. The pack consumes
local exports. Run
`glasstarn workspace pack checklist sap-order-to-cash --full` for technical
fields and linked export filters.

| Tables | Contents |
|---|---|
| KNA1 | Customers |
| VBAK, VBAP | Selected sales document headers and items |
| LIKP, LIPS | Outbound/return deliveries and physical items |
| VBRK, VBRP | Financial billing documents and physical items |
| BKPF, BSAD | Billing/payment accounting headers and cleared customer items |
| CDHDR, CDPOS | Optional document changes |

## Steps and interpretation

Creation, credit/delivery/billing blocks and releases, item rejection and price
or quantity changes, delivery, goods issue, invoices, cancellations, returns,
credit/debit requests and memos, and customer payment postings are included.
Returns and adjustment documents also reach their referenced original order
items. Classification uses standard `VBTYP` categories rather than assuming
particular customer-configured document-type names.

Delivery and billing items have their own identities. Batch splits, several
billing items against one order item, and several cleared accounting items
are supported without multiplying shared document events. Reference-free
items retain their document connection. Duplicate physical item keys still
stop the build.

Customer payments are labeled **Post customer payment**, timed from BKPF entry
date/time for a DZ payment document. Clearing links connect these documents to
invoices, including shared payments. They do not infer a bank receipt time or
allocate the payment amount between invoices. Cancellation clearing alone does
not create a payment event.

## Accounts receivable

`--variant accounts-receivable` follows customer invoices using KNA1, VBRK,
BKPF and BSAD, with optional change logs. It uses the same payment-document
evidence and preserves individual customer accounting items.

## Scope and limits

- VBAK categories C/I/H/K/L, LIKP J/T and VBRK M/N/O/P/S/5/6 form the supported
  cohort. Include the corresponding linked child rows and accounting records.
  Quotations, proposals, pro forma documents, billing requests and invoice
  lists are outside this pack's tested scope; mixed headers fail the check.
- A historical item rejection can refer to an item absent from current VBAP.
  Its event stays on the order header with the source item key. The data check
  discloses the unmatched item. No replacement item is invented, and that event
  cannot appear in a missing item's timeline.
- Goods issue retains date precision. Payment posting has entry-time precision.
  Confirm the system clock's fixed offset; daylight-saving changes are not
  inferred. Keep one SAP system/client per export set.
- Unmatched optional references are counted. Snapshot timing and linked scope
  still matter: separate table reads are not a transactional snapshot.

## Live validation

On a scoped SAP S/4HANA 2025 FPS01 trial extract, `pack check` and `pack build`
both passed for the full pack and `--variant accounts-receivable`, using the
same source exports. Five source-record timelines matched for each: common
and slow cases, split delivery items, an order-related invoice and a debit
request for the full pack; multi-item clearing, an order-related invoice and
a credit-memo cancellation for accounts receivable. Original broad exports
were retained. Scope selection used document categories and linked keys;
source duplicates and broken required references were not removed to pass.

This tests that trial scope, not every release, customization or optional
branch. The separate Public Cloud pack retains its own schema and synthetic
coverage; this trial does not test a Public Cloud tenant.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
