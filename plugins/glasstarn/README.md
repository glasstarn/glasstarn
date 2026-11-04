# Glasstarn agent plugin

The shared skill makes an approved agent harness the primary browser-free
investigation interface. Codex and Claude Code manifests are installation
adapters around this harness-neutral workflow. It can inventory approved CSV extracts, propose a reviewable
preparation recipe, run the deterministic engine and continue asking questions
from saved analysis and evidence. Full datasets and process calculations stay
in the installed `glasstarn` executable rather than model context.

The shared skill is `glasstarn`. In Claude Code, load this folder
for local testing:

```sh
claude --plugin-dir ./plugins/glasstarn
```

Then type `/glasstarn`, or just ask a question. In Codex, type `$glasstarn`. Codex discovers the
skill through the plugin manifest when this repository plugin is installed or
loaded. The executable must be installed on `PATH`, or built from this
repository at `./target/release/glasstarn`.

The plugin creates and updates investigation folders only when the user asks it
to run a process-mining workflow. It does not contain or connect to an ERP
system, upload data to a hosted Glasstarn service, or bypass the
agent host's filesystem permissions.

Other harnesses with command/file access can load the same skill folder and
references, then invoke the same CLI. No client-specific extension or MCP server
is required for headless operation. See [the shared workflow documentation](../../docs/headless-agents.md).
Dedicated native-client validation remains outstanding; shared CLI compatibility
is not a claim of tested installation in every harness.
