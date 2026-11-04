# Dynamics 365 Project Operations: project time and approvals

Project and time-entry creation, individual approval requests, and their latest
approved or rejected decision. The case is a project, with distinct time-entry
and approval-request objects. Live Core validation covered a retained draft and a
separate entry submitted, rejected, resubmitted and approved. All eight events,
six objects and fourteen links matched independent source facts and raw rows.

Only project ID and creation time are mandatory. Time entries and approvals are
optional; context such as names, resources, duration and current status does not
block construction. Rename Dataverse lookup properties `_msdyn_project_value`
and `_msdyn_timeentry_value` to their logical names without changing GUIDs.

Export every retained time-approval row, including earlier rejected instances.
Do not export only the current pending queue or the grouped approval-header UI.
Distinct request GUIDs preserve resubmissions where the source retains them.
`createdon` establishes request creation, which may follow the submission action;
it is not a complete audit of all time-entry status transitions.

Despite its name, `msdyn_approvedon` is populated for rejection as well as approval.
Interpret it with `msdyn_recordstage` (Rejected/1 or Approved/2), never as unconditional
proof of approval. Preserve raw UTC values: a date-only form display can hide
second-level timestamps. If an export truly supplies only dates, declare that
precision and the source timezone rather than inventing a time of day.

Each row represents its latest decision. Recall or later edits can change its
stage or decision timestamp; full recall/reopen history requires audit evidence.
Pending, submitted and recall states do not create approval/rejection events.
Missing decision fields omit those events. Work dates, planned project dates and
`modifiedon` do not manufacture transitions. Request and entry objects connect
through the project; the optional time-entry GUID is retained on request context.

This pack covers project time approval only. Expense approvals use the separate
[project expenses pack](../d365-project-operations-expenses/README.md). Resource
scheduling, project contracts, proforma invoices and ERP project accounting need
separate source contracts and validation. Exclude customers, descriptions, comments,
receipts, addresses and other personal details from mining inputs.

Sources: [approval lifecycle](https://learn.microsoft.com/en-us/dynamics365/project-operations/approvals/approvals-overview),
[approval views and grouping](https://learn.microsoft.com/en-us/dynamics365/project-operations/approvals/updated-approvals),
[Project Approval schema and Record Stage values](https://learn.microsoft.com/en-us/common-data-model/schema/core/applicationcommon/foundationcommon/crmcommon/projectcommon/projectserviceautomation/projectapproval),
[Time Entry schema](https://learn.microsoft.com/en-us/common-data-model/schema/core/applicationcommon/foundationcommon/crmcommon/projectcommon/projectserviceautomation/timeentry).

Product names identify the compatible source system. Glasstarn is not affiliated
with or endorsed by Microsoft; see [trademarks](../../docs/trademarks.md).
