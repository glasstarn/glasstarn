// Compare: cohort and period comparisons.

function comparisonRequestBody(extra = {}) {
  return {
    upload_id: currentProfile.upload_id,
    mapping: currentMapping,
    policy: currentPolicy,
    rules: currentRules,
    filter: currentFilter,
    ...extra,
  };
}

function microsDateTime(micros) {
  return new Date(micros / 1000).toISOString().slice(0, 19);
}

function populateCohortValues() {
  const attribute = document.querySelector("#comparison-attribute");
  const left = document.querySelector("#comparison-left");
  const right = document.querySelector("#comparison-right");
  const button = document.querySelector("#compare-cohorts");
  if (!attribute || !left || !right || !button) return;
  const selected = currentComparisonOptions?.attributes.find((item) => item.name === attribute.value)
    || currentComparisonOptions?.attributes[0];
  const values = selected?.values || [];
  left.replaceChildren(...values.map((value, index) => {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = value;
    option.selected = index === 0;
    return option;
  }));
  right.replaceChildren(...values.map((value, index) => {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = value;
    option.selected = index === 1;
    return option;
  }));
  button.disabled = values.length < 2;
}

async function loadComparisonOptions() {
  const result = document.querySelector("#comparison-result");
  const attribute = document.querySelector("#comparison-attribute");
  const range = document.querySelector("#comparison-range");
  if (!result || !attribute || !range || !currentProfile || !currentMapping) return;
  currentComparisonOptions = null;
  result.innerHTML = "<p>Preparing comparison choices for this case view…</p>";
  try {
    const response = await projectFetch("/api/compare/options", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify(comparisonRequestBody()),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `comparison options returned ${response.status}`);
    currentComparisonOptions = payload;
    attribute.replaceChildren(...payload.attributes.map((item) => {
      const option = document.createElement("option");
      option.value = item.name;
      option.textContent = item.name;
      return option;
    }));
    populateCohortValues();
    if (payload.started_at_min !== null && payload.started_at_max !== null) {
      range.textContent = `First recorded steps range from ${new Date(payload.started_at_min / 1000).toLocaleString()} to ${new Date(payload.started_at_max / 1000).toLocaleString()}.`;
      document.querySelector("#baseline-start").value = microsDateTime(payload.started_at_min);
      document.querySelector("#comparison-end").value = microsDateTime(payload.started_at_max);
    } else {
      range.textContent = "No start times are available for comparing periods.";
    }
    result.innerHTML = payload.attributes.some((item) => item.values.length >= 2)
      ? "<p>Choose two groups or two non-overlapping time periods to compare.</p>"
      : "<p>No case attribute in this view currently has two values. Time-period comparison is still available.</p>";
  } catch (error) {
    result.textContent = error instanceof Error ? error.message : "Comparison choices could not be prepared.";
  }
}

function signedDuration(seconds) {
  if (seconds > 0) return `+${formatDuration(seconds)}`;
  if (seconds < 0) return `−${formatDuration(Math.abs(seconds))}`;
  return formatDuration(0);
}

function renderComparison(comparison, kind) {
  const result = document.querySelector("#comparison-result");
  if (!result) return;
  const left = kind === "cohorts" ? comparison.left : comparison.baseline;
  const right = kind === "cohorts" ? comparison.right : comparison.comparison;
  const leftLabel = kind === "cohorts" ? left.value : "Baseline";
  const rightLabel = kind === "cohorts" ? right.value : "Comparison";
  result.replaceChildren();

  const summary = document.createElement("div");
  summary.className = "comparison-summary";
  for (const [label, cohort] of [[leftLabel, left], [rightLabel, right]]) {
    const article = document.createElement("article");
    const title = document.createElement("strong"); title.textContent = label;
    const cases = document.createElement("span"); cases.textContent = `${cohort.cases.toLocaleString()} ${itemLabels().plural.toLowerCase()} · ${cohort.duration_eligible_cases.toLocaleString()} with measurable time`;
    const duration = document.createElement("b"); duration.textContent = `${measuredDuration(cohort.mean_throughput_seconds, cohort.duration_eligible_cases)} average completion`;
    article.append(title, cases, duration); summary.append(article);
  }
  const change = document.createElement("article"); change.className = "comparison-change";
  const changeLabel = document.createElement("span"); changeLabel.textContent = `${rightLabel} minus ${leftLabel}`;
  const changeValue = document.createElement("strong"); changeValue.textContent = signedDuration(comparison.mean_throughput_difference_seconds);
  change.append(changeLabel, changeValue); summary.append(change);
  if (kind === "cohorts" && comparison.shared_items) {
    const shared = document.createElement("p"); shared.className = "comparison-shared";
    shared.textContent = `${comparison.shared_items.toLocaleString()} ${itemLabels().plural.toLowerCase()} had both values at different steps, so they are in both groups.`;
    result.append(summary, shared);
  } else result.append(summary);

  const tableWrap = document.createElement("div"); tableWrap.className = "comparison-table-wrap";
  const table = document.createElement("table"); table.className = "comparison-table";
  const head = document.createElement("thead");
  const headingRow = document.createElement("tr");
  for (const label of ["Step", leftLabel, rightLabel, "Difference", "Examples"]) {
    const heading = document.createElement("th"); heading.scope = "col"; heading.textContent = label; headingRow.append(heading);
  }
  head.append(headingRow);
  const body = document.createElement("tbody");
  body.replaceChildren(...comparison.activity_differences.slice(0, 15).map((difference) => {
    const row = document.createElement("tr");
    const activity = document.createElement("th"); activity.scope = "row"; activity.textContent = difference.activity;
    const leftShare = document.createElement("td"); leftShare.textContent = `${difference.left_case_percent.toLocaleString(undefined, { maximumFractionDigits: 1 })}%`;
    const rightShare = document.createElement("td"); rightShare.textContent = `${difference.right_case_percent.toLocaleString(undefined, { maximumFractionDigits: 1 })}%`;
    const delta = document.createElement("td"); delta.textContent = `${difference.difference_points > 0 ? "+" : ""}${difference.difference_points.toLocaleString(undefined, { maximumFractionDigits: 1 })} points`;
    const evidence = document.createElement("td");
    evidence.append(evidenceCaseButtons([...difference.left_example_cases, ...difference.right_example_cases]));
    row.append(activity, leftShare, rightShare, delta, evidence);
    return row;
  }));
  table.append(head, body); tableWrap.append(table); result.append(tableWrap);

  const limits = document.createElement("div"); limits.className = "comparison-limits";
  const limitsTitle = document.createElement("strong"); limitsTitle.textContent = "How to interpret this";
  const list = document.createElement("ul");
  const plainLimits = ["A difference between groups does not establish its cause.", "Check group sizes and how many items have measurable time.", "Different work complexity, routing or missing data may explain the difference."];
  list.replaceChildren(...plainLimits.map((text) => { const item = document.createElement("li"); item.textContent = text; return item; }));
  limits.append(limitsTitle, list); result.append(limits);
}

async function runCohortComparison() {
  const result = document.querySelector("#comparison-result");
  const button = document.querySelector("#compare-cohorts");
  const attribute = document.querySelector("#comparison-attribute").value;
  const left = document.querySelector("#comparison-left").value;
  const right = document.querySelector("#comparison-right").value;
  if (!result || !button || !attribute || !left || !right) return;
  if (left === right) { result.textContent = "Choose two different groups."; return; }
  button.disabled = true;
  result.textContent = "Comparing the two groups locally…";
  try {
    const response = await projectFetch("/api/compare/cohorts", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify(comparisonRequestBody({ attribute, left, right })),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `comparison returned ${response.status}`);
    renderComparison(payload, "cohorts");
  } catch (error) {
    result.textContent = error instanceof Error ? error.message : "The groups could not be compared.";
  } finally {
    button.disabled = false;
  }
}

async function runPeriodComparison() {
  const result = document.querySelector("#comparison-result");
  const button = document.querySelector("#compare-periods");
  const values = ["#baseline-start", "#baseline-end", "#comparison-start", "#comparison-end"]
    .map((id) => wallTimeMicros(document.querySelector(id)));
  if (!result || !button) return;
  if (values.some((value) => value === null)) { result.textContent = "Choose all four period boundaries."; return; }
  if (values[0] > values[1] || values[2] > values[3]) { result.textContent = "Each period must start before it ends."; return; }
  if (values[1] >= values[2]) { result.textContent = "The baseline must end before the comparison period starts."; return; }
  button.disabled = true;
  result.textContent = "Comparing the two time periods locally…";
  try {
    const response = await projectFetch("/api/compare/periods", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify(comparisonRequestBody({ baseline_start: values[0], baseline_end: values[1], comparison_start: values[2], comparison_end: values[3] })),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `comparison returned ${response.status}`);
    renderComparison(payload, "periods");
  } catch (error) {
    result.textContent = error instanceof Error ? error.message : "The time periods could not be compared.";
  } finally {
    button.disabled = false;
  }
}
