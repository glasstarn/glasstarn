# Object-centric process mining

Case-centric mining requires every event to belong to one case. That is a poor
fit when one real event simultaneously concerns an order, several items, a
shipment, and an invoice. Choosing one case notion can duplicate events, omit
interactions, or produce misleading paths.

Glasstarn reads OCEL 2.0 JSON, XML, and SQLite into a native model
that preserves:

- each event once;
- objects and object types;
- event and object attributes, including time-varying object history;
- qualified event-to-object relationships;
- qualified object-to-object relationships.

## Reading published files

Published OCEL 2.0 files vary. The readers accept what is valid and report what is
broken instead of refusing the whole file. All official business samples listed on
ocel-standard.org (order management, procure-to-pay, container logistics,
order-to-cash, inventory management, manufacturing maintenance) import in JSON
and SQLite form.

- A missing relationship or attribute list means none.
- Times without an offset are read as UTC, as the SQLite format stores them.
- A `null` attribute value, or a SQLite change row naming a field with no value,
  records nothing. SQLite number columns holding `'null'`, `''` or numeric text
  are read exactly; any other text in a typed column is still an error.
- A declared-string value written as a bare number keeps its exact JSON text.
- A SQLite object with no row in its type table, or a type table without
  `ocel_changed_field`, simply has no attributes or no single-field changes.
- A link to an object the file never declares is left out and reported in the data
  checks as "Links to objects missing from the file", with the count and examples.
- The same attribute value repeated at one instant is a warning; different values
  at one instant still block analysis.

Undeclared events or types, unknown elements and values that contradict their
declared type remain errors.

## Following one item type

To ask the questions asked of any process (how long, which route, where it is
slow, which items stand out) about one kind of item, follow it: each order, say,
becomes a case made of every recorded step that involves it, in time order, with
its details (for example an order value) as case fields. The result runs through
the same case engine as a single log, so every figure means the same thing.

- CLI: `glasstarn workspace query <project> --request follow.json` with
  `{"type":"follow","object_type":"order"}` returns the following summary and the
  full case analysis.
- MCP: `follow_ocel_object_type` returns the aggregate figures.
- Amounts of the followed items:
  `{"type":"values","object_type":"invoice","field":"invoice_amount"}`; see
  [amounts](process-mining.md#amounts).

Item types keep their source names (for example `opportunity`). To show a
business name instead, run
`glasstarn workspace name-type <project> --type opportunity --name Deals`;
it changes labels only, never the analysis.

The **Process** view defaults to a subway map of the connected object types.
The **Variants** view ranks routes for the followed type, for example “Variants
of Orders”; it does not flatten different object types into one global trace.
Selecting a route shows its exact directly-follows flow. **Show in Process**
highlights the followed line and matching activities in the subway while
retaining the surrounding linked-record context and active project scope.

Important linked-record milestone findings appear in **Overview**, with named
endpoints, shared-record denominators and affected-item drill-down. Their
intervals can include intervening work. Missing later events can indicate open
work or incomplete source coverage, rather than a proven process failure.

Following is a projection and discloses it: steps that involve no item of the
type are left out and counted, and a step shared by several items of the type
appears once in each of their cases, with the number of extra copies reported.

## Native analysis

The object-centric workflow provides:

- lifecycle directly-follows graphs per object type;
- object-type variants with simultaneous activities kept as unordered layers;
- activity/object participation and shared-event interactions;
- explicit qualified object relationships;
- time between activities through genuine common objects;
- synchronization decomposition for multi-object events;
- separate lifecycle-elapsed and convergence-spread bottleneck rankings;
- native typed object-centric Petri-net discovery;
- typed token replay and optimal object-lifecycle alignment;
- event- and object-linked evidence.

Equal-time lifecycle events are not placed into an invented order. Analyses
that require such an order reject the ambiguous lifecycle instead.

## Filtering

Filters may retain selected object types, event-to-object qualifiers, and
object-to-object qualifiers. Shared events remain atomic even when they connect
several retained objects. Original identities are preserved for evidence.

## Explicit case projection

When a traditional case view is required, select one object type explicitly.
The projection reports:

- source events excluded from the selected case notion;
- projected event occurrences versus atomic source events;
- convergence where one event appears in several projected cases;
- evidence-backed divergence candidates where flattening can split object
  behavior.

The projected traces can support familiar case-centric reasoning, but the
distortion report remains part of the result.

## CLI examples

```sh
# Analyze without flattening
glasstarn ocel order-management.jsonocel --json

# Focus on one or more object types
glasstarn ocel order-management.sqlite \
  --object-type order \
  --object-type item \
  --json

# Preview an explicit case notion
glasstarn ocel order-management.xmlocel \
  --project-as order \
  --case-limit 10

# Discover and save a native baseline
glasstarn ocel training.jsonocel \
  --discover-model \
  --json > baseline.ocelmodel.json

# Check a later log against it
glasstarn ocel current.jsonocel \
  --conformance-model baseline.ocelmodel.json \
  --conformance-algorithm alignment
```

## Linked interval analysis and map controls

Analysis retains whole-history timing for the followed object type and adds
**Between linked objects**. Choose a counted root type, a source activity/type,
a target activity/type, and first/last occurrence independently. Each active
root is counted once. Last delivery means the latest matching event across all
directly related deliveries, including split deliveries.

Endpoint correlation uses an explicit direct object-to-object relation or a
shared event. It does not traverse a common customer or vendor to connect
otherwise unrelated orders. Roots without an eligible endpoint remain in the
population denominator but have no measured interval. Root attributes supply
grouping; they are not silently replaced by attributes from a related object.
SQLite sources use disk-backed joins; smaller JSON/XML sources use the native
object view. Both use the same interval and reporting definitions.

Applying a linked range retains the selected roots and their recorded endpoint
relationships. Following another type can retain that linked selection; reset
it explicitly to return to the full native population. Related records retain
their histories. This is a selected-record scope, not a claim that every event
or amount belongs solely to the originating root.

The connected Process map defaults to a vertical subway layout. Objects &
Events controls sit in a collapsible panel beside the map canvas. The toolbar
opens or closes it without covering the diagram. The panel exposes type
visibility, activity-label visibility and consistent object colors; horizontal
orientation and routing discovery remain available under Map options. Hiding a
label keeps a muted station so the display does not invent adjacency. The schematic still
uses each type's most common route and discloses omitted types; use Variants for
exact recorded routes.

Routing markers are optional and derived from a discovered typed model:
diamonds denote exclusive choice and bars denote parallel split/join within one
object type. Cross-type synchronization alone is not parallel branching.
Markers without a unique neighboring visible activity are omitted. These are
inferred model structures, not proof of business rules, and are cleared when
the population changes. The observed directly-follows graph alone does not
justify routing semantics.

The MCP tool `explore_linked_interval` and workspace query `linked_interval`
accept `interval` with `root_type`, `from_type`, `to_type`, optional root
`group_by`, and `segment` endpoint/bound definitions. MCP output follows the
reviewed exposure policy and does not return internal selected-root filters.
