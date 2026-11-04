# OCEL interoperability fixtures

`order-management.jsonocel` is a compact OCEL 2.0 JSON fixture covering typed
event and object declarations, one event associated with several object types,
qualified E2O and O2O relationships, and a time-varying object attribute. Its
event count must remain three: flattening it by object would duplicate events
and invalidate the fixture's purpose.

`order-management.xmlocel` expresses the same log in the official OCEL 2.0 XML
shape. Importing either fixture must preserve the same entities, typed values,
dynamic attribute history, and qualified relationships.
