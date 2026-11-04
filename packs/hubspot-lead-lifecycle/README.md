# Lead lifecycle (HubSpot) (trial tested)

Follows contacts from lead to customer: each lifecycle stage they reach (subscriber, lead, marketing qualified lead, sales qualified lead, opportunity, customer and the company's own stages), lead status changes and hand-overs between owners.

Steps: Create contact, Lifecycle: each stage reached, Lead status: each status, Reassign contact.

Written from public knowledge of HubSpot's CRM API (v3 objects with property
history, pipelines and property options for labels), with generated regression
tests (`pm-testgen` `hubspot`) and the scoped real-account trial below.

The connection is a HubSpot service key (Settings, Integrations, Service Keys; a private app token on older accounts) with read access to
contacts, typed into the app's environment as `HUBSPOT_TOKEN` and never
stored. File the extract with `glasstarn extract hubspot <project> --spec <file>`,
starting from `extract.spec.example.json`.

## Real-account trial

Tested by the Glasstarn project on a HubSpot developer test account on 2026-10-16.

Five planted contacts passed check and build, producing 17 events. Every record's complete timeline matched independently retrieved HubSpot property histories, including exact timestamps and a New → In Progress → New lead-status loop. A first lead-status assignment made after creation was retained as an event.

Initial property values recorded at creation are represented by Create contact and retained in source history. They are not counted again as changes. In this trial, a direct backward lifecycle write returned success while leaving the stored stage unchanged; no event was invented for that unpersisted move.

Source CSV values and all property-history rows were reconciled separately from
the process map. Source fingerprints and relationship lineage remained intact,
and the saved project reopened and reanalysed successfully. Very long histories
may be trimmed by HubSpot; this small trial does not establish retention,
large-account capacity or transactional consistency between separate API reads.
Owner handovers were not exercised live because the account had one owner.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
