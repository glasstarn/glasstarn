# Dynamics 365 Project Operations: expenses and approvals

Expense creation, individual approval-request creation and the latest approved or
rejected decision. The case is an expense, with separate request objects for
retained resubmissions. Live Core validation covered a synthetic draft submitted
and approved; all three events, two objects and five links matched independent
raw source timestamps, IDs, hashes and rows. Rejection/resubmission is covered by
a literal regression fixture, with no live claim for those expense paths.

Only expense ID and creation time are mandatory. Approval history and descriptive
context are optional. Export all retained approval rows for those expenses and
rename `_msdyn_expenseentry_value` to `msdyn_expenseentry` without changing GUIDs.
Exclude time/material approvals and grouped approval-header views. Preserve
original source exports and timestamps; do not substitute the expense transaction
date, current status or `modifiedon` for workflow events.

Despite its name, `msdyn_approvedon` is populated for rejection too. Interpret it
with `msdyn_recordstage` (Rejected/1 or Approved/2). Export raw UTC values: the
form can display a date while the raw field retains seconds. A genuinely date-only
export must retain that precision limit. Later recall or edits may replace a
row's stage/decision time; a full audit requires additional history. Missing
decision fields omit those events. Approved does not establish reimbursement,
payment, posting or customer billing.

Do not export receipts, employee names, purpose text, comments or payment details.
Project GUID and current expense status are optional context.

Sources: [expense schema](https://learn.microsoft.com/en-us/common-data-model/schema/core/applicationcommon/foundationcommon/crmcommon/projectcommon/projectserviceautomation/expense),
[approval schema and stage values](https://learn.microsoft.com/en-us/common-data-model/schema/core/applicationcommon/foundationcommon/crmcommon/projectcommon/projectserviceautomation/projectapproval),
[approval lifecycle](https://learn.microsoft.com/en-us/dynamics365/project-operations/approvals/approvals-overview).

Product names identify the compatible source system. Glasstarn is not affiliated
with or endorsed by Microsoft; see [trademarks](../../docs/trademarks.md).
