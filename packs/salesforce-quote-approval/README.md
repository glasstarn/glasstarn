# Salesforce quotes and approvals (checked on a real org)

Quotes on Salesforce Sales Cloud, from the opportunity to the order, with their
approvals, followed by quote. Written from public knowledge of the standard
Quote, ProcessInstance, ProcessInstanceStep and Order objects; tested on made-up extracts (`pm-testgen` `salesforce::write_quotes`).

Files: Opportunity and OpportunityFieldHistory (as in lead to opportunity),
Quote, ProcessInstance, ProcessInstanceStep and Order (optional).
`extract.spec.example.json` is a starting extract spec.

Steps: the opportunity's steps, Create quote, Submit quote for approval, Approve
quote, Reject quote, Recall approval request, Create order. Following the
opportunity instead shows every quote tried on one deal.

Known limit: quotes of the Salesforce CPQ managed package (SBQQ__Quote__c) are
not yet covered.

## Real-org check (2026-10-15)

Passed on a Salesforce Developer Edition org with quotes enabled and a
one-step approval process on Quote, pulled through the normal approved
extract: 4 quotes (approved and turned into an order, rejected, recalled,
rejected then resubmitted and approved), each with every step in order,
within two minutes of when it happened.

- Quotes must be switched on in the org, and approvals need an approval
  process on Quote; without one there are no approval steps to find.
- The extract user must be able to see Order.QuoteId and Order.OpportunityId;
  in the test org they were hidden until a permission set allowed them.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
