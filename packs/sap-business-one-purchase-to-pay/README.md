# SAP Business One purchase to pay (not yet tested)

Purchase to pay on SAP Business One (SQL Server or HANA), from
purchase request to payment, followed by purchase order line. Written by the Glasstarn project from public knowledge of the Business One tables; tested
only on made-up tables (`pm-testgen` `sap_business_one::write`). Untested
until confirmed on a real company database.

| Table | Contents |
|---|---|
| OPRQ, PRQ1 | Purchase requests and their lines (optional) |
| OPOR, POR1 | Purchase orders and their lines |
| OPDN, PDN1 | Goods receipts and their lines |
| OPCH, PCH1 | Supplier invoices and their lines |
| OVPM, VPM2 | Payments and the invoices they pay (optional) |
| OWDD, WDD1 | Approval requests for orders and their decisions (optional) |
| OCRD | Vendors (optional; no names or contact details) |

Steps: Create purchase request, Request approval, Approve or Reject purchase order,
Create purchase order, Cancel purchase order, Receive goods, Cancel
goods receipt, Record supplier invoice, Cancel
supplier invoice, Pay supplier invoice. Documents are linked line by line through
BaseType, BaseEntry and BaseLine, so an invoice or payment reaches every order
line it covers, also through the goods receipt; an
approval reaches the order its draft became (draftKey).

Known limit: an order cancellation carries a date only (midnight on that day).

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
