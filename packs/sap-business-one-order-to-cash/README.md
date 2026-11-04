# SAP Business One order to cash (not yet tested)

Order to cash on SAP Business One (SQL Server or HANA), from
sales quotation to payment, followed by sales order line. Written by the Glasstarn project from public knowledge of the Business One tables; tested
only on made-up tables (`pm-testgen` `sap_business_one::write`). Untested
until confirmed on a real company database.

| Table | Contents |
|---|---|
| OQUT, QUT1 | Sales quotations and their lines (optional) |
| ORDR, RDR1 | Sales orders and their lines |
| ODLN, DLN1 | Deliverys and their lines |
| OINV, INV1 | Customer invoices and their lines |
| ORCT, RCT2 | Payments and the invoices they pay (optional) |
| OWDD, WDD1 | Approval requests for orders and their decisions (optional) |
| OCRD | Customers (optional; no names or contact details) |

Steps: Create sales quotation, Request approval, Approve or Reject sales order,
Create sales order, Cancel sales order, Deliver goods, Cancel
delivery, Create customer invoice, Cancel
customer invoice, Receive customer payment. Documents are linked line by line through
BaseType, BaseEntry and BaseLine, so an invoice or payment reaches every order
line it covers, also through the delivery; an
approval reaches the order its draft became (draftKey).

Known limit: an order cancellation carries a date only (midnight on that day).

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
