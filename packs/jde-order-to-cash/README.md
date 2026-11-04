# JD Edwards order to cash (not yet tested)

Order to cash on JD Edwards EnterpriseOne, from sales order to customer
payment, followed by sales order line. Written by the Glasstarn
project from public knowledge of the JD Edwards tables; tested only on made-up
tables (`pm-testgen` `jde::write_selling`). Untested until confirmed on a real
system.

| Table | Contents |
|---|---|
| F4201 | Sales order headers |
| F4211 | Sales order lines, open (F4211) and closed (F42119) in one file |
| F42199 | Line status changes (optional; the detail ledger) |
| F03B11 | Customer invoices |
| F03B13, F03B14 | Receipts and the invoices they pay (optional) |

Steps: Enter sales order, Status (each last-status code the line passed),
Ship goods, Create customer invoice, Receive customer payment. Order lines
name their invoice (SDDOC), so invoice and payment steps reach every line they
bill. Known limits: most steps carry a date only; closed lines live in F42119
and must be exported with the open ones.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
