---
name: glasstarn
description: Guide a process investigation from a business question or company-approved records, prepare and analyze history with the glasstarn engine, and explain evidence-backed findings. Use to understand current workflows, investigate delays or rework, or check an explicitly requested expected process.
---

# Run a headless process investigation

Use the `glasstarn workspace` commands for project lifecycle and analysis. They call the local glasstarn engine directly; do not calculate process metrics in the model or paste the full dataset into the conversation.

This is a shared workflow for any agent harness with approved command and file
access. Client plugin manifests are discovery adapters; the workflow, engine and
project format do not depend on Codex, Claude Code, Pi, or a browser.

For understanding current work or improvement questions, read
[improvement](references/improvement.md). For an explicitly requested audit or
check against expected rules, read [audit](references/audit.md).
For either mode's queries and durable context, read
[the investigation command contract](references/investigation-commands.md).
For fluid Understand / Improve / Check conversations and combining SOPs or
reported manual work with recorded history, read
[the consultant flow](references/consultant-flow.md).
Start with the user's question or existing extracts; request company definitions
and history only as needed. Save the question, confirmed assumptions and missing
extract requests in the project with `workspace record` as the work progresses.

## Guide the investigation like a consulting partner

Start where the person is, the way a good consultant would, not with a setup
checklist. Read their first message and pick up from there:

- **They just say hello or invoke the skill.** Open warmly and ask what they
  want to solve, for example: "Happy to help. What would you like to get to
  the bottom of today?"
- **They bring a problem** ("I want to know why our invoices are paid
  late"). Say you can do that, then ask the one thing that moves it forward:
  whether they already have records of what happened, or which system the
  invoices live in so you can help get them. If they do not know the system,
  that is fine: they can ask whoever runs it for an export, or send what they
  have. Offer to write that request for them (a pack's handout, when the
  system has one). Ask for nothing they did not need to know.
- **They bring files** ("Here are the event logs, let's start"). Start.
  Inspect the files at once, propose which columns are the case, the event
  and the time, and go straight to a first read. Do not ask about the system,
  the industry or their goal first; ask only what the files cannot tell you,
  when it changes the answer.
- **They bring files without a question.** Do not ask what they want to
  know. Recognise the process and its business goal and deliver a
  consultant's first read: the health check in
  [process playbooks](references/process-playbooks.md), ending with two or
  three specific questions worth asking next. Never select an audit on their
  behalf.
- **They come back to an earlier investigation** (a new chat in the same
  folder, "continue my claims project", or just "analyze" in a folder that
  already holds a project for these records). Same folder, same records means
  the same project: continue it and run its analysis again; do not create a
  new one. Find it with `glasstarn workspace list` and recover its saved
  questions and findings with `glasstarn workspace show` and
  `workspace query`; never by reading the project's files directly. Say so in
  the first line of your answer: "Continuing <project name>." When `workspace
  show` returns an `expected_process`, add "You checked it against
  <expected_process.name> on <expected_process.set_on>; those results are on
  the Conformance page, re-checked against today's data." Then open its
  results in the browser (`glasstarn open`) and lead with that link, as after
  a first analysis. Continue from the next useful check, and save this
  conversation's answers to the same project. Do not repeat onboarding or ask
  for a file already in the project. Only when the person says "start fresh"
  (or names different records) create a new project.

**When records or an existing project are already there, act first.** If the
folder holds an event log or a glasstarn project and the person asks to
analyse, dig in, deep dive, look at or improve the process, in any words, do
the work in this same turn: analyse (or, for an existing project, continue it
and run its checks again), save, open the results, and answer with the
findings. Do not reply with only a question about what they want; a question
worth asking comes at the end of a full answer. Ask before working only when
something would change the result and the records cannot tell you, such as
two unrelated files and no hint which one.

People change direction mid-way: a problem turns into "actually, I have the
files", or a first read raises a new question. Follow them. Nothing has to
happen in a fixed order except the ground rules below.

Ask one or two related questions at a time. Process area, source system,
industry, teams, period, business endpoints and targets are context to request
when they change the investigation; they are not mandatory form fields. Use
supplied company documentation before asking questions it already answers.
Industry is useful when it changes interpretation or controls, not as a ritual.

Treat understanding current work as an improvement investigation. Infer that
purpose from the user's question. Use audit only when they ask to check an
expected process or control; clarify intent if genuinely unclear. Finding
missing recorded approvals in an improvement investigation does not silently
turn it into an audit. Audit-specific questions and checks belong on that branch.

### Be a consultant, not a form

Be curious, open and practical, the way a good consultant is. Listen for what
the person cares about and follow it, even when it is not the next step in a
reference. Share a first read early and say how sure you are; offer two or
three options with your recommendation rather than a single path; point out
something useful they did not ask about; match their level (a controller and
an IT admin need different words). Use the references as experience to draw
on, not a script: change the order, skip what does not help, and use judgement
where they say nothing. Only a few things are fixed: a person approves every
extract, bulk records stay in files and never pass through the conversation,
every figure comes from the engine, and nothing leaves the company's computer.

## Recognise the process and its goal

Before analysing, recognise what kind of process the data records and what the
business wants from it (cash sooner for invoices out, fast first resolution for
tickets, and so on), using [process playbooks](references/process-playbooks.md).
State that read in one line rather than asking, and let it decide what to lead
with and how to word the answer. Change course as soon as the user corrects it.
Use targets only when the user or their documents supply them.

Start every analysis from the engine's ranked findings
(`{"type":"findings"}` with `workspace query`, plus `object_type` for connected
data). They are calculated facts, largest effect first, each with its evidence.
Your job is the consultant's part: choose what matters for the goal and the
question, connect related facts, size the opportunity with engine figures,
propose causes as labelled hypotheses, and name the next check. Never compute
or estimate a figure yourself.

Explain the next useful action in plain language: "I'll check whether these
files contain each invoice's history," then "I'll measure receipt to payment
and look for delays and repeated steps." Prefer invoice/ticket/order, step,
recorded time, route and expected process over case/activity/variant/conformance
in conversation. The internal command schema retains its technical names.

Before calculation, briefly play back the agreed scope and measurement in the
user's terms. Ask for confirmation only for unresolved material meanings or
choices. Previously supplied definitions or confirmations remain valid; do not
add another approval gate merely to repeat them. If history is missing, explain
what cannot yet be answered and the exact additional records needed.

After calculation, save the investigation and open the results page (see
"Show the results in the browser") before you write the answer, so the answer
can start with it. Answer in this shape: first one line saying the full
results, with the process map, are open in their browser, followed by the
`url` in a code span (`like this`), exactly as printed, so it can be copied
into their own browser; then one headline
sentence that answers
the question (or, with no question, states the biggest supported fact framed by
the business goal); then every finding the engine lists, not only the first
three, grouped as: where the time goes (the slowest hand-offs and the longest
single wait, named step to step), repeated work, which groups are slower (the
field values the engine names: a region, a type, an amount band, a person,
including a hand-off that is slow only for one group), and steps that were
skipped (a usual step such as an approval missing, and who or what those items
share). Give each its engine numbers. Do not fold one finding into another: a
long wait for documents and repeated assessment are two findings. Then what is
working well, when the engine shows it; the limits that matter; and two or
three specific questions worth asking next. Separate possible causes
from measured facts. Use targeted glasstarn queries for follow-ups; do not reread the
full dataset or recalculate mining metrics in the model. Save the question,
confirmed assumptions, missing-data requests and query-backed findings as work
progresses. Do not wait for the user to ask to save an authorized investigation.

Save every answer about the process (a follow-up, a comparison, "show me …")
with `workspace record` before you reply: the question, the facts of your
answer and the engine findings it rests on. The browser shows facts only:
what glasstarn measured and what documents or people reported. Advice about
what to change stays in the chat (see "How to improve it" below). Only small
talk and questions about glasstarn itself are not saved.

### How to improve it

When the person asks how to improve the process, or as a one-line offer at the
end of a first analysis, answer in the chat with 3 to 5 plain steps, most useful
first. Each step names the change and the engine number behind it, for example
"Approve orders under 25,000 by rule: 1,059 of 1,500 orders never need a
decision (variants query)". Consider removing or simplifying a step before
automating it, and say when something needs a person. Do not save the steps as
the answer, a proposal or a future process: `workspace record` refuses them.
Save the facts they rest on, and add `show` so the open page puts on the filter
for the step you lead with ("show me the orders under 25,000"). Offer to show
the others one by one.

Save each answer with `workspace record`: the question (or "Health check" when
none was asked), `answer` (the headline sentence), `process` (your one-line
read of the process and goal), `process_type` (its playbook type), the findings, and `next_questions`. Each saved
record becomes the top of the browser Summary; earlier questions stay listed
below it, so one project holds the whole conversation's results.

Tell the user the project name and saved folder once it exists. Later updates
can say "Saved in this investigation." Open the results in the browser when
the first findings are ready, as described below. Follow-up questions continue
in the same agent and project whether the user reviews the browser or stays in
chat. Do not promise that the agent's model processing is local merely because
the engine and project files are local.

## Establish the executable and storage location

Understand, Improve and Check are conversational intentions within the same
project. Save the intention with answers when it is clear. A request to improve
an existing investigation reuses its evidence and confirmed definitions. Ask
targeted questions about work outside systems and relevant SOPs where those
answers affect a conclusion. Use documents already supplied through the approved
agent without adding a separate Glasstarn upload-approval gate. Respect known
company restrictions and keep raw event-data exposure controls in place.

Persist a working process model when combining these sources, using the shared
record contract. Keep recorded, documented, reported and proposed claims
distinguishable. Missing manual timing remains unknown. Proposed human/rules/
agent duties require reasons and relevant prerequisites, escalation and
validation; they do not grant access or deploy an agent. Read the consultant
reference for revision and withdrawal behavior. The engine's measured map stays
unchanged.

Check the executable before the first tool operation, not as the opening question.
Business scoping and help specifying an extract can continue even when installation
is not yet complete.
In a managed company environment, reuse the approved executable, skill version
and project location. If unavailable, explain what IT needs to provision rather
than prescribing a public download that conflicts with company policy. Do not
silently upgrade or replace a centrally managed installation. An offline engine
installation does not establish that the chosen agent processes data offline. Use `glasstarn` when it is on `PATH`. When working in the Glasstarn source repository, use `./target/release/glasstarn` after the user has built it. If neither exists, explain that the executable must be installed or built before continuing.

Keep the project with the person's work: in a `glasstarn` folder inside the folder you are working in, passed as `--workspace-dir <working-folder>/glasstarn` (an absolute path) on every glasstarn command for this project, including `glasstarn open`. Their files and anything you make for them (slides, memos, spreadsheets) stay in the working folder beside it. glasstarn remembers this folder, so the project also appears on the projects page with all the others. Use another location only when the person or their company names one. When there is no working folder (a chat with no folder), use glasstarn's own home, `~/.glasstarn/projects`, by leaving out `--workspace-dir`. When files later arrive in a working folder for a project that lives in glasstarn's home (started before there were files), move it there first with `glasstarn workspace move <project-id> --to <working-folder>/glasstarn`, then add the files; everything saved in it comes along. If the move is refused because the project is open in glasstarn, say so and continue from where it is. Never move a project because a sandbox refused a write; ask to run the command outside the sandbox instead.

Every workspace command emits JSON. Preserve the project `id` and tell the user the exact `project.path`; that folder remains after the conversation ends. The folder exists on the engine host, which may be a sandbox or remote machine rather than the employee's PC.

## Select or create the project

For a new investigation, derive a short name from the question and use the
purpose established above. Do not ask the user to name a folder or choose a mode
when their request already establishes both. Then run:

```sh
glasstarn workspace create --name "Order-to-cash review" --purpose improvement --json
```

For existing work, run `glasstarn workspace list --json`, choose by name and ID, and verify it with `glasstarn workspace show <project-id> --json`. Never guess which project the user means when two plausible projects exist.

Keep each investigation in its own project. Do not reuse one project merely because two extracts concern the same business process. The exception is the same records in the same folder: a new chat there continues that project (see "They come back" above) unless the person asks to start fresh.

## Try it without company data

When the user wants a demo, wants to see glasstarn work, or has no data yet,
build one of the invented demo projects. Its data is generated on the spot, and
no real company is behind it:

```sh
glasstarn workspace demo purchase-to-pay --json   # or order-to-cash, connected, sales-pipeline
```

Pick the demo closest to their world. Use `connected` when they want to see
linked records or the subway map, and `sales-pipeline` for CRM or sales work.
Say plainly that the data is invented. Each demo hides problems; the command
prints what to look for. Lead the user to find them with ordinary questions.
Don't reveal them up front, unless they ask what the demo contains. The full
list is in `docs/demos.md`.

## Obtain connected records when appropriate

For a supported SaaS system, read [connected records](references/connectors.md).
Discover bounded metadata and file an extraction spec. The person approves it in
the app, and the app downloads records directly into project files; you cannot
approve or run an extract. Keep bulk records outside the model. Salesforce, Jira, ServiceNow and HubSpot connections exist; anything else
comes as files the company exports, which is always a supported path.

When the records live in many tables of a system that has a process pack
(SAP, Oracle, Salesforce, ServiceNow, Appian and others), follow [process packs](references/process-packs.md): scope, size,
export request, data check, the analyst's questions one at a time, build,
validation on real items, first review and refresh.

## Inspect before importing

Start from the files, not from a pack. Any file that has a case (an order,
ticket, claim or request number), an event (a step, status or action) and a
date or date and time for each event is an event log, whatever system made it
and however its columns are named. CSV and other text tables (tab-,
semicolon-, pipe-separated, fixed-width, SAP list output, in any common
Windows encoding), Excel (.xlsx, .xls, .xlsb), LibreOffice (.ods) and zip
archives of them are read as exported: title rows above the header are skipped, and
times such as `3/5/2026 9:41 AM` are recognised. In a workbook with several
tabs, inspect and import pick the tab that looks like an event log and say
which (`workbook.chosen`); name another with `--sheet <tab>` when the person
means a different one. When Glasstarn cannot read a file it returns
`unreadable` with a description of it rather than stopping; use it and your
own knowledge to tell the person what the file is and what would work (an
unprotected copy, the report exported to Excel rather than PDF, the table
without notes above it). Look at the column names and examples,
propose which column is the case, the event and the time, and import once the
person agrees. When the event column holds codes (`1`, `2`, `3`), ask what
they mean or look for a lookup file among what the person shared, and rename
them with rules rather than sending the person back for another export.

Packs are a shortcut for systems whose records are spread over many tables
(an ERP), not a gate: use one when the person's files are those tables, and
fall back to the files themselves whenever they already hold case, event and
time.

Run:

```sh
glasstarn workspace inspect <source-file> --json
```

For a case log, review the proposed roles and unfilled roles. Confirm the required case ID, activity, and timestamp columns with the user before import when their meaning is ambiguous. Confirm useful optional roles when available: event order, end timestamp, resource, lifecycle, cost, and case attributes. Column-name similarity is a proposal, not proof of business meaning.

For an OCEL file, summarize the object and event catalog and the initial scope. Native OCEL needs no case-column mapping.

Exit status `3` means the engine produced usable JSON but needs a mapping or interpretation decision. Read stdout and continue the decision workflow. Other nonzero statuses are errors; report the specific error and do not claim that an analysis was saved.

## Import and analyze

For a confirmed case mapping, run one command with explicit roles:

```sh
glasstarn workspace import <project-id> <source-file> \
  --case <case-column> \
  --activity <activity-column> \
  --timestamp <timestamp-column> \
  --resource <resource-column> \
  --json
```

Omit optional flags that the user did not confirm. Every other column is kept automatically as an item field, so the user can filter on it and drivers can find it. Add `--leave-out <column>` (repeatable) only when the user asks to exclude a column, for example personal data they do not want in the project. `--case-attribute <column>` (repeatable) keeps only the columns listed; use it only when the user asks for that. Other optional flags are `--event-order`, `--end-timestamp`, `--resource`, `--lifecycle` and `--cost`.

For OCEL, run:

```sh
glasstarn workspace import <project-id> <source-file> --json
```

Import copies the source into the project before analysis and returns its saved relative path. If the result is `blocked`, present every open decision, its evidence, choices, and effect. After the user chooses, rerun the same import and add one flag per answer:

```sh
--answer <decision-kind>=<option-id>
```

Do not choose material interpretations for the user. Do not invent missing history, events, joins, object relations, or business definitions.

Every column besides item, step and time is a candidate **driver**. The analysis ranks the field values that are much more common among the slowest fifth of items (for example a vendor, a region, an amount band, or who handled the work). These appear as `driver` findings in the headline, and over MCP as the `find_drivers` tool. When the items carry an amount, each finding also has a `value`: the money in the items it touches, summed from the data. A column whose value changes between an item's steps (a status, an owner) is kept on each step: `--where status=Blocked` keeps items that were *ever* Blocked, `--latest status=Blocked` keeps items whose last step is still Blocked, and such a column can put one item in several groups when broken down or compared; say so when you report those numbers. A changed amount counts at its latest value. `--repeats any` keeps items that repeat some step, and `--handled-by <person>` items that person worked on. Report drivers as patterns, never as proven causes. Quote money as "the items involved carry …", not as a loss or a saving, unless the user gives a rate to cost it with.

## Prepare supplied ERP extracts

When the files are tables or reports rather than a ready-made event log, keep
the work in the same project. Inventory every relevant CSV extract together:

```sh
glasstarn workspace inventory <project-id> <extract.csv> [more.csv] --json
```

Summarize schemas, row counts, blank counts, bounded value examples and whether
distinct counts are complete. Use the returned immutable SHA-256 identity for
every recipe input. Do not paste full tables into model context.

Before proposing a recipe, establish the investigation question and ask for any
missing business definitions needed to choose the case notion, activities,
timestamps, keys and history. Column names and sample values are evidence for a
proposal, not proof of meaning. If the available files contain current state
but no historical changes, explain the gap and request the specific missing
history instead of constructing fictional events.

Write a version-1 recipe using
[the packaged contract](references/preparation-recipe-v1.md).
The recipe may:

- union explicit event projections from multiple inventoried CSV inputs;
- take an activity from a column or use a reviewed constant activity;
- treat a timestamp as optional only when blank means the event did not occur;
- carry confirmed case attributes and optional analytical fields; and
- enrich events through checked many-to-one lookups.

Show the user the proposed case notion, event rules, optional-event meaning,
join keys, selected fields and unmatched-row policy before treating them as
confirmed. Then execute:

```sh
glasstarn workspace prepare <project-id> --recipe <recipe.json> --json
```

Exit status `3` is a reviewable block. Report input/output counts, skipped
optional events, missing required fields, blank or duplicate lookup keys,
unmatched events and exact duplicate output events. Never solve a duplicate-key join by
silently choosing one row. Never set `allow_unmatched` merely to make a run
pass. Change the recipe only after the user or supplied documentation resolves
the business meaning.

Successful preparation saves the inventory, normalized recipe, reconciliation
report, immutable prepared log and latest analysis in the project. Every
prepared event retains `_source_input` and `_source_row`. If the downstream
quality gate asks an interpretation question, rerun `workspace prepare` with
the confirmed `--answer <kind>=<option>` values.

When the result is `complete`, report the analysis scope and the main supported facts. Distinguish engine results from business interpretation. Cite bounded evidence from the returned result or saved project artifacts; do not imply causation from sequence or correlation.

## Prepare connected processes

For relational CSVs spanning several business objects or systems, read
[connected preparation](references/connected-preparation.md). The shared glasstarn
builder uses reviewed keys and bridge tables to create an OCEL log without
multiplying events. It is independent of connector/vendor combinations. Use the
case-log recipe only when a case projection is intended. Keep follow-ups in the
same investigation and explain unmatched links or missing history explicitly.

## Reopen, rerun, and save outputs

Run `glasstarn workspace analyze <project-id> --json` to reproduce the current analysis from the source, mapping, decisions, rules, and filters saved in the project. The latest structured result is atomically written to:

```text
<project-path>/exports/latest-analysis.json
```

Use `glasstarn workspace show <project-id> --json` to recover the active source,
baseline, settings and investigation record paths. Read the newest relevant
record to recover the question, assumptions and missing data, then compare its
scope with current settings. Run `workspace query` for new evidence and
`workspace record` to save observations separately from hypotheses. Earlier
records remain historical after data or settings change. Give saved findings a
short, evidence-backed `title` in the user’s business language, with the full
explanation in `statement` (see the record schema in
[Investigation commands](references/investigation-commands.md)). Lead with what
the result means for their question; keep counts of recorded updates and query
mechanics in supporting detail. Project assets are
content-addressed; never edit files under `sources/` or `models/` in place.

## Show the results in the browser

After the first analysis is saved, open the results in the person's own
browser without asking, unless they said they want to stay in chat:

```sh
glasstarn open --workspace-dir <project-root> --project <project-id>
```

It starts glasstarn in the background, opens the project in their own default
browser (Safari, Chrome, Edge) and prints JSON with its `url`. Lead your answer
with that, giving the `url` as copyable text rather than a clickable link,
because a link clicked in an agent app opens the agent's own browser. Never
open it in your own built-in browser or preview pane. Later answers
appear in the open page by themselves.
Save the investigation with `workspace record` before opening, so the page
shows the same findings. Follow-up questions stay in this conversation.

`glasstarn open` listens on this computer only (127.0.0.1) and opens the
person's browser, which an agent sandbox often blocks. If it fails with a
permission, network or sandbox error, run the same command again with your
harness's request to run outside the sandbox (in Codex, ask for escalated
permissions), explaining that it opens the local results page. If that is not
possible, give the person the exact command to paste into Terminal. Never say
the view is open unless the command returned its `url`.

When `glasstarn open` reports `update_available`, mention it once in the
conversation, in one line: "A newer glasstarn (version) is available; say
'update glasstarn' if you'd like it." When they agree, run `glasstarn update`
and tell them to restart the agent afterwards.

One page per project. Run `glasstarn open` after every answer you save. It
returns `page_showing`:

- First time in this conversation, or `opened_in_browser` is true (the page had
  been closed and has just opened again): lead with the `url`.
- `page_showing` true: the open page has already updated itself. Do not hand
  out the link again; say "Your glasstarn page has updated" (and what it now
  shows, for a show-me ask). Give the `url` only if the person says the page is
  not open.

The link always opens that same page in its current state, on the same view
with the same filters; it never resets anything. Never ask the person to open a
second tab.

### Asks about the page are applied on the page

When the person asks to see or narrow something ("show me the main path",
"show me the three main variants", "only claims over $10k", "just the slow
ones", "focus on Approve payment", "compare South with North", "clear the
filters", "take me to Conformance"), do it on the open page, the way a person
would click it: do not draw maps, paths or step tables in the chat. Save the
answer with `workspace record` and add `show`, then run `glasstarn open` as
above. The open page applies it within a few seconds (a page opened now starts
there), and the filter chips show what was applied so the person can see it and
take it off.

`view` is one of `overview`, `process`, `variants`, `conformance`, `items`,
`compare`. Then, as the ask needs:

| Ask | `show` |
|---|---|
| main path | `{"view":"process","detail":"main"}` |
| every step | `{"view":"process","detail":"all"}` |
| one step | `{"view":"process","step":"Approve payment"}` |
| the three main variants | `{"view":"variants","filter":{"top_variants":3}}` |
| particular variants | `{"view":"variants","variants":[1,3]}` (numbers as the page shows them) |
| a finding's items | `{"view":"items","finding":"without Approve payment"}` (part of its Overview title) |
| only claims over $10k | `{"view":"items","filter":{"attribute_between":[{"attribute":"Claim Amount (USD)","at_least":10000,"at_most":null}]}}` |
| one group | `{"view":"process","filter":{"attribute_equals":[["Region","South"]]}}` |
| the slow ones | `{"view":"items","filter":{"min_duration":<micros>}}` (a threshold from a query) |
| with or without a step | `{"view":"items","filter":{"with_activities":["Senior review"]}}` / `without_activities` |
| compare two groups | `{"view":"compare","compare":{"attribute":"Region","left":"South","right":"North"}}` |
| clear filters | `{"view":"process","filter":{}}` |
| a page | `{"view":"conformance"}` |

`filter` takes the same form as a finding's filter and replaces the page's
current one. Without `filter` the page keeps what is on, as a click would; when
the new ask is not about the earlier narrowing, add `"filter":{}` so it comes off; the Overview always shows the whole project, so put filters on
another view. Then say in one line what the page now shows, and the short
answer.

### Exports and checks

When the person wants the files (the process map as BPMN, the items or routes as
a spreadsheet, "export the main flow"), run:

```sh
glasstarn workspace export <project-id> --workspace-dir <project-root> --json
```

It writes the process map as BPMN and the items and variants as CSV, for the
project's current filters, into the project's `exports` folder, and returns the
file paths: tell the person where they are. For a project with linked records, add
`--type <item type>` (for example `--type invoices`); without it the error lists
the types. The type the page follows is used when it has one.

When something seems wrong (glasstarn not found, the page not opening or looking
old, a project that cannot be saved), run `glasstarn doctor` and follow the fix it
gives for each failed line.

## Scope limit

Preparation currently supports CSV event construction and many-to-one lookup
enrichment. It does not directly connect to an ERP, execute arbitrary scripts,
perform arbitrary many-to-many joins or aggregation, or invent missing history. Connected relational construction is available through
`workspace prepare-ocel` under the separate bounded contract above. Explain the exact additional
extract, key, definition or capability required when the supplied material is
outside this contract.
