# Prepare linked tables as an object-centric log

Use `workspace prepare-ocel` when the question spans linked objects/systems and
sources are relational CSVs. This is one vendor-independent builder. Use ordinary
`workspace prepare` for a confirmed case-log projection; do not flatten a connected
question implicitly. Native OCEL supplied by the user can be imported directly.

Inventory files together, or reuse hashed connector preparation inputs. Establish
business meanings and exact link keys from company metadata/documentation. Confirm
system/tenant/company scope, events, timestamps, qualified link direction,
multiplicity and any omitted/unmatched-link policy. Never guess identities using
similar customer names, amounts or nearby timestamps. Preserve key strings and
leading zeros. Include older parent records when recent history references them.

Write a reviewed JSON recipe with this self-contained contract:

- Root: `version: 1`, `format: "ocel"`, `inputs`, `objects`, `events`, optional
  `relationships` and `event_links`. Inputs are `{id, sha256}` from the project.
- Object rule: `{id, input, namespace, object_type, key: [column, ...]}` and optional
  `attributes: [{column, name}]`. Namespace identifies system/tenant. Keys identify
  one source object and can include company. Blank/duplicate keys block.
- Event rule: `{id, input, namespace, key: [column, ...], activity: {value: ...}}
  (or `{column: ...}`), `timestamp`, `objects: [reference, ...]`, optional
  `attributes`. Use stable history IDs or explicitly named milestone rules.
  `optional_timestamp: true` permits counted blank milestones only when agreed;
  invalid nonblank timestamps block. `timestamp` is an RFC3339 column, or
  `{date, time?, offset}` for ERP exports: date `YYYYMMDD`, `YYYY-MM-DD` or
  `DD.MM.YYYY`, time `HHMMSS` or `HH:MM:SS`, and an offset such as `+01:00`
  that the person confirmed for the system's clock. An all-zero date is blank.
- Reference: `{object: object_rule_id, columns: [column, ...], qualifier: ...}`.
  Columns provide target key components in order from the current rule's input.
  A column may be a fixed character range, `{column, start, length}`, e.g.
  the invoice number and year inside SAP's `AWKEY`.
  `optional: true` allows counted blanks. Nonblank misses still block unless
  separately approved `allow_unmatched: true`. Never add overrides to hide defects.
- Object relationship: `{id, input, source: reference, target: reference}`.
  Direction is source → target, labelled by target qualifier; source qualifier is
  a lookup role. Use bridge rows for many-to-many relationships, not duplicated
  events or Cartesian joins.
- Additional event links: `{id, input, event: event_rule_id, event_key: [column, ...],
  target: reference}`. Use a bridge to attach one payment event to several invoices.
  Unknown events/duplicate qualified links block. Every event needs an object.
  `optional_event: true` counts bridge rows whose event was not emitted (an
  invoice not yet paid) instead of blocking.
- An event rule may add `lookup: {input, key, matching, columns, optional?}` to
  read columns from the one row of another input whose `key` equals this row's
  `matching` columns (a change time from the change header). Repeated keys block.
- `carried_links: [{id, event, through, relationship, qualifier}]` also links
  each event of rule `event` that is linked to a `through` object to every object
  the named relationship links from it (a payment reaches the order lines its
  invoice charges). It follows reconciled links only; it never matches values.
- Any rule may carry `where: [{column, in: [...]}]` or `{column, not_in: [...]}`
  to keep rows of one recorded kind, e.g. goods receipts in SAP's purchase order
  history. Left-out rows are counted as `filtered`, never silently dropped.

Rule IDs are unique across categories. One object rule owns each namespace/type.
Attributes for the same business type/activity must have matching definitions.
Unknown fields, code, key normalization and arbitrary joins/aggregation are not
supported. Attributes are currently strings; static object attributes use OCEL's
initial epoch, not an inferred historical timeline. Do not claim numeric cohort
or historical-attribute analysis merely because strings were retained.

Execute `glasstarn workspace prepare-ocel <project> --recipe <file> --json`, using
the same workspace root. Recipe submission runs the mappings as supplied; review
business decisions before executing. The implementation checks exact source hashes,
streams CSV rows, reconciles keys/events/links, then runs strict native OCEL analysis.
No bulk records enter chat. Do not calculate the result in the model.

Exit 3 means reconciliation is blocked. Explain full issue counts and bounded
rule/row/code examples and the specific missing records or definitions. The active
analysis remains unchanged. Other nonzero statuses are errors, not success.
Successful output gives evidence paths, hashed OCEL source, counts and glasstarn analysis.
Recipe, report and relationship lineage are saved per run. Objects/events retain
source hash and data-row lineage. Follow-ups use existing queries and optional
browser handoff in the same investigation. There is no fixed entity/link-count
ceiling. Preparation keeps compact indexes and records in memory, so available
RAM still limits practical scale: about 1.4 GB for 650k source rows and 7 GB for
3.25M on the synthetic benchmark. Larger runs are not yet benchmarked. Explain material scale constraints before requesting a large run;
do not promise arbitrary-size preparation or source-history completeness.
