const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');

const source = require('./app-source.cjs');

function app() {
  const element = () => ({ dataset: {}, style: {}, attributes: {}, textContent: '',
    classList: { toggle() {} },
    setAttribute(key, value) { this.attributes[key] = value; },
    removeAttribute(key) { delete this.attributes[key]; },
  });
  const elements = new Map();
  const links = ['home', 'map', 'cases', 'variants', 'compare', 'conformance'].map(view => {
    const link = element(); link.dataset.view = view; return link;
  });
  const document = { body: element(), querySelector(selector) {
    if (!elements.has(selector)) elements.set(selector, element());
    return elements.get(selector);
  }, querySelectorAll() { return links; } };
  const context = vm.createContext({ document, requestAnimationFrame(callback) { callback(); } });
  vm.runInContext(source, context);
  return { run: code => vm.runInContext(code, context), elements, links, document };
}

test('view changes preserve the active investigation and identify only the selected page', () => {
  const { run, document, links } = app();
  run(`currentAnalysis = { summary: { cases: 3 } }; currentMapping = { case_id: 'ticket' };
       currentFilter = { starts_with: 'Receive' }; currentCaseBaseline = { model_id: 'approved' };
       globalThis.before = [currentAnalysis, currentMapping, currentFilter, currentCaseBaseline];`);
  for (const view of ['cases', 'variants', 'conformance', 'compare', 'home', 'map']) {
    run(`showView('${view}')`);
    assert.equal(document.body.dataset.view, view);
    assert.deepEqual(links.filter(link => link.attributes['aria-current'] === 'page').map(link => link.dataset.view), [view]);
    assert.equal(run('before.every((value, index) => value === [currentAnalysis, currentMapping, currentFilter, currentCaseBaseline][index])'), true);
  }
});

test('fit mode contains the complete graph at narrow widths; actual size is explicit', () => {
  const { run, elements } = app();
  const svg = { viewBox: { baseVal: { width: 1200, height: 600 } }, parentElement: { clientWidth: 480 }, style: {} };
  elements.set('#process-map', svg);
  run('fitProcessMaps()');
  assert.ok(parseFloat(svg.style.width) <= 480);
  assert.equal(parseFloat(svg.style.width) / parseFloat(svg.style.height), 2);
  svg.parentElement.clientWidth = 300;
  run('fitProcessMaps()');
  assert.ok(parseFloat(svg.style.width) <= 300);
  svg.viewBox.baseVal = { width: 10000, height: 5000 };
  run('fitProcessMaps()');
  assert.ok(parseFloat(svg.style.width) <= 300, 'Fit includes wide graphs instead of imposing a minimum zoom.');
  svg.viewBox.baseVal = { width: 1200, height: 600 };
  run('mapActualSize = true; fitProcessMaps()');
  assert.equal(svg.style.width, '1200px');
  svg.parentElement.clientWidth = 0;
  run('mapActualSize = false; fitProcessMaps()');
  assert.equal(svg.style.width, '1200px', 'A hidden view does not collapse the graph to zero.');
});

test('audit navigation requires an explicitly chosen audit investigation', () => {
  const { run, document } = app();
  run("currentWorkspace = {purpose:'improvement'}; showView('report'); showView('conformance');");
  assert.equal(document.body.dataset.view, 'report');
  run("currentWorkspace = {purpose:'audit'}; showView('conformance');");
  assert.equal(document.body.dataset.view, 'conformance');
});

test('Overview always shows the whole project: any filter comes off on the way there', () => {
  const { run } = app();
  run(`calls = []; analyzeMapping = async (mapping, filter) => { calls.push(filter); currentFilter = filter; };
    fitProcessMaps = () => {}; currentMapping = { case_id: 'Claim ID' };
    currentFilter = { with_activities: ['Assess claim'], complete_only: true };
    mapFindingFocus = null;
    showView('report');`);
  assert.deepEqual(JSON.parse(run('JSON.stringify(calls)')), [{}]);
  run(`showView('map'); showView('report');`);
  assert.equal(run('calls.length'), 1, 'with no filter on, nothing is recalculated');
});
