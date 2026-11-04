# SAP Concur expense reports (not yet tested)

Expense reports in SAP Concur Expense, from the first expense to paying the
employee back, followed by expense report. Written by the Glasstarn
project from public knowledge of Concur's expense reports, entries and approval
workflow; tested only on made-up extracts (`pm-testgen` `concur::write_expenses`).
Untested until confirmed on a real Concur site.

| File | Contents |
|---|---|
| Report | Expense reports: totals, policy, approval and payment status |
| Entry | The expenses on them (no descriptions or receipts) |
| WorkflowAction | The approval history: submit, approve, send back, recall |

Steps: Incur expense, Create expense report, each workflow action as the site
names it (for example Submit report, Approve (manager), Send back to employee,
Approve (expense processor)), Pay employee.

There is no direct connection to Concur yet: an administrator exports the three
files (for example through the Concur Expense API or a scheduled report), with
times in UTC.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
