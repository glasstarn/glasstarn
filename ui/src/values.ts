// Money on the Summary: totals of an amount field, where that value sits now
// by the step items stopped at. Every figure comes from the engine's value
// summary for the active view; the browser only formats it. With no amount
// field the panel stays hidden and the screens speak in counts and times.

let currentValueField = null;
let valueRevision = 0;

// Prefer a field whose name says it is money; otherwise the one with the most
// numbers. Identifiers stored as digits are possible, so the person can switch.
function mainNumberField(fields) {
  if (!fields?.length) return null;
  const money = /amount|value|price|total|gross|revenue|cost|betrag|wert/i;
  const named = fields.filter((field) => money.test(field.name));
  const pool = named.length ? named : fields;
  return [...pool].sort((a, b) => b.numbers - a.numbers || a.name.localeCompare(b.name))[0].name;
}

function formatAmount(value) {
  if (value === null || value === undefined || !Number.isFinite(Number(value))) return "–";
  const number = Number(value);
  if (Math.abs(number) >= 1_000_000) return number.toLocaleString(undefined, { notation: "compact", maximumFractionDigits: 1 });
  return number.toLocaleString(undefined, { maximumFractionDigits: Math.abs(number) >= 100 ? 0 : 2 });
}

// The headline facts, as [label, figure, detail] triples.
function valueFacts(summary) {
  const stats = summary.overall;
  const missing = stats.cases - stats.cases_with_value;
  const facts = [
    ["Total", formatAmount(stats.total), `${stats.cases_with_value.toLocaleString()} ${caseNoun(stats.cases_with_value)}${missing ? ` · ${missing.toLocaleString()} without an amount` : ""}`],
    ["Typical", formatAmount(stats.median), `median · average ${formatAmount(stats.mean)}`],
    ["Largest", formatAmount(stats.p90), `1 in 10 are above this · top ${formatAmount(stats.max)}`],
  ];
  if (stats.open_total !== null && stats.open_total !== undefined) {
    facts.push(["Still open", formatAmount(stats.open_total), `${Number(stats.open_cases).toLocaleString()} ${caseNoun(stats.open_cases)} not at a finishing step`]);
  }
  return facts;
}

function caseNoun(count) {
  const labels = itemLabels();
  return Number(count) === 1 ? labels.singular : labels.plural.toLowerCase();
}

function valueRows(summary, total) {
  const rows = summary.groups.map((group) => ({
    label: group.label,
    total: group.stats.total,
    open: group.stats.open_total,
    cases: group.stats.cases,
    share: total > 0 ? group.stats.total / total * 100 : 0,
  }));
  if (summary.other_groups) {
    rows.push({
      label: `${(summary.groups_available - summary.groups.length).toLocaleString()} more`,
      total: summary.other_groups.total,
      open: summary.other_groups.open_total,
      cases: summary.other_groups.cases,
      share: total > 0 ? summary.other_groups.total / total * 100 : 0,
    });
  }
  return rows;
}

async function fetchValues(field, grouping) {
  const response = await projectFetch("/api/values", {
    method: "POST",
    headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
    body: JSON.stringify(comparisonRequestBody({ field, grouping, limit: 6 })),
    cache: "no-store",
  });
  const payload = await response.json();
  if (!response.ok) throw new Error(payload.error || `value summary returned ${response.status}`);
  return payload;
}

async function loadValues(analysis) {
  const panel = document.querySelector("#value-panel");
  if (!panel) return;
  const fields = analysis?.number_fields || [];
  const revision = ++valueRevision;
  if (!fields.length || !currentProfile?.upload_id) { panel.hidden = true; lastValueSummary = null; return; }
  if (!fields.some((field) => field.name === currentValueField)) currentValueField = mainNumberField(fields);
  const select = document.querySelector("#value-field");
  select.replaceChildren(...fields.map((field) => {
    const option = document.createElement("option");
    option.value = field.name;
    option.textContent = field.name;
    option.selected = field.name === currentValueField;
    return option;
  }));
  panel.hidden = false;
  document.querySelector("#value-note").textContent = "Adding up…";
  try {
    const steps = await fetchValues(currentValueField, { by: "last_step" });
    if (revision !== valueRevision) return;
    renderValues(steps);
  } catch (error) {
    if (revision !== valueRevision) return;
    document.querySelector("#value-facts").replaceChildren();
    document.querySelector("#value-steps").replaceChildren();
    document.querySelector("#value-note").textContent = error instanceof Error ? error.message : "The amounts could not be added up.";
  }
}

function valueCell(tag, text, className) {
  const node = document.createElement(tag);
  node.textContent = text;
  if (className) node.className = className;
  return node;
}

function renderValueTable(body, rows, showOpen) {
  body.replaceChildren(...rows.map((row) => {
    const tr = document.createElement("tr");
    const share = document.createElement("td");
    const bar = document.createElement("div");
    bar.className = "value-share";
    share.append(bar);
    const track = document.createElement("span");
    track.className = "track";
    const fill = document.createElement("span");
    fill.style.width = `${Math.max(0, Math.min(100, row.share)).toFixed(1)}%`;
    track.append(fill);
    bar.append(track, valueCell("b", `${row.share.toLocaleString(undefined, { maximumFractionDigits: 0 })}%`));
    tr.append(
      valueCell("td", row.label),
      valueCell("td", formatAmount(row.total), "n"),
      ...(showOpen ? [valueCell("td", formatAmount(row.open), "n")] : []),
      valueCell("td", row.cases.toLocaleString(), "n"),
      share,
    );
    return tr;
  }));
}

function renderValues(steps) {
  const facts = document.querySelector("#value-facts");
  facts.replaceChildren(...valueFacts(steps).map(([label, figure, detail]) => {
    const card = document.createElement("div");
    card.className = "value-fact";
    card.append(valueCell("span", label, "k"), valueCell("b", figure), valueCell("span", detail, "s"));
    return card;
  }));
  const total = steps.overall.total;
  const showOpen = steps.overall.open_total !== null && steps.overall.open_total !== undefined;
  document.querySelector("#value-open-head").hidden = !showOpen;
  renderValueTable(document.querySelector("#value-steps"), valueRows(steps, total), showOpen);
  document.querySelector("#value-note").textContent = valueNote(steps);
  // One last step holding everything says nothing about where value sits.
  const panel = document.querySelector("#value-panel");
  if (panel) panel.hidden = valueRows(steps, total).length < 2;
  showValueFigure(steps);
}

let lastValueSummary = null;

// The money figure joins the key figures on Overview.
function showValueFigure(summary) {
  lastValueSummary = summary;
  const strip = document.querySelector("#results-kpis");
  if (!strip?.querySelector) return;
  strip.querySelector(".kpi.value-kpi")?.remove();
  const stats = summary.overall;
  if (!stats.cases_with_value || (stats.min === 0 && stats.max === 0)) return;
  const box = document.createElement("div"); box.className = "kpi value-kpi";
  const label = document.createElement("div"); label.className = "lbl"; label.textContent = `Total ${summary.field.replace(/_/g, " ")}`;
  const value = document.createElement("div"); value.className = "val"; value.textContent = formatAmount(stats.total);
  const sub = document.createElement("div"); sub.className = "sub"; sub.textContent = `Typical ${formatAmount(stats.median)}`;
  box.append(label, value, sub);
  strip.children[1] ? strip.insertBefore(box, strip.children[2] || null) : strip.append(box);
}

// The engine's caveats in the words of the page.
function valueNote(summary) {
  const stats = summary.overall;
  const labels = itemLabels();
  const notes = [`Each ${labels.singular} counts its ${summary.field} once, however many steps it has.`];
  const missing = stats.cases - stats.cases_with_value;
  if (missing) notes.push(`${missing.toLocaleString()} ${caseNoun(missing)} without an amount ${missing === 1 ? "is" : "are"} counted, not treated as zero.`);
  if (stats.cases_with_value && stats.min === 0 && stats.max === 0) notes.push(`Every recorded ${summary.field} is 0, so this field carries no amounts; try another field.`);
  notes.push(stats.open_total === null || stats.open_total === undefined
    ? `Finishing steps are not confirmed, so the last step shown may be where it finished rather than where it waits.`
    : "Open means the last recorded step is not a confirmed finishing step.");
  return notes.join(" ");
}

function initializeValues() {
  document.querySelector("#value-field")?.addEventListener("change", (event) => {
    currentValueField = event.target.value;
    void loadValues(currentAnalysis);
  });
}
