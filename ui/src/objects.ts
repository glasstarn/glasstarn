// Connected processes: object-centric (OCEL) analysis views.

function ocelSummaryCard(value, label, note) {
  const card = document.createElement("article");
  const number = document.createElement("strong"); number.textContent = value.toLocaleString();
  const title = document.createElement("span"); title.textContent = label;
  const detail = document.createElement("small"); detail.textContent = note;
  card.append(number, title, detail);
  return card;
}

function populateOcelSelect(id, values, selectedValues) {
  const select = document.querySelector(id);
  if (!select) return;
  const selected = new Set(selectedValues || []);
  select.replaceChildren(...values.map((value) => {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = value;
    option.selected = selected.has(value);
    return option;
  }));
}

function populateOcelActivitySelect(id, values, defaultIndex) {
  const select = document.querySelector(id);
  if (!select) return;
  const previous = select.value;
  select.replaceChildren(...values.map((value) => {
    const option = document.createElement("option"); option.value = value; option.textContent = value; return option;
  }));
  select.value = values.includes(previous) ? previous : values[Math.min(defaultIndex, Math.max(0, values.length - 1))] || "";
}

function renderOcelEvidence(title, facts, evidence) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Source evidence";
  const heading = document.createElement("h3"); heading.textContent = title;
  const list = document.createElement("dl");
  for (const [name, value] of facts) {
    const term = document.createElement("dt"); term.textContent = name;
    const detail = document.createElement("dd"); detail.textContent = value;
    list.append(term, detail);
  }
  const evidenceList = document.createElement("ul"); evidenceList.className = "ocel-evidence-list";
  evidenceList.replaceChildren(...evidence.map((text) => {
    const item = document.createElement("li"); item.textContent = text; return item;
  }));
  const note = document.createElement("p");
  note.textContent = evidence.length ? "Bounded examples from the original OCEL identities." : "No source examples are available for this aggregate.";
  panel.replaceChildren(label, heading, list, evidenceList, note);
}

function resetOcelEvidence() {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence(false);
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Source evidence";
  const heading = document.createElement("h3"); heading.textContent = "Select a lifecycle or relationship";
  const note = document.createElement("p"); note.textContent = "See the original events and objects behind an aggregate in the current view.";
  panel.replaceChildren(label, heading, note);
}

function ocelResultButton(title, meta, onSelect) {
  const button = document.createElement("button"); button.type = "button"; button.className = "ocel-result-row";
  const main = document.createElement("strong"); main.textContent = title;
  const detail = document.createElement("span"); detail.textContent = meta;
  const arrow = document.createElement("i"); arrow.setAttribute("aria-hidden", "true"); arrow.textContent = "→";
  button.append(main, detail, arrow);
  button.addEventListener("click", onSelect);
  return button;
}

function ocelVariantPath(variant) {
  if (!variant.layers.length) return "No recorded events";
  const steps = variant.layers
    .map((layer) => {
      const shown = layer.omitted ? [...layer.activities, `${layer.omitted.toLocaleString()} more`] : layer.activities;
      return shown.length > 1 ? `[${shown.join(" + ")}]` : shown[0];
    });
  // A very long route lists its first steps and its last.
  if (variant.layers_omitted) steps.splice(steps.length - 1, 0, `… ${variant.layers_omitted.toLocaleString()} more steps …`);
  return steps.join(" → ");
}

function formatOcelTimestamp(micros) {
  const date = new Date(micros / 1000);
  return Number.isNaN(date.getTime())
    ? `${micros.toLocaleString()} µs`
    : date.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "medium" });
}

function renderOcelVariantEvidence(variant) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Variant evidence";
  const heading = document.createElement("h3"); heading.textContent = `${variant.object_type} variant ${variant.rank}`;
  const path = document.createElement("p"); path.textContent = ocelVariantPath(variant);
  const facts = document.createElement("dl");
  for (const [name, value] of [
    ["Objects", variant.objects.toLocaleString()],
    ["Share within type", `${(variant.share_millionths / 10000).toLocaleString(undefined, { maximumFractionDigits: 2 })}%`],
    ["Duration eligible", `${variant.duration_eligible_objects.toLocaleString()} of ${variant.objects.toLocaleString()}`],
    ["Mean duration", variant.mean_duration_micros === null ? "Not available" : formatDuration(variant.mean_duration_micros / 1_000_000)],
  ]) {
    const term = document.createElement("dt"); term.textContent = name;
    const detail = document.createElement("dd"); detail.textContent = value;
    facts.append(term, detail);
  }
  const actions = document.createElement("div"); actions.className = "ocel-evidence-actions";
  actions.replaceChildren(...variant.evidence_objects.map((object) => {
    const button = document.createElement("button"); button.type = "button"; button.className = "ocel-evidence-action";
    button.textContent = `Inspect ${object}`;
    button.addEventListener("click", () => void inspectOcelObject(object));
    return button;
  }));
  const note = document.createElement("p");
  note.textContent = variant.evidence_objects.length
    ? "Choose an example object to inspect its original qualified lifecycle."
    : "This variant has no lifecycle evidence object.";
  panel.replaceChildren(label, heading, path, facts, actions, note);
}

function renderOcelLifecycle(lifecycle) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Object lifecycle";
  const heading = document.createElement("h3"); heading.textContent = lifecycle.object;
  const detail = document.createElement("p");
  detail.textContent = `${lifecycle.object_type} · ${lifecycle.events.length.toLocaleString()} ${lifecycle.events.length === 1 ? "event" : "events"} · ${lifecycle.duration_micros === null ? "duration unavailable" : formatDuration(lifecycle.duration_micros / 1_000_000)}`;
  const timeline = document.createElement("ol"); timeline.className = "ocel-lifecycle";
  timeline.replaceChildren(...lifecycle.events.map((event) => {
    const item = document.createElement("li");
    const layer = document.createElement("b"); layer.textContent = `L${event.time_layer + 1}`;
    const content = document.createElement("div");
    const activity = document.createElement("strong"); activity.textContent = event.activity;
    const evidence = document.createElement("span");
    evidence.textContent = `${event.event} · ${formatOcelTimestamp(event.at_micros)}${event.qualifiers.length ? ` · ${event.qualifiers.join(", ")}` : ""}`;
    content.append(activity, evidence); item.append(layer, content); return item;
  }));
  const note = document.createElement("p"); note.textContent = "Events sharing a layer happened at the same timestamp; no order between them is implied.";
  panel.replaceChildren(label, heading, detail, timeline, note);
}

async function inspectOcelObject(object) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel || !currentOcelUpload || !currentOcelAnalysis) return;
  panel.textContent = `Loading ${object}'s lifecycle locally…`;
  try {
    const response = await projectFetch("/api/ocel/object", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ upload_id: currentOcelUpload, object, filter: currentOcelAnalysis.filter }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `object lifecycle returned ${response.status}`);
    renderOcelLifecycle(payload.lifecycle);
  } catch (error) {
    panel.textContent = error instanceof Error ? error.message : "The object lifecycle could not be inspected.";
  }
}

function renderOcelTimeEvidence(report, measurement) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Timing evidence";
  const heading = document.createElement("h3"); heading.textContent = `${report.from_activity} → ${report.to_activity}`;
  const detail = document.createElement("p"); detail.textContent = `Connected through ${measurement.object_type}`;
  const facts = document.createElement("dl");
  for (const [name, value] of [
    ["Observations", measurement.observations.toLocaleString()],
    ["Objects", measurement.objects.toLocaleString()],
    ["Mean elapsed", formatDuration(measurement.mean_elapsed_micros / 1_000_000)],
    ["Observed range", `${formatDuration(measurement.min_elapsed_micros / 1_000_000)} – ${formatDuration(measurement.max_elapsed_micros / 1_000_000)}`],
  ]) {
    const term = document.createElement("dt"); term.textContent = name;
    const valueElement = document.createElement("dd"); valueElement.textContent = value;
    facts.append(term, valueElement);
  }
  const evidence = document.createElement("ul"); evidence.className = "ocel-evidence-list";
  evidence.replaceChildren(...measurement.evidence.map((item) => {
    const row = document.createElement("li");
    row.textContent = `${item.object}: ${item.source_event} → ${item.target_event} · ${formatDuration(item.elapsed_micros / 1_000_000)}`;
    return row;
  }));
  const actions = document.createElement("div"); actions.className = "ocel-evidence-actions";
  const objects = [...new Set(measurement.evidence.map((item) => item.object))];
  actions.replaceChildren(...objects.map((object) => {
    const button = document.createElement("button"); button.type = "button"; button.className = "ocel-evidence-action"; button.textContent = `Inspect ${object}`;
    button.addEventListener("click", () => void inspectOcelObject(object)); return button;
  }));
  const note = document.createElement("p"); note.textContent = report.semantics;
  panel.replaceChildren(label, heading, detail, facts, evidence, actions, note);
}

function renderOcelTimeBetween(report) {
  const results = document.querySelector("#ocel-time-results");
  const status = document.querySelector("#ocel-time-status");
  if (!results || !status) return;
  results.replaceChildren(...report.measurements.map((measurement) => ocelResultButton(
    `${report.from_activity} → ${report.to_activity}`,
    `${measurement.object_type} · ${measurement.objects.toLocaleString()} ${measurement.objects === 1 ? "object" : "objects"} · ${formatDuration(measurement.mean_elapsed_micros / 1_000_000)} mean elapsed`,
    () => renderOcelTimeEvidence(report, measurement),
  )));
  if (!report.measurements.length) results.textContent = "No object in this view connects those activities in that order.";
  status.textContent = report.measurements.length
    ? `${report.measurements.length.toLocaleString()} common ${report.measurements.length === 1 ? "object type" : "object types"} found.`
    : "No common object path found.";
  status.dataset.state = "ready";
}

function renderOcelSynchronizationEvidence(report, activity) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Synchronization evidence";
  const heading = document.createElement("h3"); heading.textContent = activity.activity;
  const detail = document.createElement("p");
  detail.textContent = `${activity.object_types.join(" + ")} · ${activity.multi_object_events.toLocaleString()} multi-object ${activity.multi_object_events === 1 ? "event" : "events"}`;
  const facts = document.createElement("dl");
  const rows = [
    ["Eligible events", activity.eligible_events.toLocaleString()],
    ["Insufficient history", activity.insufficient_history_events.toLocaleString()],
    ["Earlier-history objects", activity.historically_observed_object_participations.toLocaleString()],
    ["First seen at event", activity.first_seen_object_participations.toLocaleString()],
  ];
  if (activity.eligible_events) rows.push(
    ["Mean prior-point spread", formatDuration(activity.mean_arrival_spread_micros / 1_000_000)],
    ["Mean after latest prior point", formatDuration(activity.mean_elapsed_after_last_prior_micros / 1_000_000)],
    ["Mean from earliest prior point", formatDuration(activity.mean_elapsed_from_first_prior_micros / 1_000_000)],
  );
  for (const [name, value] of rows) {
    const term = document.createElement("dt"); term.textContent = name;
    const valueElement = document.createElement("dd"); valueElement.textContent = value;
    facts.append(term, valueElement);
  }
  const evidence = document.createElement("ul"); evidence.className = "ocel-evidence-list";
  evidence.replaceChildren(...activity.evidence.map((item) => {
    const row = document.createElement("li");
    const event = document.createElement("strong"); event.textContent = `${item.event} · ${formatOcelTimestamp(item.at_micros)}`;
    const timing = document.createElement("span");
    timing.textContent = `${item.historically_observed_objects.toLocaleString()} of ${item.participating_objects.toLocaleString()} objects had earlier history · ${formatDuration(item.arrival_spread_micros / 1_000_000)} spread + ${formatDuration(item.elapsed_after_last_prior_micros / 1_000_000)} after latest prior point`;
    const readiness = document.createElement("ul");
    readiness.replaceChildren(...item.readiness.map((prior) => {
      const priorRow = document.createElement("li");
      priorRow.textContent = `${prior.object} (${prior.object_type}): ${prior.prior_events.join(" + ")} at ${formatOcelTimestamp(prior.prior_at_micros)}`;
      return priorRow;
    }));
    row.append(event, timing, readiness);
    return row;
  }));
  if (!activity.evidence.length) evidence.textContent = "No eligible event has two participating objects with earlier history in this view.";
  const actions = document.createElement("div"); actions.className = "ocel-evidence-actions";
  const objects = [...new Set(activity.evidence.flatMap((item) => item.readiness.map((prior) => prior.object)))];
  actions.replaceChildren(...objects.map((object) => {
    const button = document.createElement("button"); button.type = "button"; button.className = "ocel-evidence-action"; button.textContent = `Inspect ${object}`;
    button.addEventListener("click", () => void inspectOcelObject(object)); return button;
  }));
  const note = document.createElement("p"); note.textContent = report.semantics;
  panel.replaceChildren(label, heading, detail, facts, evidence, actions, note);
}

function renderOcelSynchronization(report) {
  const results = document.querySelector("#ocel-synchronization-results");
  const status = document.querySelector("#ocel-synchronization-status");
  if (!results || !status) return;
  results.replaceChildren(...report.activities.map((activity) => ocelResultButton(
    activity.activity,
    activity.eligible_events
      ? `${activity.eligible_events.toLocaleString()} eligible of ${activity.multi_object_events.toLocaleString()} multi-object ${activity.multi_object_events === 1 ? "event" : "events"} · ${formatDuration(activity.mean_arrival_spread_micros / 1_000_000)} mean prior-point spread · ${formatDuration(activity.mean_elapsed_after_last_prior_micros / 1_000_000)} mean after latest prior point`
      : `${activity.multi_object_events.toLocaleString()} multi-object ${activity.multi_object_events === 1 ? "event" : "events"} · insufficient earlier history`,
    () => renderOcelSynchronizationEvidence(report, activity),
  )));
  if (!report.activities.length) results.textContent = "No event in this view connects two or more objects.";
  const eligible = report.activities.reduce((total, activity) => total + activity.eligible_events, 0);
  status.textContent = report.activities.length
    ? `${eligible.toLocaleString()} eligible multi-object ${eligible === 1 ? "event" : "events"} across ${report.activities.length.toLocaleString()} ${report.activities.length === 1 ? "activity" : "activities"}.`
    : "No multi-object events found.";
  status.dataset.state = "ready";
}

function sharePercent(millionths) {
  return `${(millionths / 10_000).toLocaleString(undefined, { maximumFractionDigits: 2 })}%`;
}

function renderOcelLifecycleBottleneckEvidence(report, item) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Lifecycle burden evidence";
  const heading = document.createElement("h3"); heading.textContent = `#${item.rank} · ${item.source} → ${item.target}`;
  const detail = document.createElement("p"); detail.textContent = `${item.object_type} lifecycle transition`;
  const facts = document.createElement("dl");
  for (const [name, value] of [
    ["Total elapsed burden", formatDuration(item.total_elapsed_micros / 1_000_000)],
    ["Mean elapsed", formatDuration(item.mean_elapsed_micros / 1_000_000)],
    ["Share of ranked burden", sharePercent(item.share_millionths)],
    ["Occurrences", item.occurrences.toLocaleString()],
    ["Objects", item.objects.toLocaleString()],
  ]) {
    const term = document.createElement("dt"); term.textContent = name;
    const valueElement = document.createElement("dd"); valueElement.textContent = value;
    facts.append(term, valueElement);
  }
  const evidence = document.createElement("ul"); evidence.className = "ocel-evidence-list";
  evidence.replaceChildren(...item.evidence.map((value) => {
    const row = document.createElement("li");
    row.textContent = `${value.object}: ${value.source_event} at ${formatOcelTimestamp(value.source_at_micros)} → ${value.target_event} at ${formatOcelTimestamp(value.target_at_micros)}`;
    return row;
  }));
  const actions = document.createElement("div"); actions.className = "ocel-evidence-actions";
  const objects = [...new Set(item.evidence.map((value) => value.object))];
  actions.replaceChildren(...objects.map((object) => {
    const button = document.createElement("button"); button.type = "button"; button.className = "ocel-evidence-action"; button.textContent = `Inspect ${object}`;
    button.addEventListener("click", () => void inspectOcelObject(object)); return button;
  }));
  const note = document.createElement("p"); note.textContent = report.semantics;
  panel.replaceChildren(label, heading, detail, facts, evidence, actions, note);
}

function renderOcelSynchronizationBottleneckEvidence(report, item) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Convergence burden evidence";
  const heading = document.createElement("h3"); heading.textContent = `#${item.rank} · ${item.activity}`;
  const detail = document.createElement("p"); detail.textContent = item.object_types.join(" + ");
  const facts = document.createElement("dl");
  for (const [name, value] of [
    ["Total prior-point spread", formatDuration(item.total_arrival_spread_micros / 1_000_000)],
    ["Mean prior-point spread", formatDuration(item.mean_arrival_spread_micros / 1_000_000)],
    ["Share of ranked spread", sharePercent(item.share_millionths)],
    ["Eligible events", item.eligible_events.toLocaleString()],
    ["Insufficient history", item.insufficient_history_events.toLocaleString()],
  ]) {
    const term = document.createElement("dt"); term.textContent = name;
    const valueElement = document.createElement("dd"); valueElement.textContent = value;
    facts.append(term, valueElement);
  }
  const evidence = document.createElement("ul"); evidence.className = "ocel-evidence-list";
  evidence.replaceChildren(...item.evidence.map((value) => {
    const row = document.createElement("li");
    const title = document.createElement("strong"); title.textContent = `${value.event} · ${formatOcelTimestamp(value.at_micros)}`;
    const timing = document.createElement("span"); timing.textContent = `${formatDuration(value.arrival_spread_micros / 1_000_000)} prior-point spread across ${value.historically_observed_objects.toLocaleString()} historically observed objects`;
    row.append(title, timing); return row;
  }));
  const actions = document.createElement("div"); actions.className = "ocel-evidence-actions";
  const objects = [...new Set(item.evidence.flatMap((value) => value.readiness.map((prior) => prior.object)))];
  actions.replaceChildren(...objects.map((object) => {
    const button = document.createElement("button"); button.type = "button"; button.className = "ocel-evidence-action"; button.textContent = `Inspect ${object}`;
    button.addEventListener("click", () => void inspectOcelObject(object)); return button;
  }));
  const note = document.createElement("p"); note.textContent = report.semantics;
  panel.replaceChildren(label, heading, detail, facts, evidence, actions, note);
}

function renderOcelBottlenecks(report) {
  const lifecycle = document.querySelector("#ocel-lifecycle-bottlenecks");
  const synchronization = document.querySelector("#ocel-synchronization-bottlenecks");
  const status = document.querySelector("#ocel-bottleneck-status");
  if (!lifecycle || !synchronization || !status) return;
  lifecycle.replaceChildren(...report.lifecycle.map((item) => ocelResultButton(
    `#${item.rank} · ${item.source} → ${item.target}`,
    `${item.object_type} · ${formatDuration(item.total_elapsed_micros / 1_000_000)} total · ${sharePercent(item.share_millionths)} of ranked burden`,
    () => renderOcelLifecycleBottleneckEvidence(report, item),
  )));
  if (!report.lifecycle.length) lifecycle.textContent = "No lifecycle transition has positive elapsed burden in this view.";
  synchronization.replaceChildren(...report.synchronization.map((item) => ocelResultButton(
    `#${item.rank} · ${item.activity}`,
    `${formatDuration(item.total_arrival_spread_micros / 1_000_000)} total prior-point spread · ${sharePercent(item.share_millionths)} of ranked spread`,
    () => renderOcelSynchronizationBottleneckEvidence(report, item),
  )));
  if (!report.synchronization.length) synchronization.textContent = report.synchronization_eligible_activities
    ? "Eligible multi-object timing exists, but no activity has positive prior-point spread."
    : "No activity has enough earlier object history for convergence ranking.";
  status.textContent = `${report.lifecycle.length.toLocaleString()} ranked lifecycle ${report.lifecycle.length === 1 ? "burden" : "burdens"} · ${report.synchronization.length.toLocaleString()} ranked convergence ${report.synchronization.length === 1 ? "burden" : "burdens"}.`;
  status.dataset.state = "ready";
}

function projectionSummaryFact(value, label) {
  const item = document.createElement("div");
  const strong = document.createElement("strong"); strong.textContent = value;
  const text = document.createElement("span"); text.textContent = label;
  item.append(strong, text); return item;
}

function renderOcelProjectionEvidence(title, facts, evidence, actions = []) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Projection evidence";
  const heading = document.createElement("h3"); heading.textContent = title;
  const details = document.createElement("dl");
  for (const [name, value] of facts) {
    const term = document.createElement("dt"); term.textContent = name;
    const result = document.createElement("dd"); result.textContent = value;
    details.append(term, result);
  }
  const list = document.createElement("ul"); list.className = "ocel-evidence-list";
  list.replaceChildren(...evidence.map((value) => {
    const item = document.createElement("li"); item.textContent = value; return item;
  }));
  const actionBar = document.createElement("div"); actionBar.className = "ocel-evidence-actions";
  actionBar.replaceChildren(...actions.map((object) => {
    const button = document.createElement("button"); button.type = "button"; button.className = "ocel-evidence-action"; button.textContent = `Inspect ${object}`;
    button.addEventListener("click", () => void inspectOcelObject(object)); return button;
  }));
  const note = document.createElement("p"); note.textContent = "Projection evidence preserves source identifiers. A divergence candidate signals distortion risk, not proof that an observed repetition is artificial.";
  panel.replaceChildren(label, heading, details, list, actionBar, note);
}

function renderOcelProjection(report) {
  const summary = document.querySelector("#ocel-projection-summary");
  const warnings = document.querySelector("#ocel-projection-warnings");
  const cases = document.querySelector("#ocel-projection-cases");
  const status = document.querySelector("#ocel-projection-status");
  if (!summary || !warnings || !cases || !status) return;
  summary.replaceChildren(
    projectionSummaryFact(report.projected_cases.toLocaleString(), `${report.case_object_type} cases`),
    projectionSummaryFact(`${report.included_unique_events.toLocaleString()} of ${report.source_events.toLocaleString()}`, "unique source events included"),
    projectionSummaryFact(report.projected_event_occurrences.toLocaleString(), "projected event occurrences"),
    projectionSummaryFact(report.excluded_events.toLocaleString(), "source events excluded"),
    projectionSummaryFact(report.convergence_additional_occurrences.toLocaleString(), "extra occurrences from convergence"),
    projectionSummaryFact(report.divergence_candidates.toLocaleString(), "divergence-risk candidates"),
  );
  const warningRows = [];
  for (const evidence of report.convergence_evidence) {
    warningRows.push(ocelResultButton(
      `Convergence · ${evidence.activity}`,
      `${evidence.source_event} copied into ${evidence.projected_case_count.toLocaleString()} cases · ${evidence.additional_occurrences.toLocaleString()} extra occurrences`,
      () => renderOcelProjectionEvidence(
        `${evidence.activity} · convergence`,
        [["Source event", evidence.source_event], ["Projected cases", evidence.projected_case_count.toLocaleString()], ["Extra occurrences", evidence.additional_occurrences.toLocaleString()]],
        evidence.evidence_cases.map((value) => `Projected case ${value}`),
        evidence.evidence_cases,
      ),
    ));
  }
  for (const evidence of report.divergence_evidence) {
    warningRows.push(ocelResultButton(
      `Divergence risk · ${evidence.activity}`,
      `${evidence.projected_case} repeats across ${evidence.distinct_related_objects.toLocaleString()} ${evidence.related_object_type} objects`,
      () => renderOcelProjectionEvidence(
        `${evidence.activity} · divergence risk`,
        [["Projected case", evidence.projected_case], ["Projected occurrences", evidence.projected_occurrences.toLocaleString()], ["Distinct related objects", evidence.distinct_related_objects.toLocaleString()], ["Related object type", evidence.related_object_type]],
        evidence.evidence_events.map((value) => `Source event ${value}`),
        [evidence.projected_case, ...evidence.evidence_objects],
      ),
    ));
  }
  warnings.replaceChildren(...warningRows);
  if (!warningRows.length) warnings.textContent = "No convergence or evidence-backed divergence risk was observed for this case notion.";
  cases.replaceChildren(...report.cases.map((item) => {
    const path = item.layers.map((layer) => layer.events.map((event) => event.activity).join(" + ")).join(" → ");
    return ocelResultButton(
      item.case_object,
      `${path || "No related events"}${item.truncated ? " … truncated" : ""}`,
      () => renderOcelProjectionEvidence(
        item.case_object,
        [["Events available", item.events_available.toLocaleString()], ["Events returned", item.events_returned.toLocaleString()], ["Preview truncated", item.truncated ? "Yes" : "No"]],
        item.layers.flatMap((layer) => layer.events.map((event) => `${event.source_event} · ${event.activity} · ${formatOcelTimestamp(layer.at_micros)}`)),
        [item.case_object],
      ),
    );
  }));
  if (!report.cases.length) cases.textContent = "No retained object of this type can become a projected case in the active view.";
  status.textContent = `${report.cases_returned.toLocaleString()} of ${report.cases_available.toLocaleString()} projected cases previewed. Review distortion warnings before using a case-centric model.`;
  status.dataset.state = "ready";
}

function renderOcelModelTransition(report, transition) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const places = new Map(report.places.map((place) => [place.id, place]));
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Formal model element";
  const heading = document.createElement("h3"); heading.textContent = transition.activity || `Silent routing transition τ${transition.id}`;
  const facts = document.createElement("dl");
  const arcs = [...transition.inputs, ...transition.outputs];
  for (const [name, value] of [
    ["Transition", `t${transition.id}`],
    ["Visibility", transition.activity ? "Observable activity" : "Silent routing"],
    ["Input arcs", transition.inputs.length.toLocaleString()],
    ["Output arcs", transition.outputs.length.toLocaleString()],
    ["Multiplicity", arcs.some((arc) => arc.variable) ? "At least one variable arc" : "All arcs fixed"],
  ]) {
    const term = document.createElement("dt"); term.textContent = name;
    const result = document.createElement("dd"); result.textContent = value;
    facts.append(term, result);
  }
  const connections = document.createElement("ul"); connections.className = "ocel-evidence-list";
  const describe = (arc, direction) => {
    const place = places.get(arc.place);
    return `${direction} p${arc.place} · ${place?.object_type || "unknown object type"}${place?.initial ? " · initial" : ""}${place?.final_place ? " · final" : ""}${arc.variable ? " · variable multiplicity" : " · fixed multiplicity"}`;
  };
  connections.replaceChildren(
    ...transition.inputs.map((arc) => { const item = document.createElement("li"); item.textContent = describe(arc, "From"); return item; }),
    ...transition.outputs.map((arc) => { const item = document.createElement("li"); item.textContent = describe(arc, "To"); return item; }),
  );
  const note = document.createElement("p"); note.textContent = report.semantics;
  panel.replaceChildren(label, heading, facts, connections, note);
}

function renderOcelModelType(report, stats) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Object-type model";
  const heading = document.createElement("h3"); heading.textContent = stats.object_type;
  const facts = document.createElement("dl");
  for (const [name, value] of [
    ["Objects in view", stats.objects.toLocaleString()],
    ["Objects with events", stats.objects_with_events.toLocaleString()],
    ["Lifecycle occurrences", stats.lifecycle_event_occurrences.toLocaleString()],
    ["Infrequent edges removed", stats.filtered_edges.toLocaleString()],
    ["Permissive fallback", stats.used_flower_fallback ? "Used — inspect this type carefully" : "Not used"],
  ]) {
    const term = document.createElement("dt"); term.textContent = name;
    const result = document.createElement("dd"); result.textContent = value;
    facts.append(term, result);
  }
  const note = document.createElement("p"); note.textContent = "This lifecycle net contributes typed places to the combined object-centric model. Shared visible activities are merged across object types.";
  panel.replaceChildren(label, heading, facts, note);
}

function renderOcelModel(report) {
  const summary = document.querySelector("#ocel-model-summary");
  const types = document.querySelector("#ocel-model-types");
  const transitions = document.querySelector("#ocel-model-transitions");
  const status = document.querySelector("#ocel-model-status");
  if (!summary || !types || !transitions || !status) return;
  const visible = report.transitions.filter((transition) => transition.activity !== null).length;
  summary.replaceChildren(
    projectionSummaryFact(report.object_types.length.toLocaleString(), "object types modelled"),
    projectionSummaryFact(report.places.length.toLocaleString(), "typed places"),
    projectionSummaryFact(visible.toLocaleString(), "shared visible activities"),
    projectionSummaryFact((report.transitions.length - visible).toLocaleString(), "silent routing transitions"),
    projectionSummaryFact(report.fixed_arcs.toLocaleString(), "fixed arcs"),
    projectionSummaryFact(report.variable_arcs.toLocaleString(), "variable arcs"),
  );
  types.replaceChildren(...report.object_types.map((stats) => ocelResultButton(
    stats.object_type,
    `${stats.objects_with_events.toLocaleString()} of ${stats.objects.toLocaleString()} objects with events${stats.filtered_edges ? ` · ${stats.filtered_edges.toLocaleString()} edges filtered` : ""}${stats.used_flower_fallback ? " · fallback used" : ""}`,
    () => renderOcelModelType(report, stats),
  )));
  if (!report.object_types.length) types.textContent = "No retained object type remains to model.";
  transitions.replaceChildren(...report.transitions.map((transition) => {
    const arcs = [...transition.inputs, ...transition.outputs];
    const placeById = new Map(report.places.map((place) => [place.id, place]));
    const objectTypes = [...new Set(arcs.map((arc) => placeById.get(arc.place)?.object_type).filter(Boolean))];
    return ocelResultButton(
      transition.activity || `τ${transition.id} · silent routing`,
      `${objectTypes.join(" + ") || "routing"} · ${transition.inputs.length.toLocaleString()} in / ${transition.outputs.length.toLocaleString()} out${arcs.some((arc) => arc.variable) ? " · variable arc" : ""}`,
      () => renderOcelModelTransition(report, transition),
    );
  }));
  if (!report.transitions.length) transitions.textContent = "No transition remains in this view.";
  const fallbacks = report.object_types.filter((stats) => stats.used_flower_fallback).length;
  status.textContent = `${report.places.length.toLocaleString()} places and ${report.transitions.length.toLocaleString()} transitions discovered locally${fallbacks ? ` · ${fallbacks.toLocaleString()} object-type ${fallbacks === 1 ? "model uses" : "models use"} a permissive fallback` : " · no permissive fallback used"}.`;
  status.dataset.state = "ready";
}

function ocelDeviationSummary(deviation) {
  switch (deviation.kind) {
    case "activity_not_in_model": return [`${deviation.activity} is outside the baseline`, `${deviation.event} · ${deviation.object}`];
    case "missing_tokens": return [`Missing tokens before ${deviation.activity}`, `${deviation.event} · ${deviation.object} · ${deviation.count.toLocaleString()} token(s)`];
    case "final_tokens_missing": return ["Final lifecycle state was not reached", `${deviation.object} · ${deviation.count.toLocaleString()} token(s) missing`];
    case "tokens_remaining": return ["Tokens remained outside the final state", `${deviation.object} · ${deviation.count.toLocaleString()} token(s)`];
    case "missing_object_participation": return [`${deviation.activity || "Transition"} is missing ${deviation.object_type} participation`, `${deviation.event} · ${deviation.variable ? "variable" : "fixed"} arc`];
    case "fixed_arc_multiplicity": return [`${deviation.activity || "Transition"} violated fixed ${deviation.object_type} multiplicity`, `${deviation.event} · ${deviation.observed_objects.toLocaleString()} objects observed`];
    case "unexpected_object_participation": return [`${deviation.activity || "Transition"} unexpectedly involved ${deviation.object_type}`, `${deviation.event} · ${deviation.observed_objects.toLocaleString()} objects observed`];
    default: return ["Object-centric replay deviation", deviation.kind];
  }
}

function renderOcelConformanceDeviation(report, deviation) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const [title] = ocelDeviationSummary(deviation);
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Conformance deviation";
  const heading = document.createElement("h3"); heading.textContent = title;
  const facts = document.createElement("dl");
  const entries = [
    ["Kind", deviation.kind.replaceAll("_", " ")],
    ["Event", deviation.event],
    ["Activity", deviation.activity],
    ["Object", deviation.object],
    ["Object type", deviation.object_type],
    ["Count", deviation.count?.toLocaleString()],
    ["Observed objects", deviation.observed_objects?.toLocaleString()],
    ["Arc multiplicity", deviation.variable === undefined ? null : (deviation.variable ? "Variable" : "Fixed")],
  ].filter(([, value]) => value !== undefined && value !== null);
  for (const [name, value] of entries) {
    const term = document.createElement("dt"); term.textContent = name;
    const result = document.createElement("dd"); result.textContent = value;
    facts.append(term, result);
  }
  const actions = document.createElement("div"); actions.className = "ocel-evidence-actions";
  if (deviation.object) {
    const inspect = document.createElement("button"); inspect.type = "button"; inspect.className = "ocel-evidence-action"; inspect.textContent = `Inspect ${deviation.object}`;
    inspect.addEventListener("click", () => void inspectOcelObject(deviation.object));
    actions.append(inspect);
  }
  const note = document.createElement("p"); note.textContent = report.semantics;
  panel.replaceChildren(label, heading, facts, actions, note);
}

function renderOcelConformance(report) {
  const summary = document.querySelector("#ocel-conformance-summary");
  const objects = document.querySelector("#ocel-conformance-objects");
  const deviations = document.querySelector("#ocel-conformance-deviations");
  const status = document.querySelector("#ocel-conformance-status");
  const objectsTitle = document.querySelector("#ocel-conformance-objects-title");
  const deviationsTitle = document.querySelector("#ocel-conformance-deviations-title");
  if (!summary || !objects || !deviations || !status) return;
  if (objectsTitle) objectsTitle.textContent = "Worst object lifecycles";
  if (deviationsTitle) deviationsTitle.textContent = "Replay deviations";
  const tokenIssues = report.missing_tokens + report.remaining_tokens + report.unmapped_event_occurrences;
  summary.replaceChildren(
    projectionSummaryFact(sharePercent(report.fitness_millionths), "token fitness"),
    projectionSummaryFact(`${report.lifecycle_fit_objects.toLocaleString()} of ${report.replayed_objects.toLocaleString()}`, "lifecycle-fit objects"),
    projectionSummaryFact(`${report.fit_events.toLocaleString()} of ${report.replayed_events.toLocaleString()}`, "fit source events"),
    projectionSummaryFact(report.binding_violations.toLocaleString(), "binding violations"),
    projectionSummaryFact(tokenIssues.toLocaleString(), "token or mapping issues"),
    projectionSummaryFact(report.out_of_scope_events.toLocaleString(), "events outside active modeled types"),
  );
  objects.replaceChildren(...report.objects.map((item) => ocelResultButton(
    item.object,
    `${item.object_type} · ${sharePercent(item.fitness_millionths)} fitness · ${item.events.toLocaleString()} events${item.lifecycle_fit ? " · lifecycle fit" : ` · ${item.missing_tokens.toLocaleString()} missing / ${item.remaining_tokens.toLocaleString()} remaining tokens`}`,
    () => void inspectOcelObject(item.object),
  )));
  if (!report.objects.length) objects.textContent = "No object lifecycle is in scope for this baseline and filter.";
  if (report.objects_truncated) {
    const note = document.createElement("p"); note.className = "ocel-section-note"; note.textContent = `${(report.objects_available - report.objects_returned).toLocaleString()} additional objects are included in the totals above.`;
    objects.append(note);
  }
  deviations.replaceChildren(...report.deviations.map((deviation) => {
    const [title, detail] = ocelDeviationSummary(deviation);
    return ocelResultButton(title, detail, () => renderOcelConformanceDeviation(report, deviation));
  }));
  if (!report.deviations.length) deviations.textContent = "No replay or shared-event binding deviation was observed.";
  if (report.deviations_truncated) {
    const note = document.createElement("p"); note.className = "ocel-section-note"; note.textContent = `${(report.deviations_available - report.deviations_returned).toLocaleString()} additional deviations are included in the totals above.`;
    deviations.append(note);
  }
  status.textContent = `${report.model_source} checked against ${report.source} · ${report.deviations_available.toLocaleString()} ${report.deviations_available === 1 ? "deviation" : "deviations"}. Lifecycle fitness and shared-event binding are separate checks.`;
  status.dataset.state = "ready";
}

function renderOcelAlignmentDeviation(report, object, deviation) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  showOcelEvidence();
  const observed = deviation.kind === "log_only";
  const label = document.createElement("p"); label.className = "detail-label"; label.textContent = "Optimal lifecycle alignment";
  const heading = document.createElement("h3"); heading.textContent = observed ? `${deviation.activity} was observed only` : `${deviation.activity} was expected by the model`;
  const facts = document.createElement("dl");
  const entries = [
    ["Object", object.object],
    ["Object type", object.object_type],
    ["Move", observed ? "Log only" : "Visible model only"],
    ["Event", deviation.event],
    ["Activity", deviation.activity],
    ["Model transition", deviation.transition?.toLocaleString()],
    ["Object alignment cost", object.cost.toLocaleString()],
  ].filter(([, value]) => value !== undefined && value !== null);
  for (const [name, value] of entries) {
    const term = document.createElement("dt"); term.textContent = name;
    const result = document.createElement("dd"); result.textContent = value;
    facts.append(term, result);
  }
  const actions = document.createElement("div"); actions.className = "ocel-evidence-actions";
  const inspect = document.createElement("button"); inspect.type = "button"; inspect.className = "ocel-evidence-action"; inspect.textContent = `Inspect ${object.object}`;
  inspect.addEventListener("click", () => void inspectOcelObject(object.object));
  actions.append(inspect);
  const note = document.createElement("p"); note.textContent = report.semantics;
  panel.replaceChildren(label, heading, facts, actions, note);
}

function renderOcelAlignment(report) {
  const summary = document.querySelector("#ocel-conformance-summary");
  const objects = document.querySelector("#ocel-conformance-objects");
  const deviations = document.querySelector("#ocel-conformance-deviations");
  const status = document.querySelector("#ocel-conformance-status");
  const objectsTitle = document.querySelector("#ocel-conformance-objects-title");
  const deviationsTitle = document.querySelector("#ocel-conformance-deviations-title");
  if (!summary || !objects || !deviations || !status) return;
  if (objectsTitle) objectsTitle.textContent = "Worst object alignments";
  if (deviationsTitle) deviationsTitle.textContent = "Optimal lifecycle and binding deviations";
  summary.replaceChildren(
    projectionSummaryFact(sharePercent(report.fitness_millionths), "alignment fitness"),
    projectionSummaryFact(`${report.lifecycle_fit_objects.toLocaleString()} of ${report.aligned_objects.toLocaleString()}`, "zero-cost object paths"),
    projectionSummaryFact(`${report.fit_events.toLocaleString()} of ${report.aligned_events.toLocaleString()}`, "fit source events"),
    projectionSummaryFact(report.total_cost.toLocaleString(), "lifecycle alignment cost"),
    projectionSummaryFact(report.binding_violations.toLocaleString(), "separate binding violations"),
    projectionSummaryFact(report.out_of_scope_events.toLocaleString(), "events outside active modeled types"),
  );
  objects.replaceChildren(...report.objects.map((item) => ocelResultButton(
    item.object,
    `${item.object_type} · cost ${item.cost.toLocaleString()} · ${sharePercent(item.fitness_millionths)} fitness · ${item.log_moves.toLocaleString()} observed-only / ${item.visible_model_moves.toLocaleString()} model-only moves`,
    () => void inspectOcelObject(item.object),
  )));
  if (!report.objects.length) objects.textContent = "No object lifecycle is in scope for this baseline and filter.";
  if (report.objects_truncated) {
    const note = document.createElement("p"); note.className = "ocel-section-note"; note.textContent = `${(report.objects_available - report.objects_returned).toLocaleString()} additional objects are included in the totals above.`;
    objects.append(note);
  }
  const rows = [];
  for (const object of report.objects) {
    for (const deviation of object.deviations) {
      const observed = deviation.kind === "log_only";
      rows.push(ocelResultButton(
        observed ? `${deviation.activity} was observed only` : `${deviation.activity} was expected by the model`,
        `${object.object} · ${object.object_type}${deviation.event ? ` · ${deviation.event}` : ""}`,
        () => renderOcelAlignmentDeviation(report, object, deviation),
      ));
    }
  }
  for (const deviation of report.binding_deviations) {
    const [title, detail] = ocelDeviationSummary(deviation);
    rows.push(ocelResultButton(title, detail, () => renderOcelConformanceDeviation(report, deviation)));
  }
  deviations.replaceChildren(...rows);
  if (!rows.length) deviations.textContent = "Every object has a zero-cost lifecycle alignment and no shared-event binding deviation was observed.";
  status.textContent = `${report.model_source} optimally aligned against ${report.source} · total lifecycle cost ${report.total_cost.toLocaleString()} · ${report.binding_violations.toLocaleString()} separate binding violations.`;
  status.dataset.state = "ready";
}

function ocelFlowEvidence(edge) {
  renderOcelEvidence(
    `${edge.source} → ${edge.target}`,
    [["Object type", edge.object_type], ["Occurrences", edge.occurrences.toLocaleString()], ["Objects", edge.objects.toLocaleString()], ["Mean elapsed", formatDuration(edge.mean_elapsed_micros / 1_000_000)], ["Observed range", `${formatDuration(edge.min_elapsed_micros / 1_000_000)} – ${formatDuration(edge.max_elapsed_micros / 1_000_000)}`]],
    edge.evidence.map((item) => `${item.object}: ${item.source_event} → ${item.target_event}`),
  );
}

let ocelFlowMetric = "frequency";
let ocelFlowSelection = null;

// Builds the map-view shape the prototype map draws, for one item type.
// Step counts, starts and ends come from the engine's lifecycle routes for that
// type; connections are the engine's directly-follows edges, most frequent
// first. Simultaneous steps keep their canonical order only for layout.
function ocelMapView(analysis, objectType, limit) {
  const routes = (analysis.variants || []).filter((variant) => variant.object_type === objectType).sort((a, b) => a.rank - b.rank);
  const counts = new Map(); const starts = new Map(); const ends = new Map();
  const add = (map, key, value) => map.set(key, (map.get(key) || 0) + value);
  for (const route of routes) {
    for (const layer of route.layers) for (const activity of layer.activities) add(counts, activity, route.objects);
    for (const activity of route.layers[0]?.activities || []) add(starts, activity, route.objects);
    for (const activity of route.layers.at(-1)?.activities || []) add(ends, activity, route.objects);
  }
  const available = analysis.graphs.directly_follows
    .filter((edge) => edge.object_type === objectType)
    .sort((a, b) => b.occurrences - a.occurrences || a.source.localeCompare(b.source) || a.target.localeCompare(b.target));
  const shown = available.slice(0, limit);
  const top = [...new Set(routes[0]?.layers.flatMap((layer) => layer.activities) || [])];
  const activities = [...new Set([...top, ...shown.flatMap((edge) => [edge.source, edge.target])])];
  const nodes = activities.map((activity) => ({ activity, frequency: counts.get(activity) || 0, starts: starts.get(activity) || 0, ends: ends.get(activity) || 0 }));
  const ranked = [...nodes].sort((a, b) => b.frequency - a.frequency || a.activity.localeCompare(b.activity));
  return {
    objectType, available: available.length, routes,
    top_route: top.length ? top : ranked.slice(0, 1).map((node) => node.activity),
    nodes,
    activity_ranking: ranked.map((node, index) => ({ activity: node.activity, rank: index + 1 })),
    edges: shown.map((edge) => ({
      from: edge.source, to: edge.target, frequency: edge.occurrences, case_frequency: edge.objects,
      duration_eligible_occurrences: edge.occurrences, mean_seconds: Math.round(edge.mean_elapsed_micros / 1_000_000), slow: false, source: edge,
    })),
  };
}

// The item type the map opens on: the one most involved in recorded events
// (event-to-item links, from the engine), rather than whatever sorts first.
// Ties fall back to alphabetical order. The person can still pick any type.
function mainObjectType(analysis, types) {
  const involvement = new Map();
  for (const link of analysis.graphs?.participation || []) {
    involvement.set(link.object_type, (involvement.get(link.object_type) || 0) + link.occurrences);
  }
  return [...types].sort((a, b) => (involvement.get(b) || 0) - (involvement.get(a) || 0) || a.localeCompare(b))[0];
}

function ocelNodeEvidence(view, activity) {
  const node = view.nodes.find((item) => item.activity === activity);
  const events = currentOcelAnalysis.graphs.activities.find((item) => item.activity === activity);
  renderOcelEvidence(activity, [
    ["Item type", view.objectType],
    ["Recorded for this type", (node?.frequency || 0).toLocaleString()],
    ["Items starting here", (node?.starts || 0).toLocaleString()],
    ["Items ending here", (node?.ends || 0).toLocaleString()],
    ["Events across all types", (events?.events || 0).toLocaleString()],
  ], (events?.evidence_events || []).map((event) => `Event ${event}`));
}

function renderOcelFlowMap() {
  const analysis = currentOcelAnalysis;
  const svg = document.querySelector("#ocel-process-map");
  const list = document.querySelector("#ocel-flow-list");
  const typeSelect = document.querySelector("#ocel-flow-object-type");
  const limitSelect = document.querySelector("#ocel-flow-limit");
  const status = document.querySelector("#ocel-flow-status");
  if (!analysis?.graphs || !svg || !list || !typeSelect || !limitSelect || !status) return;

  const objectType = typeSelect.value;
  const view = ocelMapView(analysis, objectType, Number(limitSelect.value) || 50);
  const typeLabel = objectType || "this item type";
  status.textContent = `${view.edges.length.toLocaleString()} of ${view.available.toLocaleString()} connections for ${typeLabel}, most frequent first; layout does not change the analysis.`;
  list.replaceChildren(...view.edges.map((edge) => ocelResultButton(
    `${edge.from} → ${edge.to}`,
    `${edge.frequency.toLocaleString()} ${edge.frequency === 1 ? "transition" : "transitions"} across ${edge.case_frequency.toLocaleString()} ${edge.case_frequency === 1 ? "item" : "items"} · ${formatDuration(edge.mean_seconds)} mean elapsed`,
    () => ocelFlowEvidence(edge.source),
  )));
  if (!view.edges.length) list.textContent = "No lifecycle transitions exist for this item type in the current view.";
  for (const button of document.querySelectorAll("#ocel-flow-metric button")) button.setAttribute("aria-pressed", String(button.dataset.metric === ocelFlowMetric));

  svg.replaceChildren();
  if (!view.nodes.length) {
    svg.setAttribute("viewBox", "0 0 700 200");
    const message = svgElement("text", { x: 350, y: 100, "text-anchor": "middle", fill: "var(--muted)" });
    message.textContent = "No lifecycle steps to draw for this item type";
    svg.append(message);
    requestAnimationFrame(fitProcessMaps);
    return;
  }
  if (!ocelFlowSelection || !view.nodes.some((node) => node.activity === ocelFlowSelection)) ocelFlowSelection = view.top_route[0];
  drawResultsMap(svg, view, {
    duration: ocelFlowMetric === "duration", selection: ocelFlowSelection, prefix: "ocel",
    onNode: (activity) => { ocelFlowSelection = activity; renderOcelFlowMap(); ocelNodeEvidence(view, activity); },
    onEdge: (edge) => ocelFlowEvidence(edge.source),
  });
  const legend = document.querySelector("#ocel-flow-legend");
  if (legend) {
    const items = ocelFlowMetric === "duration"
      ? [["edge", "Connection (elapsed time between steps of the same item)"], ["warn", "Returns to an earlier step"], [null, "Labels show average elapsed time"]]
      : [["edge", "Line width = how often the connection was recorded"], ["warn", "Returns to an earlier step"], [null, "Labels show counts"]];
    legend.replaceChildren(...items.map(([colour, text]) => {
      const item = document.createElement("span");
      if (colour) { const swatch = document.createElement("i"); swatch.style.background = `var(--${colour})`; item.append(swatch); }
      item.append(text);
      return item;
    }));
  }
  requestAnimationFrame(fitProcessMaps);
}

function renderOcelAnalysis(payload) {
  showView("report");
  currentOcelUpload = payload.upload_id;
  currentOcelAnalysis = payload.analysis;
  currentAnalysis = null;
  currentDiscovery = null;
  currentCaseBaseline = null;
  const caseBaselineName = document.querySelector("#case-baseline-name");
  if (caseBaselineName) caseBaselineName.textContent = "No expected process selected";
  const caseCheck = document.querySelector("#check-case-conformance");
  if (caseCheck) caseCheck.disabled = true;
  const caseDownload = document.querySelector("#download-case-model");
  if (caseDownload) caseDownload.disabled = true;
  clearCaseConformanceResults();
  const analysis = payload.analysis;
  const panel = document.querySelector("#ocel-panel");
  const coverage = document.querySelector("#ocel-coverage");
  const summary = document.querySelector("#ocel-summary");
  const quality = document.querySelector("#ocel-quality");
  const results = document.querySelector("#ocel-results");
  if (!panel || !coverage || !summary || !quality || !results) return;

  document.querySelector("#profile-panel").hidden = true;
  document.querySelector("#quality-panel").hidden = true;
  renderOcelKpis(analysis);
  const crumb = document.querySelector("#results-source");
  if (crumb) crumb.textContent = `${(analysis.source || "").split(/[\\/]/).pop()} · ${analysis.scope.events_shown.toLocaleString()} updates`;
  coverage.textContent = `${analysis.scope.events_shown.toLocaleString()} of ${analysis.scope.events_total.toLocaleString()} events · ${analysis.scope.objects_shown.toLocaleString()} of ${analysis.scope.objects_total.toLocaleString()} objects${analysis.scope.narrowed ? " · filtered view" : ""}`;
  summary.replaceChildren(
    ocelSummaryCard(analysis.scope.events_shown, "Recorded updates", "Each source update is counted once"),
    ocelSummaryCard(analysis.scope.objects_shown, "Connected items", `${analysis.catalog.object_types.length.toLocaleString()} item types`),
    ocelSummaryCard(analysis.scope.event_object_relations_shown, "Update–item links", "Items involved in recorded updates"),
    ocelSummaryCard(analysis.scope.object_object_relations_shown, "Item relationships", "Links recorded between items"),
  );
  populateOcelSelect("#ocel-object-types", analysis.catalog.object_types, analysis.filter.object_types);
  populateOcelSelect("#ocel-e2o-qualifiers", analysis.catalog.event_object_qualifiers, analysis.filter.event_object_qualifiers);
  populateOcelSelect("#ocel-o2o-qualifiers", analysis.catalog.object_object_qualifiers, analysis.filter.object_object_qualifiers);
  populateOcelActivitySelect("#ocel-time-from", analysis.catalog.event_types, 0);
  populateOcelActivitySelect("#ocel-time-to", analysis.catalog.event_types, 1);
  populateOcelActivitySelect("#ocel-projection-type", analysis.catalog.object_types, 0);
  const timeResults = document.querySelector("#ocel-time-results");
  if (timeResults) timeResults.replaceChildren();
  const timeStatus = document.querySelector("#ocel-time-status");
  if (timeStatus) { timeStatus.textContent = ""; timeStatus.dataset.state = ""; }
  const synchronizationResults = document.querySelector("#ocel-synchronization-results");
  if (synchronizationResults) synchronizationResults.replaceChildren();
  const synchronizationStatus = document.querySelector("#ocel-synchronization-status");
  if (synchronizationStatus) { synchronizationStatus.textContent = ""; synchronizationStatus.dataset.state = ""; }
  for (const selector of ["#ocel-lifecycle-bottlenecks", "#ocel-synchronization-bottlenecks"]) {
    const list = document.querySelector(selector); if (list) list.replaceChildren();
  }
  const bottleneckStatus = document.querySelector("#ocel-bottleneck-status");
  if (bottleneckStatus) { bottleneckStatus.textContent = ""; bottleneckStatus.dataset.state = ""; }
  for (const selector of ["#ocel-projection-summary", "#ocel-projection-warnings", "#ocel-projection-cases"]) {
    const area = document.querySelector(selector); if (area) area.replaceChildren();
  }
  const projectionStatus = document.querySelector("#ocel-projection-status");
  if (projectionStatus) { projectionStatus.textContent = ""; projectionStatus.dataset.state = ""; }
  for (const selector of ["#ocel-model-summary", "#ocel-model-types", "#ocel-model-transitions"]) {
    const area = document.querySelector(selector); if (area) area.replaceChildren();
  }
  currentOcelDiscoveredModel = null;
  const downloadModel = document.querySelector("#download-ocel-model");
  if (downloadModel) downloadModel.disabled = true;
  const modelStatus = document.querySelector("#ocel-model-status");
  if (modelStatus) { modelStatus.textContent = ""; modelStatus.dataset.state = ""; }
  for (const selector of ["#ocel-conformance-summary", "#ocel-conformance-objects", "#ocel-conformance-deviations"]) {
    const area = document.querySelector(selector); if (area) area.replaceChildren();
  }
  const conformanceStatus = document.querySelector("#ocel-conformance-status");
  if (conformanceStatus) { conformanceStatus.textContent = ""; conformanceStatus.dataset.state = ""; }
  const checkConformance = document.querySelector("#check-ocel-conformance");
  if (checkConformance) checkConformance.disabled = !currentOcelBaseline;
  const filterCount = analysis.filter.object_types.length + analysis.filter.event_object_qualifiers.length + analysis.filter.object_object_qualifiers.length;
  document.querySelector("#ocel-filter-count").textContent = filterCount ? `${filterCount.toLocaleString()} selections applied` : "All object types and relationships";

  const qualityTitle = document.createElement("strong");
  qualityTitle.textContent = analysis.ready_for_analysis ? "Ready for analysis" : "Analysis blocked by data quality";
  const qualityDetail = document.createElement("span");
  qualityDetail.textContent = `${analysis.quality.summary.errors.toLocaleString()} errors · ${analysis.quality.summary.warnings.toLocaleString()} warnings · ${analysis.quality.summary.information.toLocaleString()} information`;
  const qualityFindings = document.createElement("ul");
  qualityFindings.replaceChildren(...analysis.quality.findings.map((finding) => {
    const item = document.createElement("li"); item.textContent = `${finding.title}: ${finding.detail} (${finding.affected.toLocaleString()})`; return item;
  }));
  quality.replaceChildren(qualityTitle, qualityDetail, qualityFindings);
  results.hidden = !analysis.ready_for_analysis;
  resetOcelEvidence();

  if (analysis.graphs) {
    const flowTypes = [...new Set(analysis.graphs.directly_follows.map((edge) => edge.object_type))].sort((a, b) => a.localeCompare(b));
    const listed = flowTypes.length ? flowTypes : analysis.catalog.object_types;
    populateOcelActivitySelect("#ocel-flow-object-type", listed, Math.max(0, listed.indexOf(mainObjectType(analysis, listed))));
    renderOcelFlowMap();

    const interactions = document.querySelector("#ocel-interactions");
    interactions.replaceChildren(...analysis.graphs.interactions.map((edge) => ocelResultButton(
      `${edge.left_object_type} ↔ ${edge.right_object_type}`,
      `${edge.events.toLocaleString()} shared ${edge.events === 1 ? "event" : "events"} · ${edge.object_pairs.toLocaleString()} object ${edge.object_pairs === 1 ? "pair" : "pairs"}`,
      () => renderOcelEvidence(
        `${edge.left_object_type} ↔ ${edge.right_object_type}`,
        [["Shared events", edge.events.toLocaleString()], ["Object pairs", edge.object_pairs.toLocaleString()]],
        edge.evidence_events.map((event) => `Event ${event}`),
      ),
    )));
    if (!analysis.graphs.interactions.length) interactions.textContent = "No cross-object interactions exist in this view.";

    const participation = document.querySelector("#ocel-participation");
    participation.replaceChildren(...analysis.graphs.participation.map((edge) => ocelResultButton(
      `${edge.activity} · ${edge.object_type}`,
      `${edge.qualifier} · ${edge.occurrences.toLocaleString()} links`,
      () => renderOcelEvidence(
        edge.activity,
        [["Object type", edge.object_type], ["Role", edge.qualifier], ["Occurrences", edge.occurrences.toLocaleString()]],
        edge.evidence_events.map((event) => `Event ${event}`),
      ),
    )));
    if (!analysis.graphs.participation.length) participation.textContent = "No qualified participation exists in this view.";

    const relationships = document.querySelector("#ocel-relationships");
    relationships.replaceChildren(...analysis.graphs.relationships.map((edge) => ocelResultButton(
      `${edge.source_object_type} → ${edge.target_object_type}`,
      `${edge.qualifier} · ${edge.occurrences.toLocaleString()} links`,
      () => renderOcelEvidence(
        `${edge.source_object_type} → ${edge.target_object_type}`,
        [["Role", edge.qualifier], ["Occurrences", edge.occurrences.toLocaleString()]],
        edge.evidence.map((item) => `${item.source_object} → ${item.target_object}`),
      ),
    )));
    if (!analysis.graphs.relationships.length) relationships.textContent = "No explicit object relationships exist in this view.";
  }

  const variants = document.querySelector("#ocel-variants");
  if (variants && analysis.variants) {
    variants.replaceChildren(...analysis.variants.map((variant) => ocelResultButton(
      ocelVariantPath(variant),
      `${variant.object_type} · rank ${variant.rank} · ${variant.objects.toLocaleString()} ${variant.objects === 1 ? "object" : "objects"} · ${(variant.share_millionths / 10000).toLocaleString(undefined, { maximumFractionDigits: 2 })}%`,
      () => renderOcelVariantEvidence(variant),
    )));
    if (!analysis.variants.length) variants.textContent = "No object lifecycle variants exist in this view.";
    const omitted = Number(analysis.variants_omitted || 0);
    if (omitted) {
      const note = document.createElement("p");
      note.textContent = `${omitted.toLocaleString()} rarer ${omitted === 1 ? "variant is" : "variants are"} not listed here; follow an item type to see all its variants.`;
      variants.append(note);
    }
  }

  // A connected project uses the single-log screens: follow one item type.
  if (document.body) {
    const following=startFollowing(analysis);
    window.scrollTo({ top: 0, behavior: "instant" });
    return following;
  }
}

async function analyzeOcelFile(file) {
  if (launchToken === null) {
    setUploadStatus("The local service is not connected.", "error");
    return false;
  }
  if (file.size === 0) {
    setUploadStatus("The selected file is empty.", "error");
    return false;
  }
  setUploadStatus(`Reading and analyzing ${file.name} locally…`);
  const choose = document.querySelector("#choose-log");
  if (choose) choose.disabled = true;
  try {
    const response = await projectFetch("/api/ocel", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${launchToken}`,
        "Content-Type": "application/octet-stream",
        "X-File-Name": encodeURIComponent(file.name),
      },
      body: file,
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `object-centric analysis returned ${response.status}`);
    pendingProject = null;
    renderOcelAnalysis(payload);
    setUploadStatus(`Analyzed ${file.name} locally without flattening events into cases.`, payload.analysis.ready_for_analysis ? "ready" : "error");
    return true;
  } catch (error) {
    setUploadStatus(error instanceof Error ? error.message : "The object-centric log could not be analyzed.", "error");
    return false;
  } finally {
    if (choose && launchToken !== null) choose.disabled = false;
  }
}

function ocelFilterFromControls() {
  return {
    object_types: selectedValues(document.querySelector("#ocel-object-types")),
    event_object_qualifiers: selectedValues(document.querySelector("#ocel-e2o-qualifiers")),
    object_object_qualifiers: selectedValues(document.querySelector("#ocel-o2o-qualifiers")),
  };
}

async function applyOcelFilter(filter) {
  const status = document.querySelector("#ocel-filter-status");
  const button = document.querySelector("#apply-ocel-filter");
  if (!status || !button || !currentOcelUpload) return;
  status.dataset.state = "";
  status.textContent = "Recalculating the object view locally…";
  button.disabled = true;
  try {
    const response = await projectFetch("/api/ocel/analyze", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ upload_id: currentOcelUpload, filter }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `object-centric analysis returned ${response.status}`);
    await renderOcelAnalysis(payload);
    const eventCount = payload.analysis.scope.events_shown;
    const objectCount = payload.analysis.scope.objects_shown;
    status.textContent = `${eventCount.toLocaleString()} ${eventCount === 1 ? "event" : "events"} and ${objectCount.toLocaleString()} ${objectCount === 1 ? "object" : "objects"} in this view.`;
    status.dataset.state = "ready";
    return true;
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : "The object view could not be applied.";
    status.dataset.state = "error";
    return false;
  } finally {
    button.disabled = false;
  }
}

async function measureOcelTimeBetween() {
  const from = document.querySelector("#ocel-time-from")?.value;
  const to = document.querySelector("#ocel-time-to")?.value;
  const status = document.querySelector("#ocel-time-status");
  const button = document.querySelector("#measure-ocel-time");
  if (!from || !to || !status || !button || !currentOcelUpload || !currentOcelAnalysis) return;
  status.dataset.state = "";
  status.textContent = "Measuring common-object paths locally…";
  button.disabled = true;
  try {
    const response = await projectFetch("/api/ocel/time-between", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ upload_id: currentOcelUpload, from_activity: from, to_activity: to, filter: currentOcelAnalysis.filter }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `object-centric timing returned ${response.status}`);
    renderOcelTimeBetween(payload);
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : "The elapsed time could not be measured.";
    status.dataset.state = "error";
  } finally {
    button.disabled = false;
  }
}

async function analyzeOcelSynchronization() {
  const status = document.querySelector("#ocel-synchronization-status");
  const button = document.querySelector("#analyze-ocel-synchronization");
  if (!status || !button || !currentOcelUpload || !currentOcelAnalysis) return;
  status.dataset.state = "";
  status.textContent = "Analyzing multi-object timing locally…";
  button.disabled = true;
  try {
    const response = await projectFetch("/api/ocel/synchronization", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ upload_id: currentOcelUpload, filter: currentOcelAnalysis.filter }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `object synchronization returned ${response.status}`);
    renderOcelSynchronization(payload);
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : "Synchronization timing could not be analyzed.";
    status.dataset.state = "error";
  } finally {
    button.disabled = false;
  }
}

async function rankOcelBottlenecks() {
  const status = document.querySelector("#ocel-bottleneck-status");
  const button = document.querySelector("#rank-ocel-bottlenecks");
  if (!status || !button || !currentOcelUpload || !currentOcelAnalysis) return;
  status.dataset.state = "";
  status.textContent = "Ranking object-centric timing burdens locally…";
  button.disabled = true;
  try {
    const response = await projectFetch("/api/ocel/bottlenecks", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({ upload_id: currentOcelUpload, filter: currentOcelAnalysis.filter }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `object bottleneck ranking returned ${response.status}`);
    renderOcelBottlenecks(payload);
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : "Object-centric bottlenecks could not be ranked.";
    status.dataset.state = "error";
  } finally {
    button.disabled = false;
  }
}

async function discoverOcelModel() {
  const noiseThreshold = Number(document.querySelector("#ocel-model-noise")?.value);
  const variableArcThreshold = Number(document.querySelector("#ocel-model-variable")?.value);
  const status = document.querySelector("#ocel-model-status");
  const button = document.querySelector("#discover-ocel-model");
  if (!status || !button || !currentOcelUpload || !currentOcelAnalysis) return;
  status.dataset.state = "";
  status.textContent = "Discovering the native object-centric model locally…";
  button.disabled = true;
  try {
    const response = await projectFetch("/api/ocel/discover", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({
        upload_id: currentOcelUpload,
        filter: currentOcelAnalysis.filter,
        noise_threshold_milli: noiseThreshold,
        variable_arc_threshold_milli: variableArcThreshold,
      }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `object-centric discovery returned ${response.status}`);
    currentOcelDiscoveredModel = payload;
    const download = document.querySelector("#download-ocel-model");
    if (download) download.disabled = false;
    renderOcelModel(payload);
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : "The object-centric model could not be discovered.";
    status.dataset.state = "error";
  } finally {
    button.disabled = false;
  }
}

function downloadOcelModel() {
  if (!currentOcelDiscoveredModel) return;
  const source = currentOcelDiscoveredModel.source || "object-centric-baseline";
  const stem = source.replace(/\.(jsonocel|xmlocel|xml|sqlite|sqlite3)$/i, "").replace(/[^a-z0-9._-]+/gi, "-") || "object-centric-baseline";
  const blob = new Blob([`${JSON.stringify(currentOcelDiscoveredModel, null, 2)}\n`], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `${stem}.ocelmodel.json`;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 0);
  const status = document.querySelector("#ocel-model-status");
  if (status) {
    status.textContent = `Saved ${link.download}. Use it later as an explicit reference or approved baseline.`;
    status.dataset.state = "ready";
  }
}

async function uploadOcelBaseline(file) {
  const status = document.querySelector("#ocel-conformance-status");
  const name = document.querySelector("#ocel-baseline-name");
  const check = document.querySelector("#check-ocel-conformance");
  if (!status || !name || !check || launchToken === null) return;
  if (!/\.ocelmodel\.json$/i.test(file.name)) {
    status.textContent = "Choose a portable .ocelmodel.json baseline saved by glasstarn.";
    status.dataset.state = "error";
    return;
  }
  if (file.size === 0) {
    status.textContent = "The selected baseline is empty.";
    status.dataset.state = "error";
    return;
  }
  status.textContent = `Validating ${file.name} locally…`;
  status.dataset.state = "";
  check.disabled = true;
  try {
    const response = await projectFetch("/api/ocel/model", {
      method: "POST",
      headers: {
        Authorization: `Bearer ${launchToken}`,
        "Content-Type": "application/octet-stream",
        "X-File-Name": encodeURIComponent(file.name),
      },
      body: file,
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `baseline upload returned ${response.status}`);
    currentOcelBaseline = payload;
    name.textContent = `${payload.file_name} · ${payload.object_types.toLocaleString()} types · ${payload.places.toLocaleString()} places · ${payload.transitions.toLocaleString()} transitions`;
    status.textContent = "Baseline ready. Run conformance against the current filtered object view.";
    status.dataset.state = "ready";
    for (const selector of ["#ocel-conformance-summary", "#ocel-conformance-objects", "#ocel-conformance-deviations"]) {
      const area = document.querySelector(selector); if (area) area.replaceChildren();
    }
  } catch (error) {
    currentOcelBaseline = null;
    name.textContent = "No expected process selected";
    status.textContent = error instanceof Error ? error.message : "The baseline could not be read.";
    status.dataset.state = "error";
  } finally {
    check.disabled = !currentOcelBaseline || !currentOcelUpload || !currentOcelAnalysis;
  }
}

async function checkOcelConformance() {
  const limit = Number(document.querySelector("#ocel-conformance-limit")?.value);
  const algorithm = document.querySelector("#ocel-conformance-algorithm")?.value || "token";
  const status = document.querySelector("#ocel-conformance-status");
  const button = document.querySelector("#check-ocel-conformance");
  if (!status || !button || !currentOcelUpload || !currentOcelAnalysis || !currentOcelBaseline) return;
  status.textContent = `Checking the active object view against ${currentOcelBaseline.file_name} locally…`;
  status.dataset.state = "";
  button.disabled = true;
  try {
    const response = await projectFetch("/api/ocel/conformance", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({
        upload_id: currentOcelUpload,
        model_id: currentOcelBaseline.model_id,
        filter: currentOcelAnalysis.filter,
        detail_limit: limit,
        algorithm,
      }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `object conformance returned ${response.status}`);
    if (algorithm === "alignment") renderOcelAlignment(payload);
    else renderOcelConformance(payload);
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : "Object-centric conformance could not be checked.";
    status.dataset.state = "error";
  } finally {
    button.disabled = false;
  }
}

async function projectOcelCases() {
  const caseObjectType = document.querySelector("#ocel-projection-type")?.value;
  const caseLimit = Number(document.querySelector("#ocel-projection-case-limit")?.value);
  const eventLimit = Number(document.querySelector("#ocel-projection-event-limit")?.value);
  const status = document.querySelector("#ocel-projection-status");
  const button = document.querySelector("#project-ocel-cases");
  if (!caseObjectType || !status || !button || !currentOcelUpload || !currentOcelAnalysis) return;
  status.dataset.state = "";
  status.textContent = `Projecting ${caseObjectType} objects into cases locally…`;
  button.disabled = true;
  try {
    const response = await projectFetch("/api/ocel/project", {
      method: "POST",
      headers: { Authorization: `Bearer ${launchToken}`, "Content-Type": "application/json" },
      body: JSON.stringify({
        upload_id: currentOcelUpload,
        case_object_type: caseObjectType,
        filter: currentOcelAnalysis.filter,
        case_limit: caseLimit,
        event_limit_per_case: eventLimit,
      }),
      cache: "no-store",
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `object projection returned ${response.status}`);
    renderOcelProjection(payload);
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : "The object-centric view could not be projected.";
    status.dataset.state = "error";
  } finally {
    button.disabled = false;
  }
}
