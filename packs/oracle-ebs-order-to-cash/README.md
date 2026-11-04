# Oracle E-Business Suite order to cash (not yet tested)

Order to cash on Oracle E-Business Suite (R12), from sales order to customer payment, followed by
sales order line. Written by the Glasstarn project from public knowledge
of the Oracle tables; tested only on made-up tables (`pm-testgen`
`oracle::write_selling`). Untested until confirmed on a real system. Export with SQL from the APPS schema.

| Table | Contents |
|---|---|
| OE_ORDER_HEADERS_ALL | Sales orders |
| OE_ORDER_LINES_ALL | Sales order lines |
| OE_ORDER_HOLDS_ALL | Order holds (optional) |
| OE_HOLD_RELEASES | Hold releases (optional) |
| RA_CUSTOMER_TRX_ALL | Customer invoices |
| RA_CUSTOMER_TRX_LINES_ALL | Customer invoice lines |
| AR_RECEIVABLE_APPLICATIONS_ALL | Receipt applications (optional) |

Steps: Create sales order, Book sales order, Put order on hold, Release order hold, Ship goods, Cancel order line, Create customer invoice, Receive customer payment. Order and invoice steps reach every line they cover; invoice
lines name the order line they billed (INTERFACE_LINE_ATTRIBUTE6).

Known limits: a cancelled line is timed by its last update.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
