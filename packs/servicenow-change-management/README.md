# Change management (ServiceNow, checked on a real instance)

Follows change requests and their change tasks: Raise change, each change state (New, Assess, Authorize, Scheduled, Implement, Review, Closed), approval requested, approved or rejected, reassignment, reschedules and other kept fields; Create change task and each task state.

Written from public knowledge of ServiceNow's Table API, its standard tables and
the audit history (`sys_audit`); tested on made-up extracts
(`pm-testgen` `servicenow`) and a stand-in for the API.

1. `glasstarn extract discover <project> --system servicenow --site
   https://<instance>.service-now.com --object change_request --object change_task` lists each table's states, the
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
through the Table API and pulled through the normal approved extract: 5 normal changes (approved and closed, rejected, cancelled, handed to another group, with a change task) and the task: every planted step found in order, within two minutes.

- ServiceNow's change model adds steps of its own, and they appear as recorded: a second approval round (technical, then the change board) and change tasks it creates and later cancels.
- States must follow the change model; jumping states is refused by the instance.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
