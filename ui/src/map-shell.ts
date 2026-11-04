// The process screen's frame: one slim toolbar (scope, filters, map style,
// Count | Time, icon actions), the map filling the rest, and a step panel
// that opens only when a step is picked.

// Whether the person has picked a step on the map in front of them.
function mapHasSelection() {
  const subway = typeof activeMapStyle === "function" && activeMapStyle() === "subway";
  return !!(subway ? (typeof subwaySelection !== "undefined" && (subwaySelection || subwayConnection)) : resultsSelection);
}

// The scope chip: how many items the map is drawn from, and how many
// filters narrow them.
function mapScopeText(cases, plural, filters) {
  const base = `${Number(cases || 0).toLocaleString()} ${plural}`;
  return filters ? `${base} · ${filters} ${filters === 1 ? "filter" : "filters"}` : base;
}

// While filters are being applied the page says so at once: the scope chip
// reads "Updating…" and the map dims under a small note, instead of showing
// stale or zero counts.
function setMapUpdating(updating) {
  if (!document.body) return;
  if (updating) document.body.dataset.updating = "true"; else delete document.body.dataset.updating;
  renderMapScope();
}

function renderMapScope() {
  const chip = document.querySelector("#map-scope");
  if (!chip) return;
  if (document.body?.dataset.updating === "true" || !currentAnalysis) { chip.textContent = "Updating…"; return; }
  const cases = currentAnalysis?.summary?.cases ?? currentAnalysis?.view?.cases_shown ?? 0;
  let filters = 0;
  try { filters = typeof activeFilterCount === "function" && currentFilter ? activeFilterCount({ with_activities: [], without_activities: [], directly_follows: [], eventually_follows: [], attribute_equals: [], ...currentFilter }) : 0; } catch (_error) { filters = 0; }
  chip.textContent = mapScopeText(cases, itemLabels().plural.toLowerCase(), filters);
  renderLinkedChip();
}

// A selection made in Measure's Linked mode narrows the connected records
// themselves, outside the item filters. It gets its own chip so it is never
// in force unseen.
function ocelSelectionActive(filter) {
  if (!filter) return false;
  return Object.entries(filter).some(([key, value]) => key !== "cohort" && value !== null && value !== undefined && value !== false
    && (Array.isArray(value) ? value.length > 0 : typeof value === "object" ? Object.keys(value).length > 0 : true));
}

function renderLinkedChip() {
  const list = document.querySelector("#filter-chip-list");
  if (!list) return;
  let chip = list.querySelector?.(".filter-chip.linked");
  const active = typeof currentFollow !== "undefined" && currentFollow && ocelSelectionActive(currentOcelAnalysis?.filter);
  if (!active) { chip?.remove(); return; }
  if (chip) return;
  chip = document.createElement("span");
  chip.className = "filter-chip linked";
  const text = document.createElement("span");
  text.textContent = "Linked selection from Measure";
  const remove = document.createElement("button");
  remove.type = "button";
  remove.textContent = "×";
  remove.setAttribute("aria-label", "Remove the linked selection");
  remove.addEventListener("click", async () => {
    chip.remove();
    setMapUpdating(true);
    try { await loadJourney(currentFollow.type); } finally { setMapUpdating(false); }
  });
  chip.append(text, remove);
  list.append(chip);
}

// Count | Time stands in for the metric dropdown, which stays hidden and in
// charge so everything that reads it keeps working.
function syncMapMetric() {
  const metric = document.querySelector("#process-metric");
  const value = metric?.value || currentOverlay;
  for (const button of document.querySelectorAll?.("[data-metric-choice]") || []) {
    button.setAttribute("aria-pressed", String(value === "performance" ? button.dataset.metricChoice === "performance" : button.dataset.metricChoice === "cases"));
  }
}

// The filter chips and + Filter live in the map toolbar on the process
// screen and in the page header everywhere else.
function placeFilterChips(view) {
  const chips = document.querySelector("#filter-chips");
  const slot = document.querySelector("#map-filter-slot");
  const home = document.querySelector(".page-head");
  if (!chips || !slot || !home || typeof slot.append !== "function") return;
  const target = view === "map" ? slot : home;
  if (chips.parentElement !== target) target.append(chips);
}

// Whether a click on the map landed on nothing: the bare canvas, not a step,
// arrow, station or label drawn on it.
function mapClickIsEmpty(target) {
  return !!target && (target.tagName || "").toLowerCase() === "svg";
}

// A short note at the top of the map; it fades on its own.
let mapNoteTimer = 0;
function showMapNote(text, tone = "") {
  const note = document.querySelector("#map-note");
  if (!note) return;
  note.textContent = text;
  if (tone) note.dataset.tone = tone; else delete note.dataset.tone;
  note.hidden = false;
  clearTimeout(mapNoteTimer);
  mapNoteTimer = setTimeout(() => { note.hidden = true; }, tone === "error" ? 8000 : 4000);
}

function closeMapStep() {
  resultsSelection = null;
  if (typeof resultsEdgeFocus !== "undefined") resultsEdgeFocus = null;
  if (typeof subwaySelection !== "undefined") { subwaySelection = null; subwayConnection = null; }
  const panel = document.querySelector("#map-step-detail");
  if (panel) panel.hidden = true;
  if (typeof renderMap === "function") renderMap();
}

// After zooming, the scroll position that keeps the point under the cursor
// in place: the same share of the drawing, now at its new size.
function zoomScrollTarget(scroll, cursor, oldSize, newSize) {
  if (!oldSize) return scroll;
  return Math.max(0, (scroll + cursor) / oldSize * newSize - cursor);
}

// Like a web map: drag the canvas to move around a large map, and pinch (or
// ⌘/Ctrl + scroll) to zoom where the pointer is. A drag never counts as a
// click, so it never picks a step or closes a panel.
function initializeMapPan(wrap, which = "process") {
  let start = null, moved = false;
  const pannable = () => wrap.scrollWidth > wrap.clientWidth + 1 || wrap.scrollHeight > wrap.clientHeight + 1;
  wrap.addEventListener("pointerenter", () => wrap.classList.toggle("can-pan", pannable()));
  wrap.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || !pannable()) return;
    start = { x: event.clientX, y: event.clientY, left: wrap.scrollLeft, top: wrap.scrollTop, id: event.pointerId };
    moved = false;
  });
  wrap.addEventListener("pointermove", (event) => {
    if (!start || event.pointerId !== start.id) return;
    const dx = event.clientX - start.x, dy = event.clientY - start.y;
    if (!moved && Math.hypot(dx, dy) < 4) return;
    if (!moved) { moved = true; wrap.setPointerCapture?.(event.pointerId); wrap.classList.add("panning"); }
    wrap.scrollLeft = start.left - dx;
    wrap.scrollTop = start.top - dy;
  });
  const end = () => {
    if (!start) return;
    start = null;
    wrap.classList.remove("panning");
    if (!moved) return;
    const swallow = (event) => { event.stopPropagation(); event.preventDefault(); };
    wrap.addEventListener("click", swallow, { capture: true, once: true });
    setTimeout(() => wrap.removeEventListener("click", swallow, { capture: true }), 80);
  };
  wrap.addEventListener("pointerup", end);
  wrap.addEventListener("pointercancel", end);
  wrap.addEventListener("wheel", (event) => {
    if (!event.ctrlKey && !event.metaKey) return;
    if (typeof explorerZoom === "undefined" || typeof fitExplorer !== "function") return;
    event.preventDefault();
    const svg = [...wrap.querySelectorAll("svg")].find((item) => !item.hasAttribute("hidden") && item.getBoundingClientRect().width);
    if (!svg) return;
    const box = wrap.getBoundingClientRect(), before = svg.getBoundingClientRect();
    const cx = event.clientX - box.left, cy = event.clientY - box.top;
    const next = Math.max(0.5, Math.min(4, explorerZoom[which] * Math.exp(-event.deltaY * 0.01)));
    if (next === explorerZoom[which]) return;
    explorerZoom[which] = next;
    fitExplorer(which);
    const after = svg.getBoundingClientRect();
    wrap.scrollLeft = zoomScrollTarget(wrap.scrollLeft, cx, before.width, after.width);
    wrap.scrollTop = zoomScrollTarget(wrap.scrollTop, cy, before.height, after.height);
    wrap.classList.toggle("can-pan", pannable());
  }, { passive: false });
}

function initializeMapShell() {
  for (const button of document.querySelectorAll("[data-metric-choice]")) {
    button.addEventListener("click", () => {
      const metric = document.querySelector("#process-metric");
      if (!metric || metric.value === button.dataset.metricChoice) return;
      metric.value = button.dataset.metricChoice;
      metric.dispatchEvent(new Event("change", { bubbles: true }));
      syncMapMetric();
    });
  }
  document.querySelector("#process-metric")?.addEventListener("change", syncMapMetric);
  document.querySelector("#map-step-close")?.addEventListener("click", closeMapStep);
  for (const button of document.querySelectorAll("[data-subway-detail]")) button.addEventListener("click", () => setSubwayDetail(button.dataset.subwayDetail));
  // A click on empty map space puts the panel away, like Escape.
  for (const map of document.querySelectorAll("#results-map, #subway-map")) {
    map.addEventListener("click", (event) => {
      if (mapClickIsEmpty(event.target) && event.target === map && !document.querySelector("#map-step-detail")?.hidden) closeMapStep();
    });
  }
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape" || document.body.dataset.view !== "map" || document.querySelector(".select-menu")) return;
    // Escape puts away the nearest open thing: the objects panel, then the step panel.
    if (!document.querySelector("#map-view-controls")?.hidden && typeof toggleMapControls === "function") toggleMapControls(false);
    else if (!document.querySelector("#analysis-inspector")?.hidden) document.querySelector("#studio-close")?.click();
    else if (!document.querySelector("#saved-views-panel")?.hidden) document.querySelector("#saved-views-close")?.click();
    else if (!document.querySelector("#map-step-detail")?.hidden) closeMapStep();
  });
  // A clicked icon's tip steps aside until the pointer leaves it.
  for (const button of document.querySelectorAll(".results .icon-btn[data-tip]")) {
    button.addEventListener("click", () => { button.dataset.tipOff = "true"; });
    button.addEventListener("mouseleave", () => { delete button.dataset.tipOff; });
  }
  // On a narrow screen the side panels sit under the map, so opening one
  // brings it into view instead of seeming to do nothing.
  if (typeof MutationObserver === "function") {
    const panels = ["#map-step-detail", "#analysis-inspector", "#saved-views-panel"].map((id) => document.querySelector(id)).filter(Boolean);
    const watcher = new MutationObserver((changes) => {
      if (window.innerWidth > 900) return;
      const opened = changes.map((change) => change.target).find((panel) => !panel.hidden);
      opened?.scrollIntoView?.({ behavior: "smooth", block: "start" });
    });
    for (const panel of panels) watcher.observe(panel, { attributes: true, attributeFilter: ["hidden"] });
  }
  // The map refits whenever its canvas changes size: a side panel opening or
  // closing, the window resizing.
  const canvas = document.querySelector(".map-workspace .mapwrap");
  if (canvas) initializeMapPan(canvas);
  // Once the map is moved, the floating line legend fades so it never hides
  // the names beneath it; pointing at it brings it back.
  canvas?.addEventListener("scroll", () => canvas.closest(".map-workspace")?.classList.toggle("map-moved", canvas.scrollTop > 4 || canvas.scrollLeft > 4), { passive: true });
  // The Variants map moves and zooms the same way.
  const variantCanvas = document.querySelector(".variant-canvas .mapwrap");
  if (variantCanvas) initializeMapPan(variantCanvas, "variant");
  if (canvas && typeof ResizeObserver === "function") {
    let lastWidth = 0, frame = 0;
    new ResizeObserver(() => {
      if (!canvas.clientWidth || Math.abs(canvas.clientWidth - lastWidth) < 4) return;
      lastWidth = canvas.clientWidth;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => { if (typeof fitExplorer === "function") fitExplorer("process"); });
    }).observe(canvas);
  }
  syncMapMetric();
}
