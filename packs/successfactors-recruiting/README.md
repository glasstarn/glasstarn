# SAP SuccessFactors hiring (not yet tested)

Hiring in SAP SuccessFactors Recruiting, from the job requisition to the hired
candidate, followed by job application. Written by the Glasstarn
project from public knowledge of the Recruiting OData entities; tested only on
made-up extracts (`pm-testgen` `successfactors::write_recruiting`). Untested
until confirmed on a real instance.

| File | Contents |
|---|---|
| JobRequisition | Job requisitions: department, location, openings |
| JobApplication | Applications (no names, contact details or resumes) |
| JobApplicationStatusAuditTrail | Each application's status changes, with the status label |
| JobApplicationInterview | Interviews (optional) |
| ONB2Process | Onboarding 2.0 of each hire (optional) |

Steps: Open job requisition, Receive application, each application status as
the instance names it (for example Phone Screen, Interview, Offer, Hired,
Rejected), Interview candidate, Start onboarding, Complete onboarding, Close job
requisition. Following the requisition instead shows time to fill.

Known limit: requisition approval is not covered, because SuccessFactors does
not expose the approval date through its API (SAP's own workaround is a custom
date field filled by a rule; if a company has one, it can be added as a step).

There is no direct connection to SuccessFactors yet: an administrator exports
the files through the OData API, with times in UTC.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
