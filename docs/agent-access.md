# Agent access and privacy

Glasstarn exposes a local, read-only MCP server so an AI client can
investigate calculated results and bounded evidence. MCP is an analysis surface,
not raw-log access and not a source-system write-back channel.

## Connect from a project

1. Start Glasstarn and open a project, then choose **Connect an agent** at the
   top of the page.
2. **Choose what your agent can see**: **Hide names** (recommended) or **Show
   step and column names**, then choose **Allow access**.
3. **Add glasstarn to your agent**: pick Claude Code, Codex, Copilot Chat in VS
   Code or another MCP client, then copy the command or configuration shown.
   **Help and other options** has a configuration-file alternative.
4. **Paste this in your agent**: restart your agent, paste the starter message,
   then choose **Check connection**. The top button then reads **Agent
   connected**.

The check reports authenticated MCP handshake and tool discovery, along with
last contact time. It does not claim the client is continuously online or that
an AI conversation succeeded. Access can be enabled before an analysis is ready;
the panel tells you when a log still needs loading or its mapping confirmed.

The private preset aliases names and identifiers; it omits source information,
attribute values, absolute timestamps and source rows. Durations and aggregate
costs remain visible. The business-names preset additionally shows activity,
column, object-type and relationship names. The panel displays the exact active
policy, including an inherited policy supplied with `serve --agent-access`.
These rules apply to this MCP surface, not files separately given to an agent.

**Turn off access** revokes this project's connection immediately and persists
across restarts. Enable it again to grant a new credential. Enabling a project
does not enable newly created projects. A serve-time policy is an explicit
default for projects without their own saved choice.

### Reconnecting

Generated settings point to the project's `.agent-session.json` and pin its
identifier, rather than embedding a port or credential in client configuration.
After an application restart, reopen the project. The stdio bridge reloads the
session file for every request and follows the new local port. Retry the request
or restart the MCP server in the client if it stopped while the app was offline.
Moving the project folder or executable requires updating their configured paths.

The process must run on the same machine as the local service, with approved
filesystem and loopback access. A remote container, WSL instance or enterprise
sandbox does not automatically share the desktop's paths or localhost. An
administrator may need to provide an approved connection arrangement. Client
MCP policies still apply; this setup does not override them.

### Clients and verification

| Client | Setup supplied | Validation in this change |
|---|---|---|
| Codex | Add command and TOML entry | Official syntax and installed CLI configuration parser checked; live Codex tool invocation still to verify |
| Claude Code | Local-scope add command and project JSON alternative | Official syntax and installed CLI help checked; live Claude tool invocation still to verify |
| Copilot Chat in local VS Code | `.vscode/mcp.json` entry | Official configuration reference checked; VS Code was not installed for a live test |
| Other MCP clients | Command/argument object | Client-specific setup and compatibility need verification |

The compiled stdio bridge itself is integration-tested for handshake, tool
discovery, restart with the same running proxy, revocation/re-enabling, and
wrong-project rejection. A delayed-request-body regression also verifies
accepted-socket handling on the running local service. The browser flow is also exercised against that
bridge. This is not a claim of end-to-end testing in every named client.

Syntax references: [Codex MCP](https://developers.openai.com/codex/mcp),
[Claude Code MCP](https://code.claude.com/docs/en/mcp), and
[VS Code MCP configuration](https://code.visualstudio.com/docs/agents/reference/mcp-configuration).
The VS Code entry targets the local extension host; Agent Host and remote
sessions have different configuration locations.

Pi does not include native MCP support. Its documented alternatives are CLI
workflows or extensions; a Pi-specific integration is deferred. See the
[official Pi documentation](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/README.md#philosophy).
The existing `glasstarn` CLI remains available, but direct CLI/file access
does not acquire the MCP redaction guarantees automatically.

## Advanced connection modes

### Saved-project mode

```sh
glasstarn mcp \
  --project-dir /absolute/path/to/projects \
  --exposure-policy /absolute/path/to/agent-access.json
```

The server discovers valid project files only inside the selected directory.
Opaque handles prevent callers from supplying arbitrary paths. It reproduces
each project's fingerprinted source, mapping, decisions, preparation rules,
and saved active filter. Explicit case-centric `*.model.json` and
object-centric `*.ocelmodel.json` baselines in that directory are also exposed
through separate opaque model handles; agents cannot submit arbitrary paths or
inline models.

### Live browser mode

Start the application with explicit agent access:

```sh
glasstarn serve --agent-access /absolute/path/to/agent-access.json
```

Create or open a project in the browser. Use the port in **that project's**
browser address and the identifier at the end of its displayed folder path.
The initial project-list port is not an analysis session. Configure the MCP
process with the same reviewed policy:

```sh
glasstarn mcp \
  --live-port <project-port> \
  --project-id <project-folder-id> \
  --exposure-policy /absolute/path/to/agent-access.json
```

The MCP process delegates to the browser owner and sees the exact in-memory
source, confirmed mapping, decisions, rules, active case or object-centric
filter, and selected case-centric or object-centric baseline. Agent access is disabled when the
browser was not started with a policy. Closing a browser tab does not end the session. Restarting the application
requires reopening the project and updating its connection port **when using
this explicit-port mode**. The generated connection-file setup above avoids
that port update. Credentials
are derived from both the policy and the project identifier: an old connection
cannot authenticate to a different project if its port is reused. Changing the
policy key revokes credentials derived from the previous key.

Saved-project mode above continues to read legacy project files; it does not
discover `workspace.json` investigation folders. Use project-scoped live mode
to query a folder through MCP. Creating, importing and analyzing an investigation
folder without a browser uses the direct Rust CLI workflow described in
[Headless agents](headless-agents.md); that path does not require MCP or a live
browser session.

Case-centric model discovery uses the exact live case filter. In the browser,
choose a case-centric `.model.json` baseline to enable live token replay and
optimal alignment through MCP. `list_process_models` exposes only that selected
baseline through an opaque handle; it reports when none is selected. Rejected
model uploads preserve the last accepted baseline. Loading a different event log
clears the selection, so a baseline is never silently inherited by another log.

## Review the boundary first

Generate a conservative policy template:

```sh
glasstarn mcp-policy agent-access.json
```

Then preview the exact outbound representation of a project:

```sh
glasstarn mcp-preview orders.project.json \
  --exposure-policy agent-access.json
```

Each category can be shown, replaced by a stable keyed alias, or omitted. The
policy independently covers project names, source information, schema names,
activities, resources, cases, attributes and values, timestamps, costs, source
rows, and object-centric identifiers.

The alias key stays only in the local policy file and is never returned through
MCP. A browser launch token cannot call MCP endpoints, and an agent credential
cannot call browser APIs.

## Tool behavior

The server provides discovery and bounded tools for:

- process summary, variants, bottlenecks, and rework;
- case, transition, variant, and handover evidence;
- private case inspection;
- cohort and period comparisons;
- automation-targeting evidence and activity context;
- case-centric Inductive, Alpha, and Heuristics discovery;
- case-centric token replay and optimal alignment against an explicit local
  baseline;
- native object-centric analysis, lifecycle evidence, projection, timing,
  synchronization, bottlenecks, discovery, and conformance.

Detailed tools enforce the exposure policy independently. An omitted label does
not cause hidden activities to be reconnected or shortened; unavailable fields
remain null or make the corresponding drill-down unavailable. Evidence lookup
is limited to the active view.

Answers are held to about 4 KB of compact JSON by default, so an agent's
token cost does not grow with the data. The longest lists are cut to their
first items, in the engine's order, with a `<name>_omitted` count beside each
and a `shortened` note; totals and figures are never changed, and the
identifiers needed for the next call (`catalog`) are kept whole. Pass
`"detail": "full"` to any analysis tool for the whole answer. Listing tools,
the exposure policy and the claim contract are always returned whole.

Formal discovery is unavailable when activity names are omitted, because
removing visible transitions would change the model's meaning. Aggregate
conformance remains available under omission, while case, activity, model
source, and source-row details are independently shown, aliased, or omitted.
Discovery output is privacy-transformed analysis output and is deliberately not
accepted back as a conformance baseline.

## Claims and interpretation

Engine results identify calculated facts. Defined comparisons identify
statistical associations and carry non-causation limits. The discoverable claim
contract instructs an AI assistant to label its own interpretations and
improvement hypotheses, cite the source tool and evidence, state uncertainty,
and propose validation steps.

The application intentionally does not ask a model to convert observed manual
work directly into an automation verdict.

## Local connection files

`.agent-access.json` stores the project consent and policy; `.agent-session.json`
contains the current loopback connection credential. Both are atomically written
and owner-only on Unix. Windows uses the project folder's inherited ACLs. Keep
these files private and exclude them from shared examples, repositories or
project exports. They are not analysis evidence. The setup responses and copied
client configurations contain neither the alias key nor the bearer credential.
