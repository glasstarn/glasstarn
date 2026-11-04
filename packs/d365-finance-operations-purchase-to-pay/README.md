# Dynamics 365 Finance and Operations purchase to pay

Purchase to pay on Microsoft Dynamics 365 Finance and Supply Chain
Management, from purchase order to settled invoice, followed by purchase order line.
Live tested in a demo company for one order line with two confirmations,
two partial receipts, split invoices and open unsettled items. All seven events,
four objects and twelve event links matched independently recorded source facts,
including source hashes and rows. The invoice-only accounts-payable variant was
separately reconciled: two invoices, two posting events and no invented settlement.
Approval rejection/resubmission and settlement remain untested live. Synthetic
regressions also cover multiple lines, date-only exports and missing context.

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
| PurchTable | Purchase orders |
| PurchLine | Purchase order lines |
| WorkflowTrackingTable | Approval workflow history (optional) |
| VendPurchOrderJour | Order confirmations (optional) |
| VendPackingSlipTrans | Receipt lines |
| VendInvoiceJour | Vendor invoices |
| VendInvoiceTrans | Vendor invoice lines |
| VendTrans | Open and settled items (optional) |

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
[purchase confirmation table reference](https://learn.microsoft.com/en-us/common-data-model/schema/core/operationscommon/tables/supplychain/procurementandsourcing/transaction/vendpurchorderjour).

Steps: Create purchase order, Workflow: each approval step (Submission, Approval,
Rejection...), Confirm purchase order, Receive goods (product receipt), Post vendor invoice, Settle vendor invoice. Lines
are linked through their lot ID (InventTransId), so receipts, invoices and
settlements reach every order line they cover.

`--variant accounts-payable` runs invoice to settlement alone. Known limit: a
settlement carries a date only (midnight on that day). A blank `Closed` date
or the X++ null date `1900-01-01` represents an open item and emits no
settlement step. A settlement is not proof of a cash payment: an invoice can
also be settled against a credit note. See Microsoft’s
[settlement overview](https://learn.microsoft.com/en-us/dynamics365/finance/cash-bank-management/settlement-overview)
and [X++ null values](https://learn.microsoft.com/en-us/dynamics365/fin-ops-core/dev-itpro/dev-ref/xpp-variables-data-types#null-values-for-data-types).

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
