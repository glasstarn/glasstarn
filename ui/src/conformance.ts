// Audit checks: case conformance against a baseline model.

function clearCaseConformanceResults(message = "") {
  // A late response must never repopulate evidence for a previous view/model.
  caseConformanceRevision += 1;
  currentCaseConformance = null;
  renderResults();
  const status = document.querySelector("#case-conformance-status");
  if (status) {
    status.textContent = message;
    status.dataset.state = "";
  }
}

function downloadCaseModel() {
  if (!currentDiscovery) return;
  const source = currentDiscovery.source || currentProfile?.file_name || "process-model";
  const stem = source.replace(/\.(csv|parquet|xes|xes\.gz)$/i, "").replace(/[^a-z0-9._-]+/gi, "-") || "process-model";
  const blob = new Blob([`${JSON.stringify(currentDiscovery, null, 2)}\n`], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `${stem}.model.json`;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 0);
  const status = document.querySelector("#case-conformance-status");
  if (status) {
    status.textContent = `Saved ${link.download}. It describes the current log view; use a separately reviewed model when you need independent conformance evidence.`;
    status.dataset.state = "ready";
  }
}

async function uploadCaseBaseline(file) {
  const status = document.querySelector("#case-conformance-status");
  const name = document.querySelector("#case-baseline-name");
  const check = document.querySelector("#check-case-conformance");
  if (!status || !name || !check || launchToken === null) return;
  if (!/\.model\.json$/i.test(file.name) || /\.ocelmodel\.json$/i.test(file.name)) {
    status.textContent = "Choose a portable case-centric .model.json baseline.";
    status.dataset.state = "error";
    return;
  }
  if (file.size === 0) {
    status.textContent = "The selected baseline is empty.";
    status.dataset.state = "error";
    return;
  }
  if (file.size > 10 * 1024 * 1024) {
    status.textContent = "The selected baseline exceeds the 10 MiB safety limit.";
    status.dataset.state = "error";
    return;
  }
  clearCaseConformanceResults(`Validating ${file.name} locally…`);
  const profile = currentProfile;
  const chooser = document.querySelector("#choose-case-baseline");
  if (chooser) chooser.disabled = true;
  check.disabled = true;
  try {
    const response = await projectFetch("/api/model", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${launchToken}`,
        "Content-Type": "application/octet-stream",
        "X-File-Name": encodeURIComponent(file.name),
      },
      body: file,
      cache: "no-store",
    });
    const payload = await response.json();
    if (profile !== currentProfile) return;
    if (!response.ok) throw new Error(payload.error || `baseline upload returned ${response.status}`);
    currentCaseBaseline = payload;
    document.body.dataset.investigationPurpose = auditRequested(currentWorkspace) ? "audit" : "improvement";
    const origin = payload.training_source ? ` · model origin ${payload.training_source}` : "";
    name.textContent = `${payload.file_name}${origin}`;
    clearCaseConformanceResults("Expected process ready. Checking…");
    status.dataset.state = "ready";
    if (document.body.dataset.view === "conformance") void checkCaseConformance();
  } catch (error) {
    if (profile !== currentProfile) return;
    // Rejected uploads do not replace the server's last accepted baseline.
    const message = error instanceof Error ? error.message : "The baseline could not be read.";
    const retained = currentCaseBaseline ? ` ${currentCaseBaseline.file_name} remains selected.` : "";
    status.textContent = `${message}${retained}`;
    status.dataset.state = "error";
  } finally {
    if (chooser) chooser.disabled = false;
    if (profile === currentProfile) {
      check.disabled = !currentCaseBaseline || !currentProfile || !currentMapping || !currentAnalysis;
    }
  }
}

function caseDeviationDescription(deviation, aligned) {
  if (aligned) {
    if (deviation.kind === "log_only") {
      return [`${deviation.activity} was observed but is outside the baseline`, `Recorded event ${deviation.event_index + 1} · source row ${deviation.source_row}`];
    }
    return [`${deviation.activity} was required by the baseline but not observed`, `Model transition t${deviation.transition}`];
  }
  switch (deviation.kind) {
    case "activity_not_in_model": return [`${deviation.activity} is outside the baseline`, `Recorded event ${deviation.event_index + 1} · source row ${deviation.source_row}`];
    case "missing_tokens": return [`Required preceding step not satisfied before ${deviation.activity}`, `Recorded event ${deviation.event_index + 1} · source row ${deviation.source_row} · ${deviation.count.toLocaleString()} token(s)`];
    case "final_tokens_missing": return ["The required final state was not reached", `${deviation.count.toLocaleString()} final token(s) missing`];
    case "tokens_remaining": return ["Recorded route did not end in the expected state", `${deviation.count.toLocaleString()} token(s) remaining`];
    default: return ["Process conformance deviation", deviation.kind];
  }
}

function renderCaseConformance(report) {
  currentCaseConformance = report;
  renderResults();
  const status = document.querySelector("#case-conformance-status");
  if (!status) return;
  status.textContent = "";
  status.dataset.state = "ready";
  renderConformanceSteps(report);
}

// What did not follow the expected process, one table row per kind of
// difference, most common first: the count, its share, the amount those items
// carry, and a click that opens exactly those items.
let conformanceMoneyRevision = 0;
function renderConformanceSteps(report) {
  const body = document.querySelector("#conf-groups");
  if (!body) return;
  const steps = report?.by_step || [];
  const items = itemLabels().plural.toLowerCase();
  const checked = report?.view?.cases_shown || 0;
  if (!steps.length) {
    if (typeof resultsEmptyRow === "function" && report) resultsEmptyRow(body, 4, checked ? `Every ${itemLabels().singular.toLowerCase()} in this view followed the expected process.` : `No ${items} in this view.`);
    else body.replaceChildren();
    return;
  }
  const revision = ++conformanceMoneyRevision;
  const cell = (text, className) => { const td = document.createElement("td"); td.textContent = text; if (className) td.className = className; return td; };
  body.replaceChildren(...steps.map((group) => {
    const row = document.createElement("tr");
    const money = cell("–", "n");
    row.append(
      cell(conformanceGroupWords(group, items)[0]),
      cell(group.cases.toLocaleString(), "n"),
      cell(conformanceShare(group.cases, checked), "n"),
      money,
    );
    row.title = group.variants?.length ? `Show these ${group.cases.toLocaleString()} ${items}` : "Open an example";
    row.addEventListener("click", () => void openConformanceGroup(group));
    if (typeof conformanceGroupMoney === "function") {
      conformanceGroupMoney(group).then((found) => {
        if (revision !== conformanceMoneyRevision || !found || !Number.isFinite(Number(found.total))) return;
        money.textContent = formatAmount(found.total);
        money.title = found.field;
      }).catch(() => {});
    }
    return row;
  }));
}

async function checkCaseConformance() {
  // Every SOP check is the exact one; examples stay bounded, counts are complete.
  const limit = 20;
  const algorithm = "alignment";
  const status = document.querySelector("#case-conformance-status");
  const button = document.querySelector("#check-case-conformance");
  if (!status || !button || !currentProfile || !currentMapping || !currentAnalysis || !currentCaseBaseline) return;
  clearCaseConformanceResults("Checking every item in this view against the expected process…");
  const revision = caseConformanceRevision;
  button.disabled = true;
  try {
    const response = await projectFetch("/api/conformance", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({
        upload_id: currentProfile.upload_id,
        model_id: currentCaseBaseline.model_id,
        mapping: currentMapping,
        policy: currentPolicy,
        rules: currentRules,
        filter: currentFilter,
        detail_limit: limit,
        algorithm,
      }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (revision !== caseConformanceRevision) return;
    if (!response.ok) throw new Error(payload.error || `conformance returned ${response.status}`);
    renderCaseConformance(payload);
  } catch (error) {
    if (revision !== caseConformanceRevision) return;
    status.textContent = error instanceof Error ? error.message : "Process conformance could not be checked.";
    status.dataset.state = "error";
  } finally {
    if (revision === caseConformanceRevision) {
      button.disabled = !currentCaseBaseline || !currentProfile || !currentMapping || !currentAnalysis;
    }
  }
}
