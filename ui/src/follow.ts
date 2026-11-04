// Connected projects use the same screens as a single log. The person picks
// which kind of item to follow (deals, orders, invoices…); the engine turns
// each item of that kind into a case of its own steps, and every tab below
// is then about those items, with the same meaning as for any single log.

let restoredFollowedView = null;
let currentFollow = null;
let followPending = false;
let connectedScopeRequest = null;
let followTypes = [];
// Business names the project gives its item types ("opportunity" → "Deals").
let currentItemNames = {};

// A readable plural for an item type: the project's business name when it
// has one, otherwise "opportunity" → "Opportunities".
function itemTypeName(type, plural = true) {
  const named = currentItemNames?.[type];
  if (named) return plural ? named : named.replace(/ies$/i, "y").replace(/(ss|[^s])$/i, "$1").replace(/s$/i, "");
  // "CreditNote", "credit_note" and "credit note" all read "Credit notes".
  const words = String(type).replace(/([a-z])([A-Z])/g, "$1 $2").replace(/[_-]+/g, " ").trim();
  const lower = /[a-z]/.test(words) ? words.toLowerCase() : words;
  const capital = lower.charAt(0).toUpperCase() + lower.slice(1);
  // Many logs name types in the plural already ("orders", "packages").
  if (/[^su]s$/i.test(capital) && !/(ss|us|is)$/i.test(capital)) {
    if (plural) return capital;
    if (/ies$/i.test(capital)) return `${capital.slice(0, -3)}y`;
    if (/(ss|x|z|ch|sh)es$/i.test(capital)) return capital.slice(0, -2);
    return capital.slice(0, -1);
  }
  if (!plural) return capital;
  if (/[^aeiou]y$/i.test(capital)) return `${capital.slice(0, -1)}ies`;
  if (/(s|x|z|ch|sh)$/i.test(capital)) return `${capital}es`;
  return `${capital}s`;
}

// Types worth following, most involved in recorded steps first.
function followCandidates(analysis) {
  const involvement = new Map();
  for (const link of analysis.graphs?.participation || []) {
    involvement.set(link.object_type, (involvement.get(link.object_type) || 0) + link.occurrences);
  }
  // Every type that takes part in at least one recorded step can be followed.
  const listed = [...involvement.keys()];
  return listed.sort((a, b) => (involvement.get(b) || 0) - (involvement.get(a) || 0) || a.localeCompare(b));
}

function renderFollowBar() {
  const bar = document.querySelector("#follow-bar");
  const group = document.querySelector("#follow-types");
  if (!bar || !group) return;
  bar.hidden = !followTypes.length;
  group.replaceChildren(...followTypes.map((type) => {
    const button = document.createElement("button");
    button.type = "button";
    button.disabled = followPending;
    button.textContent = itemTypeName(type);
    if (typeof subwayTypeColour === "function") button.style.setProperty("--object-colour", `var(${subwayTypeColour(type)})`);
    button.setAttribute("aria-pressed", String(currentFollow?.type === type));
    button.addEventListener("click", () => { if (currentFollow?.type !== type) void followType(type); });
    return button;
  }));
  const hint = document.querySelector("#follow-hint");
  if (hint) hint.textContent = "";
}

// Called once a connected log is loaded: follow the main type, or keep the
// current one if it still exists.
function startFollowing(analysis) {
  followTypes = analysis.ready_for_analysis ? followCandidates(analysis) : [];
  if (!followTypes.length) { currentFollow = null; renderFollowBar(); return; }
  const saved = restoredFollowedView; restoredFollowedView = null;
  const keep = saved?.object_type && followTypes.includes(saved.object_type) ? saved.object_type : currentFollow && followTypes.includes(currentFollow.type) ? currentFollow.type : followTypes[0];
  if (typeof showOpening === "function") showOpening(currentWorkspace?.name);
  return followType(keep,saved?.filter || {}).finally(() => { if (typeof hideOpening === "function") hideOpening(); });
}

async function followType(type, filter = {}, preserveScope = false) {
  if (followPending) return false;
  const previousFollow = currentFollow;
  const previousProfile = currentProfile;
  const clearedFilters = Boolean(previousFollow && previousFollow.type !== type && !preserveScope && filterChips(currentFilter || {}).length);
  followPending = true;
  currentJourney = null;
  const status = document.querySelector("#follow-status");
  currentFollow = { type, singular: itemTypeName(type, false).toLowerCase(), plural: itemTypeName(type) };
  renderFollowBar();
  if (status) status.textContent = `Following ${currentFollow.plural.toLowerCase()}…`;
  try {
    const response = await projectFetch("/api/ocel/follow", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ upload_id: currentOcelUpload, object_type: type, preserve_scope: preserveScope }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `following returned ${response.status}`);
    currentFollow.summary = payload.follow;
    // The followed log is an ordinary case log: reuse the single-log path.
    currentProfile = { upload_id: payload.upload_id, file_name: payload.file_name, columns: [], proposed_mapping: payload.mapping };
    currentPolicy = {};
    currentRules = {};
    filterCatalog = null;
    resetFilterControls();
    if (!await analyzeMapping(payload.mapping, filter)) throw new Error("The followed history needs attention in Data quality.");
    if (connectedScopeRequest) await connectedScopeRequest;

    await loadProvenance();
    if (status) status.textContent = clearedFilters ? "Filters were cleared when changing the followed item type." : "";
    return true;
  } catch (error) {
    if (currentProfile === previousProfile) currentFollow = previousFollow;
    if (status) status.textContent = error instanceof Error ? error.message : "This item type could not be followed.";
    return false;
  } finally { followPending = false; renderFollowBar(); }
}

// The journey across linked item types, measured by the engine from the
// followed type onward. Only engine figures are shown; a hand-off the engine
// cannot link is simply not a stage.
let currentJourney = null;
let journeyRevision = 0;
let connectedScopePending = false;
let connectedScopeError = "";

async function loadJourney(type) {
  const revision = ++journeyRevision;
  const profile = currentProfile;
  connectedScopePending = true;
  connectedScopeError = "";
  currentJourney = null;
  if (typeof rerenderSummary === "function") rerenderSummary();
  try {
    const response = await projectFetch("/api/ocel/scope", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ ocel_upload_id: currentOcelUpload, object_type: type, case_view: comparisonRequestBody() }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || "journey unavailable");
    if (revision !== journeyRevision || currentProfile !== profile || currentFollow?.type !== type) return;
    connectedScopePending = false;
    currentJourney = payload.journey;
    currentOcelAnalysis = payload.analysis;
    if (typeof renderResultsMap === "function") renderResultsMap();
  } catch (_error) {
    if (revision !== journeyRevision || currentProfile !== profile) return;
    currentJourney = null;
    connectedScopeError = "Connected records could not be resolved. Change the filter or reopen the project to retry.";
    if (typeof renderResultsMap === "function") renderResultsMap();
  }
  // Drop-outs join the Summary's key findings.
  if (typeof rerenderSummary === "function") rerenderSummary();
}

async function openJourneyItems(handoff) {
  if (await followType(handoff.via_object_type, { not_reached: [[handoff.from_activity, handoff.to_activity]] }, true)) showView("cases");
}
