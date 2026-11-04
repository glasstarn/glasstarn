# Problem management (ServiceNow, checked on a real instance)

Follows problems and their problem tasks: Open problem, each problem state
(New, Assess, Root Cause Analysis, Fix in Progress, Resolved, Closed),
assignment to a group and a person, reassignment, priority changes and changes
to other kept fields; Create problem task, each task state and task
reassignment.

Written from public knowledge of ServiceNow's Table API, its standard problem
and problem_task tables and the audit history (`sys_audit`); tested on made-up extracts (`pm-testgen` `servicenow::write_problems`).

Run `glasstarn extract discover <project> --system servicenow --site
https://<instance>.service-now.com --object problem --object problem_task`
first to see the instance's states and own fields.

## Real-instance check (2026-10-15)

Passed on a ServiceNow developer instance (latest release), with records made
through the Table API and pulled through the normal approved extract: 5 problems (one handed to another person, one with a problem task, one with its priority raised): every step that could be planted found in order, within two minutes.

- The instance moves a problem to Assess on its own once a group is assigned, and accepts the later states (Root Cause Analysis, Fix in Progress, Resolved, Closed) only through its own buttons, not through the Table API; those later state steps were not planted, so they are not yet checked. They come from the same audited state field as the steps that were.
- A problem task moves to Assess once a person is assigned.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
