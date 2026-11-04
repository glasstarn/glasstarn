// Your answer: the saved investigation briefing and its executive summary.

function enableNavigation(id) {
  const navigation = document.querySelector(id);
  navigation?.classList.remove("unavailable");
  navigation?.removeAttribute("aria-disabled");
  navigation?.removeAttribute("tabindex");
}

function renderAnalysisProvenance() { if (typeof loadProvenance === "function" && currentWorkspace) void loadProvenance(); }

function renderAnalysis(analysis, filter = {}) {
  currentAnalysis = analysis;
  if (typeof caseDetailRevision !== "undefined") caseDetailRevision++;
  const itemDetail = document.querySelector("#case-detail"); if (itemDetail) itemDetail.textContent = "Choose an item in this selection to inspect its recorded history.";
  if (typeof lastValueSummary !== "undefined") lastValueSummary = null;
  renderAnalysisProvenance();
  currentDiscovery = null;
  const downloadModel = document.querySelector("#download-case-model");
  if (downloadModel) downloadModel.disabled = true;
  currentFilter = filter;
  if(typeof preservedAnalysisFilters!=="undefined")preservedAnalysisFilters={attribute_in:filter.attribute_in || {},attribute_missing:filter.attribute_missing || [],segment:filter.segment || null};
  clearCaseConformanceResults(currentCaseBaseline ? "The active view changed. Run conformance again to refresh every total and deviation." : "");
  const conformanceButton = document.querySelector("#check-case-conformance");
  if (conformanceButton) conformanceButton.disabled = !currentCaseBaseline;
  currentMapView = null;
  if (typeof resultsRouteFocus !== "undefined") resultsRouteFocus = null;
  if (typeof currentFollow !== "undefined" && currentFollow && typeof loadJourney === "function") connectedScopeRequest = loadJourney(currentFollow.type);
  currentMapSelection = { mode: "limits", activities: 0, connections: 0 };
  if (typeof resetExplorers === "function") resetExplorers();
  void loadMapView();
  populateFilterControls(analysis);
  const count = activeFilterCount({
    with_activities: [], without_activities: [], starts_with: null, ends_with: null,
    directly_follows: [], eventually_follows: [], attribute_equals: [],
    started_after: null, started_before: null, min_duration: null, max_duration: null,
    top_variants: null, min_events: null, complete_only: false,
    ...filter,
  });
  if (typeof renderFilterChips === "function") { renderFilterChips(filter); closeFilterPanel(); }
  const filterCount = document.querySelector("#filter-count");
  if (filterCount) {
    filterCount.textContent = count
      ? `${count.toLocaleString()} ${count === 1 ? "condition" : "conditions"} · ${analysis.view.percent_shown.toLocaleString(undefined, { maximumFractionDigits: 1 })}% of cases shown`
      : "No filters applied";
  }
  for (const id of ["#report-nav", "#process-map-nav", "#variants-nav", "#cases-nav", "#timing-nav", "#comparison-nav", "#conformance-nav"]) enableNavigation(id);
  currentInvestigationRecord = null;
  if (typeof renderConsultantViews === "function") renderConsultantViews(null);
  if (document.body) void refreshExecutiveReport();
  const view = document.body?.dataset.view;
  showView(["report", "map", "variants", "cases", "timing", "compare", "conformance"].includes(view) ? view : "report");
  renderResults();
  renderMap();
  if(typeof studioOpen!=="undefined" && studioOpen && !studioPicking)void refreshMapAnalysis();
  void loadComparisonOptions();
  void loadValues(analysis);
  window.scrollTo?.({ top: 0, behavior: "instant" });
}

// Standard labels for saved findings. Observations citing a group or period
// comparison are associations; other observations are calculated facts; a
// hypothesis stays a hypothesis until evidence settles it.
function findingLabel(finding, record) {
  if (finding.kind !== 'observed') return { text: 'Hypothesis', tone: 'hypothesis' };
  const types = (finding.evidence || []).map(ref => record?.supporting_evidence?.[ref]?.request?.type);
  return types.some(type => type === 'cohorts' || type === 'periods')
    ? { text: 'Statistical association', tone: 'association' }
    : { text: 'Calculated fact', tone: 'fact' };
}

// Saved agent interpretations stay structured text and never execute as HTML;
// citations resolve only to this record's embedded evidence.
let reportEvidence = {};
let reportRevision = 0;

function reportElement(tag, text, className) {
  const element = document.createElement(tag);
  if (text !== undefined) element.textContent = String(text);
  if (className) element.className = className;
  return element;
}

function readableReportTime(text) {
  return String(text).replace(/\b(\d+(?:\.\d+)?)d\b/g, '$1 days').replace(/\b1 days\b/g, '1 day');
}

// The Conformance page shows for an audit, and for any project where a check
// against an expected process has been run, however it started.
function auditRequested(project) {
  return project?.purpose === 'audit'
    || (typeof currentCaseBaseline !== 'undefined' && Boolean(currentCaseBaseline))
    || (typeof currentOcelBaseline !== 'undefined' && Boolean(currentOcelBaseline));
}

function analysisIsObjectHistory(analysis = currentAnalysis) {
  return Boolean(analysis?.mapping?.object_history
    || (analysis === currentAnalysis && typeof currentFollow !== "undefined" && currentFollow && currentOcelAnalysis));
}

// Presentation wording changes labels only; the saved narrative and query snapshots
// remain intact. No new process calculations or recommendations are made here.
function businessReportText(text) {
  return String(text).replace(/\bevents\b/gi, 'recorded updates').replace(/\bcohorts\b/gi, 'groups').replace(/\bduration eligible\b/gi, 'included in the timing comparison');
}

function findingSummary(text) {
  const sentence = String(text).match(/^.*?[.!?](?=\s|$)/)?.[0];
  return businessReportText(sentence || text);
}

// The engine's ranked facts for the view in front of the person: the case
// analysis, plus journey drop-outs when following connected data.
function engineFindings(analysis) {
  const labels = itemLabels();
  const plural = labels.plural.toLowerCase();
  const fill = (text) => String(text || "").replace(/\{items\}/g, plural).replace(/\{item\}/g, labels.singular)
    .replace(/\{type:([^}]*)\}/g, (_match, type) => itemTypeName(type).toLowerCase());
  const order = { high: 0, medium: 1, low: 2, good: 3 };
  const emphasis = processProfile(analysis)?.emphasis || [];
  const weight = (kind) => { const at = emphasis.indexOf(kind); return at === -1 ? emphasis.length : at; };
  const journey = typeof currentFollow !== "undefined" && currentFollow && typeof currentJourney !== "undefined" ? currentJourney?.findings || [] : [];
  return [...(analysisIsObjectHistory(analysis) ? [] : analysis?.key_findings || []), ...journey]
    .sort((a, b) => order[a.severity] - order[b.severity] || weight(a.kind) - weight(b.kind) || b.reach_percent - a.reach_percent)
    .map((finding) => ({ ...finding, title: fill(finding.title), detail: fill(finding.detail) }));
}

// The kind of process: the agent's read when it saved one, else the engine's
// first read from the step names.
function processProfile(analysis = currentAnalysis) {
  if (analysisIsObjectHistory(analysis)) return null;
  return (typeof lastReport !== "undefined" && lastReport?.profile) || analysis?.process || null;
}

function capitalised(text) {
  const value = String(text || "");
  return value.charAt(0).toUpperCase() + value.slice(1);
}

// One finding as an expandable row: a label, the finding and its size show
// closed; the explanation and the link to its evidence show open.
function findingCard({ tone, title, detail, tags, onOpen, openLabel, open, extra, impact }) {
  const card = reportElement("details", undefined, "summary-card");
  if (open) card.open = true;
  const head = reportElement("summary");
  const [badge, ...rest] = tags || [];
  head.append(reportElement("span", badge?.text || "", `badge ${badge?.tone || tone || "n"}`));
  head.append(reportElement("span", capitalised(title), "summary-card-title"));
  head.append(reportElement("span", impact || "", "summary-card-impact"));
  head.append(reportElement("span", undefined, "chevron"));
  const body = reportElement("div", undefined, "summary-card-body");
  if (detail) body.append(reportElement("p", detail));
  if (extra) body.append(extra);
  if (rest.length) {
    const pills = reportElement("div", undefined, "pills");
    for (const tag of rest) pills.append(reportElement("span", tag.text, `pill ${tag.tone || "n"}`));
    body.append(pills);
  }
  if (onOpen) {
    const link = reportElement("button", openLabel || "See evidence →", "text-action finding-open");
    link.type = "button";
    link.addEventListener("click", onOpen);
    body.append(link);
  }
  card.append(head, body);
  return card;
}

// The money a finding touches, in a sentence: the amount column summed over
// its items. Straight from the data, no assumptions.
function findingValueText(fact) {
  if (!fact?.value) return "";
  const items = itemLabels().plural.toLowerCase();
  return `The ${fact.value.items.toLocaleString()} ${items} involved carry ${moneyText(fact.value.total, fact.value.field, false)} in ${fact.value.field}.`;
}

const SEVERITY_TONE = { high: "bad", medium: "warn", low: "n", good: "ok" };
const SEVERITY_TAG = { bottleneck: "Bottleneck", drop_out: "Drop-out", rework: "Repeated steps", variation: "Variation", slow_tail: "Slow end", open: "Still open", working_well: "Working well", driver: "Driver", skipped_step: "Skipped step" };
// The engine's own label when it is more precise than the kind's.
const ENGINE_TAGS = new Set(["Longest wait", "Slow step", "Skipped step"]);
function findingTag(fact) {
  return ENGINE_TAGS.has(fact.tags?.[0]) ? fact.tags[0] : SEVERITY_TAG[fact.kind] || "Finding";
}

// Money with its currency when the column names one ("Claim Amount (USD)"),
// otherwise with the column's name, so a bare 4.5M is never shown.
function moneyText(total, field, named = true) {
  const amount = formatAmount(total);
  const name = String(field || "");
  if (/\busd\b|\$|dollar/i.test(name)) return `$${amount}`;
  if (/\beur\b|€|euro/i.test(name)) return `€${amount}`;
  if (/\bgbp\b|£|pound/i.test(name)) return `£${amount}`;
  return named ? `${amount} ${name.replace(/\s*\([^)]*\)\s*$/, "").replace(/_/g, " ").toLowerCase()}`.trim() : amount;
}

// Questions worth asking next: the agent's own, or ones drawn from what the
// engine found, worded as questions for the agent.
function askNextQuestions(note, facts) {
  if (note?.next_questions?.length) return note.next_questions.slice(0, 3);
  const items = itemLabels().plural.toLowerCase();
  const asks = [];
  for (const fact of facts) {
    const tags = fact.tags || [];
    if (fact.kind === 'bottleneck' && tags[1]) asks.push(`Why does ${tags[1]} take so long?`);
    if (fact.kind === 'drop_out' && tags[1]) asks.push(`Which items stopped at ${tags[1]}, and are they still in progress?`);
    if (fact.kind === 'rework' && tags[1]) asks.push(`Why do ${items} repeat “${tags[1]}”?`);
    if (fact.kind === 'variation') asks.push(`What makes ${items} leave the main variants?`);
    if (fact.kind === 'slow_tail') asks.push(`What do the slowest ${items} have in common?`);
  }
  return [...new Set(asks)].slice(0, 3);
}

function renderAskNext(note, facts) {
  const panel = document.querySelector('#ask-next');
  const list = document.querySelector('#ask-next-list');
  if (!panel || !list) return;
  const asks = askNextQuestions(note, facts);
  panel.hidden = !asks.length;
  list.replaceChildren(...asks.map((question) => reportElement('li', question)));
}

function renderInvestigationAnswer(record, built, analysis, history = []) {
  const container = document.querySelector('#investigation-answer');
  if (!container) return;
  container.replaceChildren();
  const note = record?.note;
  const facts = engineFindings(analysis);
  const cards = reportElement('section', undefined, 'summary-cards');
  const scope = analysis
    ? `Based on ${analysis.summary.cases.toLocaleString()} ${caseNoun(analysis.summary.cases)} and ${analysis.summary.events.toLocaleString()} recorded ${analysis.summary.events === 1 ? "step" : "steps"}.`
    : '';
  const heading = reportElement('div', undefined, 'summary-heading');
  const profile = processProfile(analysis);
  const ranked = 'Most important first';
  heading.append(reportElement('h2', 'What stands out'), reportElement('span', scope ? `${ranked} · ${scope.charAt(0).toLowerCase()}${scope.slice(1)}` : ranked));
  if (profile) {
    heading.append(reportElement('span', profile.source === 'agent'
      ? `${profile.label}, as read by your agent`
      : `Looks like ${profile.label}, from the step names`, 'process-read'));
  }
  cards.append(heading);
  const list = reportElement('div', undefined, 'summary-list');
  // Every finding starts closed; a click opens the one you want. `first`
  // stays true until a card is added, for the empty message below.
  let first = true;
  const take = () => { first = false; return false; };

  // The agent's answer to the latest question leads, as a card like the rest.
  if (note?.answer) {
    const asked = note.question && !/^health check$/i.test(note.question.trim());
    const extra = reportElement('div', undefined, 'summary-card-extra');
    if (asked) extra.append(reportElement('p', `Your question: “${note.question}”`, 'summary-question'));
    if (note.process) extra.append(reportElement('p', note.process, 'summary-process'));
    if (note.next_questions?.length) {
      extra.append(reportElement('p', 'Worth asking next', 'answer-label'));
      const list = reportElement('ul');
      note.next_questions.forEach((question) => list.append(reportElement('li', question)));
      extra.append(list);
    }
    list.append(findingCard({ tone: 'agent', title: note.answer, tags: [{ text: 'Your agent', tone: 'agent' }], extra, open: take() }));
  }
  // One list: the engine's measured findings carry the numbers. The agent's
  // own observations restate them, so only its ideas to check are added,
  // after the measured facts.
  const ideas = (note?.findings || []).filter((finding) => finding.kind !== 'observed');
  // Keep a concise lead without discarding the other calculated findings.
  const leadCount = 8;
  const more = reportElement('details', undefined, 'overview-more-findings');
  more.append(reportElement('summary', `More calculated findings (${Math.max(0,facts.length-leadCount)})`));
  for (const [index,fact] of facts.entries()) {
    const destination = index < leadCount ? list : more;
    destination.append(findingCard({
      tone: SEVERITY_TONE[fact.severity] || 'n',
      title: fact.title,
      detail: [fact.detail, fact.target?.kind === "handoff" ? "This compares named milestones and can include intermediate work. A missing next milestone can reflect work in progress or incomplete coverage." : "", findingValueText(fact)].filter(Boolean).join(" "),
      tags: [{ text: findingTag(fact), tone: SEVERITY_TONE[fact.severity] || 'n' }, ...fact.tags.slice(1).map((text) => ({ text }))],
      onOpen: fact.view ? () => void openFindingEvidence(fact) : null,
      openLabel: fact.target?.kind === 'handoff' ? 'Open affected Items →' : fact.filter && ['step', 'edge'].includes(fact.target?.kind) ? 'Open in Process →' : fact.filter ? `Show these ${itemLabels().plural.toLowerCase()} →` : fact.target ? 'Open in Process →' : { timing: 'Open in Timing →', map: 'Open in Process →', variants: 'Open in Variants →', journey: 'Open in Process →', cases: 'Open in Items →' }[fact.view],
      impact: [fact.impact ? fact.impact.replace(/\{items\}/g, itemLabels().plural.toLowerCase()).replace(/\{item\}/g, itemLabels().singular) : '', fact.value ? `${moneyText(fact.value.total, fact.value.field)} value` : ''].filter(Boolean).join(' · '),
      open: take(),
    }));
  }
  if (facts.length > leadCount) list.append(more);
  for (const finding of ideas) {
    list.append(findingCard({
      tone: 'hyp',
      title: finding.title || findingSummary(finding.statement),
      detail: businessReportText(finding.statement),
      tags: [{ text: 'Idea to check', tone: 'warn' }],
      open: take(),
    }));
  }
  if (first) list.append(reportElement('p', analysis?.summary.cases === 0 ? 'No items match this selection. Change or remove a filter to see findings.' : !analysis ? 'This process is based on supplied documents and reported practice. No recorded event history has been analyzed.' : 'Nothing stands out in the calculations for this view.', 'empty'));
  cards.append(list);
  const window = analysis?.recorded_window;
  if (window?.length === 2) {
    const start = new Date(window[0] / 1000), end = new Date(window[1] / 1000);
    const unusual = end.getTime() > Date.now() + 86400000 || end.getTime() - start.getTime() > 10 * 365.25 * 86400000;
    const extent = reportElement('p', `${itemLabels().plural} recorded from ${start.toLocaleDateString(undefined,{dateStyle:"medium"})} to ${end.toLocaleDateString(undefined,{dateStyle:"medium"})}.${unusual ? ' This includes future dates or spans more than ten years. Review the date interpretation and example items before drawing conclusions about delays. Source timestamps are retained.' : ''}`, unusual ? 'data-context unusual' : 'data-context');
    cards.prepend ? cards.prepend(extent) : cards.append(extent);
  }
  container.append(cards);
  if (typeof renderConsultantViews === "function") renderConsultantViews(record);
  if (record?.process_model?.steps?.length) {
    const link = reportElement("button", "Explore the process and its sources →", "text-action");
    link.type = "button"; link.addEventListener("click", () => showView("map")); container.append(link);
  }
  if (auditRequested(currentWorkspace) && currentWorkspace?.purpose !== 'audit') {
    const check = reportElement("button", "See the check against your expected process →", "text-action");
    check.type = "button"; check.addEventListener("click", () => showView("conformance")); container.append(check);
  }
  renderInvestigationVisuals(container, record);
  // The agent's assumptions and open questions belong with how the data was
  // built, not beside the findings.
  const questions = [...new Set([...(note?.missing_data || []), ...(record?.process_model?.open_questions || [])])];
  const notes = document.querySelector('#data-built-notes');
  if (notes) {
    notes.replaceChildren();
    for (const [title,items] of [['Assumed by your agent',note?.assumptions || []],['Still to confirm',questions]]) {
      if (!items.length) continue;
      notes.append(reportElement('h3',title)); const entries=reportElement('ul');
      items.forEach(item=>entries.append(reportElement('li',item)));notes.append(entries);
    }
  }

  renderAskNext(note, facts);

  const earlier = history.filter((entry) => entry.question);
  if (earlier.length) {
    const past = reportElement('details', undefined, 'answer-history');
    past.append(reportElement('summary', `Earlier questions (${earlier.length})`));
    for (const entry of earlier) {
      const item = reportElement('div', undefined, 'answer-history-item');
      item.append(reportElement('p', /^health check$/i.test(entry.question) ? 'Health check' : `“${entry.question}”`, 'answer-history-question'));
      if (entry.answer) item.append(reportElement('p', entry.answer, 'answer-history-answer'));
      const titles = (entry.findings || []).map((finding) => finding.title || findingSummary(finding.statement)).filter(Boolean).slice(0, 3);
      if (titles.length) {
        const list = reportElement('ul');
        titles.forEach((title) => list.append(reportElement('li', title)));
        item.append(list);
      }
      const when = entry.recorded_at ? new Date(entry.recorded_at) : null;
      const meta = [when && !Number.isNaN(when.getTime()) ? when.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }) : '', entry.current ? '' : 'earlier settings'].filter(Boolean).join(' · ');
      if (meta) item.append(reportElement('p', meta, 'answer-history-meta'));
      past.append(item);
    }
    container.append(past);
  }
}

// Saved chart rows have been derived and scope-checked by workspace record.
// Use text nodes and fixed presentation choices; agent text is never markup.
function renderInvestigationVisuals(container, record) {
  for (const visual of (record?.visuals || []).slice(0, 6)) {
    const section = reportElement('section', undefined, 'answer-visual');
    section.append(reportElement('h2', visual.title));
    const duration = ['cohorts','explore','linked_interval'].includes(visual.request?.type) && visual.metric !== 'cases';
    const metric = {cases:'Items', total:'Total amount', mean:'Mean', median:'Median', p90:'90th percentile'}[visual.metric] || visual.metric;
    section.append(reportElement('p', `${metric}${visual.field ? ` · ${visual.field}` : ''} · saved query scope`, 'muted'));
    const rows = (visual.rows || []).slice(0, 100);
    const max = Math.max(1, ...rows.map(row => Math.abs(Number(row.value) || 0)));
    if (rows.some(row => Number(row.value) < 0)) section.append(reportElement('p', 'Bar lengths show magnitude; negative amounts keep their minus sign.', 'muted'));
    const list = reportElement('div', undefined, visual.kind === 'table' ? 'answer-visual-table' : 'answer-visual-bars');
    for (const row of rows) {
      const item = reportElement('div', undefined, 'answer-visual-row');
      const canOpenGroup = visual.request?.type === 'cohorts' || visual.grouping?.by === 'last_step' || (visual.grouping?.by === 'attribute' && row.label !== '(no value)');
      const label = reportElement(canOpenGroup ? 'button' : 'span', row.label, canOpenGroup ? 'text-action' : undefined);
      if (canOpenGroup) { label.type = 'button'; label.title = 'Open the items in this group'; label.addEventListener('click', () => void openVisualGroup(visual,row)); }
      const number = duration ? formatDuration(row.value) : Number(row.value).toLocaleString(undefined, {maximumFractionDigits:2});
      const value = reportElement('strong', number);
      const bar = reportElement('span', undefined, 'answer-visual-track');
      const fill = reportElement('i'); fill.style.width = `${Math.abs(Number(row.value)) / max * 100}%`; bar.append(fill);
      item.append(label, ...(visual.kind === 'table' ? [] : [bar]), value);
      if (row.cases_with_value != null) item.append(reportElement('small', `${row.cases_with_value} of ${row.cases} items have an amount`));
      if (row.example_case) {
        const open = reportElement('button', 'Open example item', 'text-action'); open.type = 'button';
        open.addEventListener('click', () => void openVisualItem(visual, row.example_case)); item.append(open);
      }
      list.append(item);
    }
    section.append(list);
    if (visual.other_groups) section.append(reportElement('p', `${visual.groups_available - rows.length} further groups are omitted. Their items remain in the cited query totals.`, 'muted'));
    for (const text of visual.semantics || []) section.append(reportElement('p', text, 'muted'));
    section.append(citationButton(visual.evidence)); container.append(section);
  }
}
async function openVisualGroup(visual, row) {
  if (visual.request?.object_type && typeof currentFollow !== 'undefined' && currentFollow?.type !== visual.request.object_type) {
    if (!await followType(visual.request.object_type)) return;
  }
  const filter = { ...currentFilter };
  const attribute = visual.request?.type === 'cohorts' ? visual.request.attribute : visual.grouping?.attribute;
  if (attribute) filter.attribute_equals = [...(filter.attribute_equals || []).filter(pair => pair[0] !== attribute), [attribute,row.label]];
  else if (visual.grouping?.by === 'last_step') filter.ends_with = row.label;
  else return;
  if (visual.request?.field && (visual.request.at_least != null || visual.request.at_most != null)) filter.attribute_between = [...(filter.attribute_between || []), {attribute:visual.request.field,at_least:visual.request.at_least,at_most:visual.request.at_most}];
  restoreFilterControls(filter);
  // Go to Items first: Overview always shows the whole project.
  showView('cases');
  await analyzeMapping(currentMapping,filter,'filter');
}
async function openVisualItem(visual, id) {
  if (visual.request?.object_type && typeof currentFollow !== 'undefined' && currentFollow?.type !== visual.request.object_type) await followType(visual.request.object_type);
  await openCase(id);
}

let lastReport = null;

// Re-renders Summary with what is already loaded, e.g. once the journey arrives.
function rerenderSummary() {
  if (lastReport) renderInvestigationAnswer(lastReport.record, lastReport.built, currentAnalysis, lastReport.history);
}

// Pick up saved consultant artifacts from every view, without a page refresh.
let reportWatch = null;
function watchForAnswers() {
  clearInterval(reportWatch);
  reportWatch = setInterval(async () => {
    if (document.hidden || !currentWorkspace) return;
    try {
      const payload = await workspaceApi('/api/workspace/investigation');
      // Another page (or the agent) set a new expected process: take it on.
      if (payload.case_model_sha256 && payload.case_model_sha256 !== knownModelSha) {
        knownModelSha = payload.case_model_sha256;
        await syncCaseBaseline();
      }
      // The agent checked the process against an expected one (an SOP) while
      // this page was open: take that model on and show its check.
      if (payload.pending_model?.text && !adoptedModels.has(payload.pending_model.sha256) && typeof uploadCaseBaseline === 'function' && typeof File === 'function') {
        adoptedModels.add(payload.pending_model.sha256);
        await uploadCaseBaseline(new File([payload.pending_model.text], payload.pending_model.name || 'expected-process.model.json', { type: 'application/json' }));
        document.body.dataset.investigationPurpose = auditRequested(currentWorkspace) ? 'audit' : 'improvement';
        if (document.body.dataset.view === 'conformance' && typeof checkCaseConformance === 'function') void checkCaseConformance();
        void refreshExecutiveReport();
      }
      if ((payload.artifact || null) !== (lastReport?.artifact || null)) {
        await refreshExecutiveReport();
        const notice = document.querySelector('#consultant-update');
        // A short note that the page changed by itself, gone after a few seconds.
        if (notice) {
          notice.hidden = false;
          notice.textContent = currentInvestigationRecord?.note?.show
            ? 'Updated from your agent: showing what you asked for. Its answer is at the top of the Overview.'
            : 'Updated from your agent: its latest answer is now at the top.';
          clearTimeout(watchForAnswers.hideNote);
          watchForAnswers.hideNote = setTimeout(() => { notice.hidden = true; }, 8000);
        }
      }
      reportWatchFailures = 0;
    } catch (_error) {
      // Twice in a row with no answer: glasstarn is not running any more.
      // Say how to get it back rather than fail silently.
      if (!reportWatchStopped && ++reportWatchFailures >= 2) showServiceStopped();
    }
  }, 5000);
}

let reportWatchFailures = 0;
// Set when this page is leaving on purpose (its project was deleted), so the
// note about glasstarn not running never shows for that.
let reportWatchStopped = false;

// Every glasstarn tab, shown or in the background, tells glasstarn it is
// still open about every half minute, so glasstarn keeps running while it is
// in use and stops a while after the last tab closes.
let heartbeat = null;
let heartbeatFailures = 0;
function startHeartbeat() {
  clearInterval(heartbeat);
  const beat = async () => {
    try {
      const response = await fetch('/api/heartbeat', { headers: { Authorization: `Bearer ${launchToken}` }, cache: 'no-store' });
      if (!response.ok) throw new Error(String(response.status));
      heartbeatFailures = 0;
    } catch (_error) {
      if (!reportWatchStopped && ++heartbeatFailures >= 2) showServiceStopped();
    }
  };
  void beat();
  heartbeat = setInterval(beat, 30000);
}
// Agent models this page has already taken on, so it asks only once.
const adoptedModels = new Set();
let knownModelSha = null;

// Brings this page's expected process in line with the project's.
async function syncCaseBaseline() {
  const state = await workspaceApi('/api/workspace/state');
  if (!state?.case_model || state.case_model.model_id === currentCaseBaseline?.model_id) return;
  currentCaseBaseline = state.case_model;
  const name = document.querySelector('#case-baseline-name');
  if (name) name.textContent = state.case_model.file_name;
  const check = document.querySelector('#check-case-conformance');
  if (check) check.disabled = !currentAnalysis;
  if (typeof currentCaseConformance !== 'undefined') currentCaseConformance = null;
  document.body.dataset.investigationPurpose = auditRequested(currentWorkspace) ? 'audit' : 'improvement';
  if (document.body.dataset.view === 'conformance' && typeof checkCaseConformance === 'function') void checkCaseConformance();
  void refreshExecutiveReport();
}
function showServiceStopped() {
  if (document.querySelector('#service-stopped')) return;
  const note = reportElement('div', undefined, 'service-stopped');
  note.id = 'service-stopped';
  note.setAttribute('role', 'alert');
  note.append(reportElement('strong', 'glasstarn isn’t running.'), reportElement('span', ' Ask your agent to open it again, or run glasstarn in Terminal.'));
  document.body.append(note);
}

async function refreshExecutiveReport() {
  const panel = document.querySelector('#executive-report');
  if (!panel) return;
  const revision = ++reportRevision;
  const analysis = currentAnalysis, ocel = currentOcelAnalysis, project = currentWorkspace;
  if (!analysis && !ocel && !project) { panel.hidden = true; return; }
  panel.hidden = false;
  const nav = document.querySelector('#report-nav');
  nav?.classList.remove('unavailable'); nav?.removeAttribute('aria-disabled'); nav?.removeAttribute('tabindex');
  let payload = {};
  let warning = '';
  if (project) {
    try { payload = await workspaceApi('/api/workspace/investigation'); }
    catch (error) { warning = `Saved findings could not be loaded: ${error.message}. Showing calculated summary.`; }
  }
  if (revision !== reportRevision || analysis !== currentAnalysis || ocel !== currentOcelAnalysis || project !== currentWorkspace) return;
  if (!analysis && !ocel && !payload.record) { panel.hidden = true; return; }
  const name = project?.name || 'Process project';
  const built = { evidence: payload.record?.supporting_evidence || {} };
  reportEvidence = { ...built.evidence, 'engine:analysis': { request: { type: 'active_analysis' }, result: analysis || ocel, project, scope: analysis?.view || ocel?.scope } };
  const status = document.querySelector('#report-status');
  status.textContent = warning || (payload.stale ? 'Your agent’s last answer belongs to earlier settings; it is listed under Earlier questions. Ask your agent again for this view.' : '');
  status.hidden = !status.textContent;
  document.querySelector('#report-evidence').hidden = true;
  currentInvestigationRecord = payload.stale ? null : payload.record || null;
  lastReport = { record: currentInvestigationRecord, built, history: payload.history || [], artifact: payload.artifact || null, profile: currentInvestigationRecord?.process_profile || null };
  if (typeof renderResultsKpis === "function") renderResultsKpis();
  renderInvestigationAnswer(currentInvestigationRecord, built, analysis, lastReport.history);
  if(typeof renderSavedAgentVisuals==='function')renderSavedAgentVisuals();
  if (currentInvestigationRecord?.process_model) enableNavigation("#process-map-nav");
  maybeApplyAgentShow(currentInvestigationRecord, lastReport.artifact);
}

// "Show me the main path": an answer the agent saved with `show` moves the
// open page there by itself, once per answer. A page opened later follows it
// only while the answer is fresh, so reopening a project days later starts on
// the Overview as usual.
let shownArtifact = null;
let pendingAgentShow = null;
let projectOpened = false;
const SHOW_FRESH_MS = 10 * 60 * 1000;
function maybeApplyAgentShow(record, artifact) {
  if (!artifact || artifact === shownArtifact) return;
  shownArtifact = artifact;
  const show = record?.note?.show;
  if (!show) return;
  if (projectOpened) { void applyAgentShow(show); return; }
  // A page that is still opening (the agent's link clicked) applies a fresh
  // answer once the project has opened, so restoring the saved view cannot
  // undo it.
  const age = Date.now() - Date.parse(record.recorded_at || '');
  if (age >= -60_000 && age < SHOW_FRESH_MS) pendingAgentShow = show;
}

async function applyPendingAgentShow() {
  projectOpened = true;
  const show = pendingAgentShow;
  pendingAgentShow = null;
  if (show) await applyAgentShow(show);
}

const SHOW_VIEWS = { overview: 'report', process: 'map', variants: 'variants', conformance: 'conformance', items: 'cases', compare: 'compare' };
async function applyAgentShow(show) {
  const view = SHOW_VIEWS[show.view];
  if (!view) return;
  if (show.finding) {
    const wanted = String(show.finding).toLowerCase();
    const finding = engineFindings(currentAnalysis).find((item) => String(item.title || '').toLowerCase().includes(wanted));
    if (finding && typeof openFindingEvidence === 'function') { await openFindingEvidence(finding); return; }
  }
  if (show.filter && currentMapping && view !== 'report') {
    showView(view);
    await analyzeMapping(currentMapping, show.filter, 'filter');
  }
  if (view === 'map' && show.step && typeof openFindingEvidence === 'function') {
    await openFindingEvidence({ target: { kind: 'step', activity: show.step }, title: show.step });
    return;
  }
  showView(view);
  if (view === 'map') {
    document.querySelector(show.detail === 'all' ? '#process-all' : '#process-reset')?.click();
  }
  if (view === 'variants' && show.variants?.length && typeof selectVariants === 'function') selectVariants(show.variants);
  if (view === 'compare' && show.compare && typeof loadComparisonOptions === 'function') {
    await loadComparisonOptions();
    const pick = (selector, value) => { const field = document.querySelector(selector); if (field) { field.value = value; field.dispatchEvent?.(new Event('change')); } };
    pick('#comparison-attribute', show.compare.attribute);
    if (typeof populateCohortValues === 'function') populateCohortValues();
    pick('#comparison-left', show.compare.left);
    pick('#comparison-right', show.compare.right);
    if (typeof runCohortComparison === 'function') await runCohortComparison();
  }
}
