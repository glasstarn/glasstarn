# SAP Business ByDesign order to cash (not yet tested)

Order to cash on SAP Business ByDesign, from sales quote to payment,
followed by sales order line. Written by the Glasstarn project from
public knowledge of the ByDesign business objects and their OData services;
tested only on made-up extracts (`pm-testgen` `sap_bydesign::write`).
Untested until confirmed on a real tenant.

| File | Contents |
|---|---|
| SalesQuote | Sales quotes (optional) |
| SalesQuoteItem | Sales quote items (optional) |
| SalesOrder | Sales orders |
| SalesOrderItem | Sales order lines |
| OutboundDelivery | Deliverys |
| OutboundDeliveryItem | Delivery lines |
| CustomerInvoice | Customer invoices |
| CustomerInvoiceItem | Customer invoice items |
| IncomingPayment | Payments (optional) |
| IncomingPaymentAllocation | Payment allocations (optional) |

Steps: Create sales quote, Create sales order, Deliver goods, Create customer invoice,
Receive customer payment. Items carry the IDs of the order item they belong to, so an
invoice or payment reaches every order line it covers.

There is no direct connection to ByDesign yet: an administrator exports the
files through the tenant's OData services or reports, with times in UTC. File
and field names may differ by tenant; the pack's checks show what is missing.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
