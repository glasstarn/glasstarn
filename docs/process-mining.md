# Process-mining methods and interpretation

## Default investigation views

### Directly-follows graph

Counts observed adjacent activity pairs across complete case traces. Frequency
and timing are separate overlays. A cycle in the aggregate graph does not prove
that any one case looped; loop analysis therefore uses traces directly.

### Variants

Groups identical end-to-end activity sequences and ranks them by case count.
Coverage shows whether a few paths explain most cases or whether behavior is
highly variable.

### Browser explorers

On desktop, the navigation starts as an icon rail. Hover or keyboard focus
reveals its labels; the pin control keeps it open and remembers the choice
locally. Compact windows retain labeled navigation.

**Process** shows an aggregate observed directly-follows graph. Independent
activity and connection controls expand from the main flow to all observed
connections. Simplifying the diagram does not filter the project: node and
edge counts still use the active scope, and the coverage label reports how many
items have their whole route visible. Aggregate edges do not establish
exclusive choice, parallel execution or causality.

**Variants** places the route diagram beside a ranked distribution. Select one
or several variants to inspect their combined observed flow and exact case
coverage. Selecting V2 does not also select V1. Search matches activity names;
frequency and elapsed-time sorting apply to the ranked list. The list includes
up to 100 variants and discloses the number beyond that limit. Exact sequences
remain available, including repeated activities that share a graph node.

The explorers share active filters and the followed object type, while keeping
independent diagram selections. Metrics distinguish unique items from event
occurrences; a repeated step can count once per item and several times as an
event. Average elapsed time uses eligible recorded timestamps, not work effort.
The connected subway uses object counts or elapsed time, not event-frequency
labels. Median and trimmed-mean graph overlays are not currently offered.

Overview retains all findings produced by the available finding rules, with
additional findings expandable and saved assumptions and missing evidence
visible. This is not a claim to detect every possible business problem.

### Analysis on the process map

In **Process**, choose **Measure**, then select the starting and ending steps
on the map. A compact inspector keeps the map in view and separates **Time**,
**Breakdown** and **Items**. The endpoint selectors also provide access to steps
hidden by map detail controls. **View** opens drawing controls only when needed.

**Whole history** measures the first-to-last recorded event for each selected
item. **Between steps** measures named endpoints within that same history,
with independent first/last occurrence choices. Intermediate events remain in
the trace. Neither measure estimates manual effort. A customer history spanning
many orders is a history duration, not an order's time to cash.

The workspace leads with median elapsed time, with average and P90 alongside it.
The eligible measurement count remains visible above the result. A histogram
shows the distribution; range controls open on demand, and selected-versus-rest
patterns appear after a range is chosen. Missing endpoints, reversed endpoints
and incomplete histories have no measured interval; equal timestamps produce zero elapsed
time. Median uses the upper middle observation and P90 uses nearest rank.
Typed duration ranges include both bounds. Histogram bars have an inclusive
lower bound and exclusive upper bound, disclosed when selected.

Choose an attribute to compare groups and select recorded values. Values within
one field use OR; different fields use AND. Missing values are a separate
condition. Searchable value choices show up to 100 matches and disclose
truncation. The breakdown starts with a bar comparison of the eight largest
groups.
Switch between median, average, P90 and item count without changing the
selection. Selecting a group opens its value filter for review; it does not
apply a filter immediately. A collapsible table retains up to the 100 largest
groups, with the total group count disclosed above the chart. Numeric conditions
remain available in the shared filter controls.

A duration range compares selected versus remaining items within the other
active filters. Activity prevalence contrasts compare measurable intervals only; missing intervals
remain in the population totals and are excluded from that baseline. These
differences are associations, not causes.
Apply a selection to update all investigation views; simplifying a diagram
alone never changes that selection. Bounded example-item buttons open source
histories. The existing timing distribution, bottlenecks and slowest items
remain available in the Items pane under supporting timing evidence. Group/period comparisons
remain accessible from the same workspace.

**Save view** stores the measurement definition, active filters and source/settings
identity in the project folder. **Saved views** reopens those selections and
recalculates the result; it does not freeze or accept browser-supplied statistics.
Views based on a changed source or changed process settings remain recorded but
cannot be restored into the current analysis. Saved agent charts also appear here.

The browser, workspace query API and MCP tools share `pm-query` calculations.
`explore_process` accepts an optional `segment`, `group_by`, and whole-history
`min_micros`/`max_micros`. For a segment, put its bounds inside `segment`.
The workspace query equivalent is:

```json
{
  "type": "explore",
  "segment": {
    "from": "Approve order",
    "to": "Ship order",
    "from_occurrence": "first",
    "to_occurrence": "last",
    "min_micros": 864000000000,
    "max_micros": 1555200000000
  },
  "group_by": "vendor"
}
```

Saved grouped query evidence can support bar or table visuals using item count,
average, median or P90. Saving an agent response is what publishes its evidence;
a browser query preview alone does not create a saved agent artifact.

### Bottlenecks

Ranks transitions by total eligible duration burden, with occurrence and
eligibility denominators. If only start timestamps exist, transition durations
are start-to-start elapsed time. They are called waiting time only when end
timestamps make true waiting measurable.

### Rework

Detects immediate repeats and observed return paths inside individual traces.
It reports affected cases, repetitions, timing eligibility, and example cases.

### Organization and automation investigation

Resource mappings support handovers and descriptions of recorded performers.
Human/system classification is an explicit input. Automation targeting then
orders manual-work hotspots using observed scale, processing evidence, routing
concentration, divergence, rework, and cost where available.

This output is not an automation-readiness score. Inputs, decision rules,
exceptions, regulation, risk, and implementation effort still require outside
evidence and judgment.

## Formal discovery

| Algorithm | Intended use | Important limit |
|---|---|---|
| Inductive Miner with local infrequent-edge filtering | Sound structured default | Filtering choices affect model detail |
| Alpha Miner | Teaching and simple clean logs | Weak with noise, short loops, and complex behavior |
| Heuristics Miner | Frequency-based dependency exploration | The heuristic net is not guaranteed sound |

Inductive discovery returns a process tree and accepting Petri net. The browser
keeps that formal model separate from the observed directly-follows map.

The MCP surface exposes all three algorithms through
`discover_process_model`. Results reproduce the saved or live active view,
apply the reviewed activity-name policy, and reject an oversized model intact
rather than returning a structurally incomplete one. Heuristics output remains
an explicitly non-executable dependency net.

## Conformance

Conformance always requires an explicit baseline model. It never presents a
model discovered from the tested log as independent proof that the same log
conforms.

- Token replay is fast and reports produced, consumed, missing, and remaining
  tokens.
- Alignment uses deterministic optimal A* search and reports synchronous,
  log-only, visible-model, and invisible-model moves.

Worst-case detail is bounded while totals cover the complete active view.
Conformance diagnoses difference from the supplied model; it does not by itself
establish legal or regulatory compliance, nor that the baseline is correct.

For AI clients in saved-project mode, `list_process_models` returns opaque
handles for local `*.model.json` files. `check_process_conformance` performs
token replay and `align_process_conformance` performs optimal alignment. These
tools never accept a filesystem path or privacy-transformed discovery payload
from the agent.

In the browser, use **Save formal model** from the discovered formal view to
export a portable `.model.json`. Under **Check process conformance**, choose a
reviewed baseline and select token replay or optimal alignment. Totals cover the
active case filter; case buttons open the supporting event timeline. Changing
the analysis clears prior conformance evidence and requires a fresh check.
An empty view supports no conformance conclusion. Model-origin filenames are
hints, not proof that a baseline is independent of the tested data.

When live agent access is enabled, the selected browser baseline is also
available to MCP under the reviewed exposure policy.

## BPMN import

The importer accepts documented tasks, start/end and supported intermediate
events, exclusive gateways, parallel gateways, and cycles. It rejects
unsupported or ambiguous constructs instead of approximating them silently.
The resulting accepting Petri net can be used as an explicit conformance
baseline.

## Comparison

Cohort comparison measures activity-prevalence and timing differences between
two values of a mapped case attribute. Period comparison does the same across
two explicit, non-overlapping case-start windows. Results are statistical
associations with denominators and bounded evidence, not causal effects.

## Amounts

A source column does not say whether it holds money, so case fields are read as
numbers only when a value is unambiguous: digits with an optional sign, decimal
point and exponent, an optional leading `$`, `€`, `£` or `¥`, and comma
thousands separators in their exact positions (`1,234,567.89`). Values such as
`12,5`, `N/A` or `1.2.3` are not numbers. A field counts as a number field when
at least 99% of its recorded values read as numbers; the exceptions are
counted.

- **Filter:** `--where 'amount>=50000'` or `--where 'amount<=100'` (CLI), the
  Amount field and at-least / at-most boxes in the browser filter, or
  `attribute_between` in a saved filter. Bounds are inclusive. A case without
  a number never matches.
- **Totals:** `glasstarn values <file> --field amount [--by variant|last-step|<field>]`,
  the `values` workspace query, the `summarize_values` MCP tool, and the Value
  panel on Summary. Each case counts its value once, however many steps it has.
  Cases without a number are counted beside the figures, never as zero.
- **Open value:** with confirmed finishing steps, the amount on cases whose
  last step is not a finishing step is reported as open.
- **MCP privacy:** field names follow the attribute-name policy; totals need
  `costs` to be shown; grouping by another field needs attribute values.

A column named like a cost (`cost`, `spend`, `charge`) is proposed as a cost
per step. Amounts, prices and values are proposed as case fields, because an
invoice amount repeats on each row of its case and would otherwise be counted
once per step.

## Trace filters

- **Starts with / ends with:** match the first or last recorded event.
- **Directly follows:** the second activity is the next normalized event.
- **Eventually follows:** the second activity occurs at a strictly later
  position, with any number of intervening events.
- **Self eventual-follow:** requires two occurrences of that activity.

Multiple pair conditions may be applied and all must match. Equal-time order
uses the confirmed deterministic policy and is not evidence of causation.
