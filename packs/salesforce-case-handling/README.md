# Salesforce support case handling (checked on a real org)

Cases with their status, owner, priority and escalation history, and the emails
in and out on each case, followed by case. Written from public knowledge of
Salesforce's standard Case, CaseHistory and EmailMessage objects; tested on made-up extracts (`pm-testgen` `salesforce::write_cases`).

Files: Case, CaseHistory, EmailMessage (optional; ids, direction and time only,
no subjects or bodies). `extract.spec.example.json` is a starting extract spec.

Run `glasstarn extract discover <project> --org <alias> --object Case --pack
salesforce-case-handling` first to see which needed history (Status, Owner,
Priority) the org tracks.

Steps: Create case, each status (the company's own status names), Reassign
case, Change priority, Escalate case, Receive customer email, Email the
customer. Owner changes, which Salesforce records twice (ID and name), count
once.

## Real-org check (2026-10-15)

Passed on a Salesforce Developer Edition org, with records made by hand and
pulled through the normal approved extract: 5 cases (closed, escalated, reopened, priority raised, still open): all found with every step in order, within two minutes of when it was made.

- A case handed to a queue and back, with an email from the customer and a reply: Reassign case, Receive customer email and Email the customer all appear in order.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
