// "How this data was built": the project's saved evidence in plain words,
// collapsed at the bottom of every project page. Nothing here is computed in
// the browser; it restates the extraction reports, the matching recipe and
// its counts, and the data checks.

let currentProvenance = null;

async function loadProvenance() {
  try { currentProvenance = await workspaceApi("/api/workspace/provenance"); }
  catch (_error) { currentProvenance = null; }
  renderProvenance();
}

function provenanceDate(value) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" });
}

function provenanceFacts() {
  const facts = [];
  const sourceName = currentOcelAnalysis?.source || currentAnalysis?.source;
  if (sourceName) facts.push(["Source file", String(sourceName).split(/[\\/]/).pop()]);
  for (const extract of currentProvenance?.extractions || []) {
    const system = extract.system === "salesforce" ? "Salesforce" : String(extract.system || "System");
    const objects = (extract.objects || []).map((object) => `${object.name} (${Number(object.rows).toLocaleString()} rows)`).join(", ");
    const window = extract.window ? ` · ${provenanceDate(extract.window.start)} to ${provenanceDate(extract.window.end_exclusive)}` : "";
    const approved = extract.approved_by ? ` · approved by ${extract.approved_by} on ${provenanceDate(extract.approved_at)}` : "";
    facts.push([`${system} extract`, `${objects}${window}${approved}`]);
  }
  const preparation = currentProvenance?.preparation;
  if (preparation) {
    facts.push(["Built from", `${preparation.tables.length} tables: ${preparation.tables.join(", ")}`]);
    for (const link of preparation.links) {
      const unmatched = Number(link.unmatched || 0);
      facts.push([
        `${itemTypeName(link.from)} matched to ${itemTypeName(link.to).toLowerCase()}`,
        `on ${(link.on || []).join(" + ")} (“${link.meaning}”): ${Number(link.matched).toLocaleString()} matched${unmatched ? `, ${unmatched.toLocaleString()} unmatched` : ", none unmatched"}`,
      ]);
    }
    const totals = preparation.totals;
    facts.push(["Result", `${Number(totals.items).toLocaleString()} items, ${Number(totals.steps).toLocaleString()} steps, ${(Number(totals.item_links) + Number(totals.step_links)).toLocaleString()} links${Number(totals.issues) ? ` · ${Number(totals.issues).toLocaleString()} unresolved issues` : " · no unresolved issues"}`]);
  }
  for (const pack of currentProvenance?.packs || []) {
    const open = Number(pack.open_questions || 0);
    facts.push(["Process pack", `${pack.id}${pack.status === "untested" ? " (not yet tested on a real system)" : ""} · ${(pack.answers || []).length} ${(pack.answers || []).length === 1 ? "answer" : "answers"} saved${open ? `, ${open} open` : ""}`]);
    for (const answer of pack.answers || []) {
      const value = answer.value ? `: ${answer.value}` : "";
      facts.push([`Confirmed by ${answer.confirmed_by}, ${provenanceDate(answer.at)}`, `${answer.question} Answer: ${answer.answer}${value}`]);
    }
  }
  const summary = typeof currentFollow !== "undefined" ? currentFollow?.summary : null;
  if (summary) {
    const name = itemTypeName(summary.object_type).toLowerCase();
    const steps = (count) => `${Number(count).toLocaleString()} ${Number(count) === 1 ? "step involves" : "steps involve"}`;
    const shared = Number(summary.shared_events) ? ` ${steps(summary.shared_events)} several ${name} and ${Number(summary.shared_events) === 1 ? "is" : "are"} counted once for each (${Number(summary.shared_event_copies).toLocaleString()} extra).` : "";
    const excluded = Number(summary.excluded_events) ? ` ${steps(summary.excluded_events)} no ${itemTypeName(summary.object_type, false).toLowerCase()} and ${Number(summary.excluded_events) === 1 ? "is" : "are"} left out.` : "";
    facts.push(["Following", `Each of the ${Number(summary.cases).toLocaleString()} ${name} is one case of every step that involves it.${shared}${excluded}`]);
  }
  const quality = currentOcelAnalysis?.quality;
  if (quality) {
    const notable = quality.findings.filter((finding) => finding.severity !== "information");
    facts.push(["Data checks", notable.length ? notable.map((finding) => `${finding.title} (${Number(finding.affected).toLocaleString()})`).join(" · ") : "All passed"]);
  }
  return facts;
}

// The evidence opens as a sheet from the right, from a quiet link next to the
// file name at the top; × , Escape or a click outside closes it.
function setDataBuiltOpen(open) {
  const panel = document.querySelector("#data-built");
  const button = document.querySelector("#data-built-open");
  if (!panel) return;
  panel.hidden = !open;
  button?.setAttribute("aria-expanded", String(open));
  if (open) document.querySelector("#data-built-close")?.focus({ preventScroll: true });
}

function wireDataBuilt() {
  const button = document.querySelector("#data-built-open");
  if (!button || button.dataset.wired) return;
  button.dataset.wired = "true";
  button.addEventListener("click", () => setDataBuiltOpen(document.querySelector("#data-built")?.hidden !== false));
  document.querySelector("#data-built-close")?.addEventListener("click", () => setDataBuiltOpen(false));
  document.addEventListener("keydown", (event) => { if (event.key === "Escape" && document.querySelector("#data-built")?.hidden === false) setDataBuiltOpen(false); });
  document.addEventListener("mousedown", (event) => {
    const panel = document.querySelector("#data-built");
    if (panel && !panel.hidden && !panel.contains(event.target) && !button.contains(event.target)) setDataBuiltOpen(false);
  });
}

function renderProvenance() {
  const panel = document.querySelector("#data-built");
  const list = document.querySelector("#data-built-facts");
  if (!panel || !list) return;
  const facts = provenanceFacts();
  wireDataBuilt();
  const button = document.querySelector("#data-built-open");
  if (button) button.hidden = !facts.length;
  if (!facts.length) setDataBuiltOpen(false);
  list.replaceChildren(...facts.flatMap(([term, value]) => {
    const dt = document.createElement("dt"); dt.textContent = term;
    const dd = document.createElement("dd"); dd.textContent = value;
    return [dt, dd];
  }));
  const foot = document.querySelector("#data-built-foot");
  if (foot) foot.textContent = currentProvenance?.audit_log
    ? "The full audit log, extract files and matching rules are in the project folder."
    : "The source files and any matching rules are in the project folder.";
}
