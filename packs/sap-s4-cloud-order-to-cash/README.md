# SAP S/4HANA Public Cloud order to cash (not yet tested)

Order to cash on SAP S/4HANA Cloud, public edition, from sales
order to customer payment, followed by sales order item. Written by the Glasstarn project from public knowledge of SAP's released data views;
tested only on made-up files (`sap-p2p-sample <dir> --o2c --cloud`). Untested until
confirmed on a real system.

| File | Contents |
|---|---|
| Customer | Customers |
| SalesOrder, SalesOrderItem | Sales orders and items, with YY1_ company fields |
| OutboundDelivery, OutboundDeliveryItem | Deliveries and their items |
| BillingDocument, BillingDocumentItem | Invoices, cancellations and their items |
| JournalEntryItem | Customer lines (account type D) with clearing date and entry |

Steps: Create sales order, Create delivery, Post goods issue, Create invoice,
Cancel invoice, Receive payment, plus confirmed company fields (for example an order
confirmation date). An invoice cleared by its
cancellation gets no payment step.

`--variant accounts-receivable` runs invoice to cash from Customer,
BillingDocument and JournalEntryItem alone, followed by customer invoice.

Changes come from the change log tables (CDHDR and CDPOS, or the cloud change
documents), which are optional. Known limits: goods issue and payment carry
dates only (midnight on that day).
`sap-order-to-cash` is the same process for on-premise SAP.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
