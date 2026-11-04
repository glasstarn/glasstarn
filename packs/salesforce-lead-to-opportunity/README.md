# Salesforce lead to opportunity (checked on a real org)

Leads, their status and owner history, conversion, and the opportunities they
became with stage, amount and close-date changes, followed by lead. Written from
public knowledge of Salesforce's standard objects; tested on made-up
extracts (`pm-testgen` `salesforce::write_leads`).

Files, as the Salesforce extract writes them: Lead, LeadHistory, Opportunity,
OpportunityFieldHistory. `extract.spec.example.json` is a starting extract spec.

Run `glasstarn extract discover <project> --org <alias> --object Lead
--object Opportunity --pack salesforce-lead-to-opportunity` first: it lists the
company's own fields and says which history the pack needs that the org does
not track (Lead: Status, Owner; Opportunity: StageName, Amount, CloseDate).
History that was never tracked cannot be recovered.

Steps: Create lead, each lead status (the company's own status names), Reassign
lead, Convert lead, Create opportunity, each stage, Win or Lose opportunity,
Change amount, Move close date. Conversion is a date only (in the org's time
zone), so it sits at midnight of that day.

## Real-org check (2026-10-15)

Passed on a Salesforce Developer Edition org, with records made by hand and
pulled through the normal approved extract: 5 leads (converted, rejected, sent back to open, closed at once, still open) and 4 opportunities (won, lost, moved back a stage, a contract renewal): all 9 found with every step in order, within two minutes of when it was made; amounts match.

- Reassign lead: a lead handed to a queue and taken back shows two Reassign lead steps, in order.
- Fixed by this check: Convert lead now takes its exact time from the lead's history (the "leadConverted" row). It used the date-only ConvertedDate, which put the conversion at the start of the day, before the lead was even created.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
