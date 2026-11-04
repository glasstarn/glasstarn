// Import: column profile, mapping controls and requesting an analysis.

function createMappingField(field, columns, proposed) {
  const wrapper = document.createElement("label");
  wrapper.className = `mapping-field${field.required ? " required" : ""}`;
  const label = document.createElement("span");
  label.textContent = field.label;
  const select = document.createElement("select");
  select.id = `mapping-${field.key}`;
  select.dataset.mappingKey = field.key;
  select.required = field.required;

  const empty = document.createElement("option");
  empty.value = "";
  empty.textContent = field.required ? "Select a column" : "Not mapped";
  select.append(empty);
  for (const column of columns) {
    const option = document.createElement("option");
    option.value = column.name;
    option.textContent = `${column.name} · ${column.kind}`;
    option.selected = proposed?.[field.key] === column.name;
    select.append(option);
  }
  wrapper.append(label, select);
  return wrapper;
}

function renderProfile(profile, { quiet = false } = {}) {
  if (!quiet) showView("import");
  reportRevision += 1;
  const reportPanel = document.querySelector("#executive-report");
  if (reportPanel) reportPanel.hidden = true;
  const reportNavigation = document.querySelector("#report-nav");
  reportNavigation?.classList.add("unavailable");
  reportNavigation?.setAttribute("aria-disabled", "true");
  reportNavigation?.setAttribute("tabindex", "-1");
  currentProfile = profile;
  currentAnalysis = null;
  currentDiscovery = null;
  currentCaseBaseline = null;
  currentModelView = "observed";
  const observedModel = document.querySelector('input[name="model-view"][value="observed"]');
  if (observedModel) observedModel.checked = true;
  currentMapping = null;
  filterCatalog = null;
  currentFilter = {};
  currentPolicy = {};
  currentRules = {};
  currentComparisonOptions = null;
  resetFilterControls();
  const filterStatus = document.querySelector("#filter-status");
  if (filterStatus) filterStatus.textContent = "";
  const filterCount = document.querySelector("#filter-count");
  if (filterCount) filterCount.textContent = "No filters applied";
  const baselineName = document.querySelector("#case-baseline-name");
  if (baselineName) baselineName.textContent = "No expected process selected";
  const checkConformance = document.querySelector("#check-case-conformance");
  if (checkConformance) checkConformance.disabled = true;
  clearCaseConformanceResults();
  const panel = document.querySelector("#profile-panel");
  const fileName = document.querySelector("#profile-file-name");
  const fileMeta = document.querySelector("#profile-file-meta");
  const fields = document.querySelector("#mapping-fields");
  const rows = document.querySelector("#column-rows");
  if (!panel || !fileName || !fileMeta || !fields || !rows) return;

  fileName.textContent = profile.file_name;
  fileMeta.textContent = `${formatBytes(profile.bytes)} · ${profile.rows_sampled.toLocaleString()} rows sampled · ${profile.columns.length.toLocaleString()} columns`;
  fields.replaceChildren(...mappingFields.filter(field => field.required).map(field =>
    createMappingField(field, profile.columns, profile.proposed_mapping)));
  const optional = document.createElement("details"); optional.className = "optional-mapping";
  const optionalTitle = document.createElement("summary"); optionalTitle.textContent = "Additional fields (optional)";
  const optionalFields = document.createElement("div"); optionalFields.className = "mapping-grid";
  optionalFields.append(...mappingFields.filter(field => !field.required).map(field =>
    createMappingField(field, profile.columns, profile.proposed_mapping)));
  optional.append(optionalTitle, optionalFields); fields.append(optional);

  const tableRows = profile.columns.map((column) => {
    const row = document.createElement("tr");
    const name = document.createElement("th");
    name.scope = "row";
    name.textContent = column.name;
    const kind = document.createElement("td");
    kind.textContent = column.kind;
    const blank = document.createElement("td");
    blank.textContent = `${column.blank_percent.toLocaleString(undefined, { maximumFractionDigits: 1 })}%`;
    const distinct = document.createElement("td");
    distinct.textContent = `${column.distinct.toLocaleString()} in sample`;
    const examples = document.createElement("td");
    examples.className = "examples";
    examples.textContent = column.examples.length ? column.examples.join(" · ") : "No values observed";
    row.append(name, kind, blank, distinct, examples);
    return row;
  });
  rows.replaceChildren(...tableRows);
  const qualityPanel = document.querySelector("#quality-panel");
  if (qualityPanel) qualityPanel.hidden = true;
  const ocelPanel = document.querySelector("#ocel-panel");
  if (ocelPanel) ocelPanel.hidden = true;
  // A single log follows nothing.
  currentFollow = null;
  followTypes = [];
  renderFollowBar();
  currentJourney = null;
  const ocelNavigation = document.querySelector("#ocel-nav");
  ocelNavigation?.classList.add("unavailable");
  ocelNavigation?.setAttribute("aria-disabled", "true");
  ocelNavigation?.setAttribute("tabindex", "-1");
  const mapNavigation = document.querySelector("#process-map-nav");
  mapNavigation?.classList.add("unavailable");
  mapNavigation?.setAttribute("aria-disabled", "true");
  mapNavigation?.setAttribute("tabindex", "-1");
  for (const id of ["#variants-nav", "#cases-nav", "#comparison-nav", "#conformance-nav", "#quality-nav"]) {
    const navigation = document.querySelector(id);
    navigation?.classList.add("unavailable");
    navigation?.setAttribute("aria-disabled", "true");
    navigation?.setAttribute("tabindex", "-1");
  }
  const qualityNavigation = document.querySelector("#quality-nav");
  qualityNavigation?.classList.add("unavailable");
  qualityNavigation?.setAttribute("aria-disabled", "true");
  qualityNavigation?.setAttribute("tabindex", "-1");
  panel.hidden = false;
  panel.scrollIntoView({ behavior: "smooth", block: "start" });
}

function mappingFromControls() {
  const selected = Object.fromEntries(
    [...document.querySelectorAll("[data-mapping-key]")].map((select) => [
      select.dataset.mappingKey,
      select.value,
    ]),
  );
  const assigned = new Set(Object.values(selected).filter(Boolean));
  return {
    case_id: selected.case_id || "",
    activity: selected.activity || "",
    timestamp: selected.timestamp || "",
    event_order: selected.event_order || null,
    end_timestamp: selected.end_timestamp || null,
    resource: selected.resource || null,
    lifecycle: selected.lifecycle || null,
    cost: selected.cost || null,
    case_attributes: currentProfile.columns
      .map((column) => column.name)
      .filter((name) => !assigned.has(name)),
  };
}

function showSavedMapping(mapping) {
  for (const field of mappingFields) {
    const select = document.querySelector(`#mapping-${field.key}`);
    if (select) select.value = mapping[field.key] || "";
  }
}

let analysisRequestRevision = 0;
async function analyzeMapping(mapping, filter = {}, source = "mapping") {
  const revision = ++analysisRequestRevision;
  const profile = currentProfile;
  currentMapping = mapping;
  currentAnalysis = null;
  if(typeof studioOpen!=="undefined" && studioOpen){clearStudioAnswer();studioElement("status").textContent="Updating the selection…";}
  currentDiscovery = null;
  clearCaseConformanceResults("The analysis is changing. Complete analysis before checking conformance.");
  const conformanceButton = document.querySelector("#check-case-conformance");
  if (conformanceButton) conformanceButton.disabled = true;
  const downloadModel = document.querySelector("#download-case-model");
  if (downloadModel) downloadModel.disabled = true;
  const status = document.querySelector("#mapping-status");
  const filterStatus = document.querySelector("#filter-status");
  const qualityStatus = document.querySelector("#quality-status");
  const confirm = document.querySelector("#confirm-mapping");
  const applyFilters = document.querySelector("#apply-filters");
  const applyQuality = document.querySelector("#apply-quality");
  if (!status || !filterStatus || !qualityStatus || !confirm || !applyFilters || !applyQuality) return;
  const activeStatus = source === "filter" ? filterStatus : source === "quality" ? qualityStatus : status;
  activeStatus.dataset.state = "";
  activeStatus.textContent = source === "filter"
    ? "Updating the view…"
    : "Validating and analyzing the complete log locally…";
  if (source === "filter") setMapUpdating(true);
  confirm.disabled = true;
  applyFilters.disabled = true;
  applyQuality.disabled = true;
  try {
    const response = await projectFetch("/api/analyze", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ upload_id: currentProfile.upload_id, mapping, policy: currentPolicy, rules: currentRules, filter }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (revision !== analysisRequestRevision || profile !== currentProfile) return false;
    if (!response.ok) throw new Error(payload.error || `analysis returned ${response.status}`);
    if (payload.status === "blocked") {
      const decisions = payload.quality.summary.open_decisions;
      renderQuality(payload.quality);
      activeStatus.textContent = `Analysis paused: ${decisions.toLocaleString()} data-quality ${decisions === 1 ? "decision needs" : "decisions need"} your answer.`;
      activeStatus.dataset.state = "error";
      return;
    }
    currentMapping = mapping;
    renderAnalysis(payload.analysis, filter);
    activeStatus.textContent = source === "filter"
      ? `${payload.analysis.view.cases_shown.toLocaleString()} of ${payload.analysis.view.cases_total.toLocaleString()} ${itemLabels().plural.toLowerCase()} match.`
      : source === "quality" ? "Decisions applied. The complete log was analyzed under this interpretation." : "Mapping confirmed and the complete log was analyzed locally.";
    activeStatus.dataset.state = "ready";
    return true;
  } catch (error) {
    if (revision !== analysisRequestRevision || profile !== currentProfile) return false;
    activeStatus.textContent = plainErrorMessage(error instanceof Error ? error.message : "The event log could not be analyzed.");
    activeStatus.dataset.state = "error";
  } finally {
    if (revision === analysisRequestRevision) {
      setMapUpdating(false);
      confirm.disabled = false;
      applyFilters.disabled = false;
      applyQuality.disabled = false;
    }
  }
}

async function profileFile(file) {
  if (launchToken === null) {
    setUploadStatus("The local service is not connected.", "error");
    return false;
  }
  if (isOcelFile(file)) return analyzeOcelFile(file);
  if (!supportedFile(file)) {
    setUploadStatus("Choose a CSV, Parquet, XES, XES.gz, or OCEL 2.0 JSON/XML/SQLite file.", "error");
    return false;
  }
  if (file.size === 0) {
    setUploadStatus("The selected file is empty.", "error");
    return false;
  }

  setUploadStatus(`Reading a local sample from ${file.name}…`);
  const choose = document.querySelector("#choose-log");
  if (choose) choose.disabled = true;
  try {
    const response = await projectFetch("/api/profile", {
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
    if (!response.ok) throw new Error(payload.error || `profile returned ${response.status}`);
    renderProfile(payload);
    if (pendingProject) {
      setUploadStatus(`Profiled ${file.name} locally. Verifying it against the saved project…`);
      const reopened = await reopenPendingProject();
      setUploadStatus(reopened ? `Reopened the project from ${file.name}.` : `Profiled ${file.name}, but the saved project could not be restored.`, reopened ? "ready" : "error");
      return reopened;
    } else {
      setUploadStatus(`Profiled ${file.name} locally. Review the mapping below.`, "ready");
      return true;
    }
  } catch (error) {
    setUploadStatus(error instanceof Error ? error.message : "The event log could not be read.", "error");
    return false;
  } finally {
    if (choose && launchToken !== null) choose.disabled = false;
  }
}
