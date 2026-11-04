# Salesforce orders (checked on a real org)

Orders kept in Salesforce, for companies that do not hand orders to an ERP,
from creation to activation, with reduction orders and order products, followed
by order. Written from public knowledge of the standard Order, OrderItem and
OrderHistory objects; tested on made-up extracts (`pm-testgen`
`salesforce::write_orders`).

Files: Order, OrderHistory, OrderItem (optional). `extract.spec.example.json` is
a starting extract spec.

Steps: Create order, each order status (the company's own status names), Add
order product, Activate order, Create reduction order (reaches the order it
reduces). Activation comes from the order's activation date, so it appears even
without history tracking.

Run `glasstarn extract discover <project> --org <alias> --object Order --pack
salesforce-order-management` first to see whether the org tracks order status.

## Real-org check (2026-10-15)

Passed on a Salesforce Developer Edition org, with records made by hand and
pulled through the normal approved extract: 3 orders (one product activated, two products activated, left as draft): all found with every step in order, within two minutes; order totals match.

- Reduction orders were not checked: they are off in the test org.
- The extract user must be able to see Order.OpportunityId; in the test org it was hidden until a permission set allowed it.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
