# Oracle E-Business Suite purchase to pay (not yet tested)

Purchase to pay on Oracle E-Business Suite (R12), from requisition to payment, followed by purchase
order line. Written by the Glasstarn project from public knowledge
of the Oracle Purchasing, Receiving and Payables tables; tested only on made-up
tables (`pm-testgen` `oracle::write_purchasing`). Untested until confirmed on
a real system. Export with SQL from the APPS schema.

| Table | Contents |
|---|---|
| PO_HEADERS_ALL, PO_LINES_ALL | Purchase orders and their lines |
| PO_ACTION_HISTORY | Approval actions (optional) |
| PO_REQUISITION_LINES_ALL, PO_LINE_LOCATIONS_ALL | Requisition lines and the order shipments they became (optional) |
| RCV_TRANSACTIONS | Receipts, deliveries and returns |
| AP_INVOICES_ALL, AP_INVOICE_LINES_ALL | Supplier invoices and their lines matched to order lines |
| AP_HOLDS_ALL | Invoice holds and releases (optional) |
| AP_INVOICE_PAYMENTS_ALL | Payments (optional) |

Steps: Create requisition, Create purchase order, Submit, Forward, Approve or
Reject purchase order, Cancel order line, Receive goods, Deliver goods to
requester, Return goods to supplier, Record supplier invoice, Place invoice on
hold, Release invoice hold, Cancel supplier invoice, Pay supplier invoice.
Order and invoice steps reach every order line they cover.

`--variant accounts-payable` runs invoice to payment alone, followed by
supplier invoice. Known limit: a released hold is timed by the hold's last
update.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
