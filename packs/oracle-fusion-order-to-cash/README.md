# Oracle Fusion Cloud order to cash (not yet tested)

Order to cash on Oracle Fusion Cloud Order Management and Receivables, from sales order to customer payment, followed by
fulfillment line. Written by the Glasstarn project from public knowledge
of the Oracle tables; tested only on made-up tables (`pm-testgen`
`oracle::write_selling`). Untested until confirmed on a real system. Extract with BI Publisher (SQL on the tables below) or BI Cloud Connector.

| Table | Contents |
|---|---|
| DOO_HEADERS_ALL | Sales orders |
| DOO_FULFILL_LINES_ALL | Fulfillment lines |
| RA_CUSTOMER_TRX_ALL | Customer invoices |
| RA_CUSTOMER_TRX_LINES_ALL | Customer invoice lines |
| AR_RECEIVABLE_APPLICATIONS_ALL | Receipt applications (optional) |

Steps: Create sales order, Submit sales order, Ship goods, Cancel order line, Create customer invoice, Receive customer payment. Order and invoice steps reach every line they cover; invoice
lines name the order line they billed (which field holds it is a question for the company).

Known limits: a cancelled line is timed by its last update; holds are not yet covered.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
