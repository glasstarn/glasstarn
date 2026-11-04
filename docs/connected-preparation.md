# Preparing connected processes

`glasstarn workspace prepare-ocel` converts reviewed CSV tables into an OCEL 2.0
log and runs the existing native object-centric analysis. The builder is independent
of vendors. Salesforce + SAP, Salesforce + Oracle, CRM + accounting, or other
combinations use the same executable with different mapping recipes. Connectors
retrieve records; recipes define their meaning and links.

Start with inventoried company files or connector extracts registered as preparation
inputs. Confirm entity keys, company/tenant scope, recorded actions, timestamps,
relationship direction and multiplicity with the employee or supplied documentation.
No fuzzy entity matching or missing-history reconstruction is performed.

```sh
glasstarn workspace inventory <project> opportunities.csv orders.csv invoices.csv payments.csv payment_invoices.csv --json
glasstarn workspace prepare-ocel <project> --recipe recipe.json --json
```

Use the same `--workspace-dir` throughout when applicable. The
[synthetic sales-to-cash example](../examples/connected-sales-to-cash/README.md)
includes a recipe template and five small tables. Replace each template input's
SHA-256 with its inventory identity before execution. This example uses invented
schemas, not validated Salesforce/SAP/Oracle mappings.

## Recipe v1

The root requires `version: 1`, `format: "ocel"`, `inputs`, `objects` and `events`.
Optional `relationships` and `event_links` default to empty. Unknown fields and
executable instructions are rejected. Each input has `id` and `sha256`. Source
hashes must resolve to exactly one immutable preparation input in this project.

**Object rule:** `id`, `input`, `namespace`, `object_type`, and `key` (ordered column
list). Optional `attributes` maps `{column, name}` pairs to string attributes.
Use a namespace identifying the system and tenant, such as `erp:tenant-a`.
Composite keys like `["company", "invoice_number"]` prevent company-local IDs
from colliding. Generated identities encode namespace, type and exact key values
as JSON tuples. Identifiers are not trimmed, cast to numbers or case-normalized.
Blank or duplicate keys block. One rule owns each namespace/type combination.

**Event rule:** `id`, `input`, `namespace`, `key`, `activity`, `timestamp`, `objects`,
and optional `attributes`. `activity` accepts `{value: "Invoice issued"}` or
`{column: "activity"}`. Keys identify each recorded action within the rule;
use source history IDs where available. Different explicitly named milestone
rules can use the same source key without merging their actions. IDs include
namespace, rule ID and exact keys. Duplicate keys within a rule block. Timestamps
must be RFC3339 with offsets, or come from ERP-style parts:
`"timestamp": {"date": "CPUDT", "time": "CPUTM", "offset": "+01:00"}`. Dates may be
`YYYYMMDD`, `YYYY-MM-DD` or `DD.MM.YYYY`; times `HHMMSS` or `HH:MM:SS`; a missing
time is midnight and an all-zero date is blank. The fixed offset is a reviewed
statement about the source system's clock; daylight-saving changes are not
applied. No other local-time or business-calendar assumptions are made. `optional_timestamp: true` skips and counts blank recorded milestones;
nonblank invalid timestamps always block.
An ERP date column may also contain a compact `YYYYMMDDHHMMSS.fraction`
timestamp. Use an explicit `+00:00` offset for source fields defined as UTC;
do not apply the local system offset to a UTC workflow timestamp.

An entry in event `objects` is an object reference:

```json
{"object": "invoices", "columns": ["company", "invoice_number"], "qualifier": "paid invoice"}
```

`object` names an object rule. `columns` supplies the key values from this rule's
input, in the target key's order. Qualifiers describe the confirmed relationship.
A reference column may also be a fixed character range,
`{"column": "AWKEY", "start": 0, "length": 10}`, for keys stored inside one
field. Missing columns or mismatched key widths are errors. `optional: true` permits only
blank references, counted separately. Nonblank unmatched references block unless
`allow_unmatched: true` was deliberately approved. That override is reported;
it must not be added merely to make a run pass. Every event must end up linked
to at least one object.

**Object relationship rule:** `id`, `input`, `source`, `target`. Both ends are object
references. The direction is source → target and the target qualifier labels the
edge; the source qualifier describes its lookup role only. One-to-many relations
use repeated source keys with distinct targets. Many-to-many relations use an
explicit bridge table, not a Cartesian join.

**Additional event links:** `id`, `input`, `event` (event-rule ID), `event_key`
(ordered columns matching that rule's key) and `target` (object reference). A bridge
can link one payment action to several invoices without duplicating that action.
Unknown event references block unless the link rule sets `optional_event: true`,
which counts them as skipped (for example accounting documents with no payment yet). Duplicate qualified links block, including
links repeated between inline references and bridges. Relationship lineage records
the input hash, row, rule and endpoints for each emitted link.

**Second sources of objects:** an object rule with `existing_ok: true` may
name objects an earlier rule already made (order lines known only from their
posted shipments once the order is gone); such rows are skipped and counted.
It may share the type of an earlier rule and carry fewer attributes.

**Columns:** a column name may repeat in a file only where no rule reads it;
a repeated column a rule reads is an error naming it. A byte-order mark is
not part of the first column's name.

**Lookups:** an event rule may add `lookup: {input, key, matching, columns,
optional?, where?}`. Each row's `matching` columns must equal the `key` of exactly one
row of `input`; the listed `columns` of that row can then be used like the
rule's own (timestamp, attributes, references). Use it where a value lives in a
header beside many detail rows, such as a change time in SAP's change header.
Repeated lookup keys block; a row with no match blocks unless `optional: true`,
which skips and counts it.
`where` filters the lookup's source rows before uniqueness and matching are
checked. An event may also provide `lookups: [...]`, applied in order after
`lookup`, with at most eight joins in total. Each join has its own key index;
later joins can match fields produced by earlier ones and replace a column's
value. Missing required joins or duplicate keys still block publication in
both memory and disk preparation. Pack builds include every join source's
fingerprint and omit rules when an optional join-source table is absent.

**Carried links:** `carried_links` entries `{id, event, through, relationship,
qualifier}` follow links that are already reconciled: every event of rule `event`
linked to an object of rule `through` is also linked to each object the
relationship rule `relationship` links from it. A payment linked to an invoice
thereby reaches the order lines that invoice charges. The relationship's source
must be the `through` object. A link reached twice is added once and counted as
skipped; lineage names the relationship row that carried it.

**Row conditions:** every rule may add `where`, a list of
`{"column": "VGABE", "in": ["1"]}` or `{"column": ..., "not_in": [...]}` conditions
on exact strings. All must hold. Rows left out are reported per rule as `filtered`.

## Evidence and publication

The result reports source rows visited per rule, entities/links emitted, skipped
optional milestones, explicitly omitted blank links, unmatched references and
full issue counts. Issue examples are capped at 50 and expose rule/row/code rather
than raw keys. Source row numbers count CSV data records from one, not physical
lines in files with quoted multiline cells. Rule row totals may revisit one file;
they are not unique source-row totals.

An unresolved reconciliation issue returns `blocked` with exit status 3 and leaves
the active source and analysis unchanged. A recipe with `"set_aside": true`,
set only with the person's approval, instead leaves out each source row that
raised a row-level issue and builds again until none does (leaving a row out
can strand another, such as a link to an event that was left out). The rows
are counted per rule as `set_aside`, by reason in `set_aside_by_reason`, with
examples in `set_aside`; the full list is saved as `set-aside.json` in the
evidence folder. Issues not tied to a row, such as an empty result, still
block. Malformed-file and recipe
errors also leave the active analysis unchanged. Do not treat any nonzero result
as an analyzed connected process.

A successful result runs the strict OCEL importer and quality gate before making
`prepared-connected.sqlite` (the OCEL 2.0 SQLite exchange format) a hashed project
asset and selecting it as the active OCEL source. Existing investigation records and original preparation inputs stay
in the project. A replacement clears old case mappings and analytical baselines;
select a new baseline explicitly when needed. Standard `workspace analyze`, queries
and the optional browser then use this same connected source.

Each reconciled attempt saves its approved recipe, report and relationship lineage
(`relationship-lineage.json.gz`, gzip-compressed JSON) under a unique
`preparation/ocel-preparation-…/` folder. Objects/events contain
`_source_input` (source SHA-256) and `_source_row`. Files in `sources/` are immutable.
An identical recipe over identical sources produces an identical OCEL asset.
Entity attributes supplied by this slice are strings; numbers used as keys retain
leading zeros. Object attributes are static reference values at OCEL's initial
1970 epoch, not inferred historical changes. Do not interpret them as values known
at each historical event.

## Current bounds

CSV only; up to 64 inputs/rules per category, 8 key components and 32 attributes.
Recipes are capped at 1 MiB. There is no fixed limit on object, event or
relationship counts. Source rows stream; entity indexes and the built graph reside
in memory as compact typed records (repeated names stored once, links held as
positions), and the OCEL log and lineage are written straight from them to disk.
Practical capacity depends on available RAM and the width/connectivity of the data.

Measured on an 18 GB Apple M3 Pro with the synthetic sales-to-cash shape (single
runs, peak memory footprint of the whole command):

| Source rows | Objects / events | Links | Prepare + analyze | Reopen and analyze | Project on disk |
|---|---|---|---|---|---|
| 650k | 0.49M / 0.49M | 1.41M | 9 s, 0.4 GB | 4 s, 0.3 GB | 0.43 GB |
| 3.25M | 2.45M / 2.45M | 7.05M | 47 s, 2.1 GB | 20 s, 1.5 GB | 2.2 GB |

Measured on the made-up SAP purchase-to-pay export (300,000 orders; 9 tables plus
the change log; 3.4M steps, 2.2M items, 8.5M step links), building through the
pack: 2.5 minutes, 1.2 GB peak; reading it back and analysing: 1.7 GB peak. The
saved log is 3.4 GB on disk, mostly repeated item and step identifiers in the
link table.

Before the compact records and SQLite output, 650k rows needed 8.1 GB and 0.84 GB
of disk, and 3.25M rows did not finish within 14 GB. Larger or more connected data
is not yet benchmarked.

No arbitrary SQL/code, fuzzy matching, key normalization, inferred activities,
generic aggregation, alternate-key lookup, typed numeric business attributes or
dynamic object-attribute history are implemented. The recipe can connect explicit
native IDs, or an external reference matching a target object's approved key.
Missing/ambiguous links require better extracts or a reviewed mapping change.
Cover older parent objects when investigating newer history; record extract/history
period limitations. The builder cannot establish completeness of unavailable
source history. Cross-system relationships do not establish causes, and available
analysis queries still determine which specific business questions can be answered.
