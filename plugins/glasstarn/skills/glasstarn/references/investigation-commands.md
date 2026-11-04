# Shared investigation command contract

All harnesses use the same CLI and JSON. No running browser or MCP server is
required. Commands accept a project ID or absolute folder path and optional
`--workspace-dir`. Relative paths inside request JSON are relative to the command's
working directory. Prefer absolute input paths.

## Follow-up queries

Write one JSON request and run:

```sh
glasstarn workspace query <project> --request <query.json> --json
```

| Data | Request |
|---|---|
| Case log | `{"type":"readiness"}` |
| Case log | `{"type":"case","id":"INV-123"}` |
| Case log | `{"type":"activity","activity":"Approve","limit":10}` |
| Case log | `{"type":"transition","from":"Review","to":"Approve","limit":10}` |
| Case log | `{"type":"cohorts","attribute":"region","left":"North","right":"South"}` |
| Case log | `{"type":"map","selection":{"mode":"routes","variants":[1,2]}}` |
| Case log | `{"type":"map","selection":{"mode":"limits","activities":8,"connections":14}}` |
| Either | `{"type":"discover"}` |
| Either | `{"type":"conformance","model":"/approved/baseline.json","algorithm":"token","limit":10}` |
| Native OCEL | `{"type":"object","id":"invoice-123"}` |
| Native OCEL | `{"type":"bottlenecks"}` |
| Case log | `{"type":"findings"}` ranked calculated findings, largest effect first |
| Native OCEL | `{"type":"findings","object_type":"invoice"}` the same for one item type, plus journey drop-outs |
| Native OCEL | `{"type":"follow","object_type":"order"}` |
| Case log | `{"type":"values"}` lists the fields that hold numbers |
| Case log | `{"type":"values","field":"amount","at_least":50000,"grouping":{"by":"last_step"}}` |
| Native OCEL | `{"type":"values","object_type":"invoice","field":"invoice_amount","grouping":{"by":"variant"}}` |

Limits are 1–100. Activity/transition results include total matching cases,
bounded case IDs and a truncation flag; fetch specific case timelines as needed.
Every query respects saved mapping, policy, rules and active filters. The result
is saved as `exports/query-<id>.json`. Stdout contains `artifact` and `response`;
`response` contains `request`, `scope` (saved state) and the shared engine `result`.
Queries do not replace `exports/latest-analysis.json`.

Conformance accepts `token` or `alignment`. Omit `model` to reuse the retained
baseline. The supplied model is content-addressed and committed as the selected
baseline only after a successful check. Discovery returns an observed model in
`response.result`; write that object to a model JSON file when needed, but do not
silently treat it as the approved process. Empty conformance views are rejected.
Exit status 3 returns a quality block requiring the documented interpretation
decisions; other errors do not produce a successful query snapshot.
Case baselines are limited to 10 MiB; native OCEL baselines to 64 MiB and
1–10000 nodes, retaining compatibility with the optional browser view.

## Save questions, assumptions and findings

Write a version-1 record and run:

```sh
glasstarn workspace record <project> --request <record.json> --json
```

```json
{
  "version": 1,
  "question": "Why do invoices take longer to process?",
  "assumptions": ["Processed means the posted event, as confirmed by AP."],
  "missing_data": ["Historical approval transitions for the review period."],
  "process": "Purchase-to-pay; the goal is paying on time without surprises.",
  "process_type": "purchase_to_pay",
  "answer": "Approval waits 6 days on median and causes most late payments.",
  "next_questions": ["Shall I check which approvers hold invoices longest?"],
  "findings": [
    {
      "kind": "observed",
      "title": "Approval is recorded after review",
      "statement": "The queried cases contain the Review to Approve transition.",
      "evidence": ["exports/query-<id>.json"]
    },
    {
      "kind": "hypothesis",
      "statement": "Approval availability may explain some delays.",
      "evidence": []
    }
  ]
}
```

`answer` (up to 400 characters) is the headline the browser Summary shows
for this question; `process` (up to 200) is the agent's one-line read of the
process and its goal; `next_questions` holds up to five follow-up questions.
`process_type` is one of `order_to_cash`, `purchase_to_pay`,
`sales_pipeline`, `it_tickets`, `claims`, `onboarding`, `permits`,
`manufacturing` or `logistics`; the browser then frames Overview around that
process's goal. All of these are optional. With no question asked, use
"Health check" as the question.

Give each finding an optional `title` of 1–160 characters: a short, supported
conclusion in business language for the briefing and slide heading. Keep the
full explanation and denominators in `statement`; a title does not replace
evidence or turn a hypothesis into a fact. Existing records without titles
remain readable.

Before data arrives, save a question with assumptions/missing data and omit
`findings`. Evidence references must be actual successful query artifacts from
this project and its current settings. Observations require at least one;
hypotheses may have none. Records embed supporting results and saved scope so
later queries cannot overwrite their evidence. A settings change requires new
queries before saving new observations; old records remain historical.

The engine validates reference identity and scope, not whether the statement is
entailed by the evidence. The agent must ensure every observation follows from
its cited results, and distinguish its interpretation from glasstarn calculations.

Records are appended as `findings/investigation-<id>.json`.
`workspace show` returns their paths in chronological order. A later agent can
read the latest relevant record and current settings, then continue the same
question without relying on the original chat history. Request files are capped
at 1 MiB and stored result/record artifacts at 64 MiB.

## Amounts

Questions such as "invoices above 50,000" or "how much value is waiting at
each step" use `values`. First call it without `field` to list the fields that
hold numbers. If the list is empty, tell the user the records carry no amounts
and answer with counts and times; never estimate money. With a field, the
result gives the total, mean, median, 90th percentile, minimum and maximum, and
counts cases without a number separately instead of treating them as zero.
`at_least` and `at_most` bound the field (both inclusive). `grouping` is
`{"by":"none"}`, `{"by":"variant"}`, `{"by":"last_step"}` or
`{"by":"attribute","attribute":"region"}`. Open amounts appear only when the
project's finishing steps are confirmed. Report the field name with every
figure, because the engine does not know the currency.

## Answer size

Workspace commands print compact JSON held to about 4 KB. Long lists are cut
to their first items in the engine's order, and each cut list is followed by a
`<name>_omitted` count; a `shortened` note says so. Embedded analyses are
replaced by a `headline` (counts, median and P90 time, top routes, slowest
hand-overs, repeated steps, amounts); the whole analysis is saved in the
project as `exports/latest-analysis.json`. Add `--full` to print everything.
`{"type":"follow",...}` returns the headline; add `"detail":"full"` for the
whole case analysis. Ask for the full answer only when the question needs it.
MCP tools follow the same rule; pass `"detail":"full"` to a tool for the whole answer.

## Question-led browser charts

A saved record may include `visuals` (at most six). For example:

```json
{"title":"Invoice amount by customer","evidence":"exports/query-123.json","kind":"bar","metric":"total"}
```

Use a current grouped `values` query snapshot as evidence. `kind` is `bar` or
`table`; metrics are `cases`, `total`, `mean`, `median`, or `p90`. A `cohorts`
query supports case counts and elapsed-time means/medians. Chart numbers are
calculated from the query by the engine. Do not supply rows, numbers, HTML or
scripts. Overview refreshes after a record is saved in the same project and
shows citations and example-item links. MCP remains read-only; the local
`workspace record` command publishes the result.

Use `{"type":"attribute_choices","attribute":"customer","search":"AB","limit":20}`
to discover actual field values in the active case scope. On a connected source,
include `object_type`. Do not invent customer names or assume an absent amount
is zero. A followed customer/product history can span several transactions:
its first-to-last time is a history span, and repeated activity names are not
proof of rework. Use transaction-bound handoffs for business-cycle questions.

## Continuous intentions and working processes

Records can include `intention: "understand" | "improve" | "check"` and a bounded
`process_model`. The working model combines recorded, documented, reported and
proposed steps and connections, with source versions and unresolved questions.
Use the schema and example in [the working-process contract](../../../../../docs/working-processes.md)
when working from this repository. The schema summary below is self-contained
for installed skills. The [consultant flow](consultant-flow.md)
defines the conversational behavior and evidence boundaries.

A model has version1, title, view (`combined_current`),
sources, steps, connections and open_questions. Each source has id, kind
(`document` or `reported_practice`), title, revision and reference. Each step has
id, label, optional owner and evidence. Each connection
has from/to step IDs, optional condition and evidence. Evidence is
`{"kind":"observed","query":"exports/query-123.json"}`,
`{"kind":"documented","source":"sop-id","section":"3.1"}`,
`{"kind":"reported","source":"interview-id"}` or
`{"kind":"proposed","rationale":"Reason and remaining uncertainty"}`.
Use current map queries or full followed analyses for recorded anchors; only
actual labels in their saved results qualify. Do not supply authored numerical
measurements. Advice (a future process, an improvement plan, a step's
treatment) is given in the chat and refused by `workspace record`.

Ordinary records preserve the current working model for unchanged settings.
Supplying a model creates a revision and preserves its predecessor. Omit
supersedes to use the previous model origin automatically. Withdraw with
`clear_process_model: true`; never edit earlier records in place. These records
update Overview while it is open, using the existing polling mechanism. They do
not push raw documents, alter the measured map or approve a conformance baseline.
