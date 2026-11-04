# Incident management (ServiceNow, checked on a real instance)

Follows incidents: Open incident, each state in the instance's own list (New, In Progress, On Hold, Resolved, Closed), assignment to a group and a person, reassignment to another group or person, priority changes, and changes to other kept fields.

Written from public knowledge of ServiceNow's Table API, its standard tables and
the audit history (`sys_audit`); tested on made-up extracts
(`pm-testgen` `servicenow`) and a stand-in for the API.

1. `glasstarn extract discover <project> --system servicenow --site
   https://<instance>.service-now.com --object incident` lists each table's states, the
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
through the Table API and pulled through the normal approved extract: 7 incidents (straight through and closed, handed to another group, on hold and back, reopened, priority raised, cancelled, handed from one person to another): every step found in order, within two minutes of when it happened.

- Assigning a person to a new incident moves it to In Progress on its own (a ServiceNow rule); both steps appear at the same moment.
- Resolving needs a resolution code from the instance's own list; the codes differ between releases.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
