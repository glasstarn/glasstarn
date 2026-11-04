# Dynamics 365 Business Central order to cash (tested on a Business Central demo company)

Order to cash on Microsoft Dynamics 365 Business Central, from
sales order to payment, followed by sales order line. Written by the Glasstarn project from public knowledge of the Business Central tables; tested on made-up extracts
(`pm-testgen` `dynamics::write_business_central`) and on a Business Central
online CRONUS demo company (2026-10-16), from the standard
configuration-package export with order archiving off and on: every step
count matched the source tables, and traced orders matched their posted
documents. Not certified or endorsed by Microsoft. Approval steps were not
exercised; the demo company has no approval workflows.

| Table | Contents |
|---|---|
| Sales Header | Sales orders |
| Sales Line | Sales order lines |
| Approval Entry | Approval entries (optional) |
| Sales Shipment Line | Shipment lines |
| Sales Invoice Header | Posted sales invoices |
| Sales Invoice Line | Posted sales invoice lines |
| Cust. Ledger Entry | Ledger entries (optional) |

Steps: Create sales order, Send for approval, Approve sales order, Reject sales order,
Post shipment, Post sales invoice, Receive payment. Posted lines carry their order and
order line number, so receipts, invoices and payments reach every order line
they cover.

What a standard export gives, and how the pack copes (found on a real
Business Central company):

- **No SystemCreatedAt.** The configuration-package export, the usual admin
  route, leaves out system fields. The pack then uses the Order Date for an
  order and the Posting Date for shipments and invoices, to the day in the
  company's time zone, and says so in the check and the build.
- **Orders deleted once fully invoiced.** Business Central removes an order
  when it is fully invoiced unless order archiving is on. The pack asks once,
  then follows those order lines from their shipment and invoices, without the
  order's own creation and approval steps, and reports how many. Exporting
  the order archive with the open orders gives the full picture.
- **Other document types in the same tables.** Quotes, invoices and credit
  memos may stay in the header and line exports; only orders are read. Approval entries are also filtered by document type,
  so a quote or invoice with the same number cannot add order approval steps.

A payment carries the date the invoice closed only.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
