# Data, mapping, and saved projects

## Supported case-centric files

- Excel workbooks: `.xlsx`, `.xlsm`, `.xls`, `.xlsb`; LibreOffice `.ods`
- CSV and other supported separated or fixed-width text tables
- Parquet
- XES and XES.gz

Quick one-file commands and `workspace import` read workbook sheets through the
same table-conversion path. A workbook with several sheets uses the one that
looks most like an event history; the import reports the chosen sheet. Project
inventory can also prepare multiple extracted tables. Run `glasstarn formats
--json` for the format catalogue.

The browser upload ceiling is 100 GiB. This is not a measured processing-capacity
guarantee: workbook expansion, event counts, relationships and available memory
can limit a practical analysis before the upload ceiling is reached.

The minimum logical schema is case ID, activity, and timestamp. Column names do
not need to match those words; the profiler ranks candidates and the user
confirms the mapping.

Wide tables with one timestamp column per milestone can be recognized and
unpivoted into events. The transformation is reported in the result.

## Optional fields

| Field | Effect |
|---|---|
| End timestamp | Enables processing duration and true wait after completion |
| Event order | Becomes authoritative for within-case ordering |
| Resource | Enables resource and handover evidence |
| Lifecycle | Helps order same-time lifecycle stages |
| Cost | Adds exact sparse event costs and aggregate totals |
| Case attributes | Enable exact filtering and cohort comparison |

Costs are parsed into fixed-point millionths rather than floating point.
Missing optional values remain missing rather than becoming synthetic labels.

## Ordering

Within a case, normalized order follows the confirmed policy:

1. explicit event-order column when mapped;
2. parsed timestamp;
3. lifecycle stage where applicable;
4. source row as the deterministic final tie-breaker.

An observed order is not automatically a causal order. This is particularly
important for directly-follows and trace-pattern filters when timestamps tie.

## Validation and interpretation

Structural errors are rejected. Ambiguities that support more than one
reasonable interpretation become explicit decisions. Examples include
ambiguous date order, duplicate rows, missing timestamps, and timestamp ties.
Analysis does not silently choose a materially different process.

Preparation rules are separate and reviewable. They can rename or merge
activities, exclude named activities or cases, require a minimum case length,
and drop cases flagged incomplete. Rules never modify the input file.

## Saved project contents

### Project folders

The app and agent workspace keep each investigation in a durable folder. It
contains `workspace.json`, fingerprinted source copies, analysis settings,
findings, query evidence, exports and any expected-process model. Reopen it
from **All projects**, or ask your agent to continue the project. The source
copies and settings travel with the folder; keep it private like the original
exports.

Changes save automatically. A save failure stays visible until it is resolved.
The current view is recalculated from the saved source and settings when the
project is reopened.

### Standalone settings JSON

The lower-level CLI also supports a separate settings file created with
`analyze --save-project`. This is not the browser’s project folder. It records:

- format and engine version;
- project name;
- source path, size, modification time, and SHA-256 fingerprint;
- complete field mapping;
- interpretation policy;
- preparation rules;
- active case filter, including repeatable trace patterns.

This standalone settings JSON intentionally does not contain the event log or
cached analytical results.
The JSON is portable with its matching source file. Reopening verifies the
fingerprint before reproducing the result.

A formal `*.model.json` is a different artifact: it contains a labelled
accepting Petri net discovered by the engine or imported from the supported
BPMN subset. Conformance treats such a file as an explicit baseline. Its
fitness result is meaningful only when the model's origin and approval are
independent of the log being assessed.

## Filtering contract

Filters select whole cases and combine with AND. A retained case keeps its
complete path and evidence. Available conditions include:

- must contain or exclude activities;
- first or last activity;
- repeatable directly-follows and eventually-follows pairs;
- exact case-attribute values;
- inclusive case-start boundaries;
- minimum or maximum throughput;
- most common variants;
- minimum case length;
- complete cases only.

Unknown required labels match no cases. They do not silently disable the
condition. Every result reports the original and retained scope.

## Source evidence

Normalized events retain their source-row anchor. Case timelines expose only
the evidence fields available under the current surface and privacy policy.
Aggregates retain bounded example identifiers so a user can inspect supporting
cases without materializing the entire log in a response.
