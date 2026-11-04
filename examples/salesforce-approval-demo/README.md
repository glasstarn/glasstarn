# Salesforce approval demo (invented data)

Runs the whole connected flow offline, from the agent's extract request to a
project with a process map, Summary, Journey and amounts. No Salesforce org,
credentials or real records are involved: `sf` here is a stand-in for the
Salesforce CLI that answers the four queries of `spec.json` with the invented
files in `salesforce/`. The ERP files in `erp/` stand in for an ERP export.

The data (334 deals with stage history, 138 orders, 149 invoices, 112
payments) is written by `generate.py`, deterministically. Some deals are lost,
some won deals wait weeks for an order, some invoices are paid late or not yet,
and some payments cover two invoices.

## Steps

```sh
# 1. The agent creates a project and adds the ERP export.
glasstarn workspace create --name "Salesforce approval demo"
glasstarn workspace inventory <project> erp/orders.csv erp/invoices.csv erp/payments.csv erp/payment_invoices.csv

# 2. The agent files the extract. Nothing is downloaded yet.
glasstarn extract salesforce <project> --spec spec.json

# 3. A person opens the project in the app and chooses Approve and run.
#    Put this folder first on PATH so the app uses the stand-in sf.
PATH="$PWD:$PATH" glasstarn serve

# 4. After the extract completes, close the app (one writer at a time), then
#    fill recipe.template.json: replace each <file name> with that source's
#    sha256 from `glasstarn workspace show <project>`, and prepare.
glasstarn workspace prepare-ocel <project> --recipe recipe.json
glasstarn workspace name-type <project> --type opportunity --name Deals

# 5. Reopen the project in the app: Follow Deals, Orders, Invoices or Payments.
```

The automated test `salesforce_approval_demo_runs_from_request_to_journey` in
`crates/pm-cli/tests/headless_workspace.rs` runs these steps.
