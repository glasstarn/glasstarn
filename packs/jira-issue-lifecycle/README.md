# Jira issue lifecycle (checked on a real site)

Issues and their change history from Jira Cloud, followed by issue: Create
issue, each status in the company's own workflow, Reassign issue, Change
priority, and changes to other kept fields. For bug resolution, limit the
extract to issue type Bug (`examples/connectors/jira-bugs.spec.json`); the
steps are the same.

Written from public knowledge of Jira Cloud's REST API (version 3); tested on
made-up extracts (`pm-testgen` `jira::write`), a stand-in for the API, and a
real Jira Cloud site: a team-managed Scrum project (tasks and stories through
To Do, In Progress, In Review and Done, with rework, a reopened issue,
assignment and priority changes) and a company-managed project with its own
workflow (bugs and tasks through Triage, Waiting for customer, Fixing, QA,
Released and Won't fix). Not yet checked: resolution values set on a
transition screen.

1. `glasstarn extract discover <project> --system jira --site
   https://<site>.atlassian.net --object <PROJECT>` lists each issue type's
   statuses and the site's own fields with their names (metadata only).
2. Write the extract spec (start from `extract.spec.example.json`): fields by
   ID, including the company's own `customfield_…` fields worth keeping, and the
   changed fields to keep (`history_fields`; leaving it empty keeps every edit,
   including descriptions).
3. `glasstarn extract jira <project> --spec spec.json`; a person approves it
   in the app. The app needs `JIRA_EMAIL` and `JIRA_API_TOKEN` (a read-only
   user's API token) in its environment, and `curl`.

Files written: JiraIssue (key, id, project, created and the chosen fields) and
JiraIssueHistory (one row per changed field: issue, change ID, item, time,
author account ID, field, field ID, from and to values and their names).
People appear as Jira account IDs, not names.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
