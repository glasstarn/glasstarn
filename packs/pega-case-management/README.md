# Pega case management (not yet tested)

Cases on the Pega Platform, from creation through every recorded step to
resolution, followed by case. Pega is a platform on which each company builds
its own case types, so this pack reads what every Pega application has: the
work table (one row per case) and the work history (pc_history_work, one row
per thing that happened). Written from public knowledge of the Pega database;
tested only on made-up extracts (`pm-testgen` `workflow_platforms::write_pega`).
Untested until confirmed on a real system.

| Table | Contents |
|---|---|
| pc_work | Cases: ID, case type, status, urgency, created and resolved times |
| pc_history_work | Each case's history rows: when, who, and what happened |

Steps: Create case, every history row named by the column the company chooses
(usually pyMemo or pyMessageKey, a question the pack asks), Resolve case. The
agent then works with the company's analyst to merge or rename step names,
because each application names its own stages and assignments.

There is no direct connection to Pega yet: an administrator exports the two
tables (or a report on them), with times in UTC.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
