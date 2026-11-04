"""Writes the invented data for the Salesforce approval demo.

Deterministic: the same files every run. No real company, person or record.

Salesforce side: what an org would return for the demo spec's queries (window
January to March 2026). The scans list items with activity in the window; the
exports hold those items' records and their full stage history up to the
window end. ERP side: orders, invoices and payments for the won deals, as an
ERP export taken later would show them.
"""

import csv
import random
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
START = datetime(2026, 1, 1, tzinfo=timezone.utc)
END = datetime(2026, 4, 1, tzinfo=timezone.utc)
ERP_AS_OF = datetime(2026, 5, 15, tzinfo=timezone.utc)
rng = random.Random(20261010)

STAGES = ["Prospecting", "Qualification", "Proposal/Price Quote", "Negotiation/Review"]
NAMES = ["Northwind", "Bluefin", "Larkspur", "Copperleaf", "Halcyon", "Meridian", "Tidewater",
         "Granite Peak", "Silverline", "Oakridge", "Brightwater", "Fernhill", "Ironbark", "Sunfield"]
DEALS = ["renewal", "expansion", "new platform", "pilot", "upgrade", "services"]


def stamp(at):
    return at.strftime("%Y-%m-%dT%H:%M:%S.000Z")


def iso(at):
    return at.strftime("%Y-%m-%dT%H:%M:%SZ")


def work_hours(at):
    return at.replace(hour=rng.randint(8, 17), minute=rng.choice([0, 15, 30, 45]), second=0)


def write(path, header, rows):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", newline="") as handle:
        out = csv.writer(handle, lineterminator="\n")
        out.writerow(header)
        out.writerows(rows)


opportunities, history = [], []
history_id = 0
for number in range(1, 451):
    opp_id = f"006D{number:04d}"
    created = work_hours(datetime(2025, 11, 1, tzinfo=timezone.utc) + timedelta(days=rng.randint(0, 140)))
    amount = rng.choice([5, 8, 12, 18, 25, 40, 60, 90, 150, 240]) * 1000 + rng.randint(0, 99) * 10
    at = created
    stage = "Prospecting"
    steps = []
    path = STAGES[1:]
    # Some deals go back to proposal after negotiation.
    if rng.random() < 0.15:
        path = path + ["Proposal/Price Quote", "Negotiation/Review"]
    outcome = "Closed Won" if rng.random() < 0.62 else "Closed Lost"
    stop_early = rng.random() < 0.18
    for next_stage in path:
        if stop_early and rng.random() < 0.5:
            outcome = None
            break
        at = work_hours(at + timedelta(days=rng.randint(2, 12)))
        steps.append((at, stage, next_stage))
        stage = next_stage
    closed = None
    if outcome:
        at = work_hours(at + timedelta(days=rng.randint(1, 10)))
        steps.append((at, stage, outcome))
        closed = at
    # The org is read at the window end: later changes have not happened yet.
    steps = [step for step in steps if step[0] < END]
    if created >= END:
        continue
    for when, old, new in steps:
        history_id += 1
        history.append([f"017H{history_id:05d}", opp_id, stamp(when), "StageName", old, new])
    won = bool(steps) and steps[-1][2] == "Closed Won"
    close_date = steps[-1][0].strftime("%Y-%m-%d") if steps and steps[-1][2].startswith("Closed") else ""
    name = f"{rng.choice(NAMES)} {rng.choice(DEALS)}"
    opportunities.append({"Id": opp_id, "Name": name, "CreatedDate": stamp(created), "CloseDate": close_date,
                          "Amount": str(amount), "won_at": steps[-1][0] if won else None, "created": created})

in_window = lambda when: START <= when < END
active = {o["Id"] for o in opportunities if in_window(o["created"])}
active |= {row[1] for row in history if in_window(datetime.strptime(row[2], "%Y-%m-%dT%H:%M:%S.000Z").replace(tzinfo=timezone.utc))}
scope = [o for o in opportunities if o["Id"] in active]

orders, invoices, payments, bridge = [], [], [], []
order_number = invoice_number = payment_number = 0
open_invoices = []
for deal in scope:
    if not deal["won_at"] or rng.random() < 0.08:
        deal["ERP_Order_ID__c"] = ""
        continue
    order_number += 1
    order_id = f"{order_number + 4500:08d}"
    deal["ERP_Order_ID__c"] = order_id
    company = rng.choice(["1000", "2000"])
    handover = rng.randint(1, 6) if rng.random() < 0.8 else rng.randint(12, 35)
    ordered = work_hours(deal["won_at"] + timedelta(days=handover))
    if ordered >= ERP_AS_OF:
        continue
    amount = int(deal["Amount"])
    orders.append([company, order_id, deal["Id"], iso(ordered), str(amount)])
    if rng.random() < 0.06:
        continue
    parts = [amount] if amount < 50000 or rng.random() < 0.6 else [amount // 2, amount - amount // 2]
    issued = ordered
    for part in parts:
        issued = work_hours(issued + timedelta(days=rng.randint(2, 14)))
        if issued >= ERP_AS_OF:
            break
        invoice_number += 1
        invoice_id = f"9{invoice_number:07d}"
        invoices.append([company, invoice_id, order_id, iso(issued), str(part)])
        open_invoices.append((company, invoice_id, issued, part))

rng.shuffle(open_invoices)
waiting = []
for company, invoice_id, issued, part in open_invoices:
    late = rng.random() < 0.25
    paid = work_hours(issued + timedelta(days=rng.randint(35, 70) if late else rng.randint(8, 30)))
    if paid >= ERP_AS_OF or rng.random() < 0.08:
        continue
    waiting.append((paid, company, invoice_id))
waiting.sort()
index = 0
while index < len(waiting):
    paid, company, invoice_id = waiting[index]
    payment_number += 1
    payment_id = f"PAY-{payment_number:05d}"
    payments.append([payment_id, iso(paid)])
    bridge.append([payment_id, company, invoice_id])
    # A customer sometimes pays two invoices at once.
    if index + 1 < len(waiting) and waiting[index + 1][1] == company and rng.random() < 0.15:
        index += 1
        bridge.append([payment_id, company, waiting[index][2]])
    index += 1

fields = ["Id", "Name", "CreatedDate", "CloseDate", "Amount", "ERP_Order_ID__c"]
salesforce = HERE / "salesforce"
write(salesforce / "Opportunity.csv", fields, [[o[f] for f in fields] for o in scope])
write(salesforce / "OpportunityFieldHistory.csv", ["Id", "OpportunityId", "CreatedDate", "Field", "OldValue", "NewValue"],
      [row for row in history if row[1] in active])
write(salesforce / "scan-Opportunity.csv", ["Id"], [[o["Id"]] for o in opportunities if in_window(o["created"])])
write(salesforce / "scan-OpportunityFieldHistory.csv", ["Id", "OpportunityId"],
      [[row[0], row[1]] for row in history
       if in_window(datetime.strptime(row[2], "%Y-%m-%dT%H:%M:%S.000Z").replace(tzinfo=timezone.utc))])
erp = HERE / "erp"
write(erp / "orders.csv", ["company", "order_number", "opportunity_id", "created_at", "order_value"], orders)
write(erp / "invoices.csv", ["company", "invoice_number", "order_number", "issued_at", "invoice_amount"], invoices)
write(erp / "payments.csv", ["payment_id", "paid_at"], payments)
write(erp / "payment_invoices.csv", ["payment_id", "company", "invoice_number"], bridge)
print(f"{len(scope)} deals, {len(history)} stage changes, {len(orders)} orders, "
      f"{len(invoices)} invoices, {len(payments)} payments")
