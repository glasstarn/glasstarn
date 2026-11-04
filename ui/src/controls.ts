// Wires page controls to their handlers.

function initializeImport() {
  enhanceSegments();
  initializeSelects();
  initializeMapShell();
  keepFullTextAsTitle(document.querySelector("#results-coverage"));
  initializeNavigation();
  initializeEvidencePanel();
  initializeResults();
  initializeExplorers();
  initializeAnalysisStudio();
  initializeValues();
  initializeFilterChips();
  watchForAnswers();
  startHeartbeat();
  const choose = document.querySelector("#choose-log");
  const input = document.querySelector("#file-input");
  const openProject = document.querySelector("#open-project");
  const openProjectFolderButton = document.querySelector("#open-project-folder");
  const reopenRecent = document.querySelector("#reopen-recent");
  const projectInput = document.querySelector("#project-input");
  const dropZone = document.querySelector("#drop-zone");
  const confirm = document.querySelector("#confirm-mapping");
  const applyFilters = document.querySelector("#apply-filters");
  const resetFilters = document.querySelector("#reset-filters");
  const applyQuality = document.querySelector("#apply-quality");
  const applyOcel = document.querySelector("#apply-ocel-filter");
  const resetOcel = document.querySelector("#reset-ocel-filter");
  const measureOcelTime = document.querySelector("#measure-ocel-time");
  const analyzeOcelSynchronizationButton = document.querySelector("#analyze-ocel-synchronization");
  const rankOcelBottlenecksButton = document.querySelector("#rank-ocel-bottlenecks");
  const discoverOcelModelButton = document.querySelector("#discover-ocel-model");
  const downloadOcelModelButton = document.querySelector("#download-ocel-model");
  const chooseOcelBaselineButton = document.querySelector("#choose-ocel-baseline");
  const ocelModelInput = document.querySelector("#ocel-model-input");
  const checkOcelConformanceButton = document.querySelector("#check-ocel-conformance");
  const projectOcelCasesButton = document.querySelector("#project-ocel-cases");
  const downloadCaseModelButton = document.querySelector("#download-case-model");
  const chooseCaseBaselineButton = document.querySelector("#choose-case-baseline");
  const caseModelInput = document.querySelector("#case-model-input");
  const checkCaseConformanceButton = document.querySelector("#check-case-conformance");

  choose?.addEventListener("click", () => input?.click());
  openProject?.addEventListener("click", () => projectInput?.click());
  openProjectFolderButton?.addEventListener("click", async () => {
    try {
      const directory = await window.showDirectoryPicker({ mode: "read" });
      await openProjectFolder(directory);
    } catch (error) {
      if (error?.name !== "AbortError") setProjectStatus("The project folder could not be selected.", "error");
    }
  });
  reopenRecent?.addEventListener("click", async () => {
    const directory = await storedProjectFolder();
    if (!directory) {
      reopenRecent.hidden = true;
      return;
    }
    await openProjectFolder(directory, true);
  });
  input?.addEventListener("change", () => {
    const file = input.files?.[0];
    if (file) void profileFile(file);
    input.value = "";
  });
  projectInput?.addEventListener("change", () => {
    const file = projectInput.files?.[0];
    if (file) void openProjectFile(file);
    projectInput.value = "";
  });
  chooseOcelBaselineButton?.addEventListener("click", () => ocelModelInput?.click());
  ocelModelInput?.addEventListener("change", () => {
    const file = ocelModelInput.files?.[0];
    if (file) void uploadOcelBaseline(file);
    ocelModelInput.value = "";
  });
  chooseCaseBaselineButton?.addEventListener("click", () => caseModelInput?.click());
  caseModelInput?.addEventListener("change", () => {
    const file = caseModelInput.files?.[0];
    if (file) void uploadCaseBaseline(file);
    caseModelInput.value = "";
  });

  for (const eventName of ["dragenter", "dragover"]) {
    dropZone?.addEventListener(eventName, (event) => {
      event.preventDefault();
      dropZone.dataset.dragging = "true";
    });
  }
  for (const eventName of ["dragleave", "drop"]) {
    dropZone?.addEventListener(eventName, (event) => {
      event.preventDefault();
      delete dropZone.dataset.dragging;
    });
  }
  dropZone?.addEventListener("drop", (event) => {
    const file = event.dataTransfer?.files?.[0];
    if (file) void profileFile(file);
  });

  confirm?.addEventListener("click", () => {
    if (!currentProfile) return;
    const selections = mappingFromControls();
    const required = [selections.case_id, selections.activity, selections.timestamp];
    const status = document.querySelector("#mapping-status");
    if (!status) return;
    if (required.some((value) => !value)) {
      status.textContent = "Select a column for Case ID, Activity, and Timestamp.";
      status.dataset.state = "error";
    } else if (new Set(required).size !== required.length) {
      status.textContent = "Case ID, Activity, and Timestamp must use different columns.";
      status.dataset.state = "error";
    } else {
      void analyzeMapping(selections);
    }
  });


  document.querySelector("#model-switch")?.addEventListener("change", (event) => {
    if (event.target?.name !== "model-view") return;
    currentModelView = event.target.value;
    renderMap();
  });

  document.querySelector("#ocel-flow-object-type")?.addEventListener("change", renderOcelFlowMap);
  document.querySelector("#ocel-flow-limit")?.addEventListener("change", renderOcelFlowMap);
  for (const button of document.querySelectorAll("#ocel-flow-metric button")) {
    button.addEventListener("click", () => { ocelFlowMetric = button.dataset.metric; renderOcelFlowMap(); });
  }

  for (const [containerId, addId, kind] of [
    ["#filter-direct-pairs", "#add-direct-pair", "direct"],
    ["#filter-eventual-pairs", "#add-eventual-pair", "eventual"],
  ]) {
    const container = document.querySelector(containerId);
    document.querySelector(addId)?.addEventListener("click", () => {
      container.append(makeTracePairRow(kind));
      clearFilterStatus();
    });
    container?.addEventListener("click", (event) => {
      const remove = event.target.closest?.(".trace-pair-remove");
      if (remove) {
        remove.closest(".trace-pair-row")?.remove();
        clearFilterStatus();
      }
    });
  }
  document.querySelector("#filter-panel")?.addEventListener("change", clearFilterStatus);

  document.querySelector("#comparison-switch")?.addEventListener("change", (event) => {
    if (event.target?.name !== "comparison-type") return;
    const cohorts = event.target.value === "cohorts";
    document.querySelector("#cohort-controls").hidden = !cohorts;
    document.querySelector("#period-controls").hidden = cohorts;
  });
  document.querySelector("#comparison-attribute")?.addEventListener("change", populateCohortValues);
  document.querySelector("#compare-cohorts")?.addEventListener("click", () => void runCohortComparison());
  document.querySelector("#compare-periods")?.addEventListener("click", () => void runPeriodComparison());

  applyFilters?.addEventListener("click", () => {
    if (!currentMapping) return;
    // A field chosen without a value would be dropped silently; ask for one.
    const panel = document.querySelector("#filter-panel");
    if (panel?.dataset.group === "field" && document.querySelector("#filter-attribute")?.value && !document.querySelector("#filter-attribute-value")?.value?.trim()) {
      const status = document.querySelector("#filter-status");
      status.textContent = "Choose or type a value for this field first.";
      status.dataset.state = "error";
      return;
    }
    let filter;
    try {
      filter = filterFromControls();
    } catch (error) {
      const status = document.querySelector("#filter-status");
      status.textContent = error instanceof Error ? error.message : "Complete the trace filter.";
      status.dataset.state = "error";
      return;
    }
    if (filter.started_after !== null && filter.started_before !== null && filter.started_after > filter.started_before) {
      const status = document.querySelector("#filter-status");
      status.textContent = "The start of the time window must be before its end.";
      status.dataset.state = "error";
      return;
    }
    if (filter.min_duration !== null && filter.max_duration !== null && filter.min_duration > filter.max_duration) {
      const status = document.querySelector("#filter-status");
      status.textContent = "Minimum duration cannot exceed maximum duration.";
      status.dataset.state = "error";
      return;
    }
    void analyzeMapping(currentMapping, filter, "filter");
  });

  resetFilters?.addEventListener("click", () => {
    if (!currentMapping) return;
    resetFilterControls();
    void analyzeMapping(currentMapping, {}, "filter");
  });

  document.querySelector("#case-search")?.addEventListener("submit", (event) => {
    event.preventDefault();
    const input = document.querySelector("#case-id-input");
    const caseId = input.value.trim();
    if (caseId) void openCase(caseId);
  });

  applyQuality?.addEventListener("click", () => {
    if (!currentMapping) return;
    const answers = { ...currentPolicy };
    let missing = 0;
    for (const fieldset of document.querySelectorAll("[data-decision-kind]")) {
      const selected = fieldset.querySelector("input[type=radio]:checked");
      if (!selected && fieldset.dataset.required === "true") { missing += 1; continue; }
      if (selected?.value) answers[fieldset.dataset.decisionKind] = selected.value;
      else delete answers[fieldset.dataset.decisionKind];
    }
    const status = document.querySelector("#quality-status");
    if (missing) {
      status.textContent = `Answer ${missing.toLocaleString()} required ${missing === 1 ? "decision" : "decisions"} before analysis.`;
      return;
    }
    currentPolicy = answers;
    void analyzeMapping(currentMapping, currentFilter, "quality");
  });

  document.querySelector("#save-project")?.addEventListener("click", () => void saveCurrentProject());
  downloadCaseModelButton?.addEventListener("click", downloadCaseModel);
  checkCaseConformanceButton?.addEventListener("click", () => void checkCaseConformance());
  applyOcel?.addEventListener("click", () => void applyOcelFilter(ocelFilterFromControls()));
  resetOcel?.addEventListener("click", () => void applyOcelFilter({}));
  measureOcelTime?.addEventListener("click", () => void measureOcelTimeBetween());
  analyzeOcelSynchronizationButton?.addEventListener("click", () => void analyzeOcelSynchronization());
  rankOcelBottlenecksButton?.addEventListener("click", () => void rankOcelBottlenecks());
  discoverOcelModelButton?.addEventListener("click", () => void discoverOcelModel());
  downloadOcelModelButton?.addEventListener("click", downloadOcelModel);
  checkOcelConformanceButton?.addEventListener("click", () => void checkOcelConformance());
  projectOcelCasesButton?.addEventListener("click", () => void projectOcelCases());
}
