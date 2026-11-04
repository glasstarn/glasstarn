# Preparing company extracts through an agent

The first preparation workflow converts approved CSV extracts into a
reproducible case-centric event log without running arbitrary scripts. Codex or
Claude inspects bounded schemas, proposes a small JSON recipe, and asks the
employee to confirm its business meaning. The Rust engine executes the recipe
over the complete files, reconciles the result, saves the evidence, and runs
the normal process analysis.

This workflow does not connect to SAP or Oracle. It starts with extracts the
company has already approved and supplied.

## Inventory the extracts

Create a project, then copy and inspect all relevant CSV files in one command:

```sh
glasstarn workspace inventory <project-id> \
  order_history.csv suppliers.csv --json
```

The result identifies every immutable source by SHA-256 and reports exact row,
blank, and bounded distinct-value information for each column. At most four
example values per column are returned. Distinct-value collection is capped;
`distinct_complete` says whether the reported count is exact.

The same inventory is saved at `preparation/inventory.json`. A recipe refers to
the returned hashes, so replacing a file cannot silently change a preparation.

## Review the recipe

A version-1 recipe unions explicit event projections and may enrich them with
validated many-to-one lookups:

```json
{
  "version": 1,
  "inputs": [
    { "id": "orders", "sha256": "<hash from inventory>" },
    { "id": "suppliers", "sha256": "<hash from inventory>" }
  ],
  "events": [
    {
      "input": "orders",
      "case_id": "order_id",
      "activity": { "value": "Order created" },
      "timestamp": "created_at",
      "case_attributes": [
        { "column": "company_code", "as": "company_code" }
      ]
    },
    {
      "input": "orders",
      "case_id": "order_id",
      "activity": { "value": "Order approved" },
      "timestamp": "approved_at",
      "optional_timestamp": true
    }
  ],
  "lookups": [
    {
      "event_input": "orders",
      "event_key": "supplier_id",
      "input": "suppliers",
      "lookup_key": "supplier_id",
      "fields": [
        { "column": "region", "as": "supplier_region" }
      ]
    }
  ]
}
```

`activity` accepts either `{ "column": "status" }` or a reviewed constant
such as `{ "value": "Order created" }`. Event rules can also map
`end_timestamp`, `event_order`, `resource`, `lifecycle`, and `cost` columns.

`optional_timestamp: true` means a blank timestamp proves that event was not
recorded for that row; it is skipped and counted. Without that explicit rule, a
blank required timestamp blocks preparation. Do not mark a field optional when
a blank could mean missing history.

Lookups are many-to-one. Duplicate or blank lookup keys always block
preparation. Blank event-side keys count as unmatched, and unmatched events
also block by default. Set `allow_unmatched: true` only after the employee has
confirmed that unmatched records are analytically acceptable.

## Execute, validate, and analyze

```sh
glasstarn workspace prepare <project-id> --recipe recipe.json --json
```

The engine reports rows read and emitted for every event rule, skipped optional
timestamps, invalid required rows, lookup cardinality, matches and misses,
output cases, output events, and exact duplicate output rows. A blocked result
uses exit status `3` and does not replace the project's active analysis.

Successful preparation saves:

```text
preparation/inventory.json
preparation/recipe.json
preparation/validation.json
sources/<prepared-log-hash>.csv
exports/latest-analysis.json
```

The prepared CSV includes `_source_input` and `_source_row` on every event so a
result can be traced back to the company extract. The prepared log becomes the
project's active source and immediately runs through the same mapping,
data-quality, and analysis engine as a ready-made event log. If that stage asks
an interpretation question, rerun `workspace prepare` with one or more
`--answer <kind>=<option>` flags.

## Current boundary

This case-log workflow supports CSV event construction and many-to-one enrichment.
For typed objects and explicit cross-system relationships, use the separate
[connected preparation contract](connected-preparation.md) and `workspace prepare-ocel`.
Neither workflow executes arbitrary code, performs fuzzy matching or invents missing
history. Saved investigation questions and findings are handled by `workspace record`.
