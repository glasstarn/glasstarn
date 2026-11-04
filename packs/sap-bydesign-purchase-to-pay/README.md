# SAP Business ByDesign purchase to pay (not yet tested)

Purchase to pay on SAP Business ByDesign, from purchase request to payment,
followed by purchase order line. Written by the Glasstarn project from
public knowledge of the ByDesign business objects and their OData services;
tested only on made-up extracts (`pm-testgen` `sap_bydesign::write`).
Untested until confirmed on a real tenant.

| File | Contents |
|---|---|
| PurchaseRequest | Purchase requests (optional) |
| PurchaseRequestItem | Purchase request items (optional) |
| PurchaseOrder | Purchase orders |
| PurchaseOrderItem | Purchase order lines |
| GoodsAndServiceAcknowledgement | Goods receipts |
| GoodsAndServiceAcknowledgementItem | Goods receipt lines |
| SupplierInvoice | Supplier invoices |
| SupplierInvoiceItem | Supplier invoice items |
| SupplierPayment | Payments (optional) |
| SupplierPaymentAllocation | Payment allocations (optional) |

Steps: Create purchase request, Create purchase order, Receive goods, Record supplier invoice,
Pay supplier invoice. Items carry the IDs of the order item they belong to, so an
invoice or payment reaches every order line it covers.

There is no direct connection to ByDesign yet: an administrator exports the
files through the tenant's OData services or reports, with times in UTC. File
and field names may differ by tenant; the pack's checks show what is missing.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
