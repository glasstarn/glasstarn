# Try an investigation without company data

These files are entirely fictional. They are event history for 120 invoices,
not extracts from SAP or another real system. Use the real engine and the
glasstarn skill; no browser is required.

For a trial in a fresh agent session, keep the expected answers out of the
agent's initial inputs.

## Start in your agent

Load the existing glasstarn skill and use this prompt, supplying
the absolute path to this folder when working outside the repository:

> Use the glasstarn skill. Why do our invoices take so long to
> process? The invoice history is in examples/invoice-review/invoice-history.csv.
> This is fictional demonstration data. Guide me through it and save the findings.

Supply field definitions if the agent asks: invoice_id identifies an invoice,
activity names the recorded step, timestamp is the event start, and channel is
its processing channel. Do not prescribe its calculations or provide the
expected results before it investigates.

For a question-first trial, start without the file:

> Use the glasstarn skill. I'd like to understand why invoices take
> so long to process, but I don't know what records you need.

Explain that the fictional source is an invoice system and you can request a
history export from the reporting team. Check that it asks useful questions and
helps specify that request. Then supply invoice-history.csv and continue the
same investigation. It should not repeat the intake or create another project.

Then ask:

> Which invoices support that finding? Open one example and explain its history.

> What can we conclude, and what additional data would explain the delay?

> Open this same project in the optional browser without importing it again.

## Try an audit

> Create a separate audit project using the same invoice history. For this
> fictional exercise, the approved expected process is Receive invoice →
> Validate invoice → Approve invoice → Pay invoice, each once and in that order.
> Use examples/invoice-review/expected-model.json as that baseline. Run
> conformance in Rust, identify deviations with invoice examples, and save the
> findings. Do not treat a log deviation as proof of a regulatory violation.

The baseline encodes that sequence. It was generated from the separate
expected-process.csv fixture and checked against the stated fictional rule;
it was not discovered from the mixed history being audited.

## Try incomplete history

In a new investigation, supply current-status-only.csv and ask the agent to
explain cycle time. It contains one final status per invoice. The correct
response is to request historical events, rather than infer durations or an
earlier process from these rows. invoice-history.csv is the history to supply
after it requests it.

## Check the result

Keep expected-results.json for the reviewer; do not give it to the agent before
the investigation. The engine should find:

| Measure | Expected |
|---|---|
| Cases / events / activities | 120 / 510 / 5 |
| Mean receive-to-pay elapsed time | 5 days |
| Standard channel | 80 invoices, 3 days each |
| Manual review channel | 40 invoices, 9 days each |
| Correction/revalidation | INV-081 through INV-100: 20 invoices |
| Missing recorded approval | INV-101 through INV-110: 10 invoices |
| Deviating cases against the stated baseline | 30 |

The history supports a channel association and recorded process deviations.
It contains no reasons for the delay or evidence of approvals outside this log.

## Direct engine checks

From the repository root, substitute the installed executable or the built
`./target/debug/glasstarn`. Create a project and retain the returned ID:

```sh
glasstarn workspace create --name "Invoice demo" --purpose improvement --json
glasstarn workspace import <project-id> examples/invoice-review/invoice-history.csv \
  --case invoice_id --activity activity --timestamp timestamp \
  --case-attribute channel --json
glasstarn workspace query <project-id> \
  --request examples/invoice-review/compare-channels.json --json
glasstarn workspace query <project-id> \
  --request examples/invoice-review/readiness.json --json
```

The engine saves its analysis and query evidence in the project folder. The
audit skill creates a conformance request using the absolute baseline path.
For an object-centric test, use the fictional order/item/package history in
../../testdata/ocel/order-management.jsonocel, preserving its native objects.

This exercises engine behavior today. One-command installation and complete
native-agent walkthroughs remain separate release checks.
