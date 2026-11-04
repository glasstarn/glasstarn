# Process packs

A process pack describes one business process in one kind of system as data:
which tables and fields to export, the checks that show whether an export is
complete and linked, and a connected-preparation recipe (see
[connected preparation](connected-preparation.md)) with placeholders for the
answers only the company can give. Packs live in `packs/<id>/pack.json`; the
built-in ones are compiled into `glasstarn`. A path to another `pack.json`
can be given wherever a pack name is accepted.

```sh
glasstarn workspace pack list
glasstarn workspace pack checklist sap-purchase-to-pay
glasstarn workspace pack handout <project> --pack sap-purchase-to-pay \
  --from 2026-01-01 --to 2026-06-30 --scope 1000
glasstarn workspace inventory <project> EKKO.csv EKPO.csv ...
glasstarn workspace pack check <project> --pack sap-purchase-to-pay
glasstarn workspace pack answer <project> --pack sap-purchase-to-pay \
  --question setting:offset --choice value --value +01:00 --confirmed-by "AP lead"
glasstarn workspace pack build <project> --pack sap-purchase-to-pay
```

## Export request

`pack handout` writes `exports/<pack>-export-request.md` in the project: a
request the person forwards to the system administrator, built from the pack.
It lists every table with only the columns needed, which rows to take and the
concrete filters (the scope field and the period where the pack knows them),
how to export it from that kind of system (SE16N, a background job for large
SAP tables, a query on a warehouse copy, Business One's Query Manager, an
S/4HANA Cloud or Ariba report), the exact file names and a checklist.

## Check

`pack check` matches each table to an inventoried file named after it
(`EKKO.csv`, `sap_ekko_2026.csv`); on a refresh the file from the latest
inventory wins, and `--file TABLE=<sha256>` decides by hand. It reports, per
table, rows, the date range, the values of the scope field (such as company
code) and the company's own fields (by prefix, for example `ZZ`), with how
often each is filled, whether it holds dates, how those dates fall against the
table's own date, and which other own fields are filled with it.

Only a case, steps and times are hard requirements. A missing table or
field is returned in `left_out`, with the steps and links the build goes
ahead without, unless without it nothing would be left to mine (no item of
the case type, or no step); then it is a gap. Problems that need a new
export (nothing left to mine, rows outside the export's scope, links where
most rows find nothing) are returned as `gaps` with exit status 3. Records that do not fit but may be real (a key listed
twice, a minority of rows whose reference finds nothing) are returned as
`data_problems` with the question `data:set-aside`: leave them out, build
with the rest and list them, or ask for a corrected export. With
`set_aside`, the build leaves out each source row that would otherwise block
it, and anything that depended on it, and reports them in
`preparation.set_aside_by_reason`, with every row in `set-aside.json` beside
the build's evidence and an audit entry. The build can find records the check
cannot (a missing time); it then stops and offers the same question.
Everything a person must decide is returned as `questions`: settings (such as
the system clock's offset), link rates below the pack's threshold, and the
company's own fields. The whole report is saved as
`preparation/pack-<id>-check.json`.

## Exports as people make them

Inventory reads CSV, tab-, semicolon- or pipe-separated text, fixed-width
text and SAP list output, Excel (.xlsx, .xlsm, .xls, .xlsb), LibreOffice
(.ods) and zip archives of them; a workbook saved under another name is still
read as one. Merged cells are filled in, and a two-row header with group
headings ("Order" over "No." and "Date") becomes one ("Order No."). A
password-protected workbook is named as such, so the person can send a copy
without the password. A file that still cannot be read is listed in
`unreadable` with a description (what kind of file it seems to be, its
encoding, separator and column names, never its rows) and a suggestion, so
the agent can advise the person. Text in UTF-16 or Windows-1252 (Excel's plain "CSV", many ERP
downloads) is converted to UTF-8, and Excel's byte-order mark is ignored;
values are not changed. A workbook with several sheets, such as Business
Central's configuration-package export, becomes one table per sheet, named
after the sheet; a file named exactly after a table ("38 Purchase Header")
is preferred over one that only starts with it ("Purchase Header Archive").
A pack table may name an `archive` (a table where the system moves finished
rows, with its key and version column): when both are in the project, the
check merges them, adding for each key not among the open rows its latest
archived version. A table may also name `fallbacks`, columns used when a
required one is not exported (Business Central's Order Date when the
configuration package leaves out SystemCreatedAt); the check and the build
say so. Optional columns that are not exported are left out: the steps or
links that need one are left out of the build and named in its changes.
Single-column times in pack recipes are read in UTC, as the export
instructions ask, so a time written without its zone is still read.

Inventory reads CSV and Excel (.xlsx) files; an Excel file's first sheet is
read from its header row, skipping title rows. Native dates use the workbook's
date system and cell formats, including ISO date cells. Floating dates and
times still need a confirmed source timezone. Cached formula results can be
read; the importer does not recalculate the workbook.

When an export names columns
differently from the pack, the check lists the export's columns and likely
matches by name in `column_matches`. After the person confirms the mapping,
`pack reshape` rewrites the file as the pack's table:

```sh
glasstarn workspace pack reshape <project> --pack appian-record-events \
  --table RecordEvent --file events.xlsx \
  --column recordId=rentalRequest --column eventTypeId=eventType \
  --time-zone America/New_York
```

Times written as `m/d/yyyy h:mm AM` in the renamed columns become UTC with
`--time-zone` (a zone name from the computer's time zone database, following
daylight saving, or a fixed offset such as `-05:00`); `--day-first yes` reads
day-first dates. Cells with a time on the first line and more below (a name)
keep the time. The rewritten file is added to the project as the newest copy
of the table, and each reshape is written to the audit log.

## Answers

`pack answer` validates the choice and value against the latest check's
question and saves it in `preparation/pack-<id>-answers.json` with who
confirmed it and when, and appends it to the project audit log. Answers are
reused on every later check and build; only new questions remain open.

## Build

`pack build` requires a check without gaps and every setting answered. It
replaces `{setting:<id>}` placeholders, applies confirmed field answers (a
date field as an optional step on the table's item, carried to its lines
through the table's `fan_out` links; a field as an item detail), leaves out
optional tables that were not exported, and runs the connected preparation.
The recipe is saved as `preparation/pack-<id>-recipe.json`; the result names
each change the answers made.

## Shared parts

Tables, checks and recipe rules used by several packs are written once in
`packs/shared/<id>.json` (for example SAP vendors, or supplier invoices and
their payment). A pack lists them in `include`; their tables, settings,
checks, uniqueness rules and recipe rules are merged in order, then the pack's
own entries are added. An own entry with the same table name or id replaces the
shared one, so a pack can adjust a shared table's export text or a check.

## Variants

A pack may list `variants`, each a subset of its tables with its own name and
`case_object_type`, such as accounts payable only. `pack check --variant <id>`
checks only those tables and saves the choice; `pack build` follows it. Rules
reading other tables, links pointing at their items and steps left with no item
are left out. The same pruning applies when an `optional` table was not
exported.

## Combining two packs

A pack may list `link_targets`: items another system can point at by their
number, each with `id`, `label` (the number in words), `object` (the recipe
object rule), `table` and `key` (where the number is), `example` and `carry`
(relationship rules from that item to the items the pack follows, such as an
order to its lines). `pack combine --pack A --with B --link <table>:<target>`
joins two packs where a table of A holds the number of B's target. The
combined pack (`A+B`) keeps both packs' tables, checks and rules, adds the
question `link_field` (which field holds the number), a relationship from A's
item to B's, carried links so A's steps reach B's item and the items B
follows, and a `most` link check that reports how many rows find their item.
The combined pack is saved in the project; check, answer and build use
`--pack A+B`. Packs with a table or rule of the same name cannot be combined
yet.

## Pack format

Top level: `id`, `version`, `name`, `system`, `process`, `status`
(`untested` until the Glasstarn project tests it on a real system, then `tested`; see [trademarks and compatibility](trademarks.md)), `note`, `case_object_type`,
`size_check` (`ask`, `unit`, `steps_per_unit`), `custom_prefixes`, `tables`,
`settings`, `unique`, `checks`, `recipe` and optional `variants` (`id`, `name`,
`case_object_type`, `tables`) and `link_targets`.

A table has `name`, `label`, `export` (the filter in words), `required` and
`fields`, and optionally `date_field`, `date_label`, `noun`, `object` and
`key` (the recipe object rule it feeds), `scope_field`, `fan_out` (event-link
templates) and `optional`. A `unique` entry names a table, key columns and an
optional `where`. A link check has `from` and `to` sides (`table`, `columns`,
optional `where`), `expect` (`all` or `most`), an optional `threshold` and an
optional `question` whose text may use `{missing}`, `{total}` and
`{missing_share}`.

Set `require_nonblank: true` on a link check when a blank source reference
must count as missing. By default link checks compare populated references;
optional business links can legitimately be blank. This setting does not make
nonblank unmatched references acceptable.
