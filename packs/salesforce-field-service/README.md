# Salesforce field service (checked on a real org)

Technician visits in Salesforce Field Service, from the work order to the work
done on site, followed by work order. Written from public knowledge of the
standard WorkOrder, ServiceAppointment and AssignedResource objects and their
field history; tested on made-up extracts (`pm-testgen`
`salesforce::write_field_service`).

Files: WorkOrder, WorkOrderHistory, ServiceAppointment,
ServiceAppointmentHistory (optional), AssignedResource (optional).
`extract.spec.example.json` is a starting extract spec.

Steps: Create work order, Work order: each status, Create appointment,
Appointment: each status (for example Scheduled, Dispatched, In Progress,
Completed), Assign technician, Start work on site, Finish work on site. Each
appointment keeps its scheduled start and arrival window, so late arrivals can
be found. Following the appointment instead shows each visit on its own.

## Real-org check (2026-10-15)

Passed on a Salesforce test org, with records made through the API and pulled
through the normal approved extract: 6 work orders with 7 site visits and a technician on each (straight through, a failed visit and a second one, cancelled, on hold, rescheduled, still open): every step found in order, within two minutes; work started and finished on site come from the appointment's actual start and end times.

- Field Service had to be switched on in the org, with field history on the work order and appointment Status fields.
- Each work order's trace includes the steps of its appointments, so a revisit shows both visits.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
