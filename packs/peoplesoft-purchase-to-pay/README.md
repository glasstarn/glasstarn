# PeopleSoft purchase to pay (not yet tested)

Purchase to pay on PeopleSoft Financials and Supply Chain Management, common in
government and higher education, from requisition to payment, followed by
purchase order line. Written by the Glasstarn project from public
knowledge of the PeopleSoft tables; tested only on made-up tables (`pm-testgen`
`peoplesoft::write_purchasing`). Untested until confirmed on a real system.

| Table | Contents |
|---|---|
| PS_REQ_HDR | Requisitions (optional) |
| PS_PO_HDR, PS_PO_LINE | Purchase orders and their lines |
| PS_PO_LINE_DISTRIB | Which requisition each order line came from (optional) |
| PS_RECV_LN_SHIP | Receipts |
| PS_VOUCHER, PS_VOUCHER_LINE | Vouchers and the order lines they bill |
| PS_PYMNT_VCHR_XREF, PS_PAYMENT_TBL | Payments (optional) |

Steps: Create requisition, Approve requisition, Create purchase order, Approve
purchase order, Receive goods, Enter voucher, Approve
voucher, Pay voucher. Requisition, order and voucher steps reach every order
line they cover.

Known limits: PeopleSoft records dates without times, so steps on one day keep
their order only by step; budget checking (commitment control) is a status,
kept as a fact on each order; cancelled lines are marked, not timed.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
