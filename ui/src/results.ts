// Results: the results layout on engine data. Every figure comes from the analysis, the map view or a saved record;
// this file only lays them out.

const RESULTS_NODE_WIDTH = 330;
const RESULTS_SIDE_WIDTH = 260;
const RESULTS_NODE_HEIGHT = 38;
const RESULTS_ROW = 76;
const RESULTS_COLUMN = 340;

// Steps are pills sized to their name and count, up to the column's width.
function resultsNodeWidth(activity, frequency, main) {
  const text = String(activity).length * 7.3 + String((frequency || 0).toLocaleString()).length * 7.8;
  return Math.round(Math.min(main ? RESULTS_NODE_WIDTH : RESULTS_SIDE_WIDTH, Math.max(172, text + 64)));
}
let resultsSelection = null;
// The arrow picked on the classic map, when an arrow rather than a step was clicked.
let resultsEdgeFocus = null;
let resultsRouteFocus = null;
// The Overview finding the map was opened from, until the person goes back
// to the usual map.
let mapFindingFocus = null;

function focusResultsRoute(svg, sequence) {
  if (!sequence) return;
  const nodes = new Set(sequence);
  const edges = new Set(sequence.slice(1).map((step, index) => JSON.stringify([sequence[index], step])));
  for (const element of svg.querySelectorAll("[data-from]")) element.classList.toggle("dim", !edges.has(JSON.stringify([element.dataset.from, element.dataset.to])));
  for (const node of svg.querySelectorAll(".node")) node.classList.toggle("dim", !nodes.has(node.dataset.activity));
}

async function openResultsRoute(rank) {
  if (typeof document.createElementNS === "function" && document.querySelector("#variant-map")) { await openExplorerVariant(rank); return; }
  const variant = currentAnalysis?.variants?.[rank - 1];
  if (!variant) return;
  resultsRouteFocus = { rank, sequence: variant.sequence, cases: variant.cases };
  // No step is picked, so the whole route stays lit.
  resultsSelection = null;
  showView("map");
  if (typeof renderMapStyle === "function") renderMapStyle();
  await loadMapView({ mode: "routes", variants: [rank] });
}

// Joins a finding's filter onto the active one: lists are added to, single
// values set; empty parts of the finding's filter change nothing.
function mergeFilters(base, extra) {
  const out = { ...base };
  for (const [key, value] of Object.entries(extra || {})) {
    if (value === null || value === undefined || value === false) continue;
    if (Array.isArray(value)) {
      if (!value.length) continue;
      const prior = Array.isArray(out[key]) ? out[key] : [];
      const seen = new Set(prior.map((item) => JSON.stringify(item)));
      out[key] = [...prior, ...value.filter((item) => !seen.has(JSON.stringify(item)))];
    } else if (typeof value === "object") {
      if (!Object.keys(value).length) continue;
      out[key] = { ...(out[key] || {}), ...value };
    } else out[key] = value;
  }
  return out;
}

// Opening a finding shows exactly its items: its filter joins the active
// one (a chip says so), and its step or hand-off is picked out on the map.
async function openFindingFilter(fact) {
  const target = fact.target || {};
  const previous = currentFilter || {};
  resultsRouteFocus = null;
  if (typeof connectedVariantFocus !== "undefined") connectedVariantFocus = null;
  if (typeof mapStyle !== "undefined") mapStyle = "classic";
  resultsSelection = target.kind === "step" ? target.activity : target.kind === "edge" ? target.from : null;
  resultsEdgeFocus = target.kind === "edge" ? { from: target.from, to: target.to } : null;
  mapFindingFocus = { title: fact.title || "a finding", previous };
  // A step or hand-off is seen on the map; any other finding is a set of
  // items, listed with its filter chip.
  const onMap = target.kind === "step" || target.kind === "edge";
  showView(onMap ? "map" : "cases");
  if (typeof renderMapStyle === "function") renderMapStyle();
  await analyzeMapping(currentMapping, mergeFilters(previous, fact.filter), "filter");
  if (resultsSelection && typeof showMapStepDetail === "function") showMapStepDetail();
}

async function openFindingEvidence(fact) {
  const target = fact.target;
  if (fact.filter && currentMapping) { await openFindingFilter(fact); return; }
  if (!target) { showView(fact.view === "journey" ? "map" : fact.view); return; }
  if (target.kind === "field") { await openDriverItems(target); return; }
  if (target.kind === "handoff") {
    const handoff = currentJourney?.handoffs.find(item => item.via_object_type === target.object_type && item.from_activity === target.from && item.to_activity === target.to);
    if (handoff) await openJourneyItems(handoff); else showView("map");
    return;
  }
  resultsRouteFocus = null;
  if (typeof connectedVariantFocus !== "undefined") connectedVariantFocus = null;
  if (typeof mapStyle !== "undefined") mapStyle = "classic";
  resultsSelection = target.activity || target.from;
  resultsEdgeFocus = null;
  mapFindingFocus = { title: fact.title || "a finding" };
  showView("map");
  if (typeof renderMapStyle === "function") renderMapStyle();
  await loadMapView({ mode: "limits", activities: currentAnalysis?.summary?.activities || 100, connections: currentAnalysis?.edges?.length || 200 });
  const selectedNode = [...(document.querySelectorAll?.("#results-map .node") || [])].find(node => node.dataset.activity === resultsSelection);
  selectedNode?.scrollIntoView?.({behavior:"smooth",block:"center",inline:"center"});
  // Open the step's own panel too, so its × and the banner both lead back.
  if (typeof showMapStepDetail === "function") showMapStepDetail();
  if (target.kind === "edge" && currentMapView) {
    const edge = currentMapView.edges.find(edge => edge.from === target.from && edge.to === target.to);
    if (edge) { resultsEdgeFocus = { from: edge.from, to: edge.to }; resultsSelectEdge(edge); renderResultsMap(); }
  }
  if (typeof updateProcessExplorer === "function") updateProcessExplorer(currentMapView);
}

// A driver's items: filter to its field value, number band or person, and
// show them on the map.
async function openDriverItems(target) {
  const filter = { ...currentFilter };
  mapFindingFocus = { title: `${target.field} = ${target.value}`, previous: currentFilter || {} };
  if (target.field === "Handled by") {
    filter.handled_by = [...new Set([...(filter.handled_by || []), target.value])];
  } else if (target.at_least !== undefined && target.at_least !== null) {
    filter.attribute_between = [...(filter.attribute_between || []).filter((range) => range.attribute !== target.field), { attribute: target.field, at_least: target.at_least, at_most: target.at_most }];
  } else {
    filter.attribute_equals = [...(filter.attribute_equals || []).filter((pair) => pair[0] !== target.field), [target.field, target.value]];
  }
  showView("map");
  await analyzeMapping(currentMapping, filter, "filter");
}

// Back from a finding: its filter comes off, no step picked, the usual
// (main-flow) map.
function backToUsualMap() {
  const previous = mapFindingFocus?.previous;
  mapFindingFocus = null;
  if (previous && currentMapping) void analyzeMapping(currentMapping, previous, "filter");
  resultsRouteFocus = null;
  if (typeof connectedVariantFocus !== "undefined") connectedVariantFocus = null;
  if (typeof closeMapStep === "function") closeMapStep();
  document.querySelector("#process-reset")?.click();
}

function renderResults() {
  if (!currentAnalysis?.summary) return;
  const name = currentWorkspace?.name || "Project";
  const source = document.querySelector("#results-source");
  // A followed item type still comes from the connected source file.
  const sourceName = typeof currentFollow !== "undefined" && currentFollow && currentOcelAnalysis ? currentOcelAnalysis.source : currentAnalysis.source;
  if (source) source.textContent = `${String(sourceName || "").split(/[\\/]/).pop()} · ${currentAnalysis.summary.events.toLocaleString()} events`;
  const variants = document.querySelector("#results-variant-count");
  if (variants) variants.textContent = currentAnalysis.summary.variants.toLocaleString();
  const deviations = document.querySelector("#results-conf-count");
  if (deviations) {
    deviations.hidden = !currentCaseConformance;
    deviations.textContent = currentCaseConformance ? currentCaseConformance.deviating_cases.toLocaleString() : "";
  }
  renderResultsKpis();
  renderResultsMap();
  if (typeof renderVariantExplorer === "function") renderVariantExplorer();
  renderResultsPerformance();
  renderResultsConformance();
  renderResultsCases();
}

function resultsValue(text) {
  const fragment = document.createDocumentFragment();
  const match = /^([\d.,]+)([a-z%]+)$/i.exec(text);
  if (!match) { fragment.append(text); return fragment; }
  const unit = document.createElement("small");
  unit.textContent = match[2];
  fragment.append(match[1], unit);
  return fragment;
}

function renderResultsKpis() {
  const strip = document.querySelector("#results-kpis");
  const analysis = currentAnalysis;
  if (!strip || !analysis) return;
  const { summary, rework, view } = analysis;
  const percent = (value) => `${value.toLocaleString(undefined, { maximumFractionDigits: 1 })}%`;
  const timed = summary.duration_eligible_cases > 0;
  const conformance = currentCaseConformance;
  const extractWindow = analysis.window;
  // Against an extract window, say where items sit rather than hide any.
  const itemsSub = extractWindow
    ? [`${extractWindow.complete.toLocaleString()} complete`, `${extractWindow.started_before.toLocaleString()} began earlier`, ...(extractWindow.end_defined ? [`${extractWindow.still_open.toLocaleString()} still open`] : [])].join(" · ")
    : view.narrowed ? `of ${view.cases_total.toLocaleString()} recorded` : "All recorded";
  const items = itemLabels().plural.toLowerCase();
  const history = analysisIsObjectHistory(analysis);
  const tiles = [
    { label: itemLabels().plural, value: summary.cases.toLocaleString(), sub: itemsSub },
    { label: history ? "History span" : "Typical time", value: timed ? formatDuration(summary.median_throughput_seconds) : "—", sub: timed ? `${history ? "Object history" : "First to last recorded step"} · median${typeof summary.p90_throughput_seconds === "number" ? ` · 1 in 10 over ${formatDuration(summary.p90_throughput_seconds)}` : ""}` : "No measurable time" },
    { label: "Variants", value: summary.variants.toLocaleString(), sub: `Top 5 cover ${percent(summary.top_five_coverage_percent)}` },
    { label: "Repeated a step", value: percent(rework.case_percent), sub: history ? "May span separate transactions; not proof of rework" : rework.cases_affected ? `${rework.cases_affected.toLocaleString()} ${items}` : "No repeated steps", tone: !history && rework.cases_affected ? "bad" : "" },
  ];
  // Conformance only where an expected process is part of the question.
  if (conformance || currentWorkspace?.purpose === "audit") {
    tiles.push(conformance
      ? { label: "Follow the SOP", value: conformanceShare(conformance.fit_cases, conformance.view.cases_shown) || "—", sub: `${conformance.fit_cases.toLocaleString()} ${items} exactly · ${conformance.deviating_cases.toLocaleString()} did not`, tone: conformance.deviating_cases ? "bad" : "" }
      : { label: "Follow the SOP", value: "—", sub: "Not checked yet" });
  }
  // Every other view carries the same figures as one quiet line.
  if (typeof lastValueSummary !== "undefined" && lastValueSummary) queueMicrotask(() => showValueFigure(lastValueSummary));
  const line = document.querySelector("#kpi-line");
  if (line) {
    line.textContent = [
      `${summary.cases.toLocaleString()} ${caseNoun(summary.cases)}`,
      timed ? `median ${formatDuration(summary.median_throughput_seconds)}` : "",
      `${summary.variants.toLocaleString()} ${summary.variants === 1 ? "variant" : "variants"}`,
      rework.cases_affected ? `${percent(rework.case_percent)} repeat a step` : "",
      view.narrowed ? `${percent(view.percent_shown ?? 100)} of ${view.cases_total.toLocaleString()} shown` : "",
    ].filter(Boolean).join(" · ");
  }
  strip.replaceChildren(...tiles.map((tile) => {
    const box = document.createElement("div"); box.className = "kpi";
    const label = document.createElement("div"); label.className = "lbl"; label.textContent = tile.label;
    const value = document.createElement("div"); value.className = "val"; value.append(resultsValue(tile.value));
    const sub = document.createElement("div"); sub.className = tile.tone ? `sub ${tile.tone}` : "sub"; sub.textContent = tile.sub;
    box.append(label, value, sub);
    return box;
  }));
}

// The figure strip for a connected (object-centric) project: engine counts
// for the items, updates and links, in the same six tiles as a case project.
function renderOcelKpis(analysis) {
  const strip = document.querySelector("#results-kpis");
  if (!strip || !analysis) return;
  const { scope, catalog, quality } = analysis;
  const types = catalog.object_types.length;
  const routes = analysis.variants?.length;
  const tiles = [
    { label: "Connected items", value: scope.objects_shown.toLocaleString(), sub: scope.narrowed ? `of ${scope.objects_total.toLocaleString()} recorded` : `${types.toLocaleString()} item ${types === 1 ? "type" : "types"}` },
    { label: "Recorded updates", value: scope.events_shown.toLocaleString(), sub: "Each update counted once" },
    { label: "Steps", value: catalog.event_types.length.toLocaleString(), sub: "Distinct recorded actions" },
    { label: "Variants", value: routes === undefined ? "—" : routes.toLocaleString(), sub: `Item lifecycles across ${types.toLocaleString()} ${types === 1 ? "type" : "types"}` },
    { label: "Links between items", value: scope.object_object_relations_shown.toLocaleString(), sub: `${scope.event_object_relations_shown.toLocaleString()} update–item links` },
    { label: "Data checks", value: analysis.ready_for_analysis ? "Ready" : "Blocked", sub: `${quality.summary.errors.toLocaleString()} errors · ${quality.summary.warnings.toLocaleString()} warnings`, tone: analysis.ready_for_analysis ? "" : "bad" },
  ];
  strip.replaceChildren(...tiles.map((tile) => {
    const box = document.createElement("div"); box.className = "kpi";
    const label = document.createElement("div"); label.className = "lbl"; label.textContent = tile.label;
    const value = document.createElement("div"); value.className = "val"; value.append(resultsValue(tile.value));
    const sub = document.createElement("div"); sub.className = tile.tone ? `sub ${tile.tone}` : "sub"; sub.textContent = tile.sub;
    box.append(label, value, sub);
    return box;
  }));
}

function renderResultsControls(view) {
  const routes = view.selection.mode === "routes";
  for (const button of document.querySelectorAll?.("[data-results-detail]") || []) button.setAttribute("aria-pressed", String(button.dataset.resultsDetail === view.selection.mode));
  for (const button of document.querySelectorAll?.("[data-results-metric]") || []) button.setAttribute("aria-pressed", String(button.dataset.resultsMetric === currentOverlay));
  const stepper = document.querySelector("#results-stepper");
  const limits = document.querySelector("#results-limits");
  if (stepper) stepper.hidden = !routes;
  if (limits) limits.hidden = routes;
  const count = document.querySelector("#results-route-count");
  if (count) count.textContent = `${view.variants_shown.toLocaleString()} of ${view.variants_total.toLocaleString()} variants`;
  const fewer = document.querySelector("#results-fewer");
  const more = document.querySelector("#results-more");
  if (fewer) fewer.disabled = !routes || view.selection.variants.length <= 1;
  if (more) more.disabled = !routes || !view.variants.some((variant) => !variant.shown);
  if (!routes) {
    for (const [id, value, max] of [["activities", view.selection.activities, view.activities_total], ["connections", view.selection.connections, view.connections_total]]) {
      const slider = document.querySelector(`#results-${id}`);
      const shown = document.querySelector(`#results-${id}-v`);
      if (slider) { slider.max = String(max); slider.value = String(value); }
      if (shown) shown.textContent = `${value} of ${max}`;
    }
  }
  const coverage = document.querySelector("#results-coverage");
  if (coverage) {
    const [line, detail] = mapCoverageText(view, itemLabels().plural.toLowerCase());
    coverage.replaceChildren();
    const share = document.createElement("b"); share.textContent = line[0];
    coverage.append(share, line[1]);
    coverage.dataset.detail = detail;
  }

}

// Positions come from the shared explorer layout: the most common route is the
// middle column, other steps sit in side columns near the steps they connect.
function resultsLayout(view) {
  const layout = explorerLayout(view);
  const points = [...layout.positions.values()];
  const minColumn = Math.min(0, ...points.map((point) => point.column));
  const maxColumn = Math.max(0, ...points.map((point) => point.column));
  const minY = Math.min(...points.map((point) => point.y));
  const left = RESULTS_SIDE_WIDTH / 2 + 170;
  const centre = left - minColumn * RESULTS_COLUMN;
  const positions = new Map();
  for (const [activity, point] of layout.positions) {
    positions.set(activity, {
      x: centre + point.column * RESULTS_COLUMN,
      y: 84 + ((point.y - minY) / MAP_ROW) * RESULTS_ROW,
      column: point.column,
      width: resultsNodeWidth(activity, view.nodes?.find((node) => node.activity === activity)?.frequency, point.column === 0),
    });
  }
  const bottom = Math.max(...[...positions.values()].map((point) => point.y));
  return {
    positions,
    centre,
    startY: 30,
    endY: bottom + RESULTS_NODE_HEIGHT / 2 + 44,
    width: centre + maxColumn * RESULTS_COLUMN + RESULTS_SIDE_WIDTH / 2 + 170,
    height: bottom + RESULTS_NODE_HEIGHT / 2 + 76,
  };
}

function resultsEdgeGeometry(a, b, channel) {
  const half = RESULTS_NODE_HEIGHT / 2;
  if (a === b) {
    const x = a.x + a.width / 2;
    return { d: `M${x},${a.y - 8} C${x + 46},${a.y - 34} ${x + 46},${a.y + 34} ${x},${a.y + 8}`, lx: x + 40, ly: a.y + 4, anchor: "start", back: true };
  }
  const back = b.y < a.y || (b.y === a.y && b.x < a.x);
  if (a.x === b.x) {
    if (!back && b.y - a.y <= RESULTS_ROW * 1.1) {
      return { d: `M${a.x},${a.y + half} L${b.x},${b.y - half}`, lx: a.x + 12, ly: (a.y + b.y) / 2 + 4, anchor: "start", back: false };
    }
    const side = a.column > 0 ? 1 : -1;
    const sx = a.x + side * (a.width / 2);
    const ex = b.x + side * (b.width / 2 + 3);
    const ox = a.x + side * (Math.max(a.width, b.width) / 2 + 36 + channel * 16);
    return { d: `M${sx},${a.y} C${ox},${a.y} ${ox},${b.y} ${ex},${b.y}`, lx: ox + side * 6, ly: (a.y + b.y) / 2 + 4, anchor: side > 0 ? "start" : "end", back };
  }
  const direction = Math.sign(b.x - a.x);
  const sx = a.x + direction * (a.width / 2);
  const ex = b.x - direction * (b.width / 2);
  const offset = back ? 8 : -8;
  const sy = a.y + offset;
  const ey = b.y + offset;
  const mx = (sx + ex) / 2;
  return { d: `M${sx},${sy} C${mx},${sy} ${mx},${ey} ${ex},${ey}`, lx: mx, ly: (sy + ey) / 2 + (back ? 16 : -6), anchor: "middle", back };
}

function renderResultsMap() {
  for (const button of document.querySelectorAll?.("[data-results-metric]") || []) button.setAttribute("aria-pressed", String(button.dataset.resultsMetric === currentOverlay));
  if (currentAnalysis?.summary?.cases === 0) { clearResultsMap("No items match these filters. Remove or change a filter to explore the map."); return; }
  if (typeof connectedScopePending !== "undefined" && connectedScopePending && currentFollow && typeof activeMapStyle === "function" && activeMapStyle() === "subway") { clearResultsMap(connectedScopeError || "Resolving connected records for this selection…"); return; }
  if (typeof activeMapStyle === "function" && activeMapStyle() === "subway") { renderMapStyle(); renderSubway(); return; }
  const svg = document.querySelector("#results-map");
  const view = currentMapView;
  if (!svg) return;
  const exportButton = document.querySelector("#export-bpmn"); if (exportButton) exportButton.disabled = !view;
  if (!view) {
    const coverage = document.querySelector("#results-coverage");
    if (coverage) coverage.textContent = "Preparing the map…";
    return;
  }
  renderResultsControls(view);
  const duration = currentOverlay === "performance";
  if (resultsSelection && !view.nodes.some((node) => node.activity === resultsSelection)) resultsSelection = null;
  drawResultsMap(svg, view, {
    duration, selection: resultsSelection, prefix: "results",
    onNode: (activity) => { if(pickMapMeasurement(activity))return;resultsEdgeFocus = null;resultsSelection = activity;showMapStepDetail(); renderResultsMap(); },
    onEdge: (edge) => {
      resultsEdgeFocus = { from: edge.from, to: edge.to };
      resultsSelection = edge.from;
      if (typeof showMapStepDetail === "function") showMapStepDetail();
      resultsSelectEdge(edge);
      focusFlowEdge(svg, resultsEdgeFocus);
    },
  });
  svg.style.maxWidth = svg.dataset.width;

  const legend = document.querySelector("#results-legend");
  if (legend) {
    legend.replaceChildren(...mapLegendItems(duration, view).map(([kind, text, tip]) => {
      const item = document.createElement("span");
      if (tip) item.title = tip;
      const swatch = document.createElement("i"); swatch.className = `key-${kind}`; item.append(swatch, text);
      return item;
    }));
  }
  if (resultsSelection) renderResultsDetail(view.nodes.find((node) => node.activity === resultsSelection));
  renderMapScope();
  if (typeof updateProcessExplorer === "function") updateProcessExplorer(view);
  if (resultsRouteFocus) {
    if (!resultsRouteFocus.ranks) focusResultsRoute(svg, resultsRouteFocus.sequence);
    const coverage = document.querySelector("#results-coverage");
    if (coverage && resultsRouteFocus.item) coverage.textContent = `The path of ${itemLabels().singular} ${resultsRouteFocus.item}${resultsRouteFocus.rank ? `, route ${resultsRouteFocus.rank}${view.cases_covered === 1 ? ", the only one on this route" : `, shared by ${view.cases_covered.toLocaleString()} ${itemLabels().plural.toLowerCase()}`}` : ""}.`;
    else if (coverage) coverage.textContent = `Selected ${resultsRouteFocus.ranks ? resultsRouteFocus.ranks.map(rank=>"V"+rank).join(", ") : "V"+resultsRouteFocus.rank} · ${view.cases_covered.toLocaleString()} of ${view.cases_total.toLocaleString()} ${itemLabels().plural.toLowerCase()} (${view.percent_covered.toLocaleString(undefined,{maximumFractionDigits:1})}%). Counts refer only to the selected variants. Main flow restores independent activity and connection exploration.`;
  }
}

function clearResultsMap(message) {
  const exportButton = document.querySelector("#export-bpmn"); if (exportButton) exportButton.disabled = true;
  for (const selector of ["#results-map", "#subway-map", "#results-legend", "#subway-legend"]) document.querySelector(selector)?.replaceChildren();
  const coverage = document.querySelector("#results-coverage");
  if (coverage) coverage.textContent = message;
  const detail = document.querySelector("#results-detail");
  if (detail) detail.replaceChildren(document.createTextNode(message));
  for (const selector of ["#results-stepper", "#results-limits"]) { const element = document.querySelector(selector); if (element) element.hidden = true; }
}

// Draws a map view: start and end markers, the most common route down the
// middle, steps as pills, thin curved connections sized by how often they
// were taken, slow connections red in duration mode and returns dashed.
// Hovering a step fades everything not connected to it. Shared by the case
// map and the connected-process map, so `view` needs only nodes, edges,
// top_route and activity_ranking.
function drawResultsMap(svg, view, { duration, selection, prefix, onNode, onEdge, metric = currentOverlay }) {
  const countOf = item => metric === "frequency" ? item.frequency : (item.case_frequency ?? item.frequency);
  const layout = resultsLayout(view);
  svg.replaceChildren();
  svg.classList?.add("flow-map");
  let edgeChannel=0,minX=0,maxX=layout.width;
  for(const edge of view.edges){const a=layout.positions.get(edge.from),b=layout.positions.get(edge.to);if(!a||!b)continue;const g=resultsEdgeGeometry(a,b,a.x===b.x && (b.y<=a.y || b.y-a.y>RESULTS_ROW*1.1)?edgeChannel++:0);const text=duration?formatDuration(edge.mean_seconds):countOf(edge).toLocaleString();const space=text.length*8+24;minX=Math.min(minX,g.lx-space);maxX=Math.max(maxX,g.lx+space);}
  svg.setAttribute("viewBox", `${minX} 0 ${maxX-minX} ${layout.height}`);
  svg.dataset.width = `${maxX-minX}px`;
  const defs = svgElement("defs");
  for (const key of ["edge-ink", "return", "bad"]) {
    const marker = svgElement("marker", { id: `${prefix}-arrow-${key}`, viewBox: "0 0 10 10", refX: 8, refY: 5, markerWidth: 8, markerHeight: 8, markerUnits: "userSpaceOnUse", orient: "auto-start-reverse" });
    marker.append(svgElement("path", { d: "M1.5,1.5 L8,5 L1.5,8.5", fill: "none", stroke: `var(--${key})`, "stroke-width": 1.6, "stroke-linecap": "round", "stroke-linejoin": "round" }));
    defs.append(marker);
  }
  svg.append(defs);

  const edgeLayer = svgElement("g");
  const labelLayer = svgElement("g");
  svg.append(edgeLayer);
  const maxFrequency = Math.max(1, ...view.edges.map(countOf));
  const thickness = (count) => 1.2 + 2.6 * Math.sqrt(Math.min(1, count / maxFrequency));
  let channel = 0;
  const drawIn = [];
  // Numbers keep clear of every step pill and of each other.
  const taken = [...layout.positions.values()].map((point) => ({ x1: point.x - point.width / 2 - 4, x2: point.x + point.width / 2 + 4, y1: point.y - RESULTS_NODE_HEIGHT / 2 - 3, y2: point.y + RESULTS_NODE_HEIGHT / 2 + 3 }));
  for (const edge of view.edges) {
    const a = layout.positions.get(edge.from);
    const b = layout.positions.get(edge.to);
    if (!a || !b) continue;
    const geometry = resultsEdgeGeometry(a, b, a.x === b.x && (b.y <= a.y || b.y - a.y > RESULTS_ROW * 1.1) ? channel++ : 0);
    const slow = duration && edge.slow;
    const colour = slow ? "bad" : geometry.back ? "return" : "edge-ink";
    const width = duration ? (slow ? 2.4 : 1.5) : thickness(countOf(edge));
    const group = svgElement("g", { class: "edge", tabindex: 0, role: "button", "aria-label": `${edge.from} to ${edge.to}`, "data-from": edge.from, "data-to": edge.to });
    group.append(svgElement("path", { d: geometry.d, class: "hit" }));
    const dashed = geometry.back && !slow;
    const line = svgElement("path", { d: geometry.d, class: "edge-line", fill: "none", stroke: `var(--${colour})`, "stroke-width": width.toFixed(2), "stroke-linecap": "round", ...(dashed ? { "stroke-dasharray": "4 4" } : {}), "marker-end": `url(#${prefix}-arrow-${colour})` });
    group.append(line);
    if (!dashed) drawIn.push(line);
    const text = duration ? (edge.duration_eligible_occurrences ? formatDuration(edge.mean_seconds) : "") : countOf(edge).toLocaleString();
    if (text) {
      // The number sits on its own arrow, halfway along, in a small pill.
      // Small connections show theirs on hover or when a step is focused.
      const width = text.length * 7 + 12;
      const [x, y] = edgeLabelPoint(line, geometry, width, taken);
      taken.push({ x1: x - width / 2 - 2, x2: x + width / 2 + 2, y1: y - 11, y2: y + 11 });
      const minor = !slow && countOf(edge) < maxFrequency * 0.15;
      const label = svgElement("g", { class: `elabel-pill${slow ? " slow" : ""}${geometry.back && !slow ? " back" : ""}${minor ? " minor" : ""}`, "data-from": edge.from, "data-to": edge.to });
      label.append(svgElement("rect", { x: x - width / 2, y: y - 9, width, height: 18, rx: 9 }));
      const words = svgElement("text", { x, y: y + 4, "text-anchor": "middle", class: `elabel${slow ? " slow" : ""}` });
      words.textContent = text;
      label.append(words);
      labelLayer.append(label);
      group.addEventListener("mouseenter", () => label.classList.add("shown"));
      group.addEventListener("mouseleave", () => label.classList.remove("shown"));
    }
    group.addEventListener("click", () => onEdge(edge));
    group.addEventListener("keydown", (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); onEdge(edge); } });
    edgeLayer.append(group);
  }
  svg.append(labelLayer);
  // Connections draw in along their length the first time a map appears;
  // redraws of the same map (a click, a tab) stay still.
  const signature = view.edges.map((edge) => `${edge.from}>${edge.to}`).join("|");
  const firstDraw = svg.dataset.drawn !== signature;
  svg.dataset.drawn = signature;
  for (const line of firstDraw ? drawIn : []) {
    const length = line.getTotalLength?.();
    if (!length) continue;
    line.style.strokeDasharray = `${length}`;
    line.style.strokeDashoffset = `${length}`;
    line.classList.add("draw");
  }

  // Start and end markers, joined to the steps items begin and finish with.
  const { centre, startY, endY } = layout;
  const half = RESULTS_NODE_HEIGHT / 2;
  // A straight start or end line from a middle step would run over the main
  // route, so only the first and last middle steps draw one; the step panel
  // still says how many items start or end at every step.
  for (const node of view.nodes) {
    const point = layout.positions.get(node.activity);
    if (!point) continue;
    for (const [count, fromStart] of [[node.starts, true], [node.ends, false]]) {
      if (!count) continue;
      if (!terminalLineAllowed(layout, point, fromStart)) continue;
      const d = fromStart
        ? (point.x === centre ? `M${centre},${startY + 6} L${point.x},${point.y - half - 2}` : `M${centre + Math.sign(point.x - centre) * 6},${startY} C${point.x},${startY} ${point.x},${startY} ${point.x},${point.y - half - 2}`)
        : (point.x === centre ? `M${point.x},${point.y + half + 2} L${centre},${endY - 6}` : `M${point.x},${point.y + half + 2} C${point.x},${endY} ${point.x},${endY} ${centre + Math.sign(point.x - centre) * 6},${endY}`);
      edgeLayer.append(svgElement("path", { d, class: "edge-line end-line", fill: "none", stroke: "var(--edge-ink)", "stroke-width": duration ? 1.2 : Math.min(2.4, thickness(count)).toFixed(2), "stroke-dasharray": "2 4", "stroke-linecap": "round", "data-from": fromStart ? "" : node.activity, "data-to": fromStart ? node.activity : "" }));
    }
  }
  for (const [y, label] of [[startY, "Start"], [endY, "End"]]) {
    const group = svgElement("g", { class: "terminal" });
    group.append(svgElement("circle", { cx: centre, cy: y, r: 5 }));
    const text = svgElement("text", { x: centre + 13, y: y + 4 });
    text.textContent = label;
    group.append(text);
    svg.append(group);
  }

  const nodeLayer = svgElement("g");
  svg.append(nodeLayer);
  const maxNode = Math.max(1, ...view.nodes.map(countOf));
  for (const node of view.nodes) {
    const point = layout.positions.get(node.activity);
    if (!point) continue;
    const left = point.x - point.width / 2;
    const group = svgElement("g", { class: `node${node.activity === selection ? " sel" : ""}`, tabindex: 0, role: "button", "aria-label": `${node.activity}, ${countOf(node).toLocaleString()} ${metric === "frequency" ? "events" : "items"}`, "data-activity": node.activity });
    group.append(svgElement("rect", { class: "pill", x: left, y: point.y - half, width: point.width, height: RESULTS_NODE_HEIGHT, rx: half }));
    group.append(svgElement("circle", { class: "pip", cx: left + 17, cy: point.y, r: 3.5, opacity: (0.3 + 0.7 * countOf(node) / maxNode).toFixed(2) }));
    const countText = countOf(node).toLocaleString();
    const room = Math.max(8, Math.floor((point.width - 60 - countText.length * 7.8) / 7.3));
    const name = svgElement("text", { x: left + 30, y: point.y + 5, class: "nlabel" });
    name.textContent = node.activity.length > room ? `${node.activity.slice(0, room - 1)}…` : node.activity;
    const title = svgElement("title"); title.textContent = node.activity;
    const count = svgElement("text", { x: point.x + point.width / 2 - 15, y: point.y + 4.5, class: "ncount", "text-anchor": "end" });
    count.textContent = countText;
    group.append(title, name, count);
    const pick = () => onNode(node.activity);
    group.addEventListener("click", pick);
    group.addEventListener("keydown", (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); pick(); } });
    group.addEventListener("mouseenter", () => focusFlowStep(svg, node.activity));
    group.addEventListener("mouseleave", () => focusFlowStep(svg, selection || null));
    nodeLayer.append(group);
  }
  if (selection) focusFlowStep(svg, selection);
}

// Halfway along the drawn arrow; the layout's label point when the page
// cannot measure the path.
// Where a connection's number goes: halfway along, or the nearest spot along
// the arrow that is clear of the steps and numbers already placed.
function edgeLabelPoint(line, geometry, width = 0, taken = []) {
  const length = typeof line.getTotalLength === "function" ? line.getTotalLength() : 0;
  if (length <= 0) return [geometry.lx, geometry.ly];
  const clear = ([x, y]) => !taken.some((box) => x + width / 2 > box.x1 && x - width / 2 < box.x2 && y + 9 > box.y1 && y - 9 < box.y2);
  for (const share of [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8]) {
    const point = line.getPointAtLength(length * share);
    if (clear([point.x, point.y])) return [point.x, point.y];
  }
  const middle = line.getPointAtLength(length / 2);
  return [middle.x, middle.y];
}

// The map's one-line coverage: how much of the selection follows a route
// drawn here. The longer explanation stays available on hover.
function mapCoverageText(view, plural) {
  const percent = `${view.percent_covered.toLocaleString(undefined, { maximumFractionDigits: 0 })}%`;
  const routes = view.selection?.mode === "routes";
  const detail = routes
    ? `${view.variants_shown} of ${view.variants_total} variants · ${view.cases_covered.toLocaleString()} of ${view.cases_total.toLocaleString()} ${plural}.`
    : `${view.activities_shown} of ${view.activities_total} steps · ${view.connections_shown} of ${view.connections_total} connections · counts use all ${view.cases_total.toLocaleString()} ${plural} in the selection.`;
  return [[percent, ` of ${plural} follow a route shown here`], detail];
}

// Legend words: forward, goes back, and slow only when timing is shown.
function mapLegendItems(duration, view) {
  return [
    ["forward", "Forward"],
    ["return", "Goes back", "Goes back to an earlier step"],
    ...(duration && view?.slow_threshold_seconds ? [["slow", "Slow", `${formatDuration(view.slow_threshold_seconds)} or more on average, twice the typical connection`]] : []),
  ];
}

// Only side steps and the first or last step of the middle column get a line
// to Start or End; any other middle step's line would run over the main route.
function terminalLineAllowed(layout, point, fromStart) {
  if (point.x !== layout.centre) return true;
  const middle = [...layout.positions.values()].filter((other) => other.x === layout.centre).map((other) => other.y);
  return point.y === (fromStart ? Math.min(...middle) : Math.max(...middle));
}

// The file name for a BPMN export: the project's name, never a path.
function bpmnFileName(name) {
  const stem = String(name || "glasstarn").replace(/[^a-z0-9 ._-]+/gi, " ").replace(/\s+/g, " ").trim() || "glasstarn";
  return `${stem} - process map.bpmn`;
}

// Exports the classic map and its active case scope through the engine.
// The connected subway schematic is explicitly unavailable as BPMN.
async function exportBpmn() {
  const button = document.querySelector("#export-bpmn");
  if (!currentProfile || !currentMapping || launchToken === null || !button) return;
  button.disabled = true;
  showMapNote("Exporting the map as BPMN…");
  try {
    const name = currentWorkspace?.name || currentProfile.file_name || "Process map";
    const response = await projectFetch("/api/map/bpmn", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify(comparisonRequestBody({ selection: currentMapSelection, name })),
      cache: "no-store",
    });
    if (response.status === 404) throw new Error("no longer loaded");
    if (!response.ok) throw new Error((await response.json().catch(() => ({}))).error || `export returned ${response.status}`);
    const url = URL.createObjectURL(await response.blob());
    const link = document.createElement("a");
    link.href = url;
    link.download = bpmnFileName(name);
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 0);
    showMapNote(`Saved “${bpmnFileName(name)}” to your Downloads.`);
  } catch (error) {
    showMapNote(`The BPMN file could not be made. ${plainErrorMessage(error instanceof Error ? error.message : String(error))}`, "error");
  } finally {
    button.disabled = false;
  }
}

// Fades every connection and step that does not touch `activity`.
function focusFlowStep(svg, activity) {
  if (activity && resultsEdgeFocus && activity === resultsSelection) { focusFlowEdge(svg, resultsEdgeFocus); return; }
  if (!activity && resultsRouteFocus) { focusResultsRoute(svg, resultsRouteFocus.sequence); return; }
  const near = new Set(activity ? [activity] : []);
  svg.classList?.toggle("focusing", !!activity);
  for (const element of svg.querySelectorAll("[data-from]")) {
    const touches = !activity || element.dataset.from === activity || element.dataset.to === activity;
    element.classList.toggle("dim", !touches);
    if (activity && touches) { near.add(element.dataset.from); near.add(element.dataset.to); }
  }
  for (const node of svg.querySelectorAll(".node")) node.classList.toggle("dim", !!activity && !near.has(node.dataset.activity));
}

// Keeps one picked arrow and its two steps sharp and fades the rest.
function focusFlowEdge(svg, edge) {
  svg.classList?.toggle("focusing", true);
  for (const element of svg.querySelectorAll("[data-from]")) {
    const picked = element.dataset.from === edge.from && element.dataset.to === edge.to;
    element.classList.toggle("dim", !picked);
    element.classList.toggle("picked", picked);
  }
  for (const node of svg.querySelectorAll(".node")) {
    node.classList.toggle("dim", node.dataset.activity !== edge.from && node.dataset.activity !== edge.to);
    node.classList.remove("sel");
  }
}

function resultsFacts(title, facts, cases, next = [], kind = "Step") {
  const panel = document.querySelector("#results-detail");
  if (!panel) return;
  const label = document.createElement("p"); label.className = "detail-kind visually-hidden"; label.textContent = kind;
  const heading = document.createElement("h2"); heading.className = "detail-title"; heading.textContent = title;
  // The first two figures lead as tiles; the rest sit in two columns. A fact
  // that is a sentence reads better as a note than squeezed into a column.
  const tiles = document.createElement("div"); tiles.className = "detail-tiles";
  // Only figures make tiles; a name such as "Deliveries" reads as a fact row.
  const lead = facts.filter(([, value]) => String(value).length <= 24 && /^[−\-+]?[\d$€£¥]/.test(String(value))).slice(0, 2);
  for (const [term, value] of lead) {
    const [figure, extra] = String(value).split(" · ");
    const tile = document.createElement("div"); tile.className = "detail-tile";
    const k = document.createElement("span"); k.className = "k"; k.textContent = term;
    const v = document.createElement("span"); v.className = "v"; v.textContent = figure;
    if (extra) { const small = document.createElement("small"); small.textContent = extra; v.append(small); }
    tile.append(k, v);
    tiles.append(tile);
  }
  const list = document.createElement("dl"); list.className = "kv";
  const notes = [];
  for (const [term, value] of facts.filter((fact) => !lead.includes(fact))) {
    // A sentence reads as a note under the facts; anything else, however
    // long (a list of names), stays a row and wraps.
    if (/[.!?]$/.test(String(value)) && String(value).length > 22) { notes.push(reportElement("p", value, "detail-note")); continue; }
    const dt = document.createElement("dt"); dt.textContent = term;
    const dd = document.createElement("dd"); dd.textContent = value;
    list.append(dt, dd);
  }
  const parts = [label, heading, ...(lead.length ? [tiles] : []), ...(list.children.length ? [list] : []), ...notes];
  if (next.length) {
    const block = document.createElement("div"); block.className = "detail-next";
    const caption = document.createElement("p"); caption.className = "detail-caption"; caption.textContent = "What happens next";
    block.append(caption);
    const largest = Math.max(1, ...next.map((item) => item.count));
    for (const item of next) {
      const row = document.createElement("div"); row.className = "detail-next-row";
      const name = document.createElement("span"); name.textContent = item.name;
      const count = document.createElement("span"); count.className = "num"; count.textContent = item.count.toLocaleString();
      const track = document.createElement("div"); track.className = "track";
      const fill = document.createElement("span"); fill.style.width = `${item.count / largest * 100}%`;
      track.append(fill);
      row.append(name, count, track);
      block.append(row);
    }
    parts.push(block);
  }
  if (cases?.length) {
    const caption = document.createElement("p"); caption.className = "detail-caption"; caption.textContent = "Examples";
    const examples = document.createElement("div"); examples.className = "detail-cases";
    for (const id of cases) {
      const button = document.createElement("button"); button.type = "button"; button.textContent = id;
      button.addEventListener("click", () => void openCase(id));
      examples.append(button);
    }
    const section = document.createElement("div"); section.className = "detail-section";
    section.append(caption, examples);
    parts.push(section);
  }
  panel.replaceChildren(...parts);
}

// What the step panel says about a step comes from the whole selection,
// not from the arrows the map happens to draw at its current detail level.
function stepFlows(activity, nodes, edges) {
  const node = nodes.find((item) => item.activity === activity);
  const incoming = edges.filter((edge) => edge.to === activity && edge.from !== activity && edge.duration_eligible_occurrences);
  const slowestBefore = [...incoming].sort((a, b) => b.mean_seconds - a.mean_seconds)[0] || null;
  const next = edges
    .filter((edge) => edge.from === activity)
    .map((edge) => ({ name: edge.to === activity ? `${edge.to} again` : edge.to, count: edge.case_frequency }));
  if (node?.ends) next.push({ name: "Ends here", count: node.ends });
  next.sort((a, b) => b.count - a.count);
  return { node, slowestBefore, next: next.slice(0, 5) };
}

function renderResultsDetail(shown) {
  const view = currentMapView;
  if (!shown || !view) return;
  const all = currentAnalysis?.nodes?.length && currentAnalysis?.edges ? currentAnalysis : view;
  const { node: full, slowestBefore, next } = stepFlows(shown.activity, all.nodes, all.edges);
  const node = full || shown;
  const total = currentAnalysis?.summary?.cases || 0;
  const items = itemLabels().plural;
  resultsFacts(node.activity, [
    [`${items} reaching it`, total ? `${node.case_frequency.toLocaleString()} · ${(node.case_frequency / total * 100).toLocaleString(undefined, { maximumFractionDigits: 0 })}%` : node.case_frequency.toLocaleString()],
    ["Times recorded", node.frequency.toLocaleString()],
    ...(node.starts ? [["Starts here", node.starts.toLocaleString()]] : []),
    [view.measures_true_waiting ? "Longest wait before it" : "Slowest way in", slowestBefore ? `${formatDuration(slowestBefore.mean_seconds)} on average, from ${slowestBefore.from}` : "–"],
    ...mapValueFact(view.values?.steps?.[node.activity], "passing through"),
  ], node.example_cases, next);
  if(typeof appendStudioActions === "function")appendStudioActions(node.activity);
}

// The money through a step or arrow, as a panel fact: "Value passing through
// · 4.2M in order_value". Nothing when the items carry no amount.
function mapValueFact(total, verb) {
  const field = currentMapView?.values?.field;
  if (total === undefined || total === null || !field) return [];
  return [[`Value ${verb}`, `${formatAmount(total)} in ${field}`]];
}

function resultsSelectEdge(edge) {
  const view = currentMapView;
  resultsFacts(`${edge.from} → ${edge.to}`, [
    [itemLabels().plural, edge.case_frequency.toLocaleString()],
    ["Times recorded", edge.frequency.toLocaleString()],
    [view?.measures_true_waiting ? "Average wait" : "Average time", edge.duration_eligible_occurrences ? formatDuration(edge.mean_seconds) : "Not measurable"],
    ["Measured on", `${edge.duration_eligible_occurrences.toLocaleString()} of ${edge.frequency.toLocaleString()}`],
    // Same recorded time means the file, not the clock, set the order.
    ...(edge.same_time_occurrences ? [["Recorded at the same moment", `${edge.same_time_occurrences.toLocaleString()} of ${edge.frequency.toLocaleString()}, so which came first is unknown`]] : []),
    ...mapValueFact(view?.values?.connections?.[`${edge.from}\u0001${edge.to}`], "carried"),
  ], edge.example_cases, [], analysisIsObjectHistory() ? "Recorded history gap" : edge.slow ? "Hand-off · slow" : "Hand-off");
}

function resultsCell(text, className) {
  const cell = document.createElement("td");
  if (className) cell.className = className;
  if (text !== undefined) cell.textContent = text;
  return cell;
}

function resultsShareCell(percent, scale, colour) {
  const cell = document.createElement("td");
  const share = document.createElement("div"); share.className = "share";
  const track = document.createElement("div"); track.className = "track";
  const fill = document.createElement("span"); fill.style.width = `${Math.max(1, Math.min(100, (percent / (scale || 100)) * 100))}%`;
  if (colour) fill.style.background = `var(--${colour})`;
  track.append(fill);
  const value = document.createElement("b"); value.textContent = `${percent.toLocaleString(undefined, { maximumFractionDigits: 1 })}%`;
  share.append(track, value); cell.append(share);
  return cell;
}

function resultsEmptyRow(body, columns, text) {
  const row = document.createElement("tr");
  const cell = resultsCell(text); cell.colSpan = columns; cell.className = "empty-row";
  row.append(cell); body.replaceChildren(row);
}

function renderResultsPerformance() {
  const analysis = currentAnalysis;
  const timing = analysis?.timing;
  const svg = document.querySelector("#results-hist");
  if (!analysis || !svg) return;
  const plural = itemLabels().plural.toLowerCase();
  const hint = document.querySelector("#results-hist-hint");
  if (hint) hint.textContent = timing?.duration_eligible_cases
    ? `${timing.duration_eligible_cases.toLocaleString()} ${plural} by first-to-last time · median ${formatDuration(timing.median_seconds)} · P90 ${formatDuration(timing.p90_seconds)}`
    : `No ${plural} have measurable time`;
  if (timing) drawDurationHistogram(svg, timing);
  const note = document.querySelector("#results-hist-note");
  if (note) note.textContent = timing?.duration_eligible_cases ? `Amber bars are the slowest tenth (at or past the P90).${timing.incomplete_cases ? ` ${timing.incomplete_cases.toLocaleString()} incomplete ${plural} are left out.` : ""}` : "";
  const waiting = analysis.measures_true_waiting;
  const head = document.querySelector("#results-wait-head");
  if (head) head.textContent = waiting ? "Average wait" : "Average elapsed";
  const waitHint = document.querySelector("#results-wait-hint");
  if (waitHint) waitHint.textContent = analysisIsObjectHistory(analysis) ? "Recorded gaps across item histories; these may cross separate transactions. See Overview for measured linked-record findings." : waiting ? "Ranked by total waiting time" : "Ranked by total elapsed time, which includes work and waiting";
  const body = document.querySelector("#results-waits");
  if (!body) return;
  if (!analysis.bottlenecks.length) { resultsEmptyRow(body, 3, "No measurable connections in this view."); return; }
  const longest = Math.max(1, ...analysis.bottlenecks.map((item) => item.mean_seconds));
  body.replaceChildren(...analysis.bottlenecks.map((item) => {
    const row = document.createElement("tr");
    const average = document.createElement("td");
    const bar = document.createElement("div"); bar.className = "hbar";
    const track = document.createElement("div"); track.className = "track";
    const fill = document.createElement("span"); fill.style.width = `${(item.mean_seconds / longest) * 100}%`; fill.style.background = "var(--accent)";
    track.append(fill);
    const value = document.createElement("span"); value.className = "num"; value.textContent = formatDuration(item.mean_seconds);
    bar.append(track, value); average.append(bar);
    const edgeCell = resultsCell("");
    const edgeButton = document.createElement("button"); edgeButton.type = "button"; edgeButton.className = "evidence-link"; edgeButton.textContent = `${item.from} → ${item.to}`; edgeCell.append(edgeButton);
    row.append(edgeCell, average, resultsCell(`${item.share_percent.toLocaleString(undefined, { maximumFractionDigits: 1 })}%`, "n"));
    const open = () => void openFindingEvidence({target:{kind:"edge",from:item.from,to:item.to}});
    row.addEventListener("click",open);
    return row;
  }));
}

// The Conformance page: one plain answer, the expected process as a line of
// steps with where items left it, and what did not follow it as cards. Every
// count comes from the engine's check; the page only words and lays it out.
function expectedProcessName(report) {
  const raw = String(report?.expected_process || report?.model_source || currentCaseBaseline?.file_name || "").split(";")[0].trim();
  const name = raw.replace(/\.model\.json$/i, "").replace(/^expected-process$/i, "");
  return name.length > 80 ? `${name.slice(0, 77)}…` : name;
}

function conformanceDate(ms) {
  if (!ms) return "";
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleDateString(undefined, { day: "numeric", month: "short", year: date.getFullYear() === new Date().getFullYear() ? undefined : "numeric" });
}

// One deviation group in plain words: [title, why].
function conformanceGroupWords(group, items) {
  const step = `“${group.activity}”`;
  switch (group.kind) {
    case "skipped": return [`Skipped ${step}`, `The expected process requires ${step}; these ${items} never recorded it where it belongs.`];
    case "extra": return [`${step} out of place`, `Recorded at a point where the expected process has no place for ${step}.`];
    case "out_of_order": return [`${step} before the steps it needs`, `${step} happened without the steps the expected process puts before it.`];
    case "not_expected": return [`${step} is not in the expected process`, `These ${items} recorded a step the expected process does not have.`];
    default: return ["Stopped before the expected end", `These ${items} ended before the expected process was complete.`];
  }
}

function conformanceShare(count, total) {
  if (!total) return "";
  const share = count / total * 100;
  return `${share < 1 && share > 0 ? "<1" : Math.round(share)}%`;
}

function renderResultsConformance() {
  const headline = document.querySelector("#conf-headline");
  if (!headline) return;
  const when = document.querySelector("#conf-when");
  const figures = document.querySelector("#conf-figures");
  const line = document.querySelector("#conf-line");
  const groupsHint = document.querySelector("#conf-groups-hint");
  const report = currentCaseConformance;
  const items = itemLabels().plural.toLowerCase();
  const name = expectedProcessName(report) || "the expected process";
  if (!report) {
    headline.textContent = currentCaseBaseline ? "Checking…" : "No expected process yet";
    if (when) when.textContent = currentCaseBaseline ? "" : `Ask your agent to check these ${items} against your SOP.`;
    figures?.replaceChildren();
    line?.replaceChildren();
    renderConformanceSteps(null);
    if (groupsHint) groupsHint.textContent = "";
    return;
  }
  const checked = report.view.cases_shown;
  if (checked === 0) headline.textContent = `No ${items} in this view to check`;
  else if (!report.deviating_cases) headline.textContent = `All ${checked.toLocaleString()} ${items} followed ${name}`;
  else headline.textContent = `${report.deviating_cases.toLocaleString()} of ${checked.toLocaleString()} ${items} did not follow ${name}`;
  if (when) {
    const saved = conformanceDate(report.model_saved_at_ms);
    const scope = report.view.narrowed ? `${checked.toLocaleString()} ${items} in this view` : `all ${checked.toLocaleString()} ${items}`;
    when.textContent = `${saved ? `SOP set ${saved} · ` : ""}checked just now on ${scope}`;
  }
  if (figures) {
    const tiles = [
      { label: `${itemLabels().plural} checked`, value: checked.toLocaleString(), sub: report.view.narrowed ? `of ${report.view.cases_total.toLocaleString()} recorded` : "All recorded" },
      { label: "Followed it", value: report.fit_cases.toLocaleString(), sub: `${conformanceShare(report.fit_cases, checked)} exactly as written` },
      { label: "Did not", value: report.deviating_cases.toLocaleString(), sub: `${conformanceShare(report.deviating_cases, checked)} skipped or misplaced a step`, tone: report.deviating_cases ? "bad" : "" },
      { label: "Steps matched", value: closeness(report.fitness_millionths), sub: "Of all recorded and required steps" },
    ];
    figures.replaceChildren(...tiles.map((tile) => {
      const box = document.createElement("div"); box.className = "kpi";
      const label = document.createElement("div"); label.className = "lbl"; label.textContent = tile.label;
      const value = document.createElement("div"); value.className = "val"; value.textContent = tile.value;
      const sub = document.createElement("div"); sub.className = tile.tone ? `sub ${tile.tone}` : "sub"; sub.textContent = tile.sub;
      box.append(label, value, sub);
      return box;
    }));
  }
  renderConformanceLine(report, line);
  renderConformanceSteps(report);
  if (groupsHint) {
    const steps = report.by_step || [];
    groupsHint.textContent = steps.length ? `one ${itemLabels().singular.toLowerCase()} can appear in several rows · click a row to see its ${items}` : "";
  }
}

function closeness(millionths) { return `${Math.round(millionths / 10_000)}%`; }

// The SOP as one plain line of steps; a step that was skipped or done out of
// place carries its count. Optional steps are lighter.
function renderConformanceLine(report, line) {
  if (!line) return;
  const steps = (report.expected_steps || []).map((entry) => typeof entry === "string" ? { step: entry, optional: false } : entry);
  const groups = report.by_step || [];
  line.replaceChildren(...steps.map(({ step, optional }) => {
    const here = groups.filter((group) => group.activity === step);
    const li = document.createElement("li");
    li.className = `conf-step${here.length ? " has-issue" : ""}${optional ? " optional" : ""}`;
    const stop = document.createElement("span"); stop.className = "stop"; stop.setAttribute("aria-hidden", "true");
    const label = document.createElement("span"); label.className = "name"; label.textContent = step;
    li.append(stop, label);
    if (here.length) {
      const broke = document.createElement("span"); broke.className = "broke";
      broke.textContent = here.map((group) => `${group.cases.toLocaleString()} ${group.kind === "skipped" ? "skipped" : group.kind === "extra" ? "out of place" : group.kind === "out_of_order" ? "too early" : "differ"}`).join(" · ");
      li.append(broke);
    }
    if (optional) li.title = "Optional in the SOP";
    return li;
  }));
}

// The chip name for items opened from a Conformance card.
let conformanceFilterLabel = null;

// Opens a group's items: every tab filtered to exactly those items' variants.
async function openConformanceGroup(group) {
  if (!group?.variants?.length) {
    if (group?.example_cases?.[0]) void openCase(group.example_cases[0]);
    return;
  }
  const filter = { ...currentFilter, variants: group.variants };
  conformanceFilterLabel = {
    key: group.variants.map((steps) => steps.join("\u0001")).sort().join("\u0002"),
    text: conformanceGroupWords(group, itemLabels().plural.toLowerCase())[0],
  };
  showView("cases");
  await analyzeMapping(currentMapping, filter, "filter");
}

// The amount each group's items carry, from the engine's value summary for
// exactly those items; shown when the data has an amount field.
async function conformanceGroupMoney(group) {
  const field = (typeof currentValueField !== "undefined" && currentValueField) || (typeof mainNumberField === "function" ? mainNumberField(currentAnalysis?.number_fields) : null);
  if (!field || !group.variants?.length || typeof fetchValues !== "function") return null;
  const response = await projectFetch("/api/values", {
    method: "POST",
    headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
    body: JSON.stringify(comparisonRequestBody({ field, grouping: { by: "last_step" }, limit: 1, filter: { ...currentFilter, variants: group.variants } })),
    cache: "no-store",
  });
  if (!response.ok) return null;
  const payload = await response.json();
  return { field, total: payload?.overall?.total };
}

// Items come a page at a time from the engine: never the whole log.
let itemsOffset = 0;
let itemsRevision = 0;
const ITEMS_PAGE = 50;

// The item's variant, as a button: it filters every tab to that variant.
function variantCell(item) {
  const cell = resultsCell(undefined, "vid");
  if (!item.route_rank) { cell.textContent = "—"; return cell; }
  const button = document.createElement("button");
  const number = variantNumber(currentAnalysis?.variants?.[item.route_rank - 1]?.sequence, item.route_rank);
  button.type = "button"; button.className = "variant-chip"; button.textContent = `V${number}`;
  button.title = `Show only ${itemLabels().plural.toLowerCase()} that follow V${number}`;
  button.addEventListener("click", (event) => { event.stopPropagation(); void filterToItemVariant(item.case, number); });
  cell.append(button);
  return cell;
}

// Filters to the exact steps one item followed: its variant.
async function filterToItemVariant(caseId, rank) {
  const response = await projectFetch("/api/case", {
    method: "POST",
    headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
    body: JSON.stringify({ upload_id: currentProfile.upload_id, mapping: currentMapping, policy: currentPolicy, rules: currentRules, case_id: caseId }),
    cache: "no-store",
  });
  const payload = await response.json();
  if (!response.ok) return;
  const sequence = payload.events.map((event) => event.activity);
  const filter = { ...currentFilter };
  const key = (steps) => steps.join("\u0001");
  rememberVariantRank(sequence, rank);
  filter.variants = [...(filter.variants || []).filter((steps) => key(steps) !== key(sequence)), sequence];
  // Overview always shows the whole project, so the filtered items show on Items.
  if (document.body?.dataset.view === "report") showView("cases");
  await analyzeMapping(currentMapping, filter, "filter");
}

// A variant filter keeps the number it was picked under ("V3"): once the
// filter applies, that variant would be V1 of what is left.
// Kept in this browser per data source, so the number survives a reload.
const variantRanks = new Map();
let variantRanksSource = null;
function variantRankKey() { return `glasstarn-variant-ranks:${typeof currentProfile !== "undefined" && currentProfile?.upload_id || ""}`; }
function loadVariantRanks() {
  const source = variantRankKey();
  if (variantRanksSource === source) return;
  variantRanksSource = source; variantRanks.clear();
  try { for (const [key, rank] of JSON.parse(localStorage.getItem(source) || "[]")) variantRanks.set(key, rank); } catch {}
}
function rememberVariantRank(steps, rank) {
  if (!rank || !steps) return;
  loadVariantRanks();
  variantRanks.set(steps.join("\u0001"), rank);
  try { localStorage.setItem(variantRanksSource, JSON.stringify([...variantRanks].slice(-500))); } catch {}
}
// A variant's number as first shown, so V8 stays V8 when a filter leaves
// it the only one; otherwise its number in this view.
function variantNumber(steps, rank) {
  loadVariantRanks();
  return (steps && variantRanks.get(steps.join("\u0001"))) || rank;
}
function variantLabel(steps) {
  loadVariantRanks();
  const rank = variantRanks.get(steps.join("\u0001"));
  return rank ? `V${rank}` : `${steps[0]} → … → ${steps.at(-1)} (${steps.length} steps)`;
}

function itemRow(item) {
  const row = document.createElement("tr");
  const status = document.createElement("td");
  const pill = document.createElement("span");
  pill.className = item.repeats_step ? "pill warn" : "pill neutral";
  pill.textContent = item.repeats_step ? "Repeats a step" : "Single pass";
  status.append(pill);
  row.append(resultsCell(item.case, "mono"), resultsCell(new Date(item.started_at_micros / 1000).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" })),
    resultsCell(formatDuration(item.duration_seconds), "n"), resultsCell(item.events.toLocaleString(), "n"),
    variantCell(item), status);
  row.tabIndex = 0;
  row.addEventListener("click", () => void openCase(item.case, { rank: item.route_rank }));
  row.addEventListener("keydown", (event) => { if (event.key === "Enter") void openCase(item.case, { rank: item.route_rank }); });
  return row;
}

function renderItemPage(page) {
  const body = document.querySelector("#results-cases");
  if (!body) return;
  if (!page.items.length) { resultsEmptyRow(body, 6, page.total ? "No more items." : "No items match."); }
  else body.replaceChildren(...page.items.map(itemRow));
  const items = caseNoun(page.total);
  const range = document.querySelector("#items-range");
  if (range) range.textContent = page.total ? `${(page.offset + 1).toLocaleString()}–${(page.offset + page.items.length).toLocaleString()} of ${page.total.toLocaleString()} ${items}` : `0 ${items}`;
  const count = document.querySelector("#items-count");
  if (count) count.textContent = `${page.total.toLocaleString()} ${items}`;
  const prev = document.querySelector("#items-prev");
  const next = document.querySelector("#items-next");
  if (prev) prev.disabled = page.offset === 0;
  if (next) next.disabled = page.offset + page.items.length >= page.total;
}

async function loadItemsPage(offset = 0) {
  const revision = ++itemsRevision;
  itemsOffset = Math.max(0, offset);
  if (!currentProfile?.upload_id || typeof projectFetch !== "function") { renderSlowestItems(); return; }
  try {
    const response = await projectFetch("/api/items", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify(comparisonRequestBody({
        order: document.querySelector("#items-order")?.value || "slowest",
        search: document.querySelector("#items-search")?.value || "",
        offset: itemsOffset,
        limit: ITEMS_PAGE,
      })),
      cache: "no-store",
    });
    const page = await response.json();
    if (revision !== itemsRevision) return;
    if (!response.ok) throw new Error(page.error || "items unavailable");
    renderItemPage(page);
  } catch (_error) {
    if (revision === itemsRevision) renderSlowestItems();
  }
}

// Without the item service (tests, older sessions): the slowest items the
// analysis already carries.
function renderSlowestItems(selector = "#results-cases") {
  const timing = currentAnalysis?.timing;
  const body = document.querySelector(selector);
  if (!body) return;
  if (!timing?.slowest_cases.length) { resultsEmptyRow(body, 6, "No items have measurable time in this view."); return; }
  body.replaceChildren(...timing.slowest_cases.slice(0, 10).map(itemRow));
}

function renderResultsCases() {
  renderSlowestItems();
  renderSlowestItems("#results-slowest");
  void loadItemsPage(0);
}

function initializeItems() {
  let typing = null;
  document.querySelector("#items-search")?.addEventListener("input", () => { clearTimeout(typing); typing = setTimeout(() => void loadItemsPage(0), 250); });
  document.querySelector("#items-order")?.addEventListener("change", () => void loadItemsPage(0));
  document.querySelector("#items-prev")?.addEventListener("click", () => void loadItemsPage(itemsOffset - ITEMS_PAGE));
  document.querySelector("#items-next")?.addEventListener("click", () => void loadItemsPage(itemsOffset + ITEMS_PAGE));
}

function initializeResults() {
  initializeItems();
  document.querySelector("#app-home")?.addEventListener("click", (event) => { event.preventDefault(); showView("home"); });
  document.querySelector("#results-projects")?.addEventListener("click", () => showView("home"));
  document.querySelector("#app-import")?.addEventListener("click", () => {
    const choose = document.querySelector("#choose-log");
    if (choose && !choose.disabled && document.body.dataset.projectState !== "chooser") { choose.click(); return; }
    const create = document.querySelector(".browser-create");
    if (create) { create.open = true; create.scrollIntoView({ behavior: "smooth", block: "center" }); document.querySelector("#workspace-name")?.focus(); }
  });
  document.querySelector("#app-agent")?.addEventListener("click", () => {
    const settings = document.querySelector(".project-settings");
    const agent = document.querySelector("#agent-panel");
    if (settings && agent && !agent.hidden && document.body.dataset.projectState !== "chooser") { settings.open = true; agent.open = true; agent.scrollIntoView({ behavior: "smooth", block: "start" }); return; }
    // With no project open, show the three agent steps and draw the eye there.
    const journey = document.querySelector(".agent-journey");
    if (!journey) return;
    journey.classList.add("expanded", "spotlight");
    journey.scrollIntoView({ behavior: "smooth", block: "start" });
    setTimeout(() => journey.classList.remove("spotlight"), 1600);
  });
  document.querySelector("#export-bpmn")?.addEventListener("click", () => void exportBpmn());
  for (const button of document.querySelectorAll("[data-map-style]")) {
    button.addEventListener("click", () => {
      resultsRouteFocus = null;
      connectedVariantFocus = null;
      subwayFocus = null;
      mapStyle = button.dataset.mapStyle === "classic" ? "classic" : "subway";
      try { localStorage.setItem("glasstarn-map-style", mapStyle); } catch (_error) { /* the choice lasts this visit */ }
      renderMapStyle();
      renderResultsMap();
    });
  }
  try { mapStyle = localStorage.getItem("glasstarn-map-style") === "classic" ? "classic" : "subway"; } catch (_error) { /* default */ }
  for (const button of document.querySelectorAll("[data-results-metric]")) {
    button.addEventListener("click", () => {
      currentOverlay = button.dataset.resultsMetric;
      renderMap();
    });
  }
  for (const button of document.querySelectorAll("[data-results-detail]")) {
    button.addEventListener("click", () => void loadMapView(button.dataset.resultsDetail === "limits"
      ? { mode: "limits", activities: 0, connections: 0 }
      : { mode: "routes", variants: [1] }));
  }
  document.querySelector("#results-more")?.addEventListener("click", () => {
    const next = currentMapView?.variants.find((variant) => !variant.shown);
    if (next && currentMapSelection.mode === "routes") void loadMapView({ mode: "routes", variants: [...currentMapSelection.variants, next.rank].sort((a, b) => a - b) });
  });
  document.querySelector("#results-fewer")?.addEventListener("click", () => {
    if (currentMapSelection.mode === "routes" && currentMapSelection.variants.length > 1) void loadMapView({ mode: "routes", variants: currentMapSelection.variants.slice(0, -1) });
  });
  for (const id of ["activities", "connections"]) {
    const slider = document.querySelector(`#results-${id}`);
    slider?.addEventListener("input", () => { const shown = document.querySelector(`#results-${id}-v`); if (shown) shown.textContent = `${slider.value} of ${slider.max}`; });
    slider?.addEventListener("change", () => { resultsRouteFocus = null; connectedVariantFocus = null; void loadMapView({
      mode: "limits",
      activities: Number(document.querySelector("#results-activities")?.value || 0),
      connections: Number(document.querySelector("#results-connections")?.value || 0),
    }); });
  }
}
