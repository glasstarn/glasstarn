# Deal pipeline (HubSpot) (trial tested)

Follows deals through their pipeline: each stage they enter (the company's own stages, including closed won and closed lost), amount changes, close dates moved and hand-overs between owners.

Steps: Create deal, Stage: each stage entered, Change amount, Move close date, Reassign deal.

Written from public knowledge of HubSpot's CRM API (v3 objects with property
history, pipelines and property options for labels), with generated regression
tests (`pm-testgen` `hubspot`) and the scoped real-account trial below.

The connection is a HubSpot service key (Settings, Integrations, Service Keys; a private app token on older accounts) with read access to
deals, typed into the app's environment as `HUBSPOT_TOKEN` and never
stored. File the extract with `glasstarn extract hubspot <project> --spec <file>`,
starting from `extract.spec.example.json`.

## Real-account trial

Tested by the Glasstarn project on a HubSpot developer test account on 2026-10-16.

Five planted deals passed check and build, producing 25 events. Every complete timeline matched independent HubSpot histories and exact timestamps. Saved analysis included two won deals, one lost deal, and the Qualified To Buy → Presentation Scheduled → Qualified To Buy loop. Amount and close-date changes were also checked; writing the same amount again did not add an event.

Source CSV values and all property-history rows were reconciled separately from
the process map. Source fingerprints and relationship lineage remained intact,
and the saved project reopened and reanalysed successfully. Very long histories
may be trimmed by HubSpot; this small trial does not establish retention,
large-account capacity or transactional consistency between separate API reads.
Owner handovers were not exercised live because the account had one owner.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
