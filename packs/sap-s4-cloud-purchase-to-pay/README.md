# SAP S/4HANA Public Cloud purchase to pay (not yet tested)

The same process as [the on-premise pack](../sap-purchase-to-pay/README.md), for
SAP S/4HANA Cloud, public edition. There, the company cannot browse raw
tables, so the files come from SAP's released data views or its standard
read-only purchasing, goods movement, supplier invoice and journal entry
services: one CSV per view, readable field names, dates as `YYYY-MM-DD`, and
the company's own fields named `YY1_…`.

Written from public knowledge of those views; tested only on made-up files
(`sap-p2p-sample <dir> --cloud`). Untested until confirmed on a real tenant;
the field names are the part most likely to need adjusting.

Differences from on-premise: goods receipts and invoices carry posting dates
only (no entry time), and requisition release dates are not read. The
`accounts-payable` variant needs only Supplier, SupplierInvoice and
JournalEntryItem.

## Change log and returns

With the optional change log (CDHDR and CDPOS, or the cloud change documents) the
pack adds order release steps and the final release, price, quantity and
delivery-date changes, deleted order lines, and invoice payment blocks set and
released, each at its own time. Goods returned to the vendor and vendor credit
memos are their own steps.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
