# Quote approval (HubSpot) (trial tested)

Follows quotes through approval: each quote status (draft, pending approval, approved, rejected, approval not needed) and changes to the expiry date.

Steps: Create quote, Quote: each status, Move expiry date.

Written from public knowledge of HubSpot's CRM API (v3 objects with property
history, pipelines and property options for labels), with generated regression
tests (`pm-testgen` `hubspot`) and the scoped real-account trial below.

The connection is a HubSpot service key (Settings, Integrations, Service Keys; a private app token on older accounts) with read access to
quotes, typed into the app's environment as `HUBSPOT_TOKEN` and never
stored. File the extract with `glasstarn extract hubspot <project> --spec <file>`,
starting from `extract.spec.example.json`.

## Real-account trial

Tested by the Glasstarn project on a HubSpot developer test account on 2026-10-16.

Five planted legacy quotes passed check and build, producing 15 events. Every complete timeline matched independent HubSpot status histories and exact timestamps. One quote used HubSpot's real Approve and create review action. The rejection, revision and resubmission paths used API status changes; that is not a test of the review-screen Request changes action. Saved rework analysis retained the rejection/resubmission loops.

These paths use the legacy `hs_status` property and the legacy approval workflows. The newer CPQ quote system's properties and approval configuration are outside this test. An approved legacy quote generates a quote web page. The trial used synthetic $0 quotes; it did not test acceptance, signatures or payment collection. Live expiry-date changes were not exercised.

Source CSV values and all property-history rows were reconciled separately from
the process map. Source fingerprints and relationship lineage remained intact,
and the saved project reopened and reanalysed successfully. Very long histories
may be trimmed by HubSpot; this small trial does not establish retention,
large-account capacity or transactional consistency between separate API reads.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
