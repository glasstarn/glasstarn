glasstarn 1.0.0 is the first public release: free, open-source process mining that runs on your own computer.

## Install

```sh
npx glasstarn setup
```

This installs the `glasstarn` command and connects it to Claude Code and Codex. It needs Node.js 18 or newer. You can also ask your agent: "Install glasstarn and connect it to this agent using npx glasstarn setup." Restart your agent afterwards, then try a demo: "Set up the purchase to pay demo in glasstarn and tell me what's slow."

## What you get

- **Ask in plain words.** Put an export from your system in a folder, open your agent there and ask. Your agent runs the glasstarn engine, explains what it found with the numbers behind it, and suggests what to look at next.
- **One page that follows the conversation.** The results open in your browser and keep up as you talk: "show me the three main variants" or "only claims over $10,000" moves the page there with the filter on.
- **Process, variants and items**, timing between any two steps, drivers of slow work, and where the money sits.
- **Check against an SOP.** Your agent turns a written procedure into an expected process, checks every item exactly and shows what was skipped or out of place.
- **Linked records**, such as orders with their deliveries, invoices and payments, with the subway map.
- **Exports from your systems** in CSV, Excel and other formats, with guides for SAP, Salesforce, ServiceNow, HubSpot, Dynamics 365, Jira and more; direct pulls from Salesforce, ServiceNow, Jira and HubSpot with your approval.
- **Exports out:** `glasstarn workspace export` writes the process map as BPMN and the items and routes as spreadsheets. `glasstarn doctor` checks your setup.
- **Four demos** with invented data: purchase to pay, order to cash, order to cash with linked records, and a sales pipeline.

## Privacy

Your files and every calculation stay on your computer. Your agent sees summaries. The only request glasstarn makes on its own is a daily check of the latest version number on npm, which you can turn off with `GLASSTARN_NO_UPDATE_CHECK=1`.

## Platforms

macOS (Apple silicon and Intel), Linux (x64 and ARM64) and Windows x64, in one npm package. The attached `glasstarn-1.0.0.tgz` is the exact package published to npm; `glasstarn.sha256` holds its checksum.

Documentation: https://glasstarn.com · Contact: hello@glasstarn.com · Apache-2.0
