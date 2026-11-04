// Extract approval. An agent files an extract from the command line; it waits
// here until a person approves it, and then the app, not the agent, runs it.

let shownExtract = null;
let extractPoll = null;

function extractText(tag, text, className) {
  const node = document.createElement(tag);
  node.textContent = text;
  if (className) node.className = className;
  return node;
}

function extractDate(value) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short", timeZone: "UTC" }) + " UTC";
}

function extractStatus(text, error) {
  const status = document.querySelector("#extract-approval-status");
  status.textContent = text;
  status.classList.toggle("error", Boolean(error));
}

function renderExtractRequest(request) {
  const panel = document.querySelector("#extract-approval");
  shownExtract = request;
  if (!request) { panel.hidden = true; return; }
  panel.hidden = false;
  const preview = request.preview || {};
  const pending = request.status === "pending";
  document.querySelector("#extract-approval-title").textContent = {
    pending: "Extract waiting for your approval",
    running: "Running the approved extract",
    complete: "Extract complete",
    failed: "Extract stopped",
    declined: "Extract declined",
  }[request.status] || "Extract";
  document.querySelector("#extract-approval-when").textContent = `Requested ${extractDate(request.requested_at)}`;
  document.querySelector("#extract-approval-lead").textContent =
    `Your agent asked to download records from ${extractSystemName(preview.system)} (connection “${preview.target_org || preview.site || preview.instance}”). Nothing has been downloaded yet unless you approve. ${preview.scope || ""}`;
  const facts = document.querySelector("#extract-approval-facts");
  facts.replaceChildren();
  for (const [term, value] of [
    ["Window", `${extractDate(preview.start)} to ${extractDate(preview.end)} (end not included)`],
    ["History coverage", {
      jira: "Not verified. Jira keeps issue history unless an administrator removed it.",
      servicenow: "Not verified. Only audited fields leave history; administrators can exclude fields from auditing.",
      hubspot: "HubSpot keeps every property's history; very long histories of one property may be trimmed.",
    }[preview.system] || "Not verified. Check that history tracking was on for the whole period."],
    ["Fingerprint", request.spec_sha256.slice(0, 12)],
  ]) { facts.append(extractText("dt", term), extractText("dd", value)); }
  const rows = document.querySelector("#extract-approval-objects");
  rows.replaceChildren();
  for (const values of extractObjectRows(preview)) {
    const row = document.createElement("tr");
    for (const value of values) row.append(extractText("td", value));
    rows.append(row);
  }
  const queries = document.querySelector("#extract-approval-queries");
  queries.replaceChildren(...extractQueries(preview).map(query => extractText("li", query, "mono")));
  document.querySelector("#extract-approval-form").hidden = !pending;
  if (request.status === "running") extractStatus(`Approved by ${request.approved_by}. Downloading in the background; you can keep working.`);
  else if (request.status === "complete") {
    const counts = (request.result?.row_counts || []).map(count => `${Number(count.rows).toLocaleString()} ${count.object} rows`).join(", ");
    extractStatus(`Downloaded ${counts}. The files are saved in this project and listed in the audit log. Your agent can now prepare them.`);
  } else if (request.status === "failed") extractStatus(`Stopped: ${request.error} Nothing was added to the project.`, true);
  else if (request.status === "declined") extractStatus(`Declined by ${request.declined_by}. Nothing was downloaded.`);
  else extractStatus("");
}

function extractSystemName(system) {
  return { salesforce: "Salesforce", jira: "Jira", servicenow: "ServiceNow", hubspot: "HubSpot" }[system] || system;
}

// One row per file the extract writes: name, kind, fields, time, filters, gaps.
function extractObjectRows(preview) {
  if (preview.system === "jira") {
    const gaps = (preview.history_gaps || []).join(" ") || "None stated";
    const history = Array.isArray(preview.history_fields) ? preview.history_fields.join(", ") : "All changed fields";
    return [
      ["JiraIssue", "Records", (preview.issue_fields || []).join(", "), "created", "None", gaps],
      ["JiraIssueHistory", "Change history of JiraIssue", history, "created", "None", gaps],
    ];
  }
  if (preview.system === "servicenow") {
    const gaps = (preview.history_gaps || []).join(" ") || "None stated";
    const history = Array.isArray(preview.history_fields) ? preview.history_fields.join(", ") : "All audited fields";
    return [
      ...(preview.tables || []).map(table => [table.file, `Records (${table.table})`, table.fields.join(", "), "sys_created_on", table.scope, gaps]),
      ["ServiceNowAudit", "Audit history of those records", history, "sys_created_on", "None", gaps],
    ];
  }
  if (preview.system === "hubspot") {
    const gaps = (preview.history_gaps || []).join(" ") || "None stated";
    return [
      ...(preview.objects || []).map(object => [object.file, `Records (${object.object})`, object.properties.join(", "), object.created || (object.object === "quotes" ? "hs_createdate" : "createdate"), object.scope, gaps]),
      ["HubSpotHistory", "Property history of those records", (preview.objects || []).flatMap(object => object.history).join(", ") || "None", "timestamp", "None", gaps],
      ["HubSpotAssociation", "Links between those records", (preview.associations || []).join(", ") || "None", "", "None", gaps],
    ];
  }
  return (preview.objects || []).map(object => [
    object.name,
    object.kind === "history" ? `Change history${object.parent ? ` of ${object.parent}` : ""}` : "Records",
    object.fields.join(", "),
    `${object.time_field}${object.time_type === "date" ? " (date only)" : ""}`,
    (object.filters || []).map(filter => `${filter.field} = ${filter.equals}`).join(", ") || "None",
    (object.history_gaps || []).join(" ") || "None stated",
  ]);
}

// The exact queries the extract will run.
function extractQueries(preview) {
  if (preview.system === "jira") return [preview.jql, ...(preview.endpoints || [])].filter(Boolean);
  if (preview.system === "servicenow") return [...(preview.tables || []).map(table => `${table.table}: ${table.scope}`), ...(preview.endpoints || [])];
  if (preview.system === "hubspot") return [...(preview.objects || []).map(object => `${object.object}: ${object.scope}`), ...(preview.endpoints || [])];
  return [...(preview.scans || []), ...(preview.exports || [])].map(query => query.soql);
}

async function refreshExtracts(keep) {
  let requests;
  try { requests = (await workspaceApi("/api/workspace/extracts")).requests || []; }
  catch (_error) { return; }
  const open = requests.find(request => request.status === "pending" || request.status === "running");
  const followed = keep && shownExtract && requests.find(request => request.spec_sha256 === shownExtract.spec_sha256);
  renderExtractRequest(open || followed || null);
  clearTimeout(extractPoll);
  if ((open || followed)?.status === "running") extractPoll = setTimeout(() => void refreshExtracts(true), 2000);
}

async function decideExtract(approve) {
  const person = document.querySelector("#extract-approver").value.trim();
  if (!shownExtract) return;
  if (!person) { extractStatus("Enter your name so the approval is recorded.", true); return; }
  const buttons = [document.querySelector("#extract-approve"), document.querySelector("#extract-decline")];
  buttons.forEach(button => { button.disabled = true; });
  try {
    const result = await workspaceApi(`/api/workspace/extracts/${approve ? "approve" : "decline"}`, { spec_sha256: shownExtract.spec_sha256, person });
    renderExtractRequest(result.request);
    if (approve) extractPoll = setTimeout(() => void refreshExtracts(true), 1000);
  } catch (error) { extractStatus(error.message, true); }
  finally { buttons.forEach(button => { button.disabled = false; }); }
}

function initializeExtracts() {
  document.querySelector("#extract-approval-form").addEventListener("submit", event => { event.preventDefault(); void decideExtract(true); });
  document.querySelector("#extract-decline").addEventListener("click", () => void decideExtract(false));
  void refreshExtracts(false);
  // An agent may file a request while the project is open.
  document.addEventListener("visibilitychange", () => { if (!document.hidden) void refreshExtracts(true); });
}
