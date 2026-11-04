# Dynamics 365 Field Service: work orders and visits

Work-order creation, booking creation, booking-status timestamps and the latest
work-order closure. The case is a work order, with separate booking objects for
multiple visits. Live demo validation covered an unbooked work order and a second
order with two bookings: one scheduled, started, completed, restarted and completed
again; another scheduled then canceled. Eleven events, four objects and twenty
links matched independently recorded source timestamps and source-row lineage.
The canceled booking did not cancel the work order. This verifies distinct visit
records and repeated status history, not two completed physical visits.

The live test retained a history row whose creation time was one second after
Timestamp Time. The import preserved Timestamp Time. Microsoft warns against
reusing a completed booking; normal return visits should use separate bookings.

Export `msdyn_workorder`, its `bookableresourcebooking` records and their complete
`msdyn_bookingtimestamp` history. Bookings and history are optional: work-order
creation remains mineable when they are unavailable, with omitted steps reported.
History without the booking-to-order mapping cannot establish the correct case.

Booking history uses `msdyn_timestamptime` and the status stored on each timestamp
row. Distinct timestamp IDs preserve repeat visits and repeated statuses. It does
not infer history from the booking's current status or planned start/end dates.
A Completed booking does not establish that the work order is posted or invoiced.

`msdyn_timeclosed` records the latest closure only. It does not preserve all
work-order reopen/close cycles; those require audit history. Missing closure time
omits the closure event without inventing it from `modifiedon`. Booking timestamp
frequency is configurable, so the extract may omit some intermediate statuses.
Resource, status-source and other descriptive context are optional. Exclude
customer names, contact details, descriptions and addresses from these exports.

Sources: [booking timestamps](https://learn.microsoft.com/en-us/dynamics365/field-service/booking-timestamps),
[work-order lifecycle](https://learn.microsoft.com/en-us/dynamics365/field-service/work-order-status-booking-status),
[Booking Timestamp table](https://learn.microsoft.com/en-us/dynamics365/field-service/developer/reference/entities/msdyn_bookingtimestamp),
[Booking table](https://learn.microsoft.com/en-us/dynamics365/field-service/developer/reference/entities/bookableresourcebooking),
[Work Order table](https://learn.microsoft.com/en-us/dynamics365/field-service/developer/reference/entities/msdyn_workorder).

Product names identify the compatible source system. Glasstarn is not affiliated
with or endorsed by Microsoft; see [trademarks](../../docs/trademarks.md).
