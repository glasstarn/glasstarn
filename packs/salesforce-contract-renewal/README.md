# Salesforce contracts and renewals (checked on a real org)

Contracts on Salesforce Sales Cloud, from creation and activation to the renewal
deal, followed by contract. Written from public knowledge of the standard
Contract object, its field history, and opportunities linked to a contract;
tested on made-up extracts (`pm-testgen` `salesforce::write_contracts`).

Files: Contract, ContractHistory, Opportunity (with ContractId) and
OpportunityFieldHistory. `extract.spec.example.json` is a starting extract spec.

Steps: Create contract, Contract: each status, Activate contract, and the steps
of each renewal opportunity (Create opportunity, its stages, Win or Lose
opportunity, Change amount, Move close date) carried onto the contract it
renews. Each contract keeps its start and end dates, so renewals started late
or after the end date can be found.

Known limit: renewals made with the Salesforce CPQ managed package are not yet
covered.

## Real-org check (2026-10-15)

Passed on a Salesforce Developer Edition org, with records made by hand and
pulled through the normal approved extract: 3 contracts (activated straight away, through approval, sent back to draft and resubmitted) and a renewal opportunity: all found, steps in order, within two minutes.

- Salesforce does not record a contract going back from In Approval Process to Draft. A sent-back contract shows as two "Contract: In Approval Process" steps, which still counts as rework.
- The extract user must be able to see Opportunity.ContractId; in the test org it was hidden until a permission set allowed it.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
