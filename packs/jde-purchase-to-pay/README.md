# JD Edwards purchase to pay (not yet tested)

Purchase to pay on JD Edwards EnterpriseOne, from purchase order to payment,
followed by purchase order line. Written by the Glasstarn project
from public knowledge of the JD Edwards tables; tested only on made-up tables
(`pm-testgen` `jde::write_purchasing`). Untested until confirmed on a real
system.

| Table | Contents |
|---|---|
| F4301, F4311 | Purchase order headers and lines |
| F43199 | Line status changes (optional; the detail ledger) |
| F43121 | Receipts and voucher matches |
| F0411 | Vouchers (supplier invoices) |
| F0413, F0414 | Payments and the vouchers they pay (optional) |

Steps: Enter purchase order, Status (each last-status code the line passed,
for example Status 280, Status 400), Receive goods, Match voucher to receipt,
Enter voucher, Pay voucher. JD Edwards dates are Julian (CYYDDD) and are read as
such. Known limits: most steps carry a date only; the agent maps status codes
to the company's names with the analyst.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
