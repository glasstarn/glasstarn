# Connected extracts

The agent discovers the relevant objects and history, proposes a small extraction
specification, and files it for approval. A person approves it in the app, and the
app, not the agent, runs exactly that specification and downloads records directly
into project files. Bulk records do not enter the agent's model
context. The existing Rust preparation and analysis pipeline then uses those files.
Metadata and deliberately bounded samples used by the agent are still subject to
the company's agent data policy.

## Supported direct pulls

Glasstarn supports direct pulls from Salesforce, ServiceNow, Jira and HubSpot.
Each needs an authenticated, approved connection and a reviewed extraction
specification. The sections below describe the transport and limits for each.
Other systems can be investigated using exported files.

## Salesforce through the official CLI

This first transport requires an approved installation of the official `sf` CLI
with `data export bulk` available, an already authenticated org alias, and access
to Bulk API 2.0 queries. Provision a least-privilege account for reading the selected
objects. The runner cannot determine whether an account also has write privileges;
its own queries read records only. Authentication stays in Salesforce CLI's own
credential store. Never put tokens, passwords or credential-bearing URLs in a spec.
The runner does not install tools, authenticate, or upgrade them automatically.

Use [the example specification](../examples/connectors/salesforce-case-history.spec.json)
as a starting point, not a promise that every tenant exposes that history object.
Check tenant metadata, field tracking, retention, permissions and Bulk API support.
Choose an API version supported by both the org and installed CLI. The numeric
validation range is not a guarantee of vendor support. Custom object/field names
are supported as identifiers; relationships, arbitrary SOQL, executable code and
free-form predicates are not supported. Only equality filters are available.

Create or select an investigation with `glasstarn workspace`, then file the
extract:

```sh
glasstarn extract salesforce <project-id-or-folder> --spec spec.json
```

This prints `awaiting_approval` with a preview of every query and the spec's
`spec_sha256`. It contacts nothing. It needs no project lock, so it works while
the project is open in the app.

### Approval happens in the app

Open the project in the app. A panel titled **Extract waiting for your approval**
shows the system and connection, the window, each object with its fields, time
field, filters and declared history gaps, and every query the app will run. The
person types their name and chooses **Approve and run** or **Decline**.

- The command line cannot approve: `--approve` is refused. The approve and decline
  routes accept only the browser session's token; an agent's delegated MCP token
  is rejected.
- On approval the app runs the extract in the background. The panel shows
  progress, then the row counts or the reason it stopped.
- One approval covers one run of the exact spec bytes. Filing the same spec after
  a completed, failed or declined run opens a new request that needs a new
  approval. A spec edited after it was filed cannot be approved.
- `glasstarn extract status <project>` reports each request's state, approver
  and result without the queries.

The app looks for `sf` on the `PATH` it was started with. Start the app from a
shell where `sf` works.

### Audit log

Every request, approval, decline, completed run and failed run is appended to
`audit/log.jsonl` in the project folder: the time, the action, the operating
system user, the approver's name, the spec fingerprint, the system and window,
and, for a completed run, row counts and each output file's path and SHA-256.
Each line records the SHA-256 of the line before it, so an edited or deleted
entry breaks the chain. The log stays on the machine; nothing is sent anywhere.
The chain shows tampering but cannot prevent someone with write access to the
folder from rewriting the whole file, so copy it elsewhere when that matters.

Pass the same `--workspace-dir <directory>` used to create the project when using
a non-default root. Approval applies to the exact spec bytes; changes require a
new request. Dates are RFC3339 whole-second instants. The start is inclusive and
the end exclusive. The spec limit is 64 KiB, 16 objects and 64 fields per object.

### What the window selects (spec version 2)

The window selects **items**, not rows. An item is in scope when it has recorded
activity inside the window: a history row in the window, or, for a records object
that history describes, a `time_field` value in the window. The runner then takes
each in-scope item's **full recorded history up to the window end**, including
history from before the window, and the item's own record even when it was
created before the window. Nothing is cut at the window start, because that would
make every item already under way look as if it began there.

Each object names its `time_field`, its `kind` (`records` or `history`) and a
required `history_gaps` list; an empty list does not prove that history is
complete. A history object also names `item_field`, the field naming its item
(for example `CaseId`), and optionally `parent`, the records object in the same
spec whose items it describes (that object must select `Id`). A records object no
history describes is selected by its `time_field` inside the window.

The runner first scans for the IDs of items active in the window, then reads their
records and history with `IN` lists of at most 3,000 IDs per bulk job. The preview
shows every query with `<items active in the window>` in place of those lists.
Reports state the scope, the number of items in scope, the rows, and how many
history rows fall before the window.

After extraction, the project records the window. Analysis labels each case as
complete, started before the window, or still open, and a filter keeps only cases
that ran their whole course inside it; no case is dropped by default. "Still
open" needs the steps that finish an item, which a person confirms with
`glasstarn workspace window <project> --end-step <step>` (repeatable). Until
then it is reported as not determined.

### Date-only fields

Set `"time_type": "date"` on an object whose `time_field` is a calendar date
without a time of day, such as `Opportunity.CloseDate`. Its queries use date
literals and the window must start and end at midnight UTC. Exported values must
be `YYYY-MM-DD`. Analysis notes when timestamps are day-only and counts, per
connection, steps that share a recorded time: their order follows the file, not
the clock, and is not evidence of which came first.

The runner executes separate bulk jobs with a ten-minute CLI wait per job. A failed
job, unconfirmed completion, different columns, invalid timestamps, dates outside
the approved range or a row-count mismatch stops extraction. No preparation inputs
are registered unless every requested export completes and reconciles. A timed-out
remote job may remain in Salesforce; inspect it before deciding to retry. Do not
silently widen the date range, substitute current-state records for history, or
repeat failing jobs indefinitely.

Success saves content-addressed CSVs under `sources/`, registers them as preparation
inputs, and saves the approved `spec.json` and `report.json` under a unique
`extraction/extract-…/` run directory. Reports include who approved and when,
requested/observed dates, row counts, hashes and declared gaps. API call count is `null` because this CLI
transport does not expose it. History coverage remains explicitly unverified;
separate object jobs are not a transactional snapshot. Successful output contains
paths and summary counts, not records. Raw vendor errors are withheld from chat.

Review a bounded inventory with `workspace inventory` if needed, then use
[an approved preparation recipe](preparation-recipes.md) to turn field history
into process events. Records with their current status alone are not a historical
event log. The connector does not infer business activities or generate findings.

Jira and ServiceNow have their own transports. Salesforce and ServiceNow were
checked on real test accounts on 2026-10-15; each pack's README says what was
and was not checked. ServiceNow reads incidents, problems, changes, catalog
requests and their tasks, and any other ticket table built on `task` (HR cases,
customer service cases, a company's own tables) when the spec names it with
`"as": "ticket"`; a table not built on `task` is refused. HubSpot reads contacts,
deals, tickets and quotes changed in the window, the full history of chosen
properties (lifecycle stage, deal stage, ticket status, quote status) with
their labels, and the links between them, with a service key (a private app token on older accounts) in
`HUBSPOT_TOKEN`. All four packs were tested on a real developer test account on
2026-10-16; their READMEs state the observed paths and remaining gaps. Quotes
use `hs_createdate`; the other three record types use `createdate`. A missing
or invalid creation time stops extraction rather than silently omitting a record.
Initial property values at record creation are retained as source history;
first assignments made later produce change events. Very long source histories
may be trimmed by HubSpot, and trial coverage does not establish their completeness.
Some objects are ineligible for Bulk API queries; this version fails explicitly
rather than silently switching extraction methods.

## Connecting Salesforce to another system

Extracts remain separate object/history CSVs with the exact selected values.
Include native object IDs, history parent IDs, external order/invoice references
and relevant business attributes in the approved field list when needed. Custom
fields are supported, but are not automatically discovered or selected. Reports
record the selected columns and time field; the approved spec retains filters.

A future connected workflow can combine these with company-approved ERP exports.
The agent must establish actual link keys and relationship meaning, including
one deal creating multiple orders or multiple payments settling an invoice.
Do not guess links from customer names, similar amounts or nearby timestamps,
or implicitly remove leading zeros from identifiers. Account for tenant/company
scope when keys are not globally unique.

Name the records object as the history's `parent` so deals created before the
window but changed during it arrive with their record and full history. Objects
linked only by an external reference (an ERP order number, say) are not pulled
in automatically. Reconcile missing keys and relationships rather than treating
extraction success as proof of end-to-end coverage.

**Available:** [connected preparation](connected-preparation.md) builds an OCEL log
from inventoried linked CSVs using a reviewed mapping recipe. It preserves each
event once, typed objects/relationships and source lineage, and blocks unresolved
links. Tenant discovery and vendor-specific mapping still require confirmation;
a multi-table extract alone is not a connected analysis.

## Contributing adapters

Ship maintained adapters for common systems. Keep tenant-specific choices in specs
and recipes whenever possible. For an unsupported system or incompatible API, an
agent may help build an adapter, but it must be reviewed and tested before use;
never silently execute freshly generated extraction code against a company tenant.
There is no dynamic adapter loader in this release.

An adapter contribution should document supported API versions and permissions,
use the vendor's documented read endpoints, and keep secrets outside projects.
Download and paginate directly to local files, stream rather than collect records
in memory, and emit bounded completion metadata. Test success, pagination/partial
results, schema changes, permission failures, rate limits, redaction and count
reconciliation with synthetic fixtures. Register files only after all required
exports validate, preserve hashes and approved specs, and report unknown coverage
honestly. Include a separate authorized tenant trial before advertising support.

Discovery tools and bulk transport can differ: a vendor MCP can provide metadata
while an adapter or vendor CLI retrieves records directly. An MCP bulk-record
response is not the transport for this workflow.
