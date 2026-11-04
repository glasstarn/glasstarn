// Cases: one item's recorded history as a timeline.

let caseDetailRevision = 0;
let caseDetailSequence = null;

// How one item's path differs from the usual route (the most common one):
// steps it skipped, steps the usual route does not have, and steps it repeated.
function itemDepartures(sequence, usual) {
  if (!usual?.length) return [];
  const counts = new Map();
  for (const step of sequence) counts.set(step, (counts.get(step) || 0) + 1);
  const notes = [];
  for (const step of usual) if (!counts.has(step)) notes.push(`Skipped ${step}`);
  for (const [step, count] of counts) if (!usual.includes(step)) notes.push(`Extra step: ${step}`);
  for (const [step, count] of counts) if (count > 1) notes.push(`Repeated ${step} ×${count}`);
  return notes;
}

// The step reached after the longest wait, or -1 when nothing waited.
function itemLongestGap(events) {
  let best = -1;
  events.forEach((event, index) => { if (event.since_previous_seconds > 0 && (best < 0 || event.since_previous_seconds > events[best].since_previous_seconds)) best = index; });
  return best;
}

function itemWhen(timestamp) {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return String(timestamp);
  return date.toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

// The item sheet slides in from the right over any page, so opening an item
// never moves you away from where you were.
function setItemSheetOpen(open) {
  const sheet = document.querySelector("#item-sheet");
  if (!sheet) return;
  sheet.hidden = !open;
  if (!sheet.dataset.wired) {
    sheet.dataset.wired = "true";
    document.querySelector("#item-sheet-close")?.addEventListener("click", () => setItemSheetOpen(false));
    document.addEventListener("keydown", (event) => { if (event.key === "Escape" && !sheet.hidden && !document.querySelector(".select-menu")) setItemSheetOpen(false); });
    document.addEventListener("mousedown", (event) => {
      if (sheet.hidden || sheet.contains(event.target) || event.target.closest?.(".case-chip, #results-cases tr, .select-menu")) return;
      setItemSheetOpen(false);
    });
  }
}

// Shows the item's own path on the classic process map: just its route (by
// the route number the item list knows, or found among the ranked routes),
// otherwise every step with its path lit.
async function showItemOnMap(caseId, sequence, knownRank = null) {
  const rank = knownRank || (currentAnalysis?.variants || []).findIndex((variant) => JSON.stringify(variant.sequence) === JSON.stringify(sequence)) + 1;
  resultsRouteFocus = { rank: rank || null, item: caseId, sequence, cases: 1 };
  // A step left picked from before would fade most of the path.
  resultsSelection = null;
  if (typeof resultsEdgeFocus !== "undefined") resultsEdgeFocus = null;
  if (typeof connectedVariantFocus !== "undefined") connectedVariantFocus = null;
  if (typeof mapStyle !== "undefined") mapStyle = "classic";
  setItemSheetOpen(false);
  showView("map");
  if (typeof renderMapStyle === "function") renderMapStyle();
  await loadMapView(rank ? { mode: "routes", variants: [rank] } : { mode: "limits", activities: currentAnalysis?.summary?.activities || 100, connections: currentAnalysis?.edges?.length || 200 });
}

async function openCase(caseId, { rank = null } = {}) {
  const revision = ++caseDetailRevision;
  const profile = currentProfile;
  const filter = currentFilter;
  const detail = document.querySelector("#case-detail");
  if (!detail || !currentMapping) return;
  setItemSheetOpen(true);
  const title = document.querySelector("#item-sheet-title");
  if (title) title.textContent = `${itemLabels().singular.replace(/^./, (letter) => letter.toUpperCase())} ${caseId}`;
  detail.textContent = `Opening ${caseId}…`;
  try {
    const response = await projectFetch("/api/case", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ upload_id: currentProfile.upload_id, mapping: currentMapping, policy: currentPolicy, rules: currentRules, case_id: caseId }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (revision !== caseDetailRevision || profile !== currentProfile || filter !== currentFilter) return;
    if (!response.ok) throw new Error(payload.error || `case returned ${response.status}`);
    detail.replaceChildren();
    const sequence = payload.events.map((event) => event.activity);
    caseDetailSequence = sequence;
    const pill = (text) => { const span = document.createElement("span"); span.className = "pill neutral"; span.textContent = text; return span; };
    const facts = document.createElement("div"); facts.className = "item-facts";
    facts.append(pill(`${payload.events.length.toLocaleString()} steps`), pill(`${formatDuration(payload.throughput_seconds)} start to finish`));
    for (const [name, value] of payload.attributes) facts.append(pill(`${name}: ${value}`));
    const map = document.createElement("button"); map.type = "button"; map.className = "primary-action item-map"; map.textContent = "Show on the process map";
    map.addEventListener("click", () => void showItemOnMap(payload.case, sequence, rank));
    detail.append(facts, map);

    const departures = itemDepartures(sequence, currentAnalysis?.variants?.[0]?.sequence);
    const compare = document.createElement("div"); compare.className = "item-departures";
    const lead = document.createElement("p"); lead.className = "item-note";
    lead.textContent = !currentAnalysis?.variants?.length ? "" : departures.length ? "Compared with the usual route:" : "Follows the usual route exactly.";
    compare.append(lead);
    if (departures.length) { const list = document.createElement("ul"); for (const text of departures.slice(0, 6)) { const li = document.createElement("li"); li.textContent = text; list.append(li); } compare.append(list); }
    detail.append(compare);

    // The timeline: each step with when it happened; between steps, how long
    // it took, the longest wait called out.
    const longest = itemLongestGap(payload.events);
    const timeline = document.createElement("ol"); timeline.className = "item-timeline";
    payload.events.forEach((event, index) => {
      if (index > 0 && event.since_previous_seconds !== null) {
        const gap = document.createElement("li"); gap.className = `item-gap${index === longest ? " longest" : ""}`;
        gap.textContent = index === longest ? `Longest wait · ${formatDuration(event.since_previous_seconds)}` : `${formatDuration(event.since_previous_seconds)} later`;
        timeline.append(gap);
      }
      const step = document.createElement("li"); step.className = "item-step";
      const name = document.createElement("b"); name.textContent = event.activity;
      const when = document.createElement("span"); when.className = "item-when"; when.textContent = itemWhen(event.timestamp);
      step.append(name, when);
      if (event.resource && !/^(none|null|n\/a)$/i.test(event.resource.trim())) { const by = document.createElement("span"); by.className = "item-by"; by.textContent = event.resource; step.append(by); }
      // Columns that change between steps show where they changed.
      if (event.changes?.length) { const changed = document.createElement("span"); changed.className = "item-changes"; changed.textContent = event.changes.map(([field, value]) => `${field}: ${value}`).join(" · "); step.append(changed); }
      timeline.append(step);
    });
    detail.append(timeline);
    if (payload.gap_is_waiting === false) { const note = document.createElement("p"); note.className = "item-note"; note.textContent = "Times between steps are elapsed time on the clock, not hands-on work."; detail.append(note); }
    const provenance = document.createElement("details"); provenance.className = "source-details";
    const sourceTitle = document.createElement("summary"); sourceTitle.textContent = "Source details";
    const sourceCopy = document.createElement("p"); sourceCopy.className = "item-note"; sourceCopy.textContent = `Source rows ${payload.source_rows.join(", ")} · timestamp interpretation: ${payload.timestamp_reading}`;
    provenance.append(sourceTitle, sourceCopy); detail.append(provenance);
  } catch (error) {
    if (revision !== caseDetailRevision || profile !== currentProfile || filter !== currentFilter) return;
    detail.textContent = plainErrorMessage(error instanceof Error ? error.message : "This item could not be opened.");
  }
}
