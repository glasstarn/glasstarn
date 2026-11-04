// Process map: loading the engine's map view, its layout, and the formal model view.

function evidenceCaseButtons(caseIds) {
  const evidence = document.createElement("div");
  evidence.className = "case-evidence";
  const label = document.createElement("span");
  label.textContent = caseIds.length ? "Open an example item" : "No examples available.";
  evidence.append(label);
  for (const caseId of caseIds) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "case-chip";
    button.textContent = caseId;
    button.addEventListener("click", async () => {
      await openCase(caseId);
    });
    evidence.append(button);
  }
  return evidence;
}

function formalChildren(tree) {
  if (["sequence", "exclusive", "parallel"].includes(tree.kind)) {
    return tree.children.map((child) => ({ child, role: "" }));
  }
  if (tree.kind === "loop") {
    return [
      { child: tree.body, role: "body" },
      { child: tree.redo, role: "redo" },
    ];
  }
  return [];
}

function formalNodeLabel(tree) {
  return {
    activity: tree.activity,
    tau: "τ",
    sequence: "→",
    exclusive: "×",
    parallel: "∧",
    loop: "↻",
  }[tree.kind];
}

function formalNodeMeaning(kind) {
  return {
    activity: "observable activity",
    tau: "silent routing step",
    sequence: "sequence — children execute from left to right",
    exclusive: "exclusive choice — exactly one branch executes",
    parallel: "parallel — every branch executes in any valid interleaving",
    loop: "loop — body executes once; redo returns to the body",
  }[kind];
}

function formalLayout(tree) {
  const nodes = [];
  const edges = [];
  let nextLeafX = 110;
  let maxDepth = 0;
  let nextId = 0;

  function visit(current, depth) {
    maxDepth = Math.max(maxDepth, depth);
    const children = formalChildren(current).map(({ child, role }) => ({
      ...visit(child, depth + 1),
      role,
    }));
    const x = children.length
      ? children.reduce((total, child) => total + child.x, 0) / children.length
      : nextLeafX;
    if (!children.length) nextLeafX += 190;
    const node = { id: nextId++, tree: current, x, y: 110 + depth * 125 };
    nodes.push(node);
    for (const child of children) edges.push({ from: node, to: child, role: child.role });
    return node;
  }

  visit(tree, 0);
  return {
    nodes,
    edges,
    width: Math.max(760, nextLeafX),
    height: Math.max(400, 180 + maxDepth * 125),
  };
}

function renderFormalMap() {
  if (!currentDiscovery) return;
  const svg = document.querySelector("#process-map");
  const details = document.querySelector("#results-detail");
  if (!svg || !details) return;
  const layout = formalLayout(currentDiscovery.tree);
  svg.replaceChildren();
  svg.setAttribute("viewBox", `0 0 ${layout.width} ${layout.height}`);
  svg.style.width = `${layout.width}px`;
  svg.style.height = `${layout.height}px`;
  if (document.body) requestAnimationFrame(fitProcessMaps);

  const defs = svgElement("defs");
  appendArrowMarker(defs, "formal-arrow", "var(--edge)", 14);
  svg.append(defs);
  diagramHeader(
    svg,
    layout.width,
    "EXECUTABLE MODEL",
    `${layout.nodes.length.toLocaleString()} NODES  /  ${layout.edges.length.toLocaleString()} RELATIONS  /  CONFORMANCE READY`,
  );

  for (const edge of layout.edges) {
    const middleY = (edge.from.y + edge.to.y) / 2;
    const path = svgElement("path", {
      class: "formal-edge",
      d: `M ${edge.from.x} ${edge.from.y + 25} V ${middleY} H ${edge.to.x} V ${edge.to.y - 25}`,
      "marker-end": "url(#formal-arrow)",
    });
    svg.append(path);
    if (edge.role) {
      const label = svgElement("text", {
        class: "formal-edge-label",
        x: (edge.from.x + edge.to.x) / 2 + 8,
        y: (edge.from.y + edge.to.y) / 2,
      });
      label.textContent = edge.role;
      svg.append(label);
    }
  }

  for (const node of layout.nodes) {
    const operator = node.tree.kind !== "activity";
    const group = svgElement("g", {
      class: `formal-node ${operator ? "operator" : "activity"}`,
      role: "img",
      "aria-label": formalNodeMeaning(node.tree.kind),
    });
    if (operator) {
      group.append(svgElement("circle", { cx: node.x, cy: node.y, r: 25 }));
    } else {
      group.append(svgElement("rect", { x: node.x - 78, y: node.y - 27, width: 156, height: 54, rx: 10 }));
    }
    const label = svgElement("text", {
      x: node.x,
      y: node.y + (operator ? 7 : 4),
      "text-anchor": "middle",
      class: operator ? "formal-operator-label" : "formal-activity-label",
    });
    const fullLabel = formalNodeLabel(node.tree);
    label.textContent = fullLabel.length > 23 ? `${fullLabel.slice(0, 22)}…` : fullLabel;
    group.append(label);
    svg.append(group);
  }

  details.replaceChildren();
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Executable model";
  const title = document.createElement("h3"); title.textContent = "Sound process tree";
  const explanation = document.createElement("p");
  explanation.textContent = "This model distinguishes sequence (→), exclusive choice (×), parallel work (∧), loops (↻), and silent routing (τ). It is converted into the accepting Petri net used for conformance.";
  const facts = document.createElement("dl");
  for (const [term, value] of [
    ["Noise threshold", `${(currentDiscovery.noise_threshold_milli / 10).toLocaleString()}%`],
    ["Filtered relations", currentDiscovery.filtered_edges.toLocaleString()],
    ["Petri-net places", currentDiscovery.net.place_count.toLocaleString()],
    ["Petri transitions", currentDiscovery.net.transitions.length.toLocaleString()],
  ]) {
    const dt = document.createElement("dt"); dt.textContent = term;
    const dd = document.createElement("dd"); dd.textContent = value;
    facts.append(dt, dd);
  }
  details.append(label, title, explanation, facts);
  if (currentDiscovery.used_flower_fallback) {
    const warning = document.createElement("p");
    warning.textContent = "A permissive fallback was required for behaviour that could not be structured by a sound cut.";
    details.append(warning);
  }
}

async function loadFormalModel() {
  if (formalModelLoading || currentDiscovery || !currentProfile || !currentMapping) return;
  formalModelLoading = true;
  const details = document.querySelector("#results-detail");
  if (details) details.textContent = "Discovering a sound formal model locally…";
  try {
    const response = await projectFetch("/api/discover", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({
        upload_id: currentProfile.upload_id,
        mapping: currentMapping,
        policy: currentPolicy,
        rules: currentRules,
        filter: currentFilter,
        noise_threshold_milli: 200,
      }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `discovery returned ${response.status}`);
    if (payload.status === "blocked") throw new Error("Resolve the data-quality decisions before formal discovery.");
    currentDiscovery = payload.discovery;
    const download = document.querySelector("#download-case-model");
    if (download) download.disabled = false;
    if (currentModelView === "formal") renderFormalMap();
  } catch (error) {
    if (details) details.textContent = error instanceof Error ? error.message : "The formal model could not be discovered.";
  } finally {
    formalModelLoading = false;
  }
}

function renderMap() {
  const formal = currentModelView === "formal";
  const observed = document.querySelector("#results-map");
  const model = document.querySelector("#process-map");
  observed?.toggleAttribute("hidden", formal);
  model?.toggleAttribute("hidden", !formal);
  for (const id of ["#results-legend", "#results-coverage"]) {
    const element = document.querySelector(id);
    if (element) element.hidden = formal;
  }
  if (!formal) { renderMapStyle(); renderResultsMap(); }
  else if (currentDiscovery) renderFormalMap();
  else void loadFormalModel();
}

// Map explorer: the engine decides what is drawn (routes or step and
// connection limits) and reports coverage; the browser only lays it out.
async function loadMapView(selection = currentMapSelection) {
  if (!currentProfile || !currentMapping || launchToken === null) return;
  const revision = ++mapViewRevision;
  if (currentAnalysis?.summary?.cases === 0) { currentMapView = null; clearResultsMap("No items match these filters. Remove or change a filter to explore the map."); return; }
  const line = document.querySelector("#results-coverage");
  if (line && !currentMapView) line.textContent = "Preparing the map…";
  try {
    const response = await projectFetch("/api/map", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify(comparisonRequestBody({ selection })),
      cache: "no-store",
    });
    const payload = await response.json();
    if (revision !== mapViewRevision) return;
    if (!response.ok) throw new Error(payload.error || `map returned ${response.status}`);
    currentMapView = payload;
    currentMapSelection = payload.selection;
    if (typeof processLimitSelection !== "undefined" && payload.selection.mode === "limits") processLimitSelection = {...payload.selection};
    if (currentModelView !== "formal") renderResultsMap();
  } catch (error) {
    if (revision !== mapViewRevision) return;
    currentMapView = null;
    clearResultsMap(`The map could not be drawn: ${error.message}.`);
  }
}

const MAP_NODE_HALF_WIDTH = 100;
const MAP_NODE_HALF_HEIGHT = 38;
const MAP_ROW = 120;
const MAP_COLUMN = 280;

// The most common route runs straight down the middle; every other step sits
// in a side column beside the steps it connects, never overlapping another.
function explorerLayout(view) {
  const positions = new Map();
  const rows = new Map();
  const occupy = (column, y) => { if (!rows.has(column)) rows.set(column, []); rows.get(column).push(y); };
  for (const activity of view.top_route) {
    if (positions.has(activity)) continue;
    const y = positions.size * MAP_ROW;
    positions.set(activity, { column: 0, y });
    occupy(0, y);
  }
  const rank = new Map(view.activity_ranking.map((item) => [item.activity, item.rank]));
  const weight = (edge) => edge.frequency;
  let pending = view.nodes.map((node) => node.activity).filter((activity) => !positions.has(activity))
    .sort((a, b) => (rank.get(a) ?? Infinity) - (rank.get(b) ?? Infinity) || a.localeCompare(b));
  while (pending.length) {
    const deferred = [];
    for (const activity of pending) {
      const best = (edges) => edges.sort((a, b) => weight(b) - weight(a))[0];
      const before = best(view.edges.filter((edge) => edge.to === activity && edge.from !== activity && positions.has(edge.from)));
      const after = best(view.edges.filter((edge) => edge.from === activity && edge.to !== activity && positions.has(edge.to)));
      if (!before && !after && deferred.length < pending.length) { deferred.push(activity); continue; }
      const from = before && positions.get(before.from);
      const to = after && positions.get(after.to);
      const target = from && to && to.y > from.y ? (from.y + to.y) / 2
        : from ? from.y + MAP_ROW * 0.75 : to ? to.y - MAP_ROW * 0.75 : Math.max(0, ...[...positions.values()].map((p) => p.y)) + MAP_ROW;
      const preferred = Math.sign((from?.column || 0) + (to?.column || 0)) || 1;
      const columns = [preferred, -preferred, 2 * preferred, -2 * preferred, 3 * preferred, -3 * preferred, 4 * preferred, -4 * preferred];
      let placed = null;
      for (let shift = 0; !placed; shift += MAP_ROW / 2) {
        for (const column of columns) {
          for (const y of shift ? [target + shift, target - shift] : [target]) {
            if (y < 0 && shift) continue;
            if ((rows.get(column) || []).every((other) => Math.abs(other - y) >= MAP_ROW * 0.9)) { placed = { column, y }; break; }
          }
          if (placed) break;
        }
      }
      positions.set(activity, placed);
      occupy(placed.column, placed.y);
    }
    if (deferred.length === pending.length) {
      for (const activity of deferred) {
        const y = Math.max(0, ...[...positions.values()].map((p) => p.y)) + MAP_ROW;
        positions.set(activity, { column: 1, y });
        occupy(1, y);
      }
      break;
    }
    pending = deferred;
  }
  const columns = [...positions.values()].map((point) => point.column);
  const minColumn = Math.min(0, ...columns);
  const maxColumn = Math.max(0, ...columns);
  const minY = Math.min(...[...positions.values()].map((point) => point.y));
  const left = MAP_NODE_HALF_WIDTH + 110 - minColumn * MAP_COLUMN;
  const top = 112 - minY;
  const points = new Map();
  for (const [activity, point] of positions) points.set(activity, { x: left + point.column * MAP_COLUMN, y: top + point.y, column: point.column });
  const maxY = Math.max(...[...points.values()].map((point) => point.y));
  return {
    positions: points,
    width: Math.max(560, (maxColumn - minColumn) * MAP_COLUMN + 2 * (MAP_NODE_HALF_WIDTH + 110)),
    height: maxY + MAP_NODE_HALF_HEIGHT + 70,
  };
}
