# Headless agent workflow

Glasstarn's primary workflow runs inside an approved agent harness
without opening a browser. The agent invokes the local
`glasstarn` executable, which calls the same Rust query layer as the browser.
The model guides the workflow; it does not receive the full file or calculate
the process map itself.

The headless workflow supports ready-made case-centric Excel workbooks,
supported text tables, Parquet, XES and XES.gz logs plus native OCEL JSON, XML
and SQLite. Its preparation workflow
can also inventory approved CSV extracts, construct events from multiple input
tables, apply validated many-to-one enrichment, reconcile the result and run
the prepared log directly. Direct ERP extraction and arbitrary joins remain
subsequent work.

## The guided investigation

The agent starts with the process and the question: what would you like to
understand? A question such as "Why do invoices take so long?" already supplies
that context. If you provide files, it inspects them before asking for information
they may already contain. If you do not have files, it helps identify the needed
history and write a request for your reporting or systems team.

There is no compulsory industry/system questionnaire. The agent asks about
business definitions, source systems, dates or teams when they affect the next
step. Audit guidance applies when you ask to check an expected process or rule;
an ordinary improvement investigation does not require an ideal-process model.

Once the meaning is clear, Rust processes the records and calculates the
results. The agent explains the answer, supporting evidence, limits and useful
follow-up checks. The project retains the source and findings automatically as
the investigation progresses. You can continue asking questions or reopen the
project later without uploading those records again. The optional browser
reviews the same saved findings and process flow.

## Shared workflow, client adapters

The integration contract is the `glasstarn` CLI, its JSON responses and the
project folder. Any harness with approved command and file access can use it.
The packaged [glasstarn skill](../plugins/glasstarn/skills/glasstarn/SKILL.md)
is shared workflow guidance, including
[improvement](../plugins/glasstarn/skills/glasstarn/references/improvement.md),
[audit](../plugins/glasstarn/skills/glasstarn/references/audit.md), and
[query/record schemas](../plugins/glasstarn/skills/glasstarn/references/investigation-commands.md).

A harness such as Pi can load that skill folder through its supported skill
mechanism, or receive the same instructions explicitly, then invoke the CLI.
Preserve its reference files. A dedicated client extension is not required for
engine operation. This is interface compatibility; it is not a claim that every
client has passed a native end-to-end test.

Codex and Claude plugin manifests only adapt discovery and installation. They
contain no mining algorithms, client credentials or separate project format.
MCP provides the additional read-only query surface for clients that support it;
the CLI remains the browser-free path for project creation and preparation.

## Install or load the agent plugin

The dual-client plugin is in `plugins/glasstarn`. Its shared
`glasstarn` skill contains the project selection, mapping,
data-quality, analysis, evidence and recovery workflow.

For local Claude Code testing from this repository:

```sh
claude --plugin-dir ./plugins/glasstarn
```

Invoke `/glasstarn` in that session. A company
can distribute the same folder through its managed Claude plugin marketplace.

For Codex, add the included marketplace and install its plugin:

```sh
codex plugin marketplace add /absolute/path/to/glasstarn/plugins
codex plugin add glasstarn@glasstarn
```

An enterprise can provision the same marketplace centrally. The plugin does not
bundle the engine: install `glasstarn` on `PATH`, or build the repository with
`cargo build --release --bin glasstarn` and use
`./target/release/glasstarn`.

## Corporate installation and data handling

Use an existing company-approved engine and skill when they are provisioned.
IT can place the executable and shared skill in the approved agent environment;
that may be the employee's computer or a company sandbox. Keep the references
with the skill and use an approved project directory. Employees then use the
same guided investigation without a separate enterprise edition.

The npm setup command installs the matching engine and shared skill:

```sh
npx glasstarn setup
```

It registers the skill with installed Codex and Claude Code clients. Restart
your agent afterwards. Other command-capable agents can load the shared skill
through their own supported mechanism. For internal or offline provisioning,
use an approved release package; see [npm packaging](../packaging/npm/README.md).
Managed installations should use IT-approved versions and update sources.

The Rust engine processes full logs on its execution host. The agent can receive
bounded samples, schemas and calculated evidence, which are subject to the
chosen agent's model-processing policies. Approval to use an agent does not
establish approval for every dataset. Offline installation does not mean offline
model processing. Follow company access, storage and retention policies.

For npm installations, `glasstarn update` installs the latest release and
`glasstarn uninstall` removes the program and its Codex/Claude Code setup.
Uninstall keeps saved investigations and original exports. Remove project
folders separately only when they are no longer needed.

## The project lifecycle

To try improvement, audit, and incomplete-history recovery without company
data, use the [fictional invoice investigation](../examples/invoice-review/README.md).
It includes agent prompts, an explicit expected model, and independently
checkable results. The included object-centric fixture provides a separate
native OCEL test.

The commands below emit structured JSON. `--workspace-dir` is optional; omit it
to use the user-owned default under `~/.glasstarn/projects`.

```sh
# Create an isolated investigation folder
glasstarn workspace create \
  --name "Order-to-cash review" \
  --purpose improvement \
  --json

# Find and inspect existing projects
glasstarn workspace list --json
glasstarn workspace show <project-id> --json

# Inspect the extract before accepting a mapping
glasstarn workspace inspect orders.csv --json

# Import, analyze and save a case log
glasstarn workspace import <project-id> orders.csv \
  --case case_id \
  --activity activity \
  --timestamp timestamp \
  --resource user_id \
  --case-attribute company_code \
  --json

# Reproduce the saved analysis later
glasstarn workspace analyze <project-id> --json
```

When the supplied files are extracts rather than a ready-made event log:

```sh
glasstarn workspace inventory <project-id> \
  order_history.csv suppliers.csv --json
glasstarn workspace prepare <project-id> --recipe recipe.json --json
```

The agent proposes the recipe from the inventory; the employee confirms case,
activity, timestamp, event-construction and join meaning. The engine blocks
duplicate lookup keys, unapproved unmatched joins and missing required event
fields. See [Preparation recipes](preparation-recipes.md) for the complete
contract.

For native OCEL, omit case-column mapping:

```sh
glasstarn workspace inspect orders.jsonocel --json
glasstarn workspace import <project-id> orders.jsonocel --json
```

The profiler proposes a mapping. The employee confirms material business
meaning. If analysis returns exit status `3`, stdout contains a `blocked` JSON
result with each open interpretation decision, evidence and allowed option.
Rerun import with the confirmed choices, for example:

```sh
glasstarn workspace import <project-id> orders.csv \
  --case case_id --activity activity --timestamp timestamp \
  --answer blank_timestamps=missing_data \
  --json
```

The source is copied into `sources/` under its SHA-256 identity. Mapping,
interpretation policy, preparation rules and active filters live in
`workspace.json`. The latest structured result is written atomically to
`exports/latest-analysis.json`. That lets a later agent session reproduce the
analysis after the original extract has moved or been deleted.

## Follow-up questions and durable investigation context

`workspace query <project> --request <query.json>` runs a targeted engine query
using the saved source, mapping, decisions, rules and filter. Case evidence,
cohorts, the readable process map, discovery, formal conformance, native object
lifecycles and object bottlenecks are available through the shared query
implementations. The `map` query draws the most common routes
(`{"type":"map","selection":{"mode":"routes","variants":[1,2]}}`) or the most
frequent steps and connections
(`{"mode":"limits","activities":8,"connections":14}`), and reports what the
map leaves out and the share of cases it covers. The
`readiness` query reports one-event versus multi-event case coverage for the
agent to clarify possible current-state snapshots. A supplied conformance
baseline is retained only after a successful check and can be reused later.

`workspace record <project> --request <record.json>` appends a question,
assumptions, missing data and findings. Observations require current-project
query evidence; hypotheses are separately labelled. Each record embeds that
evidence and the saved scope. Old records survive source/settings changes, but
the engine rejects old references when saving new observations. `workspace show`
returns the record paths, so another session or harness can continue the work.
Each finding may include a short `title` (1–160 characters) for the browser
briefing and slide heading, alongside its full `statement`, `kind` and
`evidence`. Existing records without titles remain readable.
A record can also include up to six `visuals`, each with `title`, `evidence`
(a current `exports/query-<id>.json` reference), `kind` (`bar` or `table`) and
`metric` (`cases`, `total`, `mean`, `median` or `p90`). Grouped `values` queries
supply numeric distributions; `cohorts` supplies counts and elapsed-time means
or medians. The engine derives all chart rows from the saved calculation and
rejects unsupported or unmeasurable metrics. Agent-supplied chart numbers, HTML
and scripts are not accepted. The browser refreshes saved answers while an
investigation is open, shows query citations and links to example item timelines.
Visuals appear in Overview and Process → Saved views. Each visual uses a supported
bar or table format; publishing does not create arbitrary browser tabs.
MCP access remains read-only; publishing uses this local workspace command.

`workspace show` also returns `saved_views`: bounded measurement definitions saved
from the map, with the original source/settings identity and filters. These are
queries to reproduce, not stored chart numbers. Browser restoration recalculates
them and refuses incompatible source/settings changes.

An agent with approved file and code tools may also produce a separate PDF or
other artifact from query results. That is a capability of the host agent, not
of the read-only MCP server. Data access remains governed by exposure settings.
Exploration returns histograms, mean, median and P90; standard deviation and
arbitrary per-activity chart publishing are not currently part of its contract.
Elapsed intervals must not be described as working time unless the source
contains the required activity start/finish observations.

The agent remains responsible for whether its narrative follows from the cited
results; reference validation does not establish the narrative's truth.

See the packaged [command contract](../plugins/glasstarn/skills/glasstarn/references/investigation-commands.md)
for request schemas and examples. Audits require an explicit expected model;
model fit alone is not audit assurance.

## Engine connection and browser handoff

No background service is required for the headless path. Each workspace command
connects to the Rust engine in-process, performs the operation, saves the
project, and exits. This fits enterprise agent sandboxes because the existing
agent's command and filesystem controls govern every invocation.

The project folder is created on the machine where the executable runs. A local
agent therefore saves on the employee's PC; a remote or sandboxed agent saves in
that environment unless an approved shared path is supplied. Every create/show
response includes the absolute folder location.

To use the visual view later, start the same executable against the same project
root:

```sh
glasstarn serve --no-open --workspace-dir /absolute/path/to/projects --project <project-id>
```

Open the printed authenticated URL. It opens the requested investigation
directly, with its saved findings, process flow and optional slide briefing. Its retained
source and settings open without another upload. Keep the service running
while reviewing results; the saved project remains when it stops. Continue
follow-up questions in the same agent conversation. Source, models, access and settings have one writer at a time. While the browser
owns those settings, `workspace query` and `workspace record` can read a pinned
snapshot and append uniquely named evidence and answers under a separate
artifact lock. They cannot replace artifacts or change settings, sources or
permissions. A settings change during a query/record rejects that operation;
older evidence cannot support a new answer after its scope changes.

The browser's project-scoped MCP connection remains available for interactive
read-only investigation. It is separate from this direct CLI workflow: the
headless skill does not need browser consent or a live port to create, import,
analyze, and save a local project.
