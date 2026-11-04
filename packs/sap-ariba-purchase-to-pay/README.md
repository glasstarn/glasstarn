# SAP Ariba with SAP ECC or S/4HANA, purchase to pay (not yet tested)

For companies that buy through SAP Ariba and run orders, goods receipts,
invoices and payments in SAP ECC or S/4HANA (on-premise or private cloud).
Requisitions and their approval history come from three Ariba report exports;
everything after the order comes from the same SAP tables as
[the on-premise pack](../sap-purchase-to-pay/README.md). The two systems are
linked by the SAP order number Ariba keeps on each requisition and order
("ERP Order ID").

| File | Contents |
|---|---|
| AribaRequisition | Requisition ID, Title, Requester, Created Date, Submitted Date, ERP Order ID |
| AribaApproval | Requisition ID, Approval Step, Approver, Approval Role, Action (Approved / Denied), Action Date |
| AribaOrder | Order ID, ERP Order ID, Requisition ID, Ordered Date, Supplier |
| AribaInvoice (optional) | Invoice ID, Order ID, ERP Order ID, ERP Invoice ID, ERP Fiscal Year, Submitted, Exception Resolved, Approved and Rejected Dates, Status |
| LFA1, EKKO, EKPO, EKBE, RBKP, RSEG, BKPF, BSAK | As in the on-premise pack (no EBAN) |

Ariba column names depend on how the report is built, so the checklist names
the columns to include. Ariba dates carry their time in one field
(`2026-01-02 09:15:00`) and are read with their own time-zone answer.

Steps by purchase order line: Create requisition, Submit requisition, Approve
requisition (one per approval step), Deny requisition, Create purchase order,
Send order to supplier, the SAP goods receipt, then for invoices sent through
Ariba: Receive invoice in Ariba, Resolve invoice exception, Approve invoice (or
Reject invoice), then Record invoice and Clear vendor item in SAP. The data check also
asks about SAP orders created outside Ariba.

Written from public knowledge; tested only on made-up files
(`sap-p2p-sample <dir> --ariba`). Untested until confirmed on a real Ariba
site. `sap-ariba-s4-cloud-purchase-to-pay` is the same with S/4HANA Public Cloud
behind Ariba (`sap-p2p-sample <dir> --ariba --cloud`).

The on-premise side uses the current purchasing/payables schema: physical
invoice-item identities retain split-line lineage, and RBKP origin/status
fields distinguish maintenance, held, parked and reversal documents. Optional
T16FB configuration identifies final order release; without it, that final step
is omitted. See the on-premise pack's checklist and limits. The SAP-only trial
does not test live Ariba exports or their cross-system matching.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
