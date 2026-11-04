// Filter controls; the engine applies every filter.
let preservedFieldFilters = { equals: [], ranges: [] };
let preservedNotReached = [];
let preservedAnalysisFilters = {};
const uniqueConditions = values => [...new Map(values.map(value => [JSON.stringify(Array.isArray(value) ? value : [value.attribute, value.at_least ?? null, value.at_most ?? null, value.greater_than ?? null, value.less_than ?? null]),value])).values()];

function populateTraceActivitySelect(select, selected = select.value) {
  const anyActivity = document.createElement("option");
  anyActivity.value = "";
  anyActivity.textContent = "Any activity";
  select.replaceChildren(anyActivity, ...(filterCatalog?.activities || []).map((activity) => {
    const option = document.createElement("option");
    option.value = activity;
    option.textContent = activity;
    option.selected = activity === selected;
    return option;
  }));
}

function makeTracePairRow(kind, from = "", to = "") {
  const row = document.createElement("div");
  row.className = "trace-pair-row";
  const earlierLabel = document.createElement("label");
  const earlierText = document.createElement("span");
  earlierText.textContent = "Earlier activity";
  const earlier = document.createElement("select");
  earlier.className = `trace-activity-select filter-${kind}-from`;
  populateTraceActivitySelect(earlier, from);
  earlierLabel.append(earlierText, earlier);
  const laterLabel = document.createElement("label");
  const laterText = document.createElement("span");
  laterText.textContent = kind === "direct" ? "Immediately next activity" : "Later activity";
  const later = document.createElement("select");
  later.className = `trace-activity-select filter-${kind}-to`;
  populateTraceActivitySelect(later, to);
  laterLabel.append(laterText, later);
  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "text-action trace-pair-remove";
  remove.textContent = "Remove";
  remove.setAttribute("aria-label", `Remove ${kind === "direct" ? "directly" : "eventually"}-follows condition`);
  row.append(earlierLabel, laterLabel, remove);
  return row;
}

function replaceTracePairRows(containerId, kind, pairs = []) {
  const container = document.querySelector(containerId);
  const visiblePairs = pairs.length ? pairs : [["", ""]];
  container.replaceChildren(...visiblePairs.map(([from, to]) => makeTracePairRow(kind, from, to)));
}

function tracePairsFromControls(containerId, fromClass, toClass, label) {
  return [...document.querySelectorAll(`${containerId} .trace-pair-row`)].flatMap((row) => {
    const from = row.querySelector(fromClass).value;
    const to = row.querySelector(toClass).value;
    if (!from && !to) return [];
    if (!from || !to) throw new Error(`Choose both activities for ${label}.`);
    return [[from, to]];
  });
}

// Reads a typed amount the way the engine does: digits, an optional sign and
// decimal point, and comma thousands separators. Blank means no bound.
function amountBound(text, label) {
  const trimmed = String(text || "").trim().replace(/^([-+]?)[$€£¥]\s*/, "$1");
  if (!trimmed) return null;
  const plain = /^[-+]?\d{1,3}(,\d{3})+(\.\d+)?$/.test(trimmed) ? trimmed.replace(/,/g, "") : trimmed;
  const number = /^[-+]?(\d+\.?\d*|\.\d+)$/.test(plain) ? Number(plain) : NaN;
  if (!Number.isFinite(number)) throw new Error(`${label} must be a number, such as 50000.`);
  return number;
}

function numberRangeFromControls() {
  const field = document.querySelector("#filter-number-field")?.value;
  if (!field) return [];
  const atLeast = amountBound(document.querySelector("#filter-number-min").value, "Amount at least");
  const atMost = amountBound(document.querySelector("#filter-number-max").value, "Amount at most");
  if (atLeast === null && atMost === null) return [];
  if (atLeast !== null && atMost !== null && atLeast > atMost) throw new Error("Amount at least must not be above amount at most.");
  return [{ attribute: field, at_least: atLeast, at_most: atMost }];
}

function fieldIsStepLevel(field) {
  return (filterCatalog?.stepLevel || currentAnalysis?.step_level_fields || []).includes(field);
}

function fieldIsNumeric(field) {
  return (filterCatalog?.numberFields || currentAnalysis?.number_fields || []).some(item => item.name === field);
}

function typedFieldRange() {
  const attribute = document.querySelector("#filter-attribute")?.value;
  const value = document.querySelector("#filter-attribute-value")?.value;
  const op = document.querySelector("#filter-operator")?.value || "eq";
  if (!attribute || !value || (!fieldIsNumeric(attribute) && (op === "eq" || op === "latest"))) return [];
  if (!fieldIsNumeric(attribute)) throw new Error("Choose a numeric field for this comparison.");
  const number = amountBound(value, "Value");
  const range = { attribute };
  if (op === "eq") { range.at_least = number; range.at_most = number; }
  else if (op === "gt") range.greater_than = number;
  else if (op === "gte") range.at_least = number;
  else if (op === "lt") range.less_than = number;
  else if (op === "lte") range.at_most = number;
  else if (op === "between") {
    range.at_least = number;
    range.at_most = amountBound(document.querySelector("#filter-attribute-upper")?.value, "Upper value");
    if (range.at_most === null) throw new Error("Enter the upper value.");
    if (range.at_most < number) throw new Error("Upper value must be at least the lower value.");
  } else throw new Error("Choose a supported comparison.");
  return [range];
}

let fieldChoicesRevision = 0;
async function loadFieldChoices() {
  const revision = ++fieldChoicesRevision;
  const field = document.querySelector("#filter-attribute")?.value;
  const input = document.querySelector("#filter-attribute-value");
  const list = document.querySelector("#filter-value-options");
  const hint = document.querySelector("#filter-value-hint");
  if (!input || !list || !hint) return;
  list.replaceChildren();
  const numeric = fieldIsNumeric(field);
  list.hidden = !field || numeric;
  const operator = document.querySelector("#filter-operator");
  // A field that changes between steps: "Had at any step" or "Latest value is".
  const stepLevel = fieldIsStepLevel(field);
  if (operator?.options) for (const option of operator.options) {
    if (option.value === "latest") { option.hidden = !stepLevel || numeric; option.disabled = !stepLevel || numeric; continue; }
    if (option.value === "eq") option.textContent = stepLevel && !numeric ? "Had at any step" : "Equals";
    option.disabled = option.value !== "eq" && !numeric;
  }
  if (!numeric && operator && !(stepLevel && operator.value === "latest")) operator.value = "eq";
  const upper = document.querySelector("#filter-upper-row");
  if (upper) upper.hidden = operator?.value !== "between";
  input.inputMode = numeric ? "decimal" : "text";
  input.placeholder = numeric ? "Enter a number" : "Choose or search actual values";
  if (!field || numeric) { hint.textContent = numeric ? "Numbers use the selected comparison; missing values do not match." : "Choose a field to see its recorded values."; return; }
  if (!currentProfile || !currentMapping || launchToken === null) return;
  hint.textContent = "Loading recorded values…";
  const filter = { ...currentFilter, attribute_equals: (currentFilter.attribute_equals || []).filter(pair => pair[0] !== field), attribute_between: (currentFilter.attribute_between || []).filter(range => range.attribute !== field),attribute_in:Object.fromEntries(Object.entries(currentFilter.attribute_in || {}).filter(([name])=>name!==field)) };
  const identity = currentProfile;
  try {
    const response = await projectFetch("/api/filter/options", {method:"POST",headers:{Authorization:`Bearer ${launchToken}`,"Content-Type":"application/json"},body:JSON.stringify(comparisonRequestBody({attribute:field,search:input.value,filter})),cache:"no-store"});
    const payload = await response.json();
    if (revision !== fieldChoicesRevision || identity !== currentProfile) return;
    if (!response.ok) throw new Error(payload.error || "Field choices unavailable");
    list.replaceChildren(...payload.values.map(choice => {const option=document.createElement("option");option.value=choice.value;option.textContent=`${choice.value} (${choice.cases.toLocaleString()} ${choice.cases === 1 ? "item" : "items"})`;return option;}));
    list.selectedIndex = -1;
    hint.textContent = `${payload.matching_values.toLocaleString()} recorded values${payload.truncated ? "; showing 100 — type to search the rest" : ""}${payload.missing_cases ? ` · ${payload.missing_cases.toLocaleString()} items without a value` : ""}`;
  } catch (error) { if (revision === fieldChoicesRevision && identity === currentProfile) hint.textContent = error.message; }
}

function clearFilterStatus() {
  const status = document.querySelector("#filter-status");
  if (!status) return;
  status.textContent = "";
  status.dataset.state = "";
}

function filterFromControls() {
  const withActivities = document.querySelector("#filter-with-activities");
  const withoutActivities = document.querySelector("#filter-without-activities");
  const startsWith = document.querySelector("#filter-starts-with");
  const endsWith = document.querySelector("#filter-ends-with");
  const attribute = document.querySelector("#filter-attribute");
  const attributeValue = document.querySelector("#filter-attribute-value");
  const startedAfter = document.querySelector("#filter-started-after");
  const startedBefore = document.querySelector("#filter-started-before");
  const minDuration = document.querySelector("#filter-min-duration");
  const maxDuration = document.querySelector("#filter-max-duration");
  const topVariants = document.querySelector("#filter-top-variants");
  const minEvents = document.querySelector("#filter-min-events");
  const completeOnly = document.querySelector("#filter-complete-only");
  const insideWindow = document.querySelector("#filter-inside-window");
  const extractWindow = currentAnalysis?.window;
  const operatorValue = document.querySelector("#filter-operator")?.value || "eq";
  const attributeEquals = attribute.value && attributeValue.value && !fieldIsNumeric(attribute.value) && operatorValue === "eq"
    ? [[attribute.value, attributeValue.value]]
    : [];
  const attributeLatest = { ...(preservedAnalysisFilters.attribute_latest || {}) };
  if (attribute.value && attributeValue.value && operatorValue === "latest") attributeLatest[attribute.value] = [attributeValue.value];
  const variantControl = document.querySelector("#filter-variants");
  const pickedVariants = variantControl ? selectedValues(variantControl).map((index) => filterCatalog?.variants?.[Number(index)]).filter(Boolean) : null;
  const listedVariant = new Set((filterCatalog?.variants || []).map((steps) => steps.join("\u0001")));
  const variants = pickedVariants
    ? [...pickedVariants, ...(preservedAnalysisFilters.variants || []).filter((steps) => !listedVariant.has(steps.join("\u0001")))]
    : preservedAnalysisFilters.variants || [];
  const repeatsControl = document.querySelector("#filter-repeats");
  const repeatsChosen = repeatsControl ? selectedValues(repeatsControl) : null;
  return {
    ...preservedAnalysisFilters,
    not_reached: preservedNotReached,
    handled_by: document.querySelector("#filter-people") ? selectedValues(document.querySelector("#filter-people")) : preservedAnalysisFilters.handled_by || [],
    attribute_latest: attributeLatest,
    variants,
    repeats: repeatsChosen ? repeatsChosen.filter((step) => step !== "__any__") : preservedAnalysisFilters.repeats || [],
    repeats_any: repeatsChosen ? repeatsChosen.includes("__any__") : Boolean(preservedAnalysisFilters.repeats_any),
    with_activities: selectedValues(withActivities),
    without_activities: selectedValues(withoutActivities),
    starts_with: startsWith.value || null,
    ends_with: endsWith.value || null,
    directly_follows: tracePairsFromControls("#filter-direct-pairs", ".filter-direct-from", ".filter-direct-to", "directly follows"),
    eventually_follows: tracePairsFromControls("#filter-eventual-pairs", ".filter-eventual-from", ".filter-eventual-to", "eventually follows"),
    attribute_equals: uniqueConditions([...preservedFieldFilters.equals, ...attributeEquals]),
    attribute_between: uniqueConditions([...preservedFieldFilters.ranges, ...numberRangeFromControls(), ...typedFieldRange()]),
    started_after: wallTimeMicros(startedAfter),
    started_before: wallTimeMicros(startedBefore),
    min_duration: numberOrNull(minDuration, 3_600_000_000),
    max_duration: numberOrNull(maxDuration, 3_600_000_000),
    top_variants: numberOrNull(topVariants),
    min_events: numberOrNull(minEvents),
    complete_only: completeOnly.checked,
    inside_window: insideWindow.checked && extractWindow
      ? { start_micros: extractWindow.start_micros, end_micros: extractWindow.end_micros, end_activities: extractWindow.end_activities }
      : null,
  };
}

function activeFilterCount(filter) {
  return filter.with_activities.length
    + filter.without_activities.length
    + (filter.not_reached?.length || 0)
    + (filter.starts_with ? 1 : 0)
    + (filter.ends_with ? 1 : 0)
    + filter.directly_follows.length
    + filter.eventually_follows.length
    + filter.attribute_equals.length
    + Object.keys(filter.attribute_in || {}).length + (filter.attribute_missing?.length || 0) + (filter.segment ? 1 : 0) + (filter.handled_by?.length || 0) + (filter.repeats?.length || 0) + (filter.repeats_any ? 1 : 0) + (filter.variants?.length || 0) + Object.keys(filter.attribute_latest || {}).length + (filter.outside_top_variants ? 1 : 0) + (filter.not_ending_with?.length ? 1 : 0)
    + (filter.attribute_between?.length || 0)
    + [filter.started_after, filter.started_before, filter.min_duration, filter.max_duration, filter.top_variants, filter.min_events].filter((value) => value !== null && value !== undefined).length
    + (filter.complete_only ? 1 : 0)
    + (filter.inside_window ? 1 : 0);
}

function populateFilterControls(analysis) {
  if (!filterCatalog) {
    filterCatalog = {
      numberFields: analysis.number_fields || [],
      activities: analysis.nodes.map((node) => node.activity).sort((a, b) => a.localeCompare(b)),
      attributes: [...analysis.case_attributes].sort((a, b) => a.localeCompare(b)),
      people: analysis.people || [],
      stepLevel: analysis.step_level_fields || [],
      // Numbered as in the first, unfiltered view, so V3 stays V3.
      variants: (analysis.variants || []).map((variant) => variant.sequence),
    };
  }
  // Variant numbers are learned from a view with no filter on, whenever one
  // arrives, so V3 means the same thing under any filter.
  if (!currentFilter || !activeFilterCount({ with_activities: [], without_activities: [], directly_follows: [], eventually_follows: [], attribute_equals: [], ...currentFilter })) {
    filterCatalog.variants = (analysis.variants || []).map((variant) => variant.sequence);
    filterCatalog.variants.forEach((steps, index) => rememberVariantRank(steps, index + 1));
  }
  // Who worked on the items, when a person column was mapped.
  const peopleMenu = document.querySelector("#filter-menu-people");
  if (peopleMenu) peopleMenu.hidden = !filterCatalog.people?.length;
  const people = document.querySelector("#filter-people");
  if (people) {
    const chosen = new Set(currentFilter?.handled_by || []);
    people.replaceChildren(...(filterCatalog.people || []).map((person) => {
      const option = document.createElement("option");
      option.value = person;
      option.textContent = person;
      option.selected = chosen.has(person);
      return option;
    }));
  }
  // Only an extract with a recorded window can label items against it.
  document.querySelector("#filter-inside-window-row").hidden = !analysis.window;
  if (analysis.window) {
    document.querySelector("#filter-inside-window-label").textContent = analysis.window.end_defined
      ? "Only items that began and finished inside the extract window"
      : "Only items that began inside the extract window (finishing steps not confirmed yet)";
  }
  for (const id of ["#filter-with-activities", "#filter-without-activities"]) {
    const select = document.querySelector(id);
    const selected = new Set(selectedValues(select));
    select.replaceChildren(...filterCatalog.activities.map((activity) => {
      const option = document.createElement("option");
      option.value = activity;
      option.textContent = activity;
      option.selected = selected.has(activity);
      return option;
    }));
  }
  // Particular variants, by number.
  const variantSelect = document.querySelector("#filter-variants");
  if (variantSelect) {
    const key = (steps) => steps.join("\u0001");
    const chosen = new Set((currentFilter?.variants || []).map(key));
    const total = currentAnalysis?.summary?.cases || 0;
    variantSelect.replaceChildren(...(filterCatalog.variants || []).map((steps, index) => {
      const option = document.createElement("option");
      option.value = String(index);
      const share = (currentAnalysis?.variants || []).find((variant) => key(variant.sequence) === key(steps));
      option.textContent = `V${variantNumber(steps, index + 1)} · ${steps.length} steps${share && total ? ` · ${Math.round(share.cases / total * 1000) / 10}%` : ""}`;
      option.selected = chosen.has(key(steps));
      return option;
    }));
  }
  // Repeats: any step, or particular ones.
  const repeats = document.querySelector("#filter-repeats");
  if (repeats) {
    const chosen = new Set(selectedValues(repeats).length ? selectedValues(repeats) : [...(currentFilter?.repeats || []), ...(currentFilter?.repeats_any ? ["__any__"] : [])]);
    repeats.replaceChildren(...[["__any__", "Any step"], ...filterCatalog.activities.map((activity) => [activity, activity])].map(([value, text]) => {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = text;
      option.selected = chosen.has(value);
      return option;
    }));
  }
  for (const id of ["#filter-starts-with", "#filter-ends-with"]) {
    const select = document.querySelector(id);
    const selected = select.value;
    populateTraceActivitySelect(select, selected);
  }
  for (const select of document.querySelectorAll(".trace-activity-select")) populateTraceActivitySelect(select);
  const numberField = document.querySelector("#filter-number-field");
  if (numberField) {
    const fields = (filterCatalog.numberFields || analysis.number_fields || []).map((field) => field.name);
    const selectedField = numberField.value;
    const none = document.createElement("option");
    none.value = "";
    none.textContent = "No amount filter";
    numberField.replaceChildren(none, ...fields.map((name) => {
      const option = document.createElement("option");
      option.value = name;
      option.textContent = name;
      option.selected = name === selectedField;
      return option;
    }));
    for (const row of document.querySelectorAll(".number-filter")) row.hidden = fields.length === 0 && !selectedField;
  }
  const attribute = document.querySelector("#filter-attribute");
  const selectedAttribute = attribute.value;
  const any = document.createElement("option");
  any.value = "";
  any.textContent = "Choose a field…";
  attribute.replaceChildren(any, ...filterCatalog.attributes.map((name) => {
    const option = document.createElement("option");
    option.value = name;
    option.textContent = name;
    option.selected = name === selectedAttribute;
    return option;
  }));
}

function resetFilterControls() {
  preservedAnalysisFilters = {};
  preservedNotReached = [];
  preservedFieldFilters = { equals: [], ranges: [] };
  fieldChoicesRevision++;
  const operator = document.querySelector("#filter-operator"); if (operator) operator.value = "eq";
  const upper = document.querySelector("#filter-attribute-upper"); if (upper) upper.value = "";
  for (const select of document.querySelectorAll("#filter-with-activities, #filter-without-activities, #filter-repeats, #filter-variants")) {
    for (const option of select.options) option.selected = false;
  }
  for (const id of ["#filter-starts-with", "#filter-ends-with", "#filter-attribute", "#filter-attribute-value", "#filter-number-field", "#filter-number-min", "#filter-number-max", "#filter-started-after", "#filter-started-before", "#filter-min-duration", "#filter-max-duration", "#filter-top-variants", "#filter-min-events"]) {
    const input = document.querySelector(id);
    input.value = "";
  }
  replaceTracePairRows("#filter-direct-pairs", "direct");
  replaceTracePairRows("#filter-eventual-pairs", "eventual");
  document.querySelector("#filter-complete-only").checked = false;
  document.querySelector("#filter-inside-window").checked = false;
}

function restoreFilterControls(filter) {
  preservedAnalysisFilters = {attribute_in:filter.attribute_in || {},attribute_missing:filter.attribute_missing || [],segment:filter.segment || null,handled_by:filter.handled_by || [],repeats:filter.repeats || [],repeats_any:Boolean(filter.repeats_any),variants:filter.variants || [],attribute_latest:filter.attribute_latest || {},outside_top_variants:filter.outside_top_variants || null,not_ending_with:filter.not_ending_with || []};
  preservedNotReached = filter.not_reached || [];
  preservedFieldFilters = { equals: (filter.attribute_equals || []).slice(1), ranges: (filter.attribute_between || []).slice(1) };
  for (const [id, values] of [
    ["#filter-with-activities", filter.with_activities || []],
    ["#filter-without-activities", filter.without_activities || []],
    ["#filter-people", filter.handled_by || []],
    ["#filter-repeats", [...(filter.repeats || []), ...(filter.repeats_any ? ["__any__"] : [])]],
    ["#filter-variants", (filterCatalog?.variants || []).map((steps, index) => (filter.variants || []).some((chosen) => chosen.join("\u0001") === steps.join("\u0001")) ? String(index) : null).filter((value) => value !== null)],
  ]) {
    const selected = new Set(values);
    const select = document.querySelector(id);
    if (!select) continue;
    for (const option of select.options) option.selected = selected.has(option.value);
  }
  replaceTracePairRows("#filter-direct-pairs", "direct", filter.directly_follows || []);
  replaceTracePairRows("#filter-eventual-pairs", "eventual", filter.eventually_follows || []);
  for (const [id, value] of [
    ["#filter-starts-with", filter.starts_with],
    ["#filter-ends-with", filter.ends_with],
  ]) document.querySelector(id).value = value ?? "";
  const attribute = filter.attribute_equals?.[0] || ["", ""];
  document.querySelector("#filter-attribute").value = attribute[0];
  document.querySelector("#filter-attribute-value").value = attribute[1];
  const range = filter.attribute_between?.[0];
  const operator = document.querySelector("#filter-operator"); if (operator) operator.value = "eq";
  const numberField = document.querySelector("#filter-number-field");
  if (range && ![...numberField.options].some((option) => option.value === range.attribute)) {
    const option = document.createElement("option");
    option.value = range.attribute;
    option.textContent = range.attribute;
    numberField.append(option);
  }
  numberField.value = range?.attribute ?? "";
  document.querySelector("#filter-number-min").value = range?.at_least ?? "";
  document.querySelector("#filter-number-max").value = range?.at_most ?? "";
  if (!attribute[0] && range && !(range.greater_than != null && range.less_than != null) && (range.greater_than != null || range.less_than != null) && operator) {
    document.querySelector("#filter-attribute").value = range.attribute;
    operator.value = range.greater_than != null ? "gt" : "lt";
    document.querySelector("#filter-attribute-value").value = range.greater_than ?? range.less_than;
    numberField.value = "";
  }
  if ((attribute[0] || (range?.greater_than != null && range?.less_than != null)) && range && (range.greater_than != null || range.less_than != null)) { preservedFieldFilters.ranges.unshift(range); numberField.value = ""; }
  for (const [id, value] of [
    ["#filter-started-after", filter.started_after],
    ["#filter-started-before", filter.started_before],
  ]) {
    document.querySelector(id).value = value === null || value === undefined
      ? ""
      : new Date(value / 1000).toISOString().slice(0, 16);
  }
  for (const [id, value] of [
    ["#filter-min-duration", filter.min_duration],
    ["#filter-max-duration", filter.max_duration],
  ]) document.querySelector(id).value = value === null || value === undefined ? "" : String(value / 3_600_000_000);
  document.querySelector("#filter-top-variants").value = filter.top_variants ?? "";
  document.querySelector("#filter-min-events").value = filter.min_events ?? "";
  document.querySelector("#filter-complete-only").checked = Boolean(filter.complete_only);
  document.querySelector("#filter-inside-window").checked = Boolean(filter.inside_window);
}

// Filters as chips: each active condition is a removable chip; "+ Filter"
// opens a short menu, and each entry shows only its own fields.
function filterChips(filter) {
  const chips = [];
  const without = (key, value) => ({ ...filter, [key]: value });
  const items = itemLabels().plural.toLowerCase();
  (filter.handled_by || []).forEach((person) => chips.push({ text: `Handled by ${person}`, next: without("handled_by", filter.handled_by.filter((p) => p !== person)) }));
  (filter.repeats || []).forEach((step) => chips.push({ text: `Repeats ${step}`, next: without("repeats", filter.repeats.filter((s) => s !== step)) }));
  if (filter.repeats_any) chips.push({ text: "Repeats a step", next: without("repeats_any", false) });
  // Items opened from a Conformance card keep that card's name: one chip,
  // not one per variant.
  const fromCheck = typeof conformanceFilterLabel !== "undefined" && conformanceFilterLabel && filter.variants?.length
    && conformanceFilterLabel.key === filter.variants.map((steps) => steps.join("\u0001")).sort().join("\u0002");
  if (fromCheck) chips.push({ text: conformanceFilterLabel.text, next: without("variants", []) });
  else (filter.variants || []).forEach((steps, index) => chips.push({ text: `Variant ${variantLabel(steps)}`, next: without("variants", filter.variants.filter((_, i) => i !== index)) }));
  for (const [field, values] of Object.entries(filter.attribute_latest || {})) { const next = { ...filter.attribute_latest }; delete next[field]; chips.push({ text: `${field} is now ${values.join(" or ")}`, next: without("attribute_latest", next) }); }
  if (filter.outside_top_variants) chips.push({ text: `Off the ${filter.outside_top_variants} main variants`, next: without("outside_top_variants", null) });
  if (filter.not_ending_with?.length) chips.push({ text: `Not finished (not at ${filter.not_ending_with.join(", ")})`, next: without("not_ending_with", []) });
  (filter.with_activities || []).forEach((step) => chips.push({ text: `Includes ${step}`, next: without("with_activities", filter.with_activities.filter((s) => s !== step)) }));
  (filter.without_activities || []).forEach((step) => chips.push({ text: `Skips ${step}`, next: without("without_activities", filter.without_activities.filter((s) => s !== step)) }));
  if (filter.starts_with) chips.push({ text: `Starts with ${filter.starts_with}`, next: without("starts_with", null) });
  if (filter.ends_with) chips.push({ text: `Ends with ${filter.ends_with}`, next: without("ends_with", null) });
  (filter.directly_follows || []).forEach((pair, index) => chips.push({ text: `${pair[0]} then ${pair[1]}`, next: without("directly_follows", filter.directly_follows.filter((_, i) => i !== index)) }));
  (filter.not_reached || []).forEach((pair, index) => chips.push({ text: `${pair[0]} without a later ${pair[1]}`, next: without("not_reached", filter.not_reached.filter((_, i) => i !== index)) }));
  (filter.eventually_follows || []).forEach((pair, index) => chips.push({ text: `${pair[0]} before ${pair[1]}`, next: without("eventually_follows", filter.eventually_follows.filter((_, i) => i !== index)) }));
  (filter.attribute_equals || []).forEach((pair, index) => chips.push({ text: fieldIsStepLevel(pair[0]) ? `${pair[0]} was ever ${pair[1]}` : `${pair[0]} = ${pair[1]}`, next: without("attribute_equals", filter.attribute_equals.filter((_, i) => i !== index)) }));
  (filter.attribute_between || []).forEach((range, index) => {
    const bounds = [range.greater_than != null ? `> ${Number(range.greater_than).toLocaleString()}` : "", range.less_than != null ? `< ${Number(range.less_than).toLocaleString()}` : "", range.at_least !== null && range.at_least !== undefined ? `≥ ${Number(range.at_least).toLocaleString()}` : "", range.at_most !== null && range.at_most !== undefined ? `≤ ${Number(range.at_most).toLocaleString()}` : ""].filter(Boolean).join(" and ");
    chips.push({ text: `${range.attribute} ${bounds}`, next: without("attribute_between", filter.attribute_between.filter((_, i) => i !== index)) });
  });
  for(const [field,values] of Object.entries(filter.attribute_in || {})) { const next={...filter.attribute_in};delete next[field];chips.push({text:fieldIsStepLevel(field)?`${field} was ever ${values.join(" or ")}`:`${field} in ${values.join(", ")}`,next:without("attribute_in",next)}); }
  for(const field of filter.attribute_missing || [])chips.push({text:`${field} is missing`,next:without("attribute_missing",filter.attribute_missing.filter(name=>name!==field))});
  if(filter.segment){const t=filter.segment;chips.push({text:`${t.from_occurrence || "first"} ${t.from} → ${t.to_occurrence || "first"} ${t.to}${t.min_micros!=null?` ≥ ${formatDuration(t.min_micros/1e6)}`:""}${t.max_micros!=null?` ≤ ${formatDuration(t.max_micros/1e6)}`:""}`,next:without("segment",null)});}
  const day = (micros) => new Date(micros / 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" });
  if (filter.started_after !== null && filter.started_after !== undefined) chips.push({ text: `Started from ${day(filter.started_after)}`, next: without("started_after", null) });
  if (filter.started_before !== null && filter.started_before !== undefined) chips.push({ text: `Started by ${day(filter.started_before)}`, next: without("started_before", null) });
  if (filter.min_duration !== null && filter.min_duration !== undefined) chips.push({ text: `Longer than ${formatDuration(filter.min_duration / 1_000_000)}`, next: without("min_duration", null) });
  if (filter.max_duration !== null && filter.max_duration !== undefined) chips.push({ text: `Shorter than ${formatDuration(filter.max_duration / 1_000_000)}`, next: without("max_duration", null) });
  if (filter.top_variants !== null && filter.top_variants !== undefined) chips.push({ text: `Top ${filter.top_variants} variants`, next: without("top_variants", null) });
  if (filter.min_events !== null && filter.min_events !== undefined) chips.push({ text: `At least ${filter.min_events} steps`, next: without("min_events", null) });
  if (filter.complete_only) chips.push({ text: "Complete only", next: without("complete_only", false) });
  if (filter.inside_window) chips.push({ text: `Only ${items} inside the extract window`, next: without("inside_window", null) });
  return chips;
}

function renderFilterChips(filter = currentFilter || {}) {
  const list = document.querySelector("#filter-chip-list");
  if (!list) return;
  list.replaceChildren(...filterChips(filter).map((chip) => {
    const element = document.createElement("span"); element.className = "filter-chip";
    const text = document.createElement("span"); text.textContent = chip.text;
    const remove = document.createElement("button"); remove.type = "button"; remove.textContent = "×";
    remove.setAttribute("aria-label", `Remove filter: ${chip.text}`);
    remove.addEventListener("click", () => {
      restoreFilterControls(chip.next);
      document.querySelector("#apply-filters")?.click();
    });
    element.append(text, remove);
    return element;
  }));
  const amount = document.querySelector("#filter-menu-amount");
  if (amount) amount.hidden = !(filterCatalog?.numberFields || currentAnalysis?.number_fields || []).length;
}

function openFilterGroup(group) {
  const panel = document.querySelector("#filter-panel");
  if (!panel) return;
  restoreFilterControls(currentFilter || {});
  if (group === "field") {
    preservedFieldFilters = {equals:currentFilter?.attribute_equals || [],ranges:currentFilter?.attribute_between || []};
    document.querySelector("#filter-attribute").value = "";
    document.querySelector("#filter-attribute-value").value = "";
    const operator=document.querySelector("#filter-operator"); if (operator) operator.value="eq";
  }
  for (const element of panel.querySelectorAll("[data-filter-group]")) {
    element.classList.toggle("filter-off", element.dataset.filterGroup !== group);
  }
  panel.dataset.group = group;
  if (group === "field") void loadFieldChoices();
  panel.open = true;
  panel.classList.add("as-popover");
  // Amount fields are offered only when the items carry numbers; show them here.
  if (group === "amount") for (const row of panel.querySelectorAll(".number-filter")) row.hidden = false;
  placeFilterPanel(panel);
  panel.querySelector("[data-filter-group]:not(.filter-off) select, [data-filter-group]:not(.filter-off) input")?.focus({ preventScroll: true });
}

// On every page the open filter sits right under + Filter.
function placeFilterPanel(panel) {
  panel.style.left = ""; panel.style.top = ""; panel.style.right = "";
  const add = document.querySelector("#filter-add");
  const host = panel.offsetParent;
  if (!add || !host) return;
  const button = add.getBoundingClientRect(), frame = host.getBoundingClientRect();
  panel.style.left = `${Math.max(8, Math.min(button.left - frame.left, frame.width - panel.offsetWidth - 8))}px`;
  panel.style.top = `${button.bottom - frame.top + 8}px`;
}

function closeFilterPanel() {
  const panel = document.querySelector("#filter-panel");
  if (!panel) return;
  panel.open = false;
  panel.classList.remove("as-popover");
  document.querySelector("#filter-menu")?.setAttribute("hidden", "");
  document.querySelector("#filter-add")?.setAttribute("aria-expanded", "false");
}

function initializeFilterChips() {
  let choiceTimer = null;
  document.querySelector("#filter-attribute")?.addEventListener("change", () => {document.querySelector("#filter-attribute-value").value=""; void loadFieldChoices();});
  document.querySelector("#filter-value-options")?.addEventListener("change", event => { document.querySelector("#filter-attribute-value").value = event.target.value; });
  document.querySelector("#filter-operator")?.addEventListener("change", () => void loadFieldChoices());
  document.querySelector("#filter-attribute-value")?.addEventListener("input", () => {clearTimeout(choiceTimer);choiceTimer=setTimeout(() => void loadFieldChoices(),150);});
  const add = document.querySelector("#filter-add");
  const menu = document.querySelector("#filter-menu");
  if (!add || !menu) return;
  add.addEventListener("click", () => {
    const open = menu.hidden;
    menu.hidden = !open;
    add.setAttribute("aria-expanded", String(open));
  });
  for (const item of menu.querySelectorAll("[data-filter-open]")) {
    item.addEventListener("click", () => {
      menu.hidden = true;
      add.setAttribute("aria-expanded", "false");
      openFilterGroup(item.dataset.filterOpen);
    });
  }
  document.querySelector("#filter-cancel")?.addEventListener("click", () => {
    restoreFilterControls(currentFilter || {});
    closeFilterPanel();
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") closeFilterPanel();
  });
  // A click anywhere else closes the menu, and the open filter without
  // applying it, like Cancel.
  document.addEventListener("mousedown", (event) => {
    const panel = document.querySelector("#filter-panel");
    const inside = (element) => element && (element === event.target || element.contains(event.target));
    if (inside(add) || inside(menu) || event.target.closest?.(".select-menu")) return;
    if (!menu.hidden) { menu.hidden = true; add.setAttribute("aria-expanded", "false"); }
    if (panel?.classList.contains("as-popover") && !inside(panel)) {
      restoreFilterControls(currentFilter || {});
      closeFilterPanel();
    }
  });
}
