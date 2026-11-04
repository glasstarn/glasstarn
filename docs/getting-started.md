# Getting started

## 1. Build and launch

Install Rust 1.88 or newer, then run:

```sh
cargo build --release --bin glasstarn
./target/release/glasstarn
```

The executable listens only on loopback and opens an authenticated URL in your
normal browser. No separate Node.js, Python, database, or web server is needed.

If automatic browser opening is not useful:

```sh
./target/release/glasstarn serve --no-open
```

Open the printed URL while that command remains running.

The remaining examples use `glasstarn` for an installed executable. Replace
it with `./target/release/glasstarn` when using the source build directly.

## 2. Create a project and import an event history

On **Your projects**, expand **New project**, enter a name and choose **Audit**
or **Process improvement**. Create the project, then select **Import file**.
The purpose is a project label, not an assertion of compliance.

The browser’s default project location is `~/.glasstarn/projects` (under the
user profile on Windows). Use `serve --workspace-dir /absolute/path/to/projects`
to choose another location. When an agent starts a project beside your exports,
it normally uses a `glasstarn` folder in that working folder.

Select **Open →** next to a saved project to return to it. **All projects**
returns to the list. Each project keeps its own source, settings, findings,
expected process and optional agent connection. Closing the tab does not stop
its local session immediately; an idle service may stop later. Ask your agent
for a fresh link when you come back.

Changes save automatically. If saving fails, resolve the visible error and use
**Save project now** under **Current project settings** when offered. The saved
folder includes the imported source and any expected-process model. Mapping,
data-quality decisions, preparation rules and filters are restored on reopen.
The analysis is recalculated from the saved source.

Import an Excel workbook, a supported text table, Parquet, XES/XES.gz or OCEL 2.0
file. See [supported files](data-and-projects.md). The app saves a fingerprinted
copy locally. Agent investigations can also prepare several exported tables
into a case log or linked-record log, with saved checks and query evidence.

For a first test, choose:

```text
testdata/canonical/03-long-loop.csv
```

The file contains three claims, including two that request documents and loop
back through review.

## 3. Confirm the mapping

For a case-centric file, confirm which columns represent:

- Case ID
- Activity
- Timestamp

Optional mappings add better evidence:

- End timestamp enables true waiting and processing measurements.
- Event order resolves records whose timestamps are equal or absent.
- Resource enables handover analysis and human/system classification.
- Lifecycle distinguishes stages such as start and complete.
- Cost is stored exactly in fixed-point microunits.
- Other selected columns can become case attributes for filtering and
  comparison.

The profiler proposes a mapping, but you remain responsible for confirming it.

## 4. Resolve material data-quality questions

Analysis pauses when different interpretations could change event order or
case composition. Each decision shows its evidence and supported options. The
chosen interpretation becomes part of a saved project.

Warnings that do not prevent safe analysis remain visible as limitations.

## 5. Investigate the result

The initial result includes:

- a frequency/performance process map;
- process summary and view coverage;
- ranked variants;
- bottleneck and rework evidence;
- case lookup and source-row-linked timelines;
- cohort and period comparisons when the data supports them.

Select a node, transition, variant, or example case to move from an aggregate
to its evidence.

## 6. Narrow complete cases

Select **+ Filter** on Process, Variants, Measure, Conformance or Items.
Filters keep or remove whole cases; they never trim events out of a retained
case. Conditions combine with AND.

Trace controls support:

- first recorded activity;
- last recorded activity;
- one or more directly-follows pairs;
- one or more eventually-follows pairs.

Recorded order is deterministic, including timestamp ties, but it does not
prove that one activity caused another. Every filtered result states how many
cases and what percentage of the original view remain.

## 7. Come back to the project

Use **All projects** and select the project’s **Open →** button, or ask your
agent to continue the project. The saved source copy means you do not need to
select the original file again. To work on another computer, copy the whole
project folder and keep its contents together.

The lower-level CLI can also write a standalone settings JSON with
`--save-project`. That file needs its matching original source; it is a
different format from the browser’s complete project folder. See
[saved project contents](data-and-projects.md#saved-project-contents).

## CLI equivalents

```sh
# Inspect columns before analysis
glasstarn inspect orders.csv

# Analyze with proposed mapping
glasstarn analyze orders.csv

# Emit complete machine-readable output
glasstarn analyze orders.csv --json

# Retain complete cases matching a trace pattern
glasstarn analyze claims.csv \
  --starts-with Submit \
  --directly-follows Submit Review \
  --eventually-follows "Request Documents" Approve \
  --ends-with Approve

# Save and reproduce settings
glasstarn analyze orders.csv --save-project orders.project.json
glasstarn analyze --project orders.project.json
```

Run `glasstarn help` for the complete option reference.

## Run the same project entirely through an agent

Codex or Claude Code can create a project folder, inspect a ready-made log,
confirm its mapping, import it, run the Rust analysis and save the result without
starting the browser. The repository includes one shared skill with Codex and
Claude plugin manifests.

```sh
glasstarn workspace create --name "Order review" --purpose improvement --json
glasstarn workspace inspect orders.csv --json
glasstarn workspace import <project-id> orders.csv \
  --case case_id --activity activity --timestamp timestamp --json
glasstarn workspace analyze <project-id> --json
```

See [Headless agent workflow](headless-agents.md) for plugin setup, quality
decisions, project storage and optional browser handoff.

For multiple company-approved CSV extracts, use `workspace inventory` followed
by a reviewed `workspace prepare` recipe. See
[Preparation recipes](preparation-recipes.md).

## Connect an approved agent

Select **Connect an agent** at the top of the page. Review the information shared,
select **Allow access**, choose your client and copy its setup. Start the MCP
server in the client and use **Check connection** to confirm observed handshake
and tool discovery. See [Agent access](agent-access.md) for client instructions,
reconnection, verification limits and enterprise sandbox requirements.
