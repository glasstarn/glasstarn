# Connected records

Use this path when the user wants records from a supported SaaS system rather
than providing files. Salesforce is implemented through the official
`sf data export bulk` CLI. Jira Cloud is implemented through its REST API (v3)
with `curl`, using `JIRA_EMAIL` and `JIRA_API_TOKEN` from the app's environment
(never ask for the token in chat). ServiceNow is implemented through its Table
API with `curl`, using `SERVICENOW_USER` and `SERVICENOW_PASSWORD` from the app's
environment (never ask for the password in chat). HubSpot is implemented through
its CRM API with `curl`, using a service key (Settings, Integrations, Service Keys; a private app token on older accounts) in `HUBSPOT_TOKEN`
from the app's environment (never ask for the token in chat). Other systems are
planned; do not claim these connections exist. SAP/Oracle still require company-approved extracts.

1. Reuse an approved tool installation and authenticated connection. Do not
   install, log in, upgrade or broaden permissions silently.
2. For Salesforce, run `glasstarn extract discover <project> --org <alias>
   --object <Name> [--object ...] [--pack <pack>]`. It reads metadata only (fields,
   the company's own fields, which fields keep history) through the saved alias,
   saves the result in the project and names history a pack needs but the org
   does not track. Otherwise, discover bounded metadata through the available
   vendor MCP or metadata API:
   relevant objects, keys, fields, tracked history and retention. Use small samples
   only when needed and permitted. Do not fetch bulk records through MCP or paste
   them into chat. Establish business meaning with the person when ambiguous.
3. Write a version-2 Salesforce JSON spec: `system`, `target_org` (saved alias),
   `api_version`, inclusive `start`, exclusive `end`, and `objects`. Each object
   needs `name`, selected `fields`, selected `time_field`, `kind` (`history` or
   `records`), `history_gaps` and optional `filters` of `{field, equals}`. A
   history object also needs `item_field` (e.g. `CaseId`) and usually `parent`,
   the records object it describes. Set `"time_type": "date"` for a date-only
   time field such as `CloseDate`; its window must fall on midnight UTC.
   Use whole-second RFC3339 bounds and simple identifiers, including custom names.
   Credentials, arbitrary queries and executable instructions do not belong in specs.
   The window selects items with activity in it and takes each item's full history
   up to the window end, including earlier history; explain that when reviewing.
   A Jira spec (version 1) has `system: "jira"`, `site` (https address),
   `start`, `end`, `projects` (keys), optional `issue_types`, `fields` (field
   IDs), optional `history_fields` (field IDs to keep from the change history;
   empty keeps all, including description edits) and `history_gaps`.
   A ServiceNow spec (version 1) has `system: "servicenow"`, `instance` (https
   address), `start`, `end`, `tables` (each `table`, `fields`, optional
   `label_fields` for display labels, never people, and for a child table
   `parent: {field, table}`, such as change tasks by `change_request`),
   optional `history_fields` (audited fields to keep; empty keeps all) and
   `history_gaps`. Tables: incident, problem, problem_task, change_request,
   change_task, sc_request, sc_req_item, sc_task; any other ticket table built on
   task with `"as": "ticket"` (and its tasks with `"as": "ticket_task"`).
   A HubSpot spec (version 1) has `system: "hubspot"`, `start`, `end`, `objects`
   (each `object`: contacts, deals, tickets or quotes; `properties` to write;
   `history`, the properties whose full history to keep, such as `dealstage`),
   optional `associations` (pairs such as `["deals","contacts"]`) and
   `history_gaps`. Start from the HubSpot pack's `extract.spec.example.json`.
4. File it with `glasstarn extract salesforce|jira|servicenow|hubspot <project-id-or-folder> --spec <file>`
   and the investigation's workspace root. This contacts nothing and returns a
   preview. Explain its objects, fields, window, filters and gaps to the person,
   then ask them to open the project in the app and choose **Approve and run**.
   Only a person can approve, in the app; the app then runs the extract. You
   cannot approve or run it, and must not try to through the browser or by
   editing project files. A past general investigation request is not approval
   of an unspecified new external extraction. Check progress with
   `glasstarn extract status <project>`.
5. On failure, explain the sanitized error and the next bounded diagnostic check.
   Do not claim success, automatically widen scope or loop through retries. A
   timeout can leave a remote query job running; inspect before retrying. A retry
   is a new request and needs a new approval.
6. On success, use returned hashed source assets with the existing inventory and
   approved preparation-recipe workflow. Review history coverage separately even
   when `truncated` is false. `api_calls: null` means unknown, not zero. Preserve
   the report as evidence. Current-state records cannot substitute for transitions.
7. The project now records the window. Ask the person which steps finish an item
   and record them with `glasstarn workspace window <project> --end-step <step>`
   so analysis can tell complete cases from open ones. Never drop cases that began
   before the window or are still open; report the labels and offer the filter.

Use tenant metadata to confirm CLI/API and Bulk query support. This connector is
not universal across Salesforce editions, API versions or history objects. It
requires an installed, authenticated official CLI; it performs no setup itself.
A real-org validation is still pending. Do not present an example spec as a
prevalidated tenant mapping or invent a packaged process recipe.

If metadata reveals custom fields or statuses, adapt the spec and recipe first.
If the system/API needs new executable behavior, explain the unsupported boundary.
An agent can help create a maintained adapter with version checks and offline
fixtures, then obtain authorization for its tenant trial. Do not execute an
untested generated script or download a replacement adapter automatically.
Bulk transport must stay outside model context regardless of the harness used.

## Cross-system questions

For Salesforce plus ERP records, deliberately retain native IDs, history parent
IDs and approved external link fields, with business attributes needed for the
question. Extraction preserves selected values in per-object files, not a
flattened case log. Ask for explicit links and their business meaning; do not
infer identity from customer names, timestamps or amounts. Preserve leading zeros
and identify tenant/company scope. Set the history's `parent` so older records
changed during the window arrive with their record and full history.

ERP files can be supplied without an ERP connector. Read
[connected preparation](connected-preparation.md) to build the connected OCEL using
`workspace prepare-ocel` and confirmed explicit keys/relations. Do not use the
case-log preparation command for an implicit flattened join. A ready-made OCEL
can use native import directly. Record history and link gaps; relationships are
not proof of causation and do not imply every cohort query is supported.
