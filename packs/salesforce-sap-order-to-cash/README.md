# Salesforce in front of SAP order to cash (not yet tested)

Opportunities, their stage history and quotes from Salesforce (the Salesforce
connector's extract or report exports), joined to the SAP sales order through
the field in which the company keeps the SAP order number; the agent asks which
field that is. Then the same order to cash as the SAP pack behind it
(`sap-order-to-cash`).

| File | Contents |
|---|---|
| Opportunity | Id, Name, AccountId, CreatedDate, StageName, IsClosed, IsWon, CloseDate, and the SAP order number field |
| OpportunityHistory | Id, OpportunityId, StageName, CreatedDate |
| Quote (optional) | Id, OpportunityId, Name, CreatedDate, Status |
| SAP files | As in the `sap-order-to-cash` pack |

Steps by sales order line: Create opportunity, each stage reached (the
company's own stage names), Create quote, Win opportunity, then the SAP steps.
Salesforce times are UTC. Company fields ending in `__c` are found and asked
about.

Written from public knowledge; tested only on made-up files
(`sap-p2p-sample <dir> --o2c --salesforce`). Untested until confirmed.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
