# Current-state and improvement investigations

Clarify the outcome in the user's words: for example, why invoices take longer
to process. Establish the case or object notion, the interval and the business
definition of "processed". A documented SLA or prior-period comparison is useful
when supplied; do not invent a target. A request simply to understand current
work can begin with its recorded routes and timing; it does not require the
user to identify a problem or commit to automation first.

Use definitions already supplied. Ask what "processed" means when the choice
of endpoint would change the answer. Ask about teams, routing or industry only
when they help interpret a finding or select a useful comparison. Do not request
an ideal process or audit baseline for an ordinary delay investigation.

If no extracts exist, establish the source system and what the employee can
obtain, then help draft an actionable request for the company's systems or
reporting team. Explain the desired result in business terms before listing
fields. For invoices, for example, request invoice history for the agreed period,
including when each invoice was received, validated, corrected, approved and
paid if those steps exist in the company's process. Request documentation of
available history views, identifiers and field meanings when needed; do not
invent vendor table names or require a complete company data model to begin.
Save the question and an explicit missing-data request:
historical event/activity records, stable case or object IDs, event timestamps,
coverage dates and field meanings. Ask for a company schema or relevant data-model
documentation when the available fields and relationships cannot resolve this.
Request optional attributes only when they help the question. The product does
not connect directly to SAP or Oracle.

Inspect and prepare with the main skill. After a case import, run the `readiness`
query. One event per case is a reason to clarify history; it is not proof of a
bad file. Multiple events do not prove full coverage. Do not explain a whole
process's cycle time from current-status snapshots. Explicit historical milestone
timestamps can be prepared into events with reviewed meanings.

Use the saved analysis for scope, elapsed time, variants, bottlenecks, rework and
handover. Keep processing time distinct from elapsed gaps; actual waiting needs
end timestamps. Use activity/transition queries and case timelines to inspect
examples. Use cohort comparisons only with confirmed attribute meanings and
report the populations being compared. Native OCEL investigations retain object
relationships; use object lifecycle and bottleneck queries rather than silently
forcing a case notion.

Report the largest supported observations first, then plausible explanations
and the additional evidence needed to test them. Sequence and correlation do
not establish cause. Save query-backed observations and labelled hypotheses with
`workspace record`; include scope, missing data and confirmed assumptions.
Open the results in the browser as the main skill describes, once the first findings are saved.
