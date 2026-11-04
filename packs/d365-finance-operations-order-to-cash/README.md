# Dynamics 365 Finance and Operations order to cash

Order to cash on Microsoft Dynamics 365 Finance and Supply Chain
Management, from sales order to settled invoice, followed by sales order line.
Live tested in a demo company for a two-line order, confirmation, partial
shipments, split invoices and open unsettled items. All seven events, five
objects and fourteen event links matched independently recorded source facts,
including source hashes and rows. The invoice-only accounts-receivable variant
was separately reconciled: two invoices, two posting events and no invented
settlement. Approval rejection/resubmission and settlement remain untested live.
These shipment tests used explicit stock locations and direct line processing;
they do not establish warehouse-work or mobile-picking coverage. Synthetic
regressions also cover repeated confirmations, date-only exports and missing
context.

Export one legal entity per build, with every table filtered to that same
entity, including invoice-only variants. Document and lot identifiers are
company-scoped; the current pack does not combine reused identifiers from
different legal entities.

Supplier/customer accounts, item identifiers and display-only line/receipt
numbers are optional context. The vendor invoice display number is also optional
when its internal invoice identity is present. Their absence does not block the
build or remove otherwise supported events. Order,
line, invoice and movement identities, join keys and event times still identify
the process and its evidence.

Use existing Azure Synapse Link table extracts or data entities that expose the
requested fields, including source creation timestamps. Microsoft has deprecated
legacy [Export to Data Lake](https://learn.microsoft.com/en-us/dynamics365/fin-ops-core/dev-itpro/data-entities/azure-data-lake-ga-version-overview).

For small tests, administrators can also use the supported
[table browser](https://learn.microsoft.com/en-us/dynamics365/fin-ops-core/fin-ops/get-started/create-alerts)
in read-only mode and choose **Open in Microsoft Office → Export to Excel**.
Include system fields such as `createdDateTime`, filter to the intended company
and records, and retain the original workbook. Normalize field names and Excel
date values to the contract below. Verify the user's display time zone before
converting exported date-times to UTC; a workbook cell without an offset is not
proof that its displayed value is UTC. Microsoft’s
[Office integration FAQ](https://learn.microsoft.com/en-us/dynamics365/fin-ops-core/dev-itpro/office-integration/office-integration-troubleshooting)
distinguishes the static grid export from the entity-based Excel add-in. Do not
edit source tables in this view.

| Table | Contents |
|---|---|
| SalesTable | Sales orders |
| SalesLine | Sales order lines |
| WorkflowTrackingTable | Approval workflow history (optional) |
| CustConfirmJour | Order confirmations (optional) |
| CustPackingSlipTrans | Packing slip lines |
| CustInvoiceJour | Customer invoices |
| CustInvoiceTrans | Customer invoice lines |
| CustTrans | Open and settled items (optional) |

Standard confirmation tables expose document dates (`PurchOrderDate` for
purchases, `ConfirmDate` for sales). When `CreatedDateTime` is absent, the pack
uses those dates; receipts and invoices can similarly use `DeliveryDate` and
`InvoiceDate`. These are day-precision document dates, represented at midnight
in the confirmed offset, and cannot establish the actual posting instant or
precise elapsed time. Backdated documents can sort before their order creation.
The build reports every date fallback. Confirmation `RecId` keeps separate
confirmations on the same day; an exported confirmation document number can
stand in for that identity when `RecId` is absent.
Microsoft documents the date and record identity in its
[sales confirmation table reference](https://learn.microsoft.com/en-us/common-data-model/schema/core/operationscommon/tables/supplychain/salesandmarketing/transaction/custconfirmjour).

Steps: Create sales order, Workflow: each approval step (Submission, Approval,
Rejection...), Confirm sales order, Ship goods (packing slip), Post customer invoice, Settle customer invoice. Lines
are linked through their lot ID (InventTransId), so receipts, invoices and
settlements reach every order line they cover.

`--variant accounts-receivable` runs invoice to settlement alone. Known limit: a
settlement carries a date only (midnight on that day). A blank `Closed` date
or the X++ null date `1900-01-01` represents an open item and emits no
settlement step. A settlement is not proof of a cash payment: an invoice can
also be settled against a credit note. See Microsoft’s
[settlement overview](https://learn.microsoft.com/en-us/dynamics365/finance/cash-bank-management/settlement-overview)
and [X++ null values](https://learn.microsoft.com/en-us/dynamics365/fin-ops-core/dev-itpro/dev-ref/xpp-variables-data-types#null-values-for-data-types).

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
