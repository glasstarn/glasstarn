# SAP plan to produce (tested on an SAP trial system)

Production orders on SAP ECC and S/4HANA (on-premise or private cloud), from
creation to closing, followed by production order. Written by the Glasstarn
project from public knowledge of the standard SAP tables, with generated tests (`pm-testgen` `sap_operations::write_production`)
and a scoped live S/4HANA trial.

| Table | Contents |
|---|---|
| AUFK | Order master data (production orders are AUTYP 10) |
| AFKO | Production order headers: planned and actual dates |
| AFPO | Production order items: material, quantity, planned finish, the planned order and sales order line it came from |
| AFRU | Confirmations, including cancellations |
| MSEG | Goods movements against the orders (optional) |
| JCDS | Status changes (optional) |

Steps: Create production order, Release order, Issue components, Reverse
component issue, Confirm operation (partial or final), Cancel confirmation,
Receive finished goods, Reverse goods receipt, Complete order technically,
Close order, Flag order for deletion.

Each order keeps its planned finish, so late orders can be found by comparing
it with the last goods receipt, and the planned order it was converted from and
the sales order line it is made for, so orders from the planning run, made to
order or created by hand can be compared.

Known limits: SAP deletes a planned order when it is converted and keeps no
time for it, so the planning run itself appears only as where an order came
from, not as a step. One case per order; components are not yet followed as
their own items.

## Live validation scope

Tested by the Glasstarn project on SAP S/4HANA 2025 FPS01 trial data; not certified or endorsed by SAP. The table check and connected
build completed, and four order timelines matched SAP source records: a
common-route example, the engine-ranked slowest order, a movement reversal
and a confirmation cancellation. No source rows or validation checks were
removed to obtain the result.

This tests that trial scope, not every SAP release, tenant customization
or optional business branch. The trial contained no reversed finished-goods
receipt, so that branch still has only synthetic coverage.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
