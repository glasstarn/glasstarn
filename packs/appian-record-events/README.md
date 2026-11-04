# Appian record events (checked on a real workspace)

Cases built on Appian, read from record events: the event history Appian
records for a record type and that Appian Process HQ analyses. Each record is
followed from its first event to its last. Written from public knowledge of
Appian record events; tested on made-up extracts (`pm-testgen`
`workflow_platforms::write_appian`).

| File | Contents |
|---|---|
| Record | The records of the record type being studied (the cases) |
| RecordEvent | Their events: which record, which event type, when, who |
| RecordEventType | The event types' names |

Steps: each event type as the company named it (for example Created, Submitted,
Approved, Closed). Record events are switched on per record type; processes
that do not write them have no history to read, and their process reports keep
only current state.

There is no direct connection to Appian yet: an administrator exports the three
tables from the business database, with times in UTC.

## Real-workspace check (2026-10-15)

Passed on an Appian Community Edition workspace, using the record events of
the sample Sport Equipment Rental app: its event history record type has the
fields this pack reads (id, recordId, eventTypeId, user, timestamp) and an
event type lookup (Created, Updated, Commented on Rental Request). Two
requests were made and updated through the app; with the app's 25 sample
requests, all 27 records and 30 events were read, and each planted request's
steps came through in order at the right minute.

- Community Edition has no database access, so the files came from the record
  lists' "Export to Excel". That export uses display names (rentalRequest,
  eventType, eventUser, Timestamp), two or three title rows, the created
  time and the creator's name in one cell, numbers such as 1.0, and
  times in the exporting person's time zone to the minute. Glasstarn reads
  these exports as they are: inventory the Excel files, confirm which column
  is which, and run `workspace pack reshape` with `--time-zone`. In the
  record list export the visible Id (`00001`) is a formatted label; the
  record's id is the unnamed first column (`column_1`). Checked again on
  2026-10-15 straight from the two Excel files: same times as the hand
  conversion, every event matched to its record. An export from the business
  database has the field names above directly.
- The sample app records only created, updated and commented events; a
  process that writes its own event types (Submitted, Approved …) was not
  checked.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
