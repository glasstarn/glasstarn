# Dynamics 365 Customer Service: case handling

Support cases in Dynamics 365 Customer Service, from creation through queues,
activities and escalation to resolution, followed by case. Tested on a live
Customer Service Enterprise trial on 2026-10-17 UTC: five synthetic cases covered
an open case with a task, queue routing then resolution, escalation then
resolution, cancellation, and resolution followed by reopening and a second
resolution. All 13 events matched their source case, step, UTC time and source
row. Saved-project reanalysis also passed. Generated extracts (`pm-testgen`
`dynamics::write_customer_service`) provide additional offline coverage.

Files: incident, queueitem (optional), incidentresolution (optional),
activitypointer (optional; no subjects or bodies).

Steps: Create case, Route to queue: each queue, Activity: each activity type,
Escalate case, Resolve case, Cancel case. Known limits: a cancelled case is
timed by its last change; status reason changes are not read yet.

Export all case resolution rows, including resolutions whose current state is
Canceled after reopening. Their original `createdon` preserves the earlier
resolution; `modifiedon` and `actualend` can instead reflect reopening. The pack
retains repeated resolutions but has no standalone reopen step without audit
history. A queue-item snapshot also cannot establish every previous queue move.

Restrict the activity export to phone calls, emails, appointments and tasks.
Exclude Case Resolution activities, which are already read from
`incidentresolution`, to avoid counting resolutions twice. Only task activities
were exercised live; phone, email and appointment execution is not yet tested.
Export choice labels and queue names as requested by the pack, and retain raw
UTC timestamps when converting Dataverse extracts to CSV.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
