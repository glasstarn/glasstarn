# nCino loan origination (not yet tested)

Loans in nCino on Salesforce, from creation through underwriting and credit
approval to closing and booking, followed by loan. Written from public
knowledge of nCino's loan object, Salesforce field history and approval
processes; tested only on made-up extracts (`pm-testgen`
`salesforce::write_ncino`). Untested until confirmed on a real org.

nCino runs inside Salesforce, so the Salesforce discovery and extract are
reused. nCino's object reference is not public: run discovery first to confirm
the field names (newer orgs may use `nCino__` instead of `LLC_BI__`).

Files: LLC_BI__Loan__c, LLC_BI__Loan__History, ProcessInstance and
ProcessInstanceStep (optional). `extract.spec.example.json` is a starting
extract spec.

Steps: Create loan, Stage: each stage (the bank's own stage names, for example
Qualification, Proposal, Underwriting, Credit Approval, Processing, Doc Prep,
Closing, Booked), Status: each status (for example Declined, Withdrawn),
Change loan amount, Submit for credit approval, Approve loan, Decline loan,
Recall approval request.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
