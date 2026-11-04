# Request fulfillment (ServiceNow, checked on a real instance)

Follows requested items and their catalog tasks: Request item, approval, each stage and state of the item, Create catalog task, each task state and task reassignment.

Written from public knowledge of ServiceNow's Table API, its standard tables and
the audit history (`sys_audit`); tested on made-up extracts
(`pm-testgen` `servicenow`) and a stand-in for the API.

1. `glasstarn extract discover <project> --system servicenow --site
   https://<instance>.service-now.com --object sc_req_item --object sc_task` lists each table's states, the
   company's own fields (`u_…`) by name, the active assignment groups, and
   whether the audit history can be read (metadata only).
2. Write the extract spec (start from `extract.spec.example.json`): fields by
   name, the fields that also get display labels (`label_fields`; never
   people), and the audited fields to keep (`history_fields`; empty keeps every
   audited field).
3. `glasstarn extract servicenow <project> --spec spec.json`; a person
   approves it in the app. The app needs `SERVICENOW_USER` and
   `SERVICENOW_PASSWORD` (a read-only user that can read these tables and
   `sys_audit`) in its environment, and `curl`.

Files written: one per table (sys_id, number, created, the chosen fields as
stored values, and a `<field>_label` column per label field) and
ServiceNowAudit (one row per changed field: table, record, audit ID, time,
user name, field, old and new stored values and their labels). Values set
when a record is created are not changes and are left out. People appear as
stored IDs, not names. Only audited fields leave steps.

## Real-instance check (2026-10-15)

Passed on a ServiceNow developer instance (latest release), with records made
through the Table API and pulled through the normal approved extract: 3 catalog requests (delivered, rejected, task closed incomplete) and their catalog tasks: every planted step found in order, within two minutes.

- In the test instance the ordered item's own workflow raised no approval records or tasks, so approvals were set on the item and the tasks made by hand, as a fulfiller would. A catalog flow that makes its own approvals and tasks is not yet checked.
- Fixed by this check: a past workflow stage with no name in the choice list (such as request_approved) now reads as words (Request approved).

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
