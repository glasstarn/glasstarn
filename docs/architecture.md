# Architecture

Glasstarn ships as one Rust executable with four user-facing modes:

```text
browser UI ─────┐
CLI / agent skill ─┼─> shared query layer ─> validation / normalization / engine
MCP ─────────────┤
saved project ───┘
```

The UI, CLI, and MCP server do not maintain separate analytical
implementations. They resolve inputs and policies, call the shared query layer,
and present its typed results.

## Workspace crates

| Crate | Responsibility |
|---|---|
| `pm-core` | Compact logs, filtering, aggregations, discovery, replay, alignments, and native object-centric algorithms |
| `pm-ingest` | File readers, profiling, mapping, validation, interpretation policy, normalization, and BPMN import |
| `pm-query` | Label-resolved product queries, evidence, comparisons, briefs, and result caching |
| `pm-project` | Reproducible project format, source fingerprints, atomic saves, and writer locking |
| `pm-server` | Authenticated loopback HTTP service and embedded browser assets |
| `pm-mcp` | Read-only, privacy-gated MCP tools over saved or live application state |
| `pm-cli` | The `glasstarn` executable and all command-line presentation |
| `pm-testgen` | Constructed process models and deterministic test-log generation |
| `pm-bench` | Product-path case-centric and object-centric benchmarks |

## Case-centric analysis path

1. Read and profile a bounded sample.
2. Confirm or provide a mapping.
3. Validate the complete source.
4. Pause for any material interpretation decisions.
5. Apply recorded preparation rules.
6. Normalize and deterministically order events into case traces.
7. Select complete cases for the active view.
8. Calculate maps, variants, timing, rework, organization, comparisons, and
   evidence from that same view.

Filters select complete cases. Preparation rules alter the analytical log
without changing the source file. Keeping that distinction explicit prevents a
filter from inventing a transition by deleting an event in the middle of a
trace.

## Object-centric analysis path

OCEL ingestion preserves events, objects, types, attributes, and qualified
relationships as separate compact structures. Native lifecycle, interaction,
synchronization, discovery, and conformance algorithms run without first
selecting a case notion. A case projection is a separate, explicit query with
distortion reporting.

## Runtime modes

### Browser

`glasstarn` or `glasstarn serve` starts an HTTP service bound to
`127.0.0.1`. Each launch generates a 256-bit bearer token embedded in the URL
fragment. API requests require that token and valid loopback Host/Origin
headers. The initial session manages a project directory. Each opened project
gets an independent loopback session, browser token and immutable project
identity. Its folder is locked for that session's lifetime. Switching tabs or
projects does not retarget an existing agent connection.

Uploads first stream to temporary storage. Accepted browser changes save
content-addressed source/model copies followed by an atomic `workspace.json`
manifest update. Relative asset paths make the complete folder movable while
closed. Reopening verifies fingerprints and rejects symlinked assets. Earlier
asset copies remain on disk; automatic retention cleanup is not implemented.
The manager caps simultaneous project sessions at 64; restart to release them.
Requests remain serialized within each project; different projects have
independent request loops. This is a local application, not a multi-user server.

### CLI

Commands execute the same query layer directly. Human-readable output is the
default; `--json` exposes stable structured results. `--brief` emits an
evidence-oriented Markdown handoff for an AI assistant.

The `workspace` command group is the persistent, browser-free application
surface. It creates and lists investigation folders, inspects ready-made case or
OCEL logs, imports an explicitly confirmed mapping, records interpretation
decisions, runs analysis and atomically saves the latest structured result. Each
command connects to the Rust engine in-process and exits, so no service or MCP
session is required. Codex and Claude Code share one packaged skill that
orchestrates these commands; the skill contains workflow guidance, not a second
analytical implementation.

### MCP

The normal MCP mode opens only explicitly scoped local project files and OCEL
files. Optional live delegation forwards MCP JSON-RPC to the running browser
owner, so the agent sees the exact confirmed mapping, decisions, rules, and
active view. Browser and agent credentials are separate.

## Reproducibility and caching

Legacy `.project.json` files record settings and a source fingerprint, not a
copy of the data. New investigation folders also retain source/model copies;
`workspace.json` records the active selection and settings. The two formats
remain distinct; legacy exports can be imported into an opened project.
Project writes use a deterministic single-writer lock and atomic replacement.
Disposable cache entries are content-addressed by every analytical input,
including source identity, mapping, decisions, rules, filter, analysis kind,
format, and engine version. A missing or damaged cache is a safe miss; the
source of truth remains the project plus the fingerprinted event log.

## Determinism

Stable public labels break analytical ties wherever possible. Event ordering
uses explicit event order, then timestamp, lifecycle, and source row under the
confirmed interpretation. Result ordering, bounded evidence selection, model
construction, replay, and equal-cost alignment choices are deterministic.
