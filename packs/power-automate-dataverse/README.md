# Microsoft Power Automate and Dataverse (not yet tested)

Workflows built on Power Automate over Dataverse tables, read from the two
histories the platform keeps: the Dataverse audit log (who created, changed,
assigned or deleted each record, and when) and the cloud flow run history
(each run of each flow, when it started and ended, and whether it failed).
Records are followed by record; flows and their runs are items of their own.
Written from public knowledge of the audit and flowrun tables; tested only on
made-up extracts (`pm-testgen` `workflow_platforms::write_power_automate`).
Untested until confirmed on a real environment.

| File | Contents |
|---|---|
| audit | Audit rows of the table studied, with the action's label |
| flowrun | Cloud flow runs (optional) |
| workflow | The flows those runs belong to (optional) |

Steps: each audit action (Create, Update, Assign, Delete...), Start flow run,
Finish flow run, Fail flow run.

Known limits: auditing must be on for the table; which fields an update
changed is not read yet (only that it was updated); flow runs are kept 28 days
by default; a run is not yet linked to the record it acted on.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
