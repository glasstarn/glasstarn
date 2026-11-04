# SAP Sales Cloud in front of SAP order to cash (not yet tested)

Opportunities and quotes from SAP Sales Cloud report exports, joined to the SAP
sales order by the ERP sales order number on the quote, then the same order to
cash as the SAP pack behind it (`sap-order-to-cash`).

| File | Contents |
|---|---|
| SalesCloudOpportunity | Opportunity ID, Name, Account ID, Created On, Status, Closed On |
| SalesCloudQuote | Quote ID, Opportunity ID, Created On, Submitted On, Approved On, Sent On, Accepted On, ERP Sales Order ID, Status |
| SAP files | As in the `sap-order-to-cash` pack |

Steps by sales order line: Create opportunity, Create quote, Submit quote for
approval, Approve quote, Send quote to customer, Customer accepts quote, Win
opportunity, then the SAP steps. Lost opportunities stay on their own. The data
check asks about SAP orders entered directly in SAP.

Written from public knowledge; tested only on made-up files
(`sap-p2p-sample <dir> --o2c --sales-cloud`). Untested until confirmed.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
