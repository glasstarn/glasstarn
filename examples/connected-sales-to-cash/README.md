# Fictional connected sales-to-cash trial

These five tiny CSVs use invented schemas and contain no customer data. They are
an offline preparation example, not a vendor extraction pack.

The scenario has one opportunity, three orders, three invoices and three payment
records. One payment covers two invoices; another invoice has two payment records.
Orders and invoices reuse numbers across companies, and numbers retain leading
zeros. Expected: 10 objects, 10 atomic events, 10 object relationships and 20
event-to-object relationships. Linking a payment to several invoices must not
multiply the payment event.

Create a project, inventory all five CSVs, and replace each `inputs[].sha256` in
`recipe.template.json` with the hash returned for that named table. Save the filled
recipe separately as `recipe.json`, then run:

```sh
glasstarn workspace prepare-ocel <project-id> --recipe recipe.json --json
```

Use [the connected preparation guide](../../docs/connected-preparation.md) for the
inventory command, mapping contract and interpretation limits. The automated CLI
trial fills real inventory hashes and tests successful construction, reproducible
output, missing references, duplicate keys/links and retention of the prior active
analysis on a blocked attempt. No SaaS credentials or live calls are needed.
