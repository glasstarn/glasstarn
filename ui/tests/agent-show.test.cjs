const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');

// "Show me …" in the agent: the saved answer's `show` is applied on the open
// page with the same filters and selections a person would click.
const source = require('./app-source.cjs');
function app() {
  const element = () => ({ textContent: '', dataset: {}, children: [], hidden: false, value: '', click() { this.clicked = (this.clicked || 0) + 1; },
    replaceChildren(...c) { this.children = c; }, append(...c) { this.children.push(...c); }, addEventListener() {}, setAttribute() {}, removeAttribute() {}, classList: { add() {}, remove() {}, toggle() {} } });
  const elements = new Map();
  const context = vm.createContext({
    document: { body: { dataset: {} }, querySelector(s) { if (!elements.has(s)) elements.set(s, element()); return elements.get(s); }, querySelectorAll: () => [], createElement: element },
    requestAnimationFrame() {}, Event: class { constructor(type) { this.type = type; } },
  });
  const run = (code) => vm.runInContext(code, context);
  run(source);
  run(`globalThis.calls = [];
    currentMapping = { case_id: 'Claim ID' }; currentFilter = {}; currentAnalysis = { key_findings: [] };
    showView = (view) => calls.push(['view', view]);
    analyzeMapping = async (_m, filter) => { calls.push(['filter', JSON.stringify(filter)]); };
    selectVariants = (ranks) => calls.push(['variants', JSON.stringify(ranks)]);
    openFindingEvidence = async (fact) => calls.push(['finding', fact.title]);
    engineFindings = () => [{ title: '41 claims reached Pay claim without Approve payment', filter: { with_activities: ['Pay claim'] } }];
    loadComparisonOptions = async () => calls.push(['compare-options']);
    populateCohortValues = () => {};
    runCohortComparison = async () => calls.push(['compare', document.querySelector('#comparison-attribute').value, document.querySelector('#comparison-left').value, document.querySelector('#comparison-right').value]);`);
  return { run, elements, calls: () => JSON.parse(run('JSON.stringify(calls)')) };
}

test('"show me the main path" moves the page to Process on its main flow', async () => {
  const { run, elements, calls } = app();
  await run(`applyAgentShow({ view: 'process', detail: 'main' })`);
  assert.deepEqual(calls(), [['view', 'map']]);
  assert.equal(elements.get('#process-reset').clicked, 1);
});

test('"show me the three main variants" puts on a top-3 filter on Variants', async () => {
  const { run, calls } = app();
  await run(`applyAgentShow({ view: 'variants', filter: { top_variants: 3 } })`);
  assert.deepEqual(calls(), [['view', 'variants'], ['filter', '{"top_variants":3}'], ['view', 'variants']]);
});

test('particular variants, a step, a finding and clearing filters use the page\'s own controls', async () => {
  let page = app();
  await page.run(`applyAgentShow({ view: 'variants', variants: [1, 3] })`);
  assert.deepEqual(page.calls().at(-1), ['variants', '[1,3]']);
  page = app();
  await page.run(`applyAgentShow({ view: 'process', step: 'Approve payment' })`);
  assert.deepEqual(page.calls(), [['finding', 'Approve payment']]);
  page = app();
  await page.run(`applyAgentShow({ view: 'items', finding: 'without approve payment' })`);
  assert.deepEqual(page.calls(), [['finding', '41 claims reached Pay claim without Approve payment']]);
  page = app();
  await page.run(`applyAgentShow({ view: 'process', filter: {} })`);
  assert.deepEqual(page.calls().slice(0, 2), [['view', 'map'], ['filter', '{}']]);
});

test('the Overview never takes a filter, and comparing two groups fills in the comparison', async () => {
  let page = app();
  await page.run(`applyAgentShow({ view: 'overview', filter: { top_variants: 3 } })`);
  assert.deepEqual(page.calls(), [['view', 'report']]);
  page = app();
  await page.run(`applyAgentShow({ view: 'compare', compare: { attribute: 'Region', left: 'South', right: 'North' } })`);
  assert.deepEqual(page.calls(), [['view', 'compare'], ['compare-options'], ['compare', 'Region', 'South', 'North']]);
});

test("a page opened from the agent's link applies a fresh answer after it has opened, once", () => {
  const { run } = app();
  run(`globalThis.applied = []; applyAgentShow = async (show) => { applied.push(show.view); };`);
  const now = new Date().toISOString();
  run(`maybeApplyAgentShow({ recorded_at: '${now}', note: { show: { view: 'items' } } }, 'findings/b.json')`);
  assert.equal(run('applied.length'), 0, 'a page that is still opening waits, so restoring the saved view cannot undo it');
  run(`applyPendingAgentShow(); applyPendingAgentShow();`);
  assert.deepEqual(JSON.parse(run('JSON.stringify(applied)')), ['items']);
  run(`maybeApplyAgentShow({ recorded_at: '${now}', note: { show: { view: 'items' } } }, 'findings/b.json')`);
  assert.equal(run('applied.length'), 1, 'the same answer is not applied twice');
  run(`maybeApplyAgentShow({ recorded_at: '${now}', note: {} }, 'findings/plain.json')`);
  run(`maybeApplyAgentShow({ recorded_at: '${now}', note: { show: { view: 'conformance' } } }, 'findings/c.json')`);
  assert.deepEqual(JSON.parse(run('JSON.stringify(applied)')), ['items', 'conformance'], 'a new answer while the page is open applies straight away');
});

test('a page opened days later does not follow an old answer', () => {
  const { run } = app();
  run(`globalThis.applied = []; applyAgentShow = async (show) => { applied.push(show.view); };`);
  const old = new Date(Date.now() - 3 * 86400000).toISOString();
  run(`maybeApplyAgentShow({ recorded_at: '${old}', note: { show: { view: 'variants' } } }, 'findings/a.json'); applyPendingAgentShow();`);
  assert.equal(run('applied.length'), 0);
});

test('a page that opened with no ask follows the first ask made while it is open', () => {
  const { run } = app();
  run(`globalThis.applied = []; applyAgentShow = async (show) => { applied.push(show.view); };`);
  const now = new Date().toISOString();
  run(`maybeApplyAgentShow({ recorded_at: '${now}', note: {} }, 'findings/first.json'); applyPendingAgentShow();`);
  run(`maybeApplyAgentShow({ recorded_at: '${now}', note: { show: { view: 'variants' } } }, 'findings/next.json')`);
  assert.deepEqual(JSON.parse(run('JSON.stringify(applied)')), ['variants']);
});
