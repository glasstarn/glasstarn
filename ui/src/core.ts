// Browser-native TypeScript: type inference only, so this exact source can be
// embedded as JavaScript without a runtime, bundler, or generated-file drift.
// The UI is several classic scripts sharing one global scope; ui/index.html
// loads them in order, this file first and main.ts last.

const TOKEN_KEY = "open-process-launch-token";
const PROJECT_DB = "open-process-project-folders";
const PROJECT_STORE = "handles";
const RECENT_FOLDER_KEY = "recent-project-folder";
const mappingFields = [
  { key: "case_id", label: "Item identifier", required: true },
  { key: "activity", label: "Step", required: true },
  { key: "timestamp", label: "Recorded time", required: true },
  { key: "end_timestamp", label: "End timestamp", required: false },
  { key: "event_order", label: "Event order", required: false },
  { key: "resource", label: "Person or team", required: false },
  { key: "lifecycle", label: "Lifecycle", required: false },
  { key: "cost", label: "Cost", required: false },
];

let launchToken = null;
let currentProfile = null;
let currentAnalysis = null;
let currentDiscovery = null;
let currentCaseBaseline = null;
let caseConformanceRevision = 0;
let currentCaseConformance = null;
let currentOverlay = "cases";
let currentModelView = "observed";
let formalModelLoading = false;
let currentMapping = null;
let filterCatalog = null;
let currentFilter = {};
let currentPolicy = {};
let currentRules = {};
let pendingProject = null;
let currentComparisonOptions = null;
let currentOcelAnalysis = null;
let currentOcelUpload = null;
let currentOcelDiscoveredModel = null;
let currentOcelBaseline = null;
let mapActualSize = false;
// The map explorer's selection and the engine's map view for the active view.
let currentMapSelection = { mode: "limits", activities: 0, connections: 0 };
let currentMapView = null;
let mapViewRevision = 0;
let currentInvestigationRecord = null;

const viewNames = { home: "Projects", report: "Overview", map: "Process", objects: "Connected processes", variants: "Variants", cases: "Items", timing: "Analysis", compare: "Compare", conformance: "Conformance", quality: "Data checks", import: "Check your data" };
const viewDescriptions = {
  map: "See the steps recorded across all items, and the average time between them.",
  variants: "A variant is one distinct route: the exact sequence of recorded steps an item followed.",
  cases: "Follow one invoice, ticket or other item from its first recorded step to its last.",
  timing: "See how long items take from first to last recorded step, and which connections account for the most time.",
  compare: "Compare completion times and process differences between groups or periods.",
  conformance: "Compare what happened with your approved process. Review exceptions before drawing an audit conclusion.",
};
function itemLabels(mapping = currentMapping) {
  // A followed item type names itself.
  if (typeof currentFollow !== "undefined" && currentFollow) return { singular: currentFollow.singular, plural: currentFollow.plural };
  const field = (mapping?.case_id || "").toLowerCase();
  if (/invoice/.test(field)) return { singular: "invoice", plural: "Invoices" };
  if (/ticket|incident/.test(field)) return { singular: "ticket", plural: "Tickets" };
  if (/purchase_order|purchase order|\bpo\b/.test(field)) return { singular: "purchase order", plural: "Purchase orders" };
  if (/order/.test(field)) return { singular: "order", plural: "Orders" };
  if (/lead/.test(field)) return { singular: "lead", plural: "Leads" };
  if (/opportunit/.test(field)) return { singular: "opportunity", plural: "Opportunities" };
  if (/claim/.test(field)) return { singular: "claim", plural: "Claims" };
  // "Application ID", "case number", "Shipment": the column names the item.
  const named = field.replace(/[_-]+/g, " ").replace(/\b(id|number|no|nr|key|ref|reference)\b/g, "").trim();
  if (/^[a-z]+( [a-z]+)?$/.test(named) && !/^(case|concept|trace|name)$/.test(named)) {
    const plural = /[^aeiou]y$/.test(named) ? `${named.slice(0, -1)}ies` : /(s|x|z|ch|sh)$/.test(named) ? `${named}es` : `${named}s`;
    return { singular: named, plural: plural.charAt(0).toUpperCase() + plural.slice(1) };
  }
  return { singular: "item", plural: "Work items" };
}
function measuredDuration(seconds, eligible) {
  return eligible > 0 ? formatDuration(seconds) : "Time unavailable";
}
// A launch link may name a project and a page to land on. It may also carry
// an access key (older links, project sessions), which is kept for this tab;
// glasstarn's own page works without one.
function tokenFromLaunchUrl() {
  const fragment = new URLSearchParams(window.location.hash.slice(1));
  const query = new URLSearchParams(window.location.search);
  const launch = fragment.get("token") || fragment.get("project") ? fragment : query;
  const token = launch.get("token");
  const project = launch.get("project");
  if (project && /^[0-9a-f]{32}$/.test(project)) requestedWorkspaceId = project;
  const view = launch.get("view");
  if (["map", "report", "variants", "cases"].includes(view)) requestedView = view;
  if (token && /^[0-9a-f]{64}$/.test(token)) {
    sessionStorage.setItem(TOKEN_KEY, token);
    if (fragment.get("token")) history.replaceState(null, "", window.location.pathname);
    return token;
  }
  return sessionStorage.getItem(TOKEN_KEY);
}

function setConnection(state, label) {
  const connection = document.querySelector("#connection");
  const connectionLabel = document.querySelector("#connection-label");
  if (!connection || !connectionLabel) return;
  connection.dataset.state = state;
  connectionLabel.textContent = label;
}

function showAccessError() {
  const error = document.querySelector("#access-error");
  if (error) error.hidden = false;
}

function formatBytes(bytes) {
  const units = ["B", "KiB", "MiB", "GiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toLocaleString(undefined, { maximumFractionDigits: unit ? 1 : 0 })} ${units[unit]}`;
}

function setUploadStatus(message, state = "busy") {
  const status = document.querySelector("#upload-status");
  const text = document.querySelector("#upload-status-text");
  if (!status || !text) return;
  status.hidden = false;
  status.dataset.state = state;
  text.textContent = message;
}

function setProjectStatus(message, state = "") {
  const status = document.querySelector("#project-status");
  if (!status) return;
  status.textContent = message;
  status.dataset.state = state;
}

function supportedFile(file) {
  return /\.(csv|parquet|xes|xes\.gz|jsonocel|xmlocel|xml|sqlite|sqlite3)$/i.test(file.name);
}

function isOcelFile(file) {
  return /\.(jsonocel|xmlocel|xml|sqlite|sqlite3)$/i.test(file.name);
}

function selectedValues(select) {
  return [...select.selectedOptions].map((option) => option.value);
}

function numberOrNull(input, scale = 1) {
  if (!input.value) return null;
  const value = Number(input.value);
  return Number.isFinite(value) && value >= 0 ? Math.round(value * scale) : null;
}

function wallTimeMicros(input) {
  if (!input.value) return null;
  const milliseconds = Date.parse(`${input.value}Z`);
  return Number.isFinite(milliseconds) ? milliseconds * 1000 : null;
}

function formatDuration(totalSeconds) {
  const seconds = Math.abs(totalSeconds);
  if (seconds >= 86400) return `${(totalSeconds / 86400).toLocaleString(undefined, { maximumFractionDigits: 1 })}d`;
  if (seconds >= 3600) return `${(totalSeconds / 3600).toLocaleString(undefined, { maximumFractionDigits: 1 })}h`;
  if (seconds >= 60) return `${(totalSeconds / 60).toLocaleString(undefined, { maximumFractionDigits: 1 })}m`;
  return `${totalSeconds.toLocaleString()}s`;
}

function svgElement(name, attributes = {}) {
  const element = document.createElementNS("http://www.w3.org/2000/svg", name);
  for (const [key, value] of Object.entries(attributes)) element.setAttribute(key, String(value));
  return element;
}

function appendArrowMarker(defs, id, color, size = 18) {
  const marker = svgElement("marker", {
    id,
    viewBox: "0 0 10 10",
    refX: 9.35,
    refY: 5,
    markerWidth: size,
    markerHeight: size,
    markerUnits: "userSpaceOnUse",
    orient: "auto",
  });
  marker.append(svgElement("path", {
    d: "M 0.5 0.5 L 9.6 5 L 0.5 9.5 L 2.7 5 z",
    fill: color,
  }));
  defs.append(marker);
}

function diagramHeader(svg, width, eyebrow, detail) {
  const header = svgElement("g", { class: "diagram-header", "aria-hidden": "true" });
  header.append(svgElement("circle", { cx: 32, cy: 30, r: 4 }));
  const title = svgElement("text", { x: 44, y: 34, class: "diagram-eyebrow" });
  title.textContent = eyebrow;
  const meta = svgElement("text", { x: width - 28, y: 34, "text-anchor": "end", class: "diagram-meta" });
  meta.textContent = detail;
  header.append(title, meta, svgElement("line", { x1: 24, y1: 54, x2: width - 24, y2: 54 }));
  svg.append(header);
}

async function verifyLocalService() {
  // The page works without a key: the service answers its own page. A key in
  // the link (older links, project sessions) is still sent.
  const token = tokenFromLaunchUrl() || "";

  try {
    const response = await projectFetch("/api/health", {
      headers: { Authorization: `Bearer ${token}` },
      cache: "no-store",
    });
    if (!response.ok) throw new Error(`health returned ${response.status}`);
    const health = await response.json();
    if (health.status !== "ok") throw new Error("unexpected health response");
    launchToken = token;
    setConnection("ready", "Local service ready");
    showUpdateNote(health.update_available);
    const choose = document.querySelector("#choose-log");
    if (choose) choose.disabled = false;
    const openProject = document.querySelector("#open-project");
    if (openProject) openProject.disabled = false;
    const folder = document.querySelector("#open-project-folder");
    if (folder && "showDirectoryPicker" in window) folder.hidden = false;
    if (health.project_folders) await initializeWorkspaces();
    else void refreshRecentProjectAction();
  } catch (_error) {
    sessionStorage.removeItem(TOKEN_KEY);
    setConnection("error", "Connection lost");
    showAccessError();
  }
}

// Switches (.seg) get a knob that slides under the chosen button. It follows
// aria-pressed changes and size changes, so callers only set aria-pressed.
function placeSegmentKnob(seg) {
  const knob = seg.querySelector(".seg-knob");
  const chosen = seg.querySelector('button[aria-pressed="true"]');
  if (!knob) return;
  if (!chosen || !chosen.offsetWidth) { knob.style.opacity = chosen ? knob.style.opacity : "0"; return; }
  knob.style.opacity = "1";
  knob.style.width = `${chosen.offsetWidth}px`;
  knob.style.transform = `translateX(${chosen.offsetLeft}px)`;
}

function enhanceSegments(root = document) {
  for (const seg of root.querySelectorAll?.(".seg") || []) {
    if (seg.querySelector(".seg-knob")) continue;
    const knob = document.createElement("span");
    knob.className = "seg-knob";
    knob.setAttribute("aria-hidden", "true");
    seg.prepend(knob);
    const update = () => placeSegmentKnob(seg);
    new MutationObserver(update).observe(seg, { subtree: true, attributes: true, attributeFilter: ["aria-pressed"] });
    if (typeof ResizeObserver === "function") new ResizeObserver(update).observe(seg);
    update();
  }
}

// Long one-line summaries are cut to fit; the whole sentence stays readable
// on hover.
function keepFullTextAsTitle(element) {
  if (!element) return;
  const update = () => { const text = element.textContent.trim(); element.title = element.dataset.detail ? `${text}. ${element.dataset.detail}` : text; };
  new MutationObserver(update).observe(element, { childList: true, characterData: true, subtree: true, attributes: true, attributeFilter: ["data-detail"] });
  update();
}

// Errors in words a person can act on. Raw system messages (file paths,
// "Failed to fetch") stay out of sight; the useful part is kept.
function plainErrorMessage(text) {
  const message = String(text || "");
  if (/No such file or directory|os error 2/i.test(message) && /sources\//.test(message)) {
    return "This project's data file is missing from its folder. It may have been moved or deleted. Ask your agent to import the data again, or put the file back in the project's sources folder.";
  }
  if (/no longer available|choose it again|no longer loaded/i.test(message)) {
    return "This project was opened again in another tab or window, so this page is out of date. Reload this page to keep working.";
  }
  if (/Failed to fetch|NetworkError|Load failed|ERR_NETWORK/i.test(message)) {
    return "glasstarn stopped answering. Check that the app is still running on this Mac, then try again.";
  }
  return message;
}

// A newer glasstarn exists: one quiet note in the top bar, with how to get it.
function showUpdateNote(version) {
  const connection = document.querySelector("#connection");
  if (!version || !connection?.after || document.querySelector("#update-note")) return;
  const note = document.createElement("span");
  note.id = "update-note";
  note.className = "pill neutral update-note";
  note.textContent = `glasstarn ${version} is available`;
  note.title = "Ask your agent to update glasstarn, or run glasstarn update in Terminal.";
  connection.after(note);
}
