# SAP equipment maintenance (tested on an SAP trial system)

Maintenance on SAP ECC and S/4HANA (on-premise or private cloud), from the
reported problem to the closed order, followed by maintenance order. Written by
the Glasstarn project from public knowledge of the standard SAP
tables (Plant Maintenance), with generated tests (`pm-testgen`
`sap_operations::write_maintenance`) and a scoped live S/4HANA trial.

| Table | Contents |
|---|---|
| QMEL | Maintenance notifications (the reported problems) |
| QMIH | Their equipment and breakdown times (optional) |
| EQUI | Equipment (optional) |
| AUFK | Order master data (maintenance orders are AUTYP 30) |
| AFIH | Maintenance order headers: equipment, notification, activity type |
| AFRU | Confirmations of work done |
| MSEG | Spare parts issued to and returned from the orders (optional) |
| JCDS | Status changes of orders and notifications (optional) |

Steps: Report problem, Equipment breaks down, Create maintenance order, Release
maintenance order, Issue spare parts, Return spare parts, Confirm work (partial
or final), Cancel confirmation, Complete order technically, Close order, Start
work on notification, Complete notification.

Notifications, orders and equipment are linked, so a piece of equipment can be
followed across all its breakdowns and repairs.

## Live validation scope

Tested by the Glasstarn project on SAP S/4HANA 2025 FPS01 trial data; not certified or endorsed by SAP. The table check and connected
build completed, and four order timelines matched SAP source records: a
common-route example, the engine-ranked slowest order, a movement reversal
and a confirmation cancellation. No source rows or validation checks were
removed to obtain the result.

This tests that trial scope, not every SAP release, tenant customization
or optional business branch. One equipment object without observed events
remained visible as a non-blocking quality warning.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
