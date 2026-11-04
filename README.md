# glasstarn

See how your work really flows.

glasstarn is free, open-source process mining that runs on your own computer.
Put an export from your system in a folder, open your AI agent there (Claude
Code, Codex, Cursor, Antigravity or any agent app that can run commands) and
ask your question in plain words. Your agent runs the glasstarn engine, explains
what it found and opens the results in your browser. Keep talking, and that one
page follows the conversation by itself: new answers appear on it, and "show me
the three main variants" or "only claims over $10,000" moves it there with the
filter on.

Your files and every calculation stay on your computer. The only request glasstarn makes on its own is a once-a-day check of the latest version number on npm, which you can turn off (see [SECURITY.md](SECURITY.md)).

## Install

```sh
npx glasstarn setup
```

This installs the `glasstarn` command and connects it to Claude Code and Codex.
It needs Node.js 18 or newer. You can also ask your agent: "Install glasstarn
and connect it to this agent using npx glasstarn setup." Restart your agent
afterwards, then try a demo: "Set up the purchase to pay demo in glasstarn and
tell me what's slow."

Documentation for people using glasstarn is at
[glasstarn.com/docs](https://glasstarn.com/docs/start/install/).

## What it does

- Reads Excel workbooks, supported text tables, Parquet, XES, and XES.gz
  case-centric event logs. See [supported files](docs/data-and-projects.md).
- Reads OCEL 2.0 JSON, XML, and SQLite without flattening objects into cases.
- Discovers process maps, variants, bottlenecks, rework, and handovers.
- Filters complete cases by attributes, time, duration, variants, activities,
  and repeatable trace patterns.
- Discovers sound process trees and Petri nets with Inductive Miner, with Alpha
  and Heuristics miners available for specific uses.
- Checks explicit models using token replay or optimal alignments.
- Imports a supported, explicit subset of BPMN 2.0 for conformance checking.
- Compares cohorts and non-overlapping time periods without claiming causation.
- Highlights evidence worth investigating for automation while leaving the
  business judgment to a person or connected AI assistant.
- Links aggregates back to bounded example cases, source rows, events, and
  objects.
- Constructs a case-centric event log from multiple approved CSV extracts using
  saved event rules, checked many-to-one lookups and reconciliation evidence.

## Build from source

You need Rust 1.88 or newer.

```sh
git clone https://github.com/glasstarn/glasstarn.git
cd glasstarn
cargo build --release --bin glasstarn
./target/release/glasstarn
```

The application starts a service bound to `127.0.0.1`, chooses a free port,
and opens an authenticated URL in your normal browser. The UI and all of its
assets are embedded in the executable.

To try the included sequential example from the CLI:

```sh
./target/release/glasstarn analyze testdata/canonical/01-sequential.csv
```

To start without opening a browser:

```sh
./target/release/glasstarn serve --no-open
```

See [Getting started](docs/getting-started.md) for the complete first workflow.

## The event-log contract

A basic event log needs three fields:

| Field | Meaning |
|---|---|
| Case ID | Which process instance the event belongs to |
| Activity | What happened |
| Timestamp | When it happened |

End timestamp, event order, resource, lifecycle, cost, and case attributes are
optional. The browser profiles a bounded sample and proposes a mapping for you
to confirm. It pauses rather than guessing when a data-quality decision could
materially change the result.

Object-centric logs use the OCEL 2.0 event, object, type, and qualified
relationship model instead of a case ID. See
[Object-centric mining](docs/object-centric.md).

## AI-assisted investigation

The local MCP server lets an AI client query calculated summaries, variants,
bottlenecks, rework, comparisons, automation-targeting evidence, privacy-safe
case-centric model discovery and explicit-baseline conformance, and native
object-centric discovery and conformance results. Raw-log access is not an MCP
capability.

Before connecting a client, generate a policy and preview exactly what the
client will receive:

```sh
glasstarn mcp-policy agent-access.json
glasstarn mcp-preview orders.project.json \
  --exposure-policy agent-access.json
```

The documentation uses `glasstarn` for an installed executable. When running
the source build directly, use `./target/release/glasstarn` instead.

Agent access is off unless you explicitly start it with a reviewed exposure
policy. See [Agent access and privacy](docs/agent-access.md).

## Design boundaries

Glasstarn is deliberately a focused local investigation tool. It is
not a hosted multi-tenant platform, connector catalogue, dashboard builder,
task-mining recorder, or source-system automation service.

Important interpretation boundaries are part of the product:

- A directly-follows relation is observed order, not proof of causation.
- Start-to-start elapsed time is not called waiting time unless end timestamps
  make true waiting measurable.
- A frequent manual activity is an investigation target, not an automation
  verdict.
- Conformance is only as meaningful as the explicit baseline model supplied.
- A case-centric projection of object-centric data is disclosed as a
  projection, including convergence and divergence risks.

## Documentation

- [Getting started](docs/getting-started.md)
- [Architecture](docs/architecture.md)
- [Data, mapping, and saved projects](docs/data-and-projects.md)
- [Process-mining methods and limits](docs/process-mining.md)
- [Object-centric mining](docs/object-centric.md)
- [Agent access and privacy](docs/agent-access.md)
- [Harness-neutral headless workflow](docs/headless-agents.md)
- [Agent-led preparation recipes](docs/preparation-recipes.md)
- [Release benchmark procedure](packaging/RELEASE_REPORT.md)
- [Trademarks and compatibility](docs/trademarks.md)

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test and propose a
change, and [SECURITY.md](SECURITY.md) for how to report a vulnerability.

Glasstarn is built for you to use yourself. If your team would like a hand,
Tropical Glacier offers quick consulting or training. Get in touch at
[hello@glasstarn.com](mailto:hello@glasstarn.com).

## Correctness and performance

The workspace test suite combines hand-verified canonical logs, constructed
model oracles, invariants, regression tests, and end-to-end browser/server
tests. Benchmark commands exercise the same analysis paths used by the product
and write machine-identified reports with their raw results.

Run the correctness suite:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Generate a local release evidence report:

```sh
BENCHMARK_EVENTS=1000000 \
OCEL_BENCHMARK_EVENTS=100000 \
packaging/release-report.sh
```

Published performance figures should always be accompanied by the report that
identifies the exact commit, machine, toolchain, and workload.

## Trademarks

Glasstarn is independent and is not affiliated with or endorsed by SAP, Oracle,
Salesforce, ServiceNow, Microsoft or any other vendor whose data it reads.
Product names are trademarks of their owners and are used only to describe
compatibility; see [trademarks and compatibility](docs/trademarks.md).
