const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');

// Exercise the shipped browser code, with controllable network completion order.
const source = require('./app-source.cjs');
function app() {
  const elements = new Map();
  const element = () => ({
    textContent: '', dataset: {}, disabled: false, value: '20', children: [],
    replaceChildren(...children) { this.children = children; },
    append(...children) { this.children.push(...children); },
    addEventListener() {},
    setAttribute() {},
  });
  const context = vm.createContext({
    document: {
      querySelector(selector) {
        if (!elements.has(selector)) elements.set(selector, element());
        return elements.get(selector);
      },
      createElement: element,
      createElementNS: (_, tag) => element(tag),
    },
  });
  const run = (code) => vm.runInContext(code, context);
  run(source);
  run(`launchToken = 'test'; currentProfile = { upload_id: 'log' };
    currentMapping = {}; currentAnalysis = {}; currentCaseBaseline = { model_id: 'old', file_name: 'old.model.json' };
    globalThis.rendered = []; renderCaseConformance = report => rendered.push(report);`);
  return { context, run, elements };
}
function deferredFetch(context) {
  let resolve;
  context.fetch = () => new Promise(done => { resolve = done; });
  return (body, ok = true) => resolve({ ok, status: ok ? 200 : 422, json: async () => body });
}

test('late conformance cannot replace evidence after a view change', async () => {
  const { context, run, elements } = app();
  const respond = deferredFetch(context);
  const pending = run('checkCaseConformance()');
  run(`currentAnalysis = null; clearCaseConformanceResults('Analysis changed');`);
  respond({ marker: 'old view' });
  await pending;
  assert.equal(context.rendered.length, 0);
  assert.equal(elements.get('#case-conformance-status').textContent, 'Analysis changed');
  assert.equal(elements.get('#check-case-conformance').disabled, true);
});

test('late failure cannot erase a newer result', async () => {
  const { context, run, elements } = app();
  const respond = deferredFetch(context);
  const pending = run('checkCaseConformance()');
  run(`clearCaseConformanceResults('New baseline ready');`);
  respond({ error: 'old request failed' }, false);
  await pending;
  assert.equal(elements.get('#case-conformance-status').textContent, 'New baseline ready');
});

test('rejected baseline retains the model still selected by the server', async () => {
  const { context, run, elements } = app();
  const respond = deferredFetch(context);
  const pending = run(`uploadCaseBaseline({ name: 'broken.model.json', size: 10 })`);
  respond({ error: 'Invalid net' }, false);
  await pending;
  assert.equal(run('currentCaseBaseline.model_id'), 'old');
  assert.match(elements.get('#case-conformance-status').textContent, /old.model.json remains selected/);
  assert.equal(elements.get('#check-case-conformance').disabled, false);
});

test('accepted baseline stays synchronized when the active filter changes during upload', async () => {
  const { context, run, elements } = app();
  const respond = deferredFetch(context);
  const pending = run(`uploadCaseBaseline({ name: 'new.model.json', size: 10 })`);
  run(`clearCaseConformanceResults('Filter changed');`);
  respond({ model_id: 'new', file_name: 'new.model.json', places: 2, transitions: 1 });
  await pending;
  assert.equal(run('currentCaseBaseline.model_id'), 'new');
  assert.equal(elements.get('#check-case-conformance').disabled, false);
});

test('a baseline response for a replaced log is ignored', async () => {
  const { context, run } = app();
  const respond = deferredFetch(context);
  const pending = run(`uploadCaseBaseline({ name: 'new.model.json', size: 10 })`);
  run(`currentProfile = { upload_id: 'replacement' }; currentCaseBaseline = null; clearCaseConformanceResults();`);
  respond({ model_id: 'new', file_name: 'new.model.json', places: 2, transitions: 1 });
  await pending;
  assert.equal(run('currentCaseBaseline'), null);
});

test('empty views do not claim every case conforms', () => {
  const { run, elements } = app();
  run(source.slice(source.indexOf('function renderCaseConformance('), source.indexOf('\nlet conformanceMoneyRevision')));
  run(`renderResults = () => {}; renderCaseConformance({ algorithm: 'astar_alignment', fitness_millionths: 1000000,
    fit_cases: 0, view: { cases_shown: 0, cases_total: 3 }, cases: [], by_step: [], deviations_shown: 0,
    deviating_cases: 0, source: 'log.csv', model_source: 'baseline.model.json' });
    renderResultsConformance();`);
  assert.match(elements.get('#conf-headline').textContent, /^No .* in this view to check$/);
  assert.doesNotMatch(elements.get('#conf-headline').textContent, /followed/);
});

// The redesigned page: one plain answer, the SOP as a line of steps, and one
// card per kind of difference. Figures are the engine's; only words are added.
function checked(run) {
  run(source.slice(source.indexOf('function renderCaseConformance('), source.indexOf('\nlet conformanceMoneyRevision')));
  run(`currentMapping = { case_id: 'Claim ID' }; renderResults = () => {};
    renderCaseConformance({ algorithm: 'astar_alignment', fitness_millionths: 970934, fit_cases: 628, deviating_cases: 572,
      view: { cases_shown: 1200, cases_total: 1200, narrowed: false }, cases: [],
      expected_process: 'Claims SOP v2.1; appeals allowed after closure', model_saved_at_ms: Date.UTC(2026, 10, 1, 12),
      expected_steps: [{ step: 'Approve claim', optional: true }, { step: 'Approve payment', optional: true }, { step: 'Notify customer', optional: false }],
      by_step: [
        { activity: 'Notify customer', kind: 'skipped', cases: 553, example_cases: ['CLM-1'], variants: [['a', 'b']] },
        { activity: 'Approve payment', kind: 'skipped', cases: 41, example_cases: ['CLM-2'], variants: [['a', 'c']] },
      ] });
    renderResultsConformance();`);
}

test('the Conformance page leads with one plain answer, the same figure strip as other pages, and when the SOP was set', () => {
  const { run, elements } = app();
  checked(run);
  assert.equal(elements.get('#conf-headline').textContent, '572 of 1,200 claims did not follow Claims SOP v2.1');
  assert.match(elements.get('#conf-when').textContent, /^SOP set .*1.* · checked just now on all 1,200 claims$/);
  const tiles = elements.get('#conf-figures').children.map((box) => box.children.map((part) => part.textContent));
  assert.deepEqual(tiles.map((tile) => tile.slice(0, 2)), [['Claims checked', '1,200'], ['Followed it', '628'], ['Did not', '572'], ['Steps matched', '97%']]);
  assert.ok(elements.get('#conf-figures').children.every((box) => box.className === 'kpi'));
});

test('the Conformance page shows the SOP as steps, with counts only where it broke', () => {
  const { run, elements } = app();
  checked(run);
  const steps = elements.get('#conf-line').children;
  assert.equal(steps.length, 3);
  assert.equal(steps[0].className, 'conf-step optional');
  assert.equal(steps[0].children.length, 2, 'a followed step carries no label under it');
  assert.equal(steps[1].className, 'conf-step has-issue optional');
  assert.equal(steps[1].children.at(-1).textContent, '41 skipped');
  assert.equal(steps[2].children.at(-1).textContent, '553 skipped');
});

test('what did not follow the SOP is one table row per kind, largest first, with no method words', () => {
  const { run, elements } = app();
  checked(run);
  const rows = elements.get('#conf-groups').children;
  assert.equal(rows.length, 2);
  const cells = (row) => row.children.map((cell) => cell.textContent);
  assert.deepEqual(cells(rows[0]).slice(0, 3), ['Skipped “Notify customer”', '553', '46%']);
  assert.deepEqual(cells(rows[1]).slice(0, 3), ['Skipped “Approve payment”', '41', '3%']);
  const page = [...elements.values()].map((element) => element.textContent).join(' ');
  assert.doesNotMatch(page, /token|fitness|alignment|Run audit check|Examples to show|How this was checked/i);
});

test('a Conformance row opens exactly its items, under one named filter', async () => {
  const { run } = app();
  checked(run);
  run(`globalThis.shown = []; showView = (view) => shown.push(view); analyzeMapping = async (_mapping, filter) => { globalThis.applied = filter; };
    currentFilter = { attribute_in: { Region: ['South'] } };`);
  await run(`openConformanceGroup(currentCaseConformance.by_step[1])`);
  assert.deepEqual(JSON.parse(run('JSON.stringify(applied)')), { attribute_in: { Region: ['South'] }, variants: [['a', 'c']] });
  assert.deepEqual(JSON.parse(run('JSON.stringify(shown)')), ['cases']);
  assert.equal(run('conformanceFilterLabel.text'), 'Skipped “Approve payment”');
});

test('every check from the page is the exact one', async () => {
  const { context, run } = app();
  let body;
  context.fetch = async (_url, init) => { body = JSON.parse(init.body); return { ok: true, status: 200, json: async () => ({}) }; };
  await run('checkCaseConformance()');
  assert.equal(body.algorithm, 'alignment');
});
