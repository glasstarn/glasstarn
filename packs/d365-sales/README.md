# Dynamics 365 Sales: lead to opportunity

Leads and opportunities in Dynamics 365 Sales, from lead to won or lost deal,
with phone calls, emails and appointments regarding each opportunity.

Confirmed on a live demo environment: qualified leads to won and lost
opportunities, a disqualified lead without an opportunity, an open lead, and an
open opportunity created directly. Saved phone, draft email and appointment
activities were also exercised. Independently reconciled all 13 events and
15 object links against original source records, timestamps and source-row
fingerprints. Task is covered by generated regression tests but was not planted
in this Sales live test. No calls, emails or invitations were sent.

Files: lead (optional), opportunity, activitypointer (optional; activities
regarding the opportunities, no subjects or bodies). Include only Phone Call,
Email, Appointment and Task activities. Exclude Opportunity Close activities:
the opportunity table already provides the outcome events.

Steps: Create lead, Disqualify lead, Create opportunity, Activity:
each supported activity type, Win opportunity, Lose opportunity.

Opportunity creation is recorded as Create opportunity whether it came from
a lead or was created directly. An originating lead, when present, remains
linked; the creation timestamp alone does not establish a qualification action.
Lead creation and disqualification events belong to lead objects. Lead-only
records do not become opportunity cases, and their events do not automatically
appear in the opportunity case projection.

The live export supplies `actualclosedate` as a calendar date. The prepared
log represents it at UTC midnight, which is not a known decision instant. A
same-day closure can therefore sort before creation; do not use this source to
claim exact closure ordering or elapsed time. Preserve the source date rather
than inventing a time. Disqualification uses `modifiedon`, which later edits
can change. Stage changes, reopening and repeated closure history require
additional history sources and are not inferred from these snapshots.

Activity events use creation time. A saved phone record, draft email or internal
appointment does not establish that communication occurred or was completed.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
