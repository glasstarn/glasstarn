# Salesforce CPQ quote to renewal (checked on a real org)

Quotes made with the Salesforce CPQ managed package (Revenue Cloud), from the
quote through approval, order and contract to the renewal deal, followed by
quote. Written from public knowledge of the CPQ objects (SBQQ namespace) and
their links; tested on made-up extracts (`pm-testgen`
`salesforce::write_cpq`).

Files: Opportunity (with SBQQ__RenewedContract__c) and OpportunityFieldHistory,
SBQQ__Quote__c, SBQQ__Quote__History, Order (optional), Contract (optional).
`extract.spec.example.json` is a starting extract spec.

Steps: the opportunity's steps, Create quote, Quote: each status (for example
In Review, Approved, Denied, Presented, Accepted), Create order, Activate
order, Create contract, Activate contract, and the renewal deal's steps carried
onto the contract and the quote it came from.

For standard Salesforce quotes, use `salesforce-quote-approval`; for standard
contracts and renewals, `salesforce-contract-renewal`.

## Real-org check (2026-10-15)

Passed on a Salesforce test org, with records made through the API and pulled
through the normal approved extract: a Trailhead CPQ org: 6 quotes (accepted, ordered and contracted; denied; rejected by the customer; sent back to draft; ordered only; still draft): every planted step found in order, within two minutes.

- CPQ writes a "Quote: Draft" status row right after a quote is created, so most quotes start with Create quote then Quote: Draft.
- CPQ made no contract for a quote with only one-time products; the contract in the check was made by hand with the quote and order set, as CPQ sets them for subscriptions. Contracts made by CPQ itself are not yet checked.
- Fixed by this check: tables with underscores in their names (SBQQ__Quote__c) were not matched to their files; an empty field-history table (the org tracked no opportunity fields) no longer stops the build.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
