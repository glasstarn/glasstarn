# Ticket handling (ServiceNow, any task-based table, checked on a real instance)

Follows tickets in any ServiceNow table built on `task`: HR cases
(`sn_hr_core_case`), customer service cases (`sn_customerservice_case`),
security incidents (`sn_sec_inc`), facilities requests, or a company's own
ticket table (`u_…`). Steps: Open ticket, each state in the table's own list,
assignment to a group and a person, reassignment, priority changes and changes
to other kept fields; with a task table, Create task, each task state and task
reassignment.

Written from public knowledge of ServiceNow's Table API, the `task` table every
ticket table extends, and the audit history (`sys_audit`); tested on made-up extracts (`pm-testgen` `servicenow::write_tickets`).

1. `glasstarn extract discover <project> --system servicenow --site
   https://<instance>.service-now.com --object sn_hr_core_case` checks the table
   is built on `task` and lists its states and the company's own fields.
2. In the extract spec, read the ticket table with `"as": "ticket"` and its task
   table with `"as": "ticket_task"` and its parent field (`parent` for HR
   tasks). The connector refuses a table that is not built on `task`.
3. Answer the two settings: the ticket table and the task table (or `none`).

## Real-instance check (2026-10-15)

Passed on a ServiceNow developer instance (latest release), with records made
through the Table API and pulled through the normal approved extract: incidents with their incident tasks, read as general tickets ("as": "ticket" and "as": "ticket_task"): 2 tickets, one with two tasks (one closed complete, one handed to another group and closed incomplete), every step found in order, within two minutes.

- The test instance had no HR or customer service tables, so incidents and incident tasks stood in for them; both are built on task, as HR and customer service cases are. A table not built on task was refused, as designed.
- Not yet checked on an HR Service Delivery or Customer Service Management table itself.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
