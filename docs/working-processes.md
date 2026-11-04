# Working process understanding

An investigation can combine recorded steps with documentary expectations,
reported manual work and proposed changes. This supplements the measured map;
it never adds events or estimated durations to the log.

The user continues the conversation in their approved agent. They can move
between Understand, Improve and Check within the same project. Saved answers and working processes update across browser views while the
page is visible.
There is no separate browser chat.

## Save a working model

Use `glasstarn workspace record <project> --request <request.json>`. A record can
include `intention` (`understand`, `improve`, `check`) and `process_model`:

```json
{
  "version": 1,
  "question": "What happens outside the recorded system?",
  "intention": "understand",
  "answer": "The SOP requires an email review; its timing is unknown.",
  "process_model": {
    "version": 1,
    "title": "Invoice handling including manual review",
    "view": "combined_current",
    "sources": [
      {"id":"sop","kind":"document","title":"Invoice SOP","revision":"v2","reference":"Company-supplied SOP"}
    ],
    "steps": [
      {"id":"issue","label":"Issue invoice","evidence":[{"kind":"observed","query":"exports/query-123.json"}]},
      {"id":"review","label":"Review documents","owner":"Finance","evidence":[{"kind":"documented","source":"sop","section":"Section 3"}]}
    ],
    "connections": [
      {"from":"issue","to":"review","evidence":[{"kind":"documented","source":"sop","section":"Section 3"}]}
    ],
    "open_questions": ["How long does the manual review take?"]
  }
}
```

Replace the example query path with an actual successful query artifact from
this project and its current settings. A `map` query supplies recorded nodes
and edges. Native connected data may use `follow` with `detail: "full"` for
recorded step anchors. An object-history adjacency cannot be cited as a business
process connection. Every recorded claim must match the exact label/connection
in its cited query; reference validation does not establish the rest of an
agent's interpretation as true.

The model accepts 1–40 steps, at most 100 connections and 20 sources. Step/source
IDs use letters, digits, underscores and hyphens. Each step and connection needs
1–8 evidence entries:

| Kind | Fields | Meaning |
|---|---|---|
| `observed` | `query` | Recorded anchor validated against a saved map/full-follow query |
| `documented` | `source`, `section` | Requirement in a declared document |
| `reported` | `source` | Practice attributed to a declared `reported_practice` source |
| `proposed` | `rationale` | Explicit proposal or interpretation |

A source records `id`, `kind` (`document` or `reported_practice`), `title`,
`revision` and `reference`. Use an explicit unknown revision when unavailable.
An employee account's reference identifies who supplied it and its context.
This command stores bounded structured claims and references; it does not parse
or upload full documents. Material supplied to an approved agent does not require
another Glasstarn document-upload gate. Existing company restrictions and event
exposure controls still apply.

Connections reference step IDs with `from` and `to`, optional `condition`, and
their own evidence. Array order does not establish sequence. The
working model is not an approved control baseline or executable process. No timing/count/cost fields are accepted
in these authored steps; measured results remain in the engine query evidence.

## Advice stays in the conversation

The project and the browser hold facts: what glasstarn measured and what
documents or people reported. Advice about what to change (a future process,
an improvement plan, a step's treatment) is given by the agent in the
conversation. `workspace record` refuses `view: "proposed_future"`, an
`improvement` plan and step `assessment`s.

## Revision

Follow-up records retain the model and intention unless explicitly replaced.
A replacement model records `supersedes` automatically; omit that field or supply
the exact preceding model artifact. Earlier records remain immutable. A retained
model carries `process_model_origin` and its original revision time. Use
`clear_process_model: true` to withdraw it; this is mutually exclusive with a
replacement model. Changed project settings do not inherit an older model.
`workspace show` lists earlier investigation artifacts for resumption/review.

A browser session can remain open while CLI queries and records append under the
existing artifact lock. Neither source settings nor access permissions change.
The read-only MCP surface stays read-only.

## Boundaries

The browser displays the current working graph in Process, including
document-only models. It preserves revisions on disk; it does not provide
a complete revision-comparison interface, automatic SOP parsing, a reference
process catalog or automatic document ingestion. A working model must not silently
become an approved Check baseline. BPMN export is optional; the existing BPMN
button exports the measured map, not this working model.


## Browser updates

Document-only models can be reviewed in the browser without an event analysis;
measured views stay unavailable until data is analyzed. Saved records are
checked every five seconds across browser views while the page is visible.
A conversational answer must be published through `workspace record` to appear;
the browser does not read the chat transcript or execute model-written markup.

Linked-record milestone findings appear in Overview with their exact endpoints
and links to the affected records. They can span intermediate work and do not
claim direct adjacency. There is no separate Stage progression page. Missing
next events can reflect work in progress or incomplete coverage, rather than
abandonment.
