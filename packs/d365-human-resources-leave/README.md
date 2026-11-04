# Dynamics 365 Human Resources leave requests

Request creation and recorded workflow actions, followed by leave request.
Live demo validation covered an open draft and a separate request submitted
then canceled through the requester workflow. Four events, two requests and
four links matched independently recorded source timestamps, IDs, hashes and
rows. The configured automatic approval had not completed before cancellation;
manual approval/rejection remains untested. Literal regression tests also cover
repeated workflow submissions, timezone offsets and creation-only extracts.

Only `LeaveRequest.RecId` and `CreatedDateTime` are needed for creation events.
Request numbers, current status, classification and company are optional
context. Worker names, health reasons, comments and attachments are unnecessary.
Export one legal entity at a time and preserve original UTC instants. For a
static Excel export, verify the exporting user's display timezone first.

Optional `WorkflowTrackingTable` input adds submission, approval, rejection,
request-change, delegation, escalation and cancellation events. Preserve every
tracking row and workflow instance. Join through `WorkflowTrackingStatusTable`
to obtain `LeaveRequestRecId`, checking both the document table and company.
Do not assume a workflow document record belongs to a leave request based only
on a matching record number. Table-browser Excel exports can display the workflow
reference as its instance number; resolve that representation through
`InstanceNumber` instead of treating it as a raw status-table record ID.

Planned leave dates describe the absence, not when the request was submitted or
approved. `RequestDate` is a date-only submission display and is not used when
actual workflow history can be exported. `modifiedDateTime` and current status
are not a substitute for approval or cancellation history. With no history,
the pack emits creation alone and does not invent later steps.

Microsoft documents [request submission and draft saving](https://learn.microsoft.com/en-us/dynamics365/human-resources/hr-employee-self-service-request-time-off),
[automatic versus workflow approval](https://learn.microsoft.com/en-us/dynamics365/human-resources/hr-employee-self-service-manage-requests),
and the [leave request workflow](https://learn.microsoft.com/en-us/dynamics365/human-resources/hr-leave-and-absence-workflow).
Approval configuration is a source-system responsibility; the pack does not
change who can approve requests. This scope does not cover recruiting, payroll,
compensation or every Human Resources workload.

Product names are trademarks of their owners and identify the source system.
Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
