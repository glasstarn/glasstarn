# Audit and conformance investigations

Enter this branch when the user asks whether work followed an expected process
or control. Explain it as "checking what happened against your agreed rules."
Do not introduce audit questions merely because an improvement log contains
unusual paths or missing approvals.

Establish the review period, populations, expected process/control and exceptions
allowed by the company. Request an approved baseline or explicit documented
requirements in addition to historical evidence. Record the origin, effective
period and limitations of that expectation as assumptions in the project.

An expected sequence can be checked through formal conformance when an explicit
supported model is available. Case baselines use portable model JSON; native
OCEL uses its object-centric model JSON. An approved BPMN model can be converted
using `glasstarn import-bpmn` (unsupported constructs are rejected). Ask the
person to confirm a model you translated from text, after showing its results
(see below), not before. Do not claim free-form policy
text has been automatically converted into all audit controls.

Discovery describes observed behaviour. A model discovered from the reviewed
log is not independent evidence of compliance: high fitness may simply reflect
that same behaviour. Use a discovered model as a baseline only when the employee
explicitly adopts it as the expected process and disclose that provenance.

**From a written SOP or procedure:** read it, write the expected process as a
model with step names (below), run the check straight away, and show the
results. Then, at the end of the same answer, list in plain words how you read
the SOP (the step order, which steps are optional, the choices) and ask the
person to confirm or correct it; re-run if they correct it. Never stop to ask
for confirmation before showing any result. Use the exact step names from the
records. Write the model as a JSON file in the working folder:

```json
{"source": "Claims Handling SOP v2.1",
 "tree": {"kind": "sequence", "children": [
   {"kind": "activity", "activity": "Claim submitted"},
   {"kind": "loop", "body": {"kind": "tau"},
    "redo": {"kind": "sequence", "children": [
      {"kind": "activity", "activity": "Request documents"},
      {"kind": "activity", "activity": "Documents received"}]}},
   {"kind": "exclusive", "children": [
      {"kind": "activity", "activity": "Senior review"}, {"kind": "tau"}]},
   {"kind": "exclusive", "children": [
      {"kind": "activity", "activity": "Approve claim"},
      {"kind": "activity", "activity": "Reject claim"}]}]}}
```

`sequence` is steps in order, `exclusive` is one of several (with `tau` for
"or nothing", making a step optional), `loop` repeats `redo` any number of
times (with `tau` as body), `parallel` is steps in any order. Rules that depend
on a field, such as "senior review for claims over $25,000", cannot be written
in the model: check them with a filter (an amount range plus a missing step)
and report them alongside.

The result's `by_step` lists every deviation by step and kind with its case
count and examples: report those counts in plain words ("41 claims were paid
without a recorded payment approval"), map each to the SOP control it breaks,
and save them as findings with `workspace record`. A conformance check makes the
browser's Conformance page appear, so open the results as usual.

Run a `conformance` query with the explicit model. An expected process with
optional steps, choices or loops (every SOP written as steps) is always checked
by exact alignment, whatever `algorithm` says, because token replay can flag
compliant cases there; the result's `algorithm` says which ran. Its `by_step`
kinds are `skipped` (a required step never recorded) and `extra` (a step
recorded where the SOP has no place for it). A successful query retains the selected model in the project for
subsequent queries and the optional browser. Invalid replacements do not replace
the accepted baseline. Report scope, fit/deviating populations, algorithm,
deviation evidence and incomplete-history limitations. Inspect returned case or
object examples. Truncated examples are not the complete exception population.

Conformance means fit to the chosen formal model. It does not establish complete
control effectiveness, authorization, segregation of duties or audit assurance.
Separate recorded deviations from their business explanations. Save observations
with query references and hypotheses separately. If a requested business-control
check is unavailable, name the missing capability rather than simulating a
deterministic check in narrative.
