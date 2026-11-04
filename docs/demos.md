# Demo projects on invented data

Four ready-made projects you can open in a minute. The data is invented and
generated on your machine when you ask for it: no file is downloaded, and no
real company, person or system is behind it. Every run gives the same data, so
these numbers match on every machine.

```sh
glasstarn workspace demo purchase-to-pay
glasstarn workspace demo order-to-cash
glasstarn workspace demo connected
glasstarn workspace demo sales-pipeline
```

Then open the app with `glasstarn serve`. You can also ask your agent: "set up
the purchase-to-pay demo and tell me what's slow". The agent runs the same
command.

Each demo hides one or two problems for you to find.

| Demo | Size | Find this |
|---|---|---|
| **Purchase to pay** | 1,500 orders | Orders over 25,000 wait 6–14 days for one approver. 14 invoices are paid twice. One vendor's invoices often fail the price match and need a price change. |
| **Order to cash** | 1,200 orders | New customers wait days on a credit hold, and some cancel. The Reno warehouse ships 4–7 days after packing; the others ship the same or next day. |
| **Order to cash, connected** | 1,500 orders from 300 customers, with their deliveries, invoices and payments | Linked records for the subway map, as messy as a real extract: credit checks and blocks, changed, rejected and cancelled orders, one to three deliveries per order with blocks, second picks, failed deliveries and returns, invoices per order or per delivery (some raised early), invoice blocks, disputes, credit memos, partial payments and unpaid invoices. Retail customers pay weeks later than the others, many after one or two reminders. |
| **Sales pipeline** | 1,600 leads | Webinar leads need several contact attempts and stall before qualification. Deals over 50,000 go round discount approval up to three times. |

For a real company's data, use the [five-minute demo](demo-bpic2019.md) on the
public BPI Challenge 2019 purchasing log. You download that file yourself.
