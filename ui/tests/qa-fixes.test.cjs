// Fixes from the 2026-10-22 independent test of the Process tab.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');
const read = (file) => fs.readFileSync(path.join(__dirname, '..', file), 'utf8');

const context = vm.createContext({});
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

// A tiny stand-in for SVG elements: dataset plus a class list.
function fakeElement(data, classes = []) {
  const set = new Set(classes);
  return { dataset: data, classList: { toggle: (name, on) => (on ? set.add(name) : set.delete(name)), remove: (name) => set.delete(name), has: (name) => set.has(name) } };
}

test('clicking an arrow keeps that arrow and its two steps sharp', () => {
  const edges = [fakeElement({ from: 'A', to: 'B' }), fakeElement({ from: 'B', to: 'C' })];
  const nodes = [fakeElement({ activity: 'A' }, ['sel']), fakeElement({ activity: 'B' }), fakeElement({ activity: 'C' })];
  const svg = { classList: { toggle() {} }, querySelectorAll: (selector) => (selector === '.node' ? nodes : edges) };
  context.svg = svg;
  run("focusFlowEdge(svg, { from: 'A', to: 'B' })");
  assert.ok(edges[0].classList.has('picked') && !edges[0].classList.has('dim'));
  assert.ok(edges[1].classList.has('dim'));
  assert.ok(!nodes[0].classList.has('dim') && !nodes[0].classList.has('sel'));
  assert.ok(nodes[2].classList.has('dim'));
});

test('only a click on the bare canvas counts as empty', () => {
  assert.equal(run("mapClickIsEmpty({ tagName: 'svg' })"), true);
  assert.equal(run("mapClickIsEmpty({ tagName: 'rect' })"), false);
  assert.equal(run('mapClickIsEmpty(null)'), false);
});

test('errors read plainly instead of system text', () => {
  assert.match(run("plainErrorMessage('Failed to fetch')"), /stopped answering/);
  assert.doesNotMatch(run("plainErrorMessage('No such file or directory (os error 2): sources/x.csv')"), /os error/);
});

test('applying filters shows an updating state and the filter needs a value', () => {
  const css = read('src/app.css'), controls = read('src/controls.ts'), filters = read('src/filters.ts');
  assert.match(css, /body\[data-updating="true"\] \.results \.map-workspace::after \{ content: "Updating the view…"/);
  assert.match(controls, /Choose or type a value for this field first\./);
  assert.match(filters, /focus\(\{ preventScroll: true \}\)/);
  assert.match(filters, /function placeFilterPanel\(panel\)/);
});

test('a linked selection counts as active only when it narrows more than the followed type', () => {
  assert.equal(run('ocelSelectionActive({ cohort: { root_type: "Order" }, object_types: [], event_object_qualifiers: [] })'), false);
  assert.equal(run('ocelSelectionActive({ object_types: ["Order"] })'), true);
  assert.equal(run('ocelSelectionActive(null)'), false);
});

test('a page left behind by another tab says to reload, in plain words', () => {
  assert.match(run("plainErrorMessage('the uploaded file is no longer available; choose it again')"), /another tab or window.*Reload this page/);
});

test('Measure explains itself in plain words and A/B sit on the steps', () => {
  const method = JSON.parse(run('JSON.stringify(studioPlainMethod("segment"))'));
  assert.ok(method.some((line) => /from step A to step B/.test(line)));
  assert.ok(!method.join(' ').match(/eligible|cohort|one-hop|bins/));
  const html = read('index.html');
  assert.match(html, /Measure from A’s<select id="studio-from-occurrence" data-seg="true"><option value="first">First time<\/option><option value="last">Last time/);
  assert.match(html, /Measure to B’s<select id="studio-to-occurrence" data-seg="true"><option value="first">First time<\/option><option value="last" selected>Last time/);
  assert.match(read('src/map-analysis.ts'), /function measurementBadgePoint\(node\)/);
});

test('a vertical subway is drawn at its own size, not enlarged', () => {
  assert.equal(run('explorerFitWidth(400, 3000, 1000, 700, "vertical")'), 400);
  assert.equal(run('explorerFitWidth(1400, 3000, 1000, 700, "vertical")'), 968);
});

test('saved views can be removed and the map stays up while filters change', () => {
  assert.match(read('src/map-analysis.ts'), /\/api\/workspace\/views\/remove/);
});

test('the subway summary names the records instead of counting item types', () => {
  assert.deepEqual(JSON.parse(run('JSON.stringify(subwaySummaryText(["Orders", "Invoices", "Shipments", "Payments", "Returns"], 20, 5))')),
    ['Orders with invoices, shipments, payments and returns', ' · 20 steps, 5 where they meet']);
  assert.deepEqual(JSON.parse(run('JSON.stringify(subwaySummaryText(["Orders", "Invoices"], 12, 0))')), ['Orders with invoices', ' · 12 steps']);
  assert.deepEqual(JSON.parse(run('JSON.stringify(subwaySummaryText(["Orders"], 1, 0))')), ['Orders', ' · 1 step']);
});

test('a filter with fields left out counts as no filters, not six', () => {
  assert.equal(run('activeFilterCount({ with_activities: [], without_activities: [], directly_follows: [], eventually_follows: [], attribute_equals: [] })'), 0);
});

test('Measure explanations use the mono face so they read as notes, not results', () => {
  assert.match(read('src/app.css'), /:is\(#studio-semantics, #studio-definition, \.studio-rules > p, #studio-range-note, \.studio-note, #studio-contrast-note\) \{ font-family: var\(--mono\)/);
});

test('zooming keeps the point under the pointer where it was', () => {
  // The pointer is 300px into a 1000px drawing; at 2000px that point is 600px in, 200px from the edge.
  assert.equal(run('zoomScrollTarget(100, 200, 1000, 2000)'), 400);
  assert.equal(run('zoomScrollTarget(0, 50, 1000, 500)'), 0);
  assert.equal(run('zoomScrollTarget(40, 10, 0, 500)'), 40);
});

test('the evidence sheet opens from the top bar, not a drawer at the foot', () => {
  const html = read('index.html');
  assert.match(html, /id="data-built-open"[^>]*hidden>How this data was built<\/button>/);
  assert.match(html, /<section class="data-built" id="data-built" role="dialog"/);
  assert.doesNotMatch(html, /<details class="data-built"/);
});

test('an item is compared with the usual route in plain words', () => {
  const notes = JSON.parse(run('JSON.stringify(itemDepartures(["Create", "Change Price", "Change Price", "Invoice", "Pay"], ["Create", "Receipt", "Invoice", "Pay"]))'));
  assert.deepEqual(notes, ['Skipped Receipt', 'Extra step: Change Price', 'Repeated Change Price ×2']);
  assert.deepEqual(JSON.parse(run('JSON.stringify(itemDepartures(["A", "B"], ["A", "B"]))')), []);
});

test('the longest wait in an item is the step reached after the biggest gap', () => {
  assert.equal(run('itemLongestGap([{ since_previous_seconds: null }, { since_previous_seconds: 60 }, { since_previous_seconds: 9000 }, { since_previous_seconds: 30 }])'), 2);
  assert.equal(run('itemLongestGap([{ since_previous_seconds: null }, { since_previous_seconds: 0 }])'), -1);
});

test('an item opens in a sheet over any page, not at the foot of Items', () => {
  const html = read('index.html');
  assert.match(html, /<section class="item-sheet" id="item-sheet" role="dialog"/);
  assert.doesNotMatch(html, /class="panel item-search"/);
  assert.doesNotMatch(read('src/cases.ts'), /showView\("cases"\);\n  const detail/);
});

test('a variant row spells out its steps in one short line', () => {
  assert.equal(run('variantStepsText(["A", "B", "C"])'), 'A → B → C');
  assert.equal(run('variantStepsText(["A", "B", "C", "D", "E", "F", "G"])'), 'A → B → 3 more → F → G');
  const html = read('index.html');
  assert.match(html, /<button type="button" id="variant-all">Select all<\/button><button type="button" id="variant-reset">Clear<\/button>/);
});

test('a route shown on a map is lit end to end, with no step picked for you', () => {
  const explorers = read('src/explorers.ts');
  assert.match(explorers, /selection:variantStepPicked\?variantSelectedStep:null/);
  assert.match(read('src/results.ts'), /resultsRouteFocus = \{ rank, sequence: variant.sequence, cases: variant.cases \};\n  \/\/ No step is picked, so the whole route stays lit.\n  resultsSelection = null;/);
  assert.match(read('src/cases.ts'), /resultsSelection = null;/);
});

test('the evidence sheet drops its preamble and panels close with a round button', () => {
  assert.doesNotMatch(read('index.html'), /For the person checking the evidence/);
  assert.match(read('src/app.css'), /#map-controls-close\) \{ display: inline-grid; place-items: center; width: 32px !important;[^}]*border-radius: 50% !important/);
});

test('items are called by their business name when the column says so', () => {
  assert.equal(run('itemLabels({ case_id: "purchase_order" }).plural'), 'Purchase orders');
  assert.equal(run('itemLabels({ case_id: "sales_order" }).plural'), 'Orders');
  assert.equal(run('itemLabels({ case_id: "lead_id" }).plural'), 'Leads');
});

test('an open filter shows on every page and sits under + Filter', () => {
  assert.match(read('src/app.css'), /body\[data-view\] \.results #filter-panel\.as-popover:not\(\[hidden\]\) \{ display: block; \}/);
  assert.doesNotMatch(read('src/filters.ts'), /if \(document\.body\.dataset\.view !== "map"\) return;/);
});

test('a map opened from a finding can always go back to the usual map', () => {
  const results = read('src/results.ts');
  assert.match(results, /mapFindingFocus = \{ title: fact\.title \|\| "a finding" \};/);
  assert.match(results, /function backToUsualMap\(\)/);
  assert.match(read('src/explorers.ts'), /"Show the usual map"/);
});

test('Variants uses the full width with rounded cards', () => {
  const css = read('src/app.css');
  assert.match(css, /body\[data-view="variants"\] \.results \.project-content > \* \{ max-width: none; \}/);
  assert.match(css, /\.results \.variant-layout > \.panel \{ border: 0; border-radius: 14px;/);
});

test('money on a finding and on a map panel reads from the data, or not at all', () => {
  run('currentMapping = { case_id: "purchase_order" };');
  assert.equal(run('findingValueText({ value: { field: "order_amount", total: 2102963.6, items: 83 } })'), 'The 83 purchase orders involved carry 2.1M in order_amount.');
  assert.equal(run('findingValueText({})'), '');
  run('currentMapView = { values: { field: "order_amount", steps: { Pay: 1500 }, connections: {} } };');
  assert.deepEqual(JSON.parse(run('JSON.stringify(mapValueFact(1500, "passing through"))')), [['Value passing through', '1,500 in order_amount']]);
  assert.deepEqual(JSON.parse(run('JSON.stringify(mapValueFact(undefined, "carried"))')), []);
  assert.match(read('index.html'), /<section id="studio-drivers" class="studio-drivers" hidden aria-label="Likely drivers">/);
});

test('a "handled by" filter shows as a chip and counts as a filter', () => {
  const chips = JSON.parse(run('JSON.stringify(filterChips({ handled_by: ["Dana Whitfield"] }).map(chip => chip.text))'));
  assert.deepEqual(chips, ['Handled by Dana Whitfield']);
  assert.equal(run('activeFilterCount({ with_activities: [], without_activities: [], directly_follows: [], eventually_follows: [], attribute_equals: [], handled_by: ["Dana Whitfield"] })'), 1);
  assert.match(read('src/results.ts'), /filter\.handled_by = \[\.\.\.new Set\(\[\.\.\.\(filter\.handled_by \|\| \[\]\), target\.value\]\)\];/);
});

test('a finding opens with exactly its items: its filter joins the active one', () => {
  const merged = JSON.parse(run(`JSON.stringify(mergeFilters({ attribute_equals: [["region", "West"]], min_events: null }, { directly_follows: [["A", "B"]], attribute_equals: [["region", "West"]], repeats: [], min_duration: 5, complete_only: false }))`));
  assert.deepEqual(merged.directly_follows, [['A', 'B']]);
  assert.deepEqual(merged.attribute_equals, [['region', 'West']]);
  assert.equal(merged.min_duration, 5);
  assert.equal(merged.repeats, undefined);
  const chips = JSON.parse(run('JSON.stringify(filterChips({ repeats: ["Match Invoice"], outside_top_variants: 3, not_ending_with: ["Pay", "Close"] }).map(chip => chip.text))'));
  assert.deepEqual(chips, ['Repeats Match Invoice', 'Off the 3 main variants', 'Not finished (not at Pay, Close)']);
});

test('+ Filter offers "Handled by" when people are recorded', () => {
  const html = read('index.html');
  assert.match(html, /data-filter-open="people" id="filter-menu-people" hidden><b>Handled by<\/b>/);
  assert.match(html, /<select id="filter-people" multiple aria-label="Handled by"><\/select>/);
  assert.match(read('src/filters.ts'), /handled_by: document\.querySelector\("#filter-people"\) \? selectedValues/);
});

test('when glasstarn stops, the open page says how to get it back', async () => {
  const vm = require('node:vm');
  const source = require('./app-source.cjs');
  const appended = [];
  const el = () => ({ dataset: {}, append(...c) { this.children = c; }, setAttribute() {}, textContent: '' });
  const context = vm.createContext({ document: { hidden: false, body: { append: n => appended.push(n) }, querySelector: () => null, createElement: el }, setInterval: f => { context.tick = f; }, clearInterval() {} });
  vm.runInContext(source, context);
  vm.runInContext(`currentWorkspace = { id: 'x' }; workspaceApi = async () => { throw new Error('offline'); }; watchForAnswers();`, context);
  await context.tick(); assert.equal(appended.length, 0);
  await context.tick(); assert.equal(appended.length, 1);
  assert.equal(appended[0].id, 'service-stopped');
});

test('the Conformance page shows once a check has run, even in an improvement project', () => {
  const vm = require('node:vm');
  const source = require('./app-source.cjs');
  const context = vm.createContext({});
  vm.runInContext(source, context);
  const run = code => vm.runInContext(code, context);
  assert.equal(run(`auditRequested({ purpose: 'improvement' })`), false);
  assert.equal(run(`auditRequested({ purpose: 'audit' })`), true);
  run(`currentCaseBaseline = { file_name: 'expected.json' }`);
  assert.equal(run(`auditRequested({ purpose: 'improvement' })`), true);
});

test('a newer glasstarn shows one quiet note in the top bar', () => {
  const vm = require('node:vm');
  const source = require('./app-source.cjs');
  const added = [];
  const connection = { after: n => added.push(n) };
  const context = vm.createContext({ document: { querySelector: s => s === '#connection' ? connection : null, createElement: () => ({}) } });
  vm.runInContext(source, context);
  vm.runInContext(`showUpdateNote(null)`, context);
  assert.equal(added.length, 0);
  vm.runInContext(`showUpdateNote('0.2.0')`, context);
  assert.equal(added.length, 1);
  assert.match(added[0].textContent, /0\.2\.0 is available/);
});

test('with a filter on, Variants starts with every path in it ticked', () => {
  const vm = require('node:vm');
  const source = require('./app-source.cjs');
  const node = () => ({ dataset: {}, style: {}, classList: { toggle() {}, add() {}, remove() {} }, replaceChildren() {}, append() {}, setAttribute() {}, querySelectorAll: () => [] });
  const context = vm.createContext({ document: { querySelector: node, querySelectorAll: () => [], body: { dataset: {} } } });
  vm.runInContext(source, context);
  const run = code => vm.runInContext(code, context);
  run(`currentAnalysis = { summary: { variants: 49 }, variants: Array.from({ length: 20 }, () => ({})) }; currentFilter = {}; resetExplorers();`);
  assert.equal(run('JSON.stringify(variantSelection)'), '[1]');
  run(`currentFilter = { outside_top_variants: 3 }; resetExplorers();`);
  assert.equal(run('variantSelection.length'), 49);
});

test('the Conformance page lists deviations by step, most common first', () => {
  const vm = require('node:vm');
  const source = require('./app-source.cjs');
  const element = () => ({ children: [], hidden: false, append(...c) { this.children.push(...c); }, replaceChildren(...c) { this.children = c; }, addEventListener() {} });
  const list = element();
  const context = vm.createContext({ document: { querySelector: s => s === '#conf-groups' ? list : null, createElement: element } });
  vm.runInContext(source, context);
  vm.runInContext(`currentMapping = { case_id: 'Claim ID' }; renderConformanceSteps({ view: { cases_shown: 1200 }, by_step: [{ activity: 'Pay claim', kind: 'out_of_order', cases: 41 }] })`, context);
  assert.equal(list.children.length, 1);
  assert.equal(list.children[0].children[0].textContent, '“Pay claim” before the steps it needs');
  assert.equal(list.children[0].children[1].textContent, '41');
});