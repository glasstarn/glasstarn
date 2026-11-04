# Run a system export through a process pack

Use this when the company's records live in an ERP such as SAP and someone there
can export standard tables, but no ready event log exists. A process pack holds
what an implementation consultant would bring: the tables and fields to export,
the checks that tell whether the export is complete, and the standard way to
turn the tables into steps. You act as the consultant; the company's own
analyst answers your questions. Nobody from Glasstarn steps in.

There is no live connection to the system. The company's administrator exports
files; you never ask for system credentials.

List packs with `glasstarn workspace pack list --json`. Pick the pack by the
process and the system:

| Process | System | Pack |
|---|---|---|
| Purchase to pay | SAP ECC or S/4HANA, on-premise or private cloud | `sap-purchase-to-pay` |
| Purchase to pay | S/4HANA Public Cloud | `sap-s4-cloud-purchase-to-pay` |
| Purchase to pay | SAP Ariba with ECC or S/4HANA | `sap-ariba-purchase-to-pay` |
| Purchase to pay | SAP Ariba with S/4HANA Public Cloud | `sap-ariba-s4-cloud-purchase-to-pay` |
| Order to cash | SAP ECC or S/4HANA, on-premise or private cloud | `sap-order-to-cash` |
| Order to cash | S/4HANA Public Cloud | `sap-s4-cloud-order-to-cash` |
| Order to cash | SAP Sales Cloud with ECC or S/4HANA | `sap-sales-cloud-order-to-cash` |
| Order to cash | SAP Sales Cloud with S/4HANA Public Cloud | `sap-sales-cloud-s4-cloud-order-to-cash` |
| Order to cash | Salesforce with ECC or S/4HANA | `salesforce-sap-order-to-cash` |
| Order to cash | Salesforce with S/4HANA Public Cloud | `salesforce-s4-cloud-order-to-cash` |
| Lead to opportunity | Salesforce | `salesforce-lead-to-opportunity` |
| Support case handling | Salesforce | `salesforce-case-handling` |
| Issue lifecycle, bug resolution | Jira Cloud | `jira-issue-lifecycle` |
| Incident management | ServiceNow | `servicenow-incident-management` |
| Change management | ServiceNow | `servicenow-change-management` |
| Request fulfillment (catalog requests) | ServiceNow | `servicenow-request-fulfillment` |
| Problem management | ServiceNow | `servicenow-problem-management` |
| Lead lifecycle (lead to customer) | HubSpot | `hubspot-lead-lifecycle` |
| Deal pipeline | HubSpot | `hubspot-deal-pipeline` |
| Support tickets | HubSpot | `hubspot-ticket-pipeline` |
| Quote approval | HubSpot | `hubspot-quote-approval` |
| Any other ticket table built on task: HR cases, customer service cases, security incidents, facilities, the company's own tables | ServiceNow | `servicenow-ticket-handling` |
| Plan to produce (production orders) | SAP ECC or S/4HANA | `sap-plan-to-produce` |
| Equipment maintenance | SAP ECC or S/4HANA | `sap-plant-maintenance` |
| Journal entries and month-end close | SAP ECC or S/4HANA | `sap-record-to-report` |
| Purchase to pay, order to cash | SAP Business One | `sap-business-one-purchase-to-pay`, `sap-business-one-order-to-cash` |
| Purchase to pay, order to cash | SAP Business ByDesign | `sap-bydesign-purchase-to-pay`, `sap-bydesign-order-to-cash` |
| Expense reports | SAP Concur | `concur-expense` |
| Hiring and onboarding | SAP SuccessFactors | `successfactors-recruiting` |
| Quotes and approvals | Salesforce (standard quotes) | `salesforce-quote-approval` |
| Quote to renewal | Salesforce CPQ | `salesforce-cpq-quote-to-renewal` |
| Orders kept in Salesforce | Salesforce | `salesforce-order-management` |
| Field service visits | Salesforce Field Service | `salesforce-field-service` |
| Contracts and renewals | Salesforce | `salesforce-contract-renewal` |
| Loan origination | nCino on Salesforce | `salesforce-ncino-loan-origination` |
| Case management | Pega | `pega-case-management` |
| Case management (record events) | Appian | `appian-record-events` |
| Automated workflows | Power Automate and Dataverse | `power-automate-dataverse` |
| Purchase to pay, order to cash | Oracle E-Business Suite | `oracle-ebs-purchase-to-pay`, `oracle-ebs-order-to-cash` |
| Purchase to pay, order to cash | Oracle Fusion Cloud | `oracle-fusion-purchase-to-pay`, `oracle-fusion-order-to-cash` |
| Purchase to pay, order to cash | JD Edwards EnterpriseOne | `jde-purchase-to-pay`, `jde-order-to-cash` |
| Purchase to pay (government, higher education) | PeopleSoft | `peoplesoft-purchase-to-pay` |
| Purchase to pay, order to cash | NetSuite | `netsuite-purchase-to-pay`, `netsuite-order-to-cash` |
| Purchase to pay, order to cash | Dynamics 365 Finance and Operations | `d365-finance-operations-purchase-to-pay`, `d365-finance-operations-order-to-cash` |
| Purchase to pay, order to cash | Dynamics 365 Business Central | `d365-business-central-purchase-to-pay`, `d365-business-central-order-to-cash` |
| Lead to opportunity | Dynamics 365 Sales | `d365-sales` |
| Support case handling | Dynamics 365 Customer Service | `d365-customer-service` |

**Two systems in one process.** When a company's process crosses two systems
(for example requests in Appian that name an SAP purchase order), combine their
packs instead of looking for a pack per pair: `glasstarn workspace pack
combine <project> --pack <first> --with <second> --link <table>:<target>`, where
`<table>` is the first pack's table whose rows name the item and `<target>` one
of the second pack's `link_targets` (shown by `pack list`, for example
`purchase_order`). Then use `--pack <first>+<second>` for check, answer and
build. The check asks which field holds the number (`setting:link_field`) and
reports how many rows find their item; numbers saved without leading zeros are
the usual cause of misses. Steps on the first system's items then reach the
linked item and the items the second pack follows.

For Salesforce, step 3 is an extract instead of an export: first run
`glasstarn extract discover <project> --org <alias> --object <Name> ...
--pack <pack>` (metadata only, no records, no approval). Tell the person which
needed history the org does not track; it cannot be recovered, only switched on
from now. Then write the extract spec (each pack has `extract.spec.example.json`)
and follow [connected records](connectors.md) for the person's approval.

For Jira, discovery is `glasstarn extract discover <project> --system jira
--site https://<site>.atlassian.net --object <PROJECT>`, and the extract is filed
with `glasstarn extract jira <project> --spec <file>`. Name the company's own
fields (`customfield_…`) by the names discovery returns when you ask about them.

For ServiceNow, discovery is `glasstarn extract discover <project> --system
servicenow --site https://<instance>.service-now.com --object <table>` (once per
table the pack reads, such as `change_request` and `change_task`), and the
extract is filed with `glasstarn extract servicenow <project> --spec <file>`.
Start from the pack's `extract.spec.example.json`. If discovery says the audit
history cannot be read, only creation steps will be visible; say so before
filing. For HR cases, customer service cases or any other ticket table built on
`task`, use `servicenow-ticket-handling`: read the table with `"as": "ticket"`
and its task table with `"as": "ticket_task"` and its parent field, and answer
the pack's two settings (the ticket table, and the task table or `none`). The
connector refuses a table that is not built on `task`.

Ask for the change log tables too (CDHDR and CDPOS, or the cloud change
documents): they are optional, but they hold approvals, releases, blocks and
changes. Settings come first: some name a company field (for example the
Salesforce field holding the SAP order number), and the check reads that field
once the setting is answered, so run the check again after answering them.

Ask which system the company runs if unknown. When the question is only about
supplier invoices and payments (accounts payable) or customer invoices and
cash (accounts receivable), add `--variant accounts-payable` or
`--variant accounts-receivable` to the checklist and check; the build follows
the checked variant. A pack's `status` is `tested` once the Glasstarn project
ran it on a real or trial system (its README says what was and was not
covered) and `untested` until then; for an untested pack, say so once. Never
call a pack certified, verified or endorsed by the vendor.

## The usual path

These steps are the usual path, not a script. Follow the person: reorder,
combine or skip steps when the conversation calls for it (they may arrive with
files already, know their volumes, or only want accounts payable), and come
back to a step when something new turns up. The fixed points are the ground
rules: the person approves extracts, bulk records stay in files, and every
figure comes from the engine.

1. **Scope.** Settle the process, the company code or unit, the months to look
   at and the business goal. Create the project and save the question with
   `workspace record`. State your one-line read of the process and its goal.
2. **Size.** Ask the pack's `size_check.ask` question (from the checklist).
   Multiply the answer by `steps_per_unit` for a rough step count. Measured on
   made-up SAP data: 125k order lines gave 460k steps, built in 11 seconds with
   0.5 GB of memory; 2.5M steps from other data needed about 2 GB. Say so when a
   much larger run is likely.
3. **Export request.** Run `glasstarn workspace pack handout <project> --pack
   <pack> --from <first day> --to <last day> --scope <company code>` (repeat
   `--scope`; add `--variant` when one was chosen). It saves a forwardable
   request in the project (`exports/<pack>-export-request.md`): every table
   with only the columns needed, which rows and filters, how to export it from
   that kind of system (SE16N, a background job for large tables, a warehouse
   query, Query Manager, a cloud report), and the exact file names. Give the
   person that file to forward to the administrator; do not retype it in chat.
   Summarise it in two lines: how many files, the period and the scope.
   The handout is the standard part; everything the person told you that the
   administrator should know goes in `--note` (repeat it), in plain words: the
   goal and why it matters, a deadline, who to send the files to, what may not
   leave the company (for example no names), the company's own document types
   or fields, a warehouse copy they mentioned, a large table to split by month.
   Read back over the whole conversation for these, not only the last message.
   When the administrator answers (a table missing, a column named
   differently, a file too big), or the data check finds gaps, write the
   handout again with notes that say exactly what to change, rather than
   starting over. The saved file is a draft for the person: edit it directly
   when that reads better (drop a table they will not need, explain a column
   in their words, add a sentence for their admin), as long as file names and
   column names stay exact.
4. **Data check.** Inventory every file the administrator sends, then run:

   ```sh
   glasstarn workspace inventory <project> EKKO.csv EKPO.csv ... --json
   glasstarn workspace pack check <project> --pack <pack> --json
   ```

   Files can be CSV, tab- or semicolon-separated text, or Excel (.xlsx), as
   the person or administrator made them, in any common Windows encoding.
   Each sheet of a workbook becomes a table, read from its header row below
   any title rows, so a whole admin export can be inventoried as one file.
   When a standard export lacks a column the pack can do without (an exact
   creation time, an amount), the check and the build say what is used
   instead or left out; tell the person in one line rather than asking for a
   new export. Only a missing case, step or time stops the check: a missing
   table or column the build can do without is listed in `left_out` with the
   steps it loses, and the build goes ahead; tell the person in one line each
   what the analysis will not show, and that a fuller export can be added
   later. Exit status 3 means export gaps: nothing left to mine without what
   is missing, rows outside the export's scope (contracts in a purchase-order
   export), or links where most rows find nothing (a wrong period, filter or
   column).

   A few records that do not fit are not an export failure: a number listed
   twice, an invoice whose supplier is not in the list, a missing date. They
   are often a finding in themselves. The check lists them in
   `data_problems` and asks the `data:set-aside` question first. Put it in
   business words with the counts, for example "12 of 1,591 invoices name a
   supplier that isn't in the supplier list. Shall I leave those 12 out, build
   with the rest and show them as a data-quality finding, or ask for a
   corrected export?" Recommend setting aside when they are a small share;
   when they are a large share, say so and lean towards a corrected export.
   The build can also find records the check cannot see (a missing time); it
   then stops with the same question in `next`. Never answer it yourself.
   After a build that set records aside, report them alongside the analysis
   as a data-quality finding: counts by reason from
   `preparation.set_aside_by_reason`, in business words, and where the full
   list is (`set-aside.json` in the evidence folder). Save it with
   `workspace record` with the other findings.

   Real exports often name columns by their labels (`Rental Request`,
   `Event Type`) and write times as the exporting person saw them
   (`3/5/2026 9:41 AM`). Before asking for a new export, look at
   `column_matches`: for each table it lists the pack's missing columns, the
   export's columns and likely matches by name. Propose the matches to the
   person in one message, ask which columns hold the ones no name gives away,
   and, when times have no zone, which time zone they were exported in. A
   file not named after a table (`events.xlsx`) is matched the same way, from
   the inventory's column names. Then rewrite it:

   ```sh
   glasstarn workspace pack reshape <project> --pack <pack> --table RecordEvent \
     --file events.xlsx --column recordId="Rental Request" --column eventTypeId="Event Type" \
     --time-zone America/New_York --json
   ```

   `--time-zone` takes a zone name (daylight saving is followed) or an
   offset such as `-05:00`; `--day-first yes` reads `5/3/2026` as 5 March. An
   unnamed column is `column_1`, `column_2` … by position. The rewritten copy
   is the newest, so the next check reads it, and the person's confirmed
   mapping is in the audit log. You see column names and a few examples only;
   every row goes through the engine. Use the check to confirm the mapping: if
   links come back near 0% matched, a column was mapped wrongly (a list's
   visible `Id` of `00001` where events hold `1`) and should be asked about
   again.

   For what is truly missing, tell the person exactly what to ask the
   administrator for, then inventory the new files and check again. Report the
   row counts, date ranges and the company codes found, and confirm the codes
   match the agreed scope.
5. **Questions.** Ask the `open_questions` one at a time, in business words,
   with the evidence from the question text and the recommended answer, for
   example: "35% of purchase orders have a date in the company's own field
   ZZAPPR_DAT, usually 2 days after the order was created, filled together with
   an approver name. Is that a real approval step? I'd recommend showing it as a
   step called 'Approve purchase order'." Save each answer only after the person
   confirms it:

   ```sh
   glasstarn workspace pack answer <project> --pack <pack> \
     --question custom:EKKO.ZZAPPR_DAT --choice step --value "Approve purchase order" \
     --confirmed-by "<name or role of the person who answered>" --json
   ```

   You propose; the person confirms. Never answer a question yourself, never
   choose for them, and never change the recipe silently. Only setting
   questions (such as the system clock's time zone) are needed before the first
   build. The standard pack can run first and questions about the company's own
   fields can follow; each confirmed answer is applied at the next build.
6. **Build.** Run `glasstarn workspace pack build <project> --pack <pack> --json`.
   It applies the saved answers, runs the connected preparation and saves the
   recipe, report and audit entry. Mention every item in `pack.changes` and
   anything in `pack.left_out` or `pack.unanswered_questions`. Exit status 3
   is a reconciliation block, handled as in
   [connected preparation](connected-preparation.md).
7. **Validate.** Before the first review, pick three real items: one on the most
   common route, the slowest, and one with an unusual step. Use the `follow`
   and `case` or `object` queries in [investigation commands](investigation-commands.md)
   to get their document numbers and steps, and ask the analyst to compare them
   with what the system shows. Save the outcome with `workspace record` as a
   confirmed assumption, or fix the cause before going on.
8. **First review, then refresh.** Run the first review exactly as the main
   skill describes (ranked findings, sized for the goal). For a refresh, the
   administrator repeats the same export; inventory the new files, check and
   build again. Saved answers are reused and only new questions are asked.
   When two files could be the same table, the latest inventory wins; resolve
   any remaining doubt with `--file TABLE=<sha256>`.

Every answer, with who confirmed it and when, is shown to the person under "How
this data was built" in the browser and is written to the project's audit log.
