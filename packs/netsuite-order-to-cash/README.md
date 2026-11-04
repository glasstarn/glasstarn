# NetSuite order to cash (not yet tested)

Order to cash on Oracle NetSuite, from sales order to payment,
followed by sales order line. Written by the Glasstarn project from public
knowledge of NetSuite's SuiteQL tables; tested only on made-up extracts
(`pm-testgen` `netsuite::write`). Untested until confirmed on a real account.

| File | Contents |
|---|---|
| Transaction | SalesOrd, ItemShip, CustInvc and CustPymt transactions |
| TransactionLine | The item lines of the orders, with their transaction type |
| PreviousTransactionLineLink | Which receipt, bill or payment followed which order line or bill |
| SystemNote | Status and approval changes on the orders (optional) |

Steps: Create sales order, Status: each status the order passed (the system
note field is a question for the company), Ship goods, Create invoice,
Receive customer payment. Bills and payments reach every order line they cover.

There is no direct connection to NetSuite yet: an administrator runs the
SuiteQL queries (for example through SuiteAnalytics Connect or the REST query
service) and exports the four files, with times in UTC.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
