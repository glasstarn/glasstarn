// Projects: saved workspaces, remembered project folders and project files.

let currentWorkspace = null;
let requestedWorkspaceId = null;
// The page a launch link asked to land on, carried into the project's session.
let requestedView = null;
let workspacePending = 0;
let workspaceUnsaved = false;
let workspaceNavigating = false;
const workspaceMutations = new Set(["/api/profile", "/api/ocel", "/api/ocel/analyze", "/api/ocel/scope", "/api/analyze", "/api/model", "/api/ocel/model", "/api/project/reopen", "/api/project/save"]);

function workspaceStatus(message, error = false) {
  const status = document.querySelector("#workspace-status");
  if (status) { status.textContent = error ? plainErrorMessage(message) : message; status.dataset.state = error ? "error" : "ready"; }
}

async function workspaceApi(path, body) {
  const response = await fetch(path, {
    method: body === undefined ? "GET" : "POST",
    headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body), cache: "no-store",
  });
  const data = await response.json();
  if (!response.ok) throw new Error(data.error || "The project could not be opened.");
  return data;
}

async function saveWorkspace() {
  if (!currentWorkspace) return;
  workspaceUnsaved = true;
  workspaceStatus("Saving project locally…");
  try {
    await workspaceApi("/api/workspace/save", {});
    workspaceUnsaved = false;
    workspaceStatus("Project saved on this computer.");
  } catch (error) {
    workspaceStatus(`Project not saved: ${error.message}. Keep this tab open and retry Save project now.`, true);
    throw error;
  }
}

// Save accepted changes before finishing; retain the accepted server state in
// the UI if disk persistence fails, and visibly block unsafe navigation.
// Read-only analysis tools do not cause disk writes.
async function projectFetch(path, options) {
  const changesWorkspace = currentWorkspace && workspaceMutations.has(path);
  if (changesWorkspace && workspaceNavigating) throw new Error("A project is being opened. Please wait.");
  if (changesWorkspace) { workspacePending += 1; workspaceUnsaved = true; }
  try {
    const response = await fetch(path, options);
    if (changesWorkspace && response.ok) await saveWorkspace().catch(() => {});
    return response;
  } finally {
    if (changesWorkspace) workspacePending -= 1;
  }
}

async function refreshWorkspaces() {
  const data = await workspaceApi("/api/workspaces");
  const select = document.querySelector("#workspace-select");
  const selected = select.value || currentWorkspace?.id || "";
  select.replaceChildren(new Option("Choose a project", ""));
  for (const project of data.projects) {
    select.add(new Option(`${project.name} · ${project.purpose === "audit" ? "Audit" : "Improvement"}`, project.id));
  }
  select.value = selected;
  const list = document.querySelector("#workspace-list");
  if (list) {
    list.replaceChildren();
    if (!data.projects.length) list.append(reportElement("p", "No saved projects yet. Start with your agent, or try a demo project.", "project-empty"));
    for (const project of data.projects) {
      const button = reportElement("button", undefined, "project-row"); button.type = "button";
      const copy = reportElement("span");
      copy.append(reportElement("strong", project.name), reportElement("small", project.purpose === "audit" ? "Audit" : "Process improvement"));
      button.append(copy, reportElement("span", project.id === data.current?.id ? "Continue →" : "Open →"));
      button.addEventListener("click", () => { select.value = project.id; void openWorkspace(false); });
      // Rename and Delete sit beside the row; Delete moves it to the Trash.
      const row = reportElement("div", undefined, "project-item");
      const tools = reportElement("span", undefined, "project-tools");
      const rename = reportElement("button", "Rename", "text-action"); rename.type = "button";
      rename.addEventListener("click", () => void renameProject(project));
      const remove = reportElement("button", "Delete", "text-action"); remove.type = "button";
      remove.addEventListener("click", () => void deleteProject(project));
      tools.append(rename, remove);
      row.append(button, tools);
      list.append(row);
    }
  }
  return data;
}

async function renameProject(project) {
  const name = typeof window.prompt === "function" ? window.prompt("New name for this project", project.name) : null;
  if (!name || !name.trim() || name.trim() === project.name) return;
  try {
    await workspaceApi("/api/workspaces/rename", { id: project.id, name: name.trim() });
    await refreshWorkspaces();
    workspaceStatus(`Renamed to ${name.trim()}.`);
  } catch (error) { workspaceStatus(error.message, true); }
}

async function deleteProject(project) {
  const sure = typeof window.confirm === "function" && window.confirm(`Delete “${project.name}”? It moves to the Trash, where you can still get it back.`);
  if (!sure) return;
  try {
    const deleted = await workspaceApi("/api/workspaces/delete", { id: project.id });
    // Deleted from its own page: that page closes, so go to the project list.
    if (deleted.list_url) {
      reportWatchStopped = true;
      window.location.assign(`${deleted.list_url}?trashed=${encodeURIComponent(project.name)}`);
      return;
    }
    await refreshWorkspaces();
    workspaceStatus(`“${project.name}” is in the Trash.`);
  } catch (error) { workspaceStatus(error.message, true); }
}

async function openWorkspace(create, newTab = false) {
  if (workspaceNavigating || workspacePending) {
    workspaceStatus("Wait for the current operation to finish before opening a project.", true);
    return;
  }
  const id = document.querySelector("#workspace-select").value;
  if (!create && !id) { workspaceStatus("Choose a project first.", true); return; }
  // Open synchronously so the browser treats this as a user-initiated tab.
  const tab = newTab ? window.open("about:blank", "_blank") : null;
  if (newTab && !tab) { workspaceStatus("Allow a new tab for this local application and try again.", true); return; }
  if (tab) tab.opener = null;
  workspaceNavigating = true;
  try {
    await saveWorkspace();
    const body = create ? { name: document.querySelector("#workspace-name").value.trim(), purpose: document.querySelector("#workspace-purpose").value } : { id };
    const opened = await workspaceApi(`/api/workspaces/${create ? "create" : "open"}`, body);
    if (tab) { tab.location.href = opened.url; workspaceStatus(`${opened.project.name} opened in a separate tab.`); }
    else window.location.assign(opened.url);
  } catch (error) {
    if (tab) tab.close();
    workspaceStatus(error.message, true);
  } finally { workspaceNavigating = false; }
}

async function initializeWorkspaces() {
  document.querySelector("#workspace-panel").hidden = false;
  const data = await refreshWorkspaces();
  const trashed = new URLSearchParams(window.location.search).get("trashed");
  if (trashed) {
    workspaceStatus(`“${trashed}” is in the Trash.`);
    history.replaceState(null, "", window.location.pathname);
  }
  currentWorkspace = data.current;
  document.body.dataset.investigationPurpose = auditRequested(currentWorkspace) ? "audit" : "improvement";
  document.body.dataset.projectState = currentWorkspace ? "open" : "chooser";
  if (!currentWorkspace && requestedWorkspaceId) {
    const id = requestedWorkspaceId; requestedWorkspaceId = null;
    workspaceStatus("Opening your project…");
    try {
      const opened = await workspaceApi("/api/workspaces/open", { id });
      window.location.assign(requestedView ? `${opened.url}&view=${requestedView}` : opened.url);
      return;
    } catch (error) { workspaceStatus(`Could not open the linked project: ${error.message}. Choose a project below.`, true); }
  }
  document.querySelector("#workspace-create").addEventListener("submit", event => { event.preventDefault(); void openWorkspace(true); });
  document.querySelector("#workspace-open").addEventListener("click", () => void openWorkspace(false));
  document.querySelector("#workspace-new-tab").addEventListener("click", () => void openWorkspace(false, true));
  document.querySelector("#workspace-refresh").addEventListener("click", () => void refreshWorkspaces().catch(error => workspaceStatus(error.message, true)));
  document.querySelector("#workspace-refresh-list")?.addEventListener("click", () => void refreshWorkspaces().catch(error => workspaceStatus(error.message, true)));
  document.querySelector("#workspace-save").addEventListener("click", () => {
    if (workspacePending) { workspaceStatus("Wait for the current operation to finish.", true); return; }
    void saveWorkspace().catch(() => {});
  });
  window.addEventListener("beforeunload", event => {
    if (workspacePending || workspaceUnsaved) { event.preventDefault(); event.returnValue = ""; }
  });
  // The old browser-directory picker reopens legacy single-file exports only.
  document.querySelector("#open-project-folder").hidden = true;
  document.querySelector("#reopen-recent").hidden = true;
  document.querySelector("#choose-log").disabled = !currentWorkspace;
  document.querySelector("#open-project").disabled = !currentWorkspace;
  if (!currentWorkspace) return;
  document.querySelector("#hero-title").textContent = "Your projects";
  document.querySelector(".hero-description").textContent = "Open a project to see its findings, process map and evidence.";
  document.querySelector("#workspace-save").hidden = false;
  document.querySelector("#workspace-select").value = currentWorkspace.id;
  document.querySelector("#workspace-title").textContent = currentWorkspace.name;
  document.title = `${currentWorkspace.name} · glasstarn`;
  const purpose = currentWorkspace.purpose === "audit" ? "Audit" : "Process improvement";
  document.querySelector("#workspace-description").textContent = `${purpose} project. Your data, findings and supporting evidence are saved in it.`;
  document.querySelector("#workspace-location").textContent = currentWorkspace.folder;
  showOpening(currentWorkspace.name);
  try {
    const state = await workspaceApi("/api/workspace/state");
    currentItemNames = state.item_names || {};
    restoredFollowedView = state.followed_view || null;
    if (!state.profile && !state.ocel) {
      document.querySelector("#hero-title").textContent = "Add data to this project.";
      document.querySelector(".hero-description").textContent = "Import your event history below, or continue in your agent with this project. Analysis views appear once the data is ready.";
    }
    if (state.profile) await restoreSavedProfile(state);
    else if (state.ocel) renderOcelAnalysis(state.ocel);
    if (state.case_model) {
      currentCaseBaseline = state.case_model;
      document.querySelector("#case-baseline-name").textContent = state.case_model.file_name;
      document.querySelector("#check-case-conformance").disabled = !currentAnalysis;
    }
    if (state.ocel_model) {
      currentOcelBaseline = state.ocel_model;
      document.querySelector("#ocel-baseline-name").textContent = state.ocel_model.file_name;
      document.querySelector("#check-ocel-conformance").disabled = !currentOcelAnalysis;
    }
    document.body.dataset.investigationPurpose = auditRequested(currentWorkspace) ? "audit" : "improvement";
    await refreshExecutiveReport();
    if (!currentAnalysis && !currentOcelAnalysis && currentInvestigationRecord) showView("report");
    if (requestedView && (currentAnalysis || currentOcelAnalysis)) { showView(requestedView); requestedView = null; }
    else if (currentAnalysis || currentOcelAnalysis) {
      const last = typeof rememberedProjectView === "function" ? rememberedProjectView() : null;
      if (last && last !== document.body.dataset.view) showView(last);
    }
    workspaceStatus("Project opened. Changes are saved locally.");
    if (typeof applyPendingAgentShow === "function") await applyPendingAgentShow();
  } catch (error) { workspaceStatus(`Could not restore the saved view: ${error.message}`, true); }
  finally { if (!followPending) hideOpening(); }
  initializeExtracts();
  await initializeAgentSetup();
}

// A saved project opens straight to its results: while the source is checked
// and the analysis rebuilt, the page says it is opening instead of showing
// the column-mapping screen. The mapping screen appears only when the saved
// project cannot be restored and the person has something to fix.
async function restoreSavedProfile(state) {
  const reopening = !!state.case_project;
  if (reopening) showOpening(currentWorkspace?.name);
  renderProfile(state.profile, { quiet: reopening });
  if (!reopening) return;
  pendingProject = state.case_project;
  let restored = false;
  try { restored = await reopenPendingProject(); }
  finally { hideOpening(); }
  if (!restored) showView("import");
}

function showOpening(name) {
  const panel = document.querySelector("#opening");
  if (!panel) return;
  const title = document.querySelector("#opening-title");
  if (title) title.textContent = name ? `Opening ${name}…` : "Opening your project…";
  panel.hidden = false;
  document.body.dataset.opening = "true";
}

function hideOpening() {
  const panel = document.querySelector("#opening");
  if (panel) panel.hidden = true;
  delete document.body.dataset.opening;
}

function projectDatabase() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(PROJECT_DB, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(PROJECT_STORE);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function storedProjectFolder() {
  try {
    const database = await projectDatabase();
    return await new Promise((resolve, reject) => {
      const transaction = database.transaction(PROJECT_STORE, "readonly");
      const request = transaction.objectStore(PROJECT_STORE).get(RECENT_FOLDER_KEY);
      request.onsuccess = () => resolve(request.result || null);
      request.onerror = () => reject(request.error);
      transaction.oncomplete = () => database.close();
    });
  } catch (_error) {
    return null;
  }
}

async function rememberProjectFolder(handle) {
  const database = await projectDatabase();
  await new Promise((resolve, reject) => {
    const transaction = database.transaction(PROJECT_STORE, "readwrite");
    transaction.objectStore(PROJECT_STORE).put(handle, RECENT_FOLDER_KEY);
    transaction.oncomplete = resolve;
    transaction.onerror = () => reject(transaction.error);
  });
  database.close();
}

async function hasDirectoryPermission(handle, requestAccess) {
  const options = { mode: "read" };
  if (await handle.queryPermission(options) === "granted") return true;
  return requestAccess && await handle.requestPermission(options) === "granted";
}

async function refreshRecentProjectAction() {
  const button = document.querySelector("#reopen-recent");
  if (!button || !("showDirectoryPicker" in window)) return;
  const handle = await storedProjectFolder();
  button.hidden = !handle;
  if (handle) button.textContent = `Reopen ${handle.name}`;
}

async function openProjectFile(file) {
  if (launchToken === null) {
    setProjectStatus("The local service is not connected.", "error");
    return null;
  }
  if (!/\.json$/i.test(file.name)) {
    setProjectStatus("Choose an Open Process project JSON file.", "error");
    return null;
  }
  if (file.size === 0 || file.size > 1024 * 1024) {
    setProjectStatus("The project file must be between 1 byte and 1 MiB.", "error");
    return null;
  }
  setProjectStatus(`Reading ${file.name} locally…`);
  try {
    const response = await projectFetch("/api/project/open", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${launchToken}`,
        "Content-Type": "application/json",
        "X-File-Name": encodeURIComponent(file.name),
      },
      body: file,
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `project returned ${response.status}`);
    pendingProject = payload.project;
    const sourceName = payload.project.source.path.split(/[\\/]/).pop();
    const engineNote = payload.engine_changed ? " It was created by a different engine version." : "";
    setProjectStatus(`Project loaded.${engineNote} Choose its source event log: ${sourceName}`, payload.engine_changed ? "" : "ready");
    const chooseLabel = document.querySelector("#choose-log-label");
    if (chooseLabel) chooseLabel.textContent = "Choose source event log";
    return payload.project;
  } catch (error) {
    pendingProject = null;
    setProjectStatus(error instanceof Error ? error.message : "The project could not be opened.", "error");
    return null;
  }
}

function sourceFileName(project) {
  return project.source.path.split(/[\\/]/).pop();
}

async function openProjectFolder(directory, requestAccess = true) {
  if (!await hasDirectoryPermission(directory, requestAccess)) {
    setProjectStatus("Permission to read that project folder was not granted.", "error");
    return false;
  }
  setProjectStatus(`Looking for a project in ${directory.name}…`);
  try {
    const projects = [];
    for await (const entry of directory.values()) {
      if (entry.kind === "file" && entry.name.endsWith(".open-process.json")) projects.push(entry);
    }
    if (projects.length === 0) throw new Error("This folder has no .open-process.json project file.");
    if (projects.length > 1) throw new Error("This folder has more than one project file. Keep one project per portable folder.");
    const project = await openProjectFile(await projects[0].getFile());
    if (!project) return false;
    const sourceName = sourceFileName(project);
    let sourceHandle;
    try {
      sourceHandle = await directory.getFileHandle(sourceName);
    } catch (_error) {
      pendingProject = null;
      throw new Error(`The project source ${sourceName} is not in this folder.`);
    }
    const source = await sourceHandle.getFile();
    const reopened = await profileFile(source);
    if (!reopened) return false;
    let remembered = true;
    try {
      await rememberProjectFolder(directory);
      await refreshRecentProjectAction();
    } catch (_error) {
      remembered = false;
    }
    const memoryNote = remembered ? " This folder is remembered in this browser." : " Select this folder again next time; this browser did not allow it to be remembered.";
    setProjectStatus(`Reopened ${project.name || projects[0].name} from ${directory.name}.${memoryNote}`, "ready");
    return true;
  } catch (error) {
    setProjectStatus(error instanceof Error ? error.message : "The project folder could not be opened.", "error");
    return false;
  }
}

async function reopenPendingProject() {
  if (!pendingProject || !currentProfile) return false;
  const project = pendingProject;
  currentMapping = project.mapping;
  currentPolicy = project.policy || {};
  currentRules = project.rules || {};
  showSavedMapping(currentMapping);
  const status = document.querySelector("#mapping-status");
  if (status) {
    status.dataset.state = "";
    status.textContent = "Verifying the source fingerprint and reopening the saved analysis…";
  }
  try {
    const response = await projectFetch("/api/project/reopen", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ upload_id: currentProfile.upload_id, project }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `reopen returned ${response.status}`);
    if (payload.status === "blocked") {
      pendingProject = null;
      const chooseLabel = document.querySelector("#choose-log-label");
      if (chooseLabel) chooseLabel.textContent = "Choose an event log";
      renderQuality(payload.quality);
      if (status) {
        status.textContent = "The project reopened, but this engine requires an additional data-quality decision.";
        status.dataset.state = "error";
      }
      return false;
    }
    const savedFilter = project.view_filter || {};
    renderAnalysis(payload.analysis, savedFilter);
    restoreFilterControls(savedFilter);
    pendingProject = null;
    const chooseLabel = document.querySelector("#choose-log-label");
    if (chooseLabel) chooseLabel.textContent = "Choose an event log";
    setProjectStatus(`Reopened ${project.name || "the saved project"}; the source fingerprint matches.`, "ready");
    if (status) {
      status.textContent = "Saved mapping, interpretation, and preparation rules restored.";
      status.dataset.state = "ready";
    }
    return true;
  } catch (error) {
    setProjectStatus(error instanceof Error ? error.message : "The project could not be reopened.", "error");
    if (status) {
      status.textContent = error instanceof Error ? error.message : "The saved analysis could not be reopened. Choose the original source file to try again.";
      status.dataset.state = "error";
    }
    return false;
  }
}

function projectFileStem(fileName) {
  const stem = fileName.replace(/\.(xes\.gz|parquet|csv|xes)$/i, "");
  return (stem || "analysis").replace(/[^a-z0-9._-]+/gi, "-");
}

async function saveCurrentProject() {
  const status = document.querySelector("#save-project-status");
  const button = document.querySelector("#save-project");
  if (!status || !button || !currentProfile || !currentMapping) return;
  status.dataset.state = "";
  status.textContent = "Fingerprinting the source and preparing the project…";
  button.disabled = true;
  try {
    const name = projectFileStem(currentProfile.file_name);
    const response = await projectFetch("/api/project/save", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ upload_id: currentProfile.upload_id, mapping: currentMapping, policy: currentPolicy, rules: currentRules, filter: currentFilter, name }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `save returned ${response.status}`);
    const blob = new Blob([`${JSON.stringify(payload, null, 2)}\n`], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `${name}.open-process.json`;
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 0);
    status.textContent = "Project saved with its source fingerprint and interpretation.";
    status.dataset.state = "ready";
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : "The project could not be saved.";
    status.dataset.state = "error";
  } finally {
    button.disabled = false;
  }
}
