const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

function app() {
  const element = () => ({ dataset: {}, style: {}, children: [], textContent: '', className: '', listeners: {},
    replaceChildren(...children) { this.children = children; },
    append(...children) { this.children.push(...children); },
    addEventListener(type, listener) { this.listeners[type] = listener; },
  });
  const elements = new Map();
  const fragment = () => ({ children: [], append(...parts) { this.children.push(...parts); } });
  const context = vm.createContext({ document: {
    querySelector(selector) { if (!elements.has(selector)) elements.set(selector, element()); return elements.get(selector); },
    createElement: element,
    createDocumentFragment: fragment,
  } });
  const run = code => vm.runInContext(code, context);
  run(source);
  run(`currentMapping = { case_id: 'invoice_id' };
    currentAnalysis = { view: { cases_shown: 120, cases_total: 120, narrowed: false },
      summary: { cases: 120, events: 510, activities: 5, variants: 3, top_five_coverage_percent: 100,
        median_throughput_seconds: 259200, p90_throughput_seconds: 777600, mean_throughput_seconds: 432000, duration_eligible_cases: 120 },
      rework: { case_percent: 16.666666, cases_affected: 20 } };`);
  const text = node => typeof node === 'string' ? node : node.textContent || (node.children || []).map(text).join('');
  const strip = () => elements.get('#results-kpis').children.map(tile =>
    ({ label: tile.children[0].textContent, value: text(tile.children[1]), sub: tile.children[2].textContent }));
  return { run, strip, elements };
}

test('the figure strip restates engine figures in the prototype order', () => {
  const { run, strip, elements } = app();
  run("currentWorkspace = { purpose: 'improvement' }; renderResultsKpis();");
  assert.deepEqual(strip().map(item => item.label), ['Invoices', 'Typical time', 'Variants', 'Repeated a step']);
  assert.deepEqual(strip().map(item => item.value), ['120', '3d', '3', '16.7%']);
  assert.equal(strip()[1].sub, 'First to last recorded step · median · 1 in 10 over 9d');
  assert.equal(strip()[3].sub, '20 invoices');
  assert.equal(elements.get('#kpi-line').textContent, '120 invoices · median 3d · 3 variants · 16.7% repeat a step');
});

test('an extract window labels items instead of dropping them', () => {
  const { run, strip } = app();
  run("currentWorkspace = { purpose: 'improvement' }; currentAnalysis.window = { complete: 90, started_before: 25, still_open: 5, end_defined: true }; renderResultsKpis();");
  assert.equal(strip()[0].value, '120');
  assert.equal(strip()[0].sub, '90 complete · 25 began earlier · 5 still open');
  run("currentAnalysis.window = { complete: 95, started_before: 25, still_open: 0, end_defined: false }; renderResultsKpis();");
  assert.equal(strip()[0].sub, '95 complete · 25 began earlier');
});

test('conformance comes from the latest engine check', () => {
  const { run, strip } = app();
  run("currentWorkspace = { purpose: 'audit' }; renderResultsKpis();");
  assert.deepEqual(strip()[4], { label: 'Follow the SOP', value: '—', sub: 'Not checked yet' });
  run("currentWorkspace = { purpose: 'improvement' }; renderResultsKpis();");
  assert.equal(strip().length, 4, 'no conformance tile without an expected process');
  run("currentWorkspace = { purpose: 'audit' };");
  run("currentCaseConformance = { fitness_millionths: 935700, fit_cases: 90, deviating_cases: 30, view: { cases_shown: 120 } }; renderResultsKpis();");
  // Plain words: the share of items that followed it exactly, not a fitness score.
  assert.deepEqual(strip()[4], { label: 'Follow the SOP', value: '75%', sub: '90 invoices exactly · 30 did not' });
});

// The same elapsed figure has a different meaning on a long-lived object.
test('object history never inherits business cycle or rework problem labels', () => {
  const { run, strip } = app();
  run("currentAnalysis.mapping = { object_history: true }; currentAnalysis.process = { time_label: 'Time to cash' }; renderResultsKpis();");
  assert.equal(strip()[1].label, 'History span');
  assert.match(strip()[1].sub, /Object history/);
  assert.match(strip()[3].sub, /not proof of rework/);
  assert.equal(run('processProfile(currentAnalysis)'), null);
  run("currentAnalysis.key_findings = [{kind:'rework',severity:'high',title:'Problem',reach_percent:70}];");
  assert.equal(run('engineFindings(currentAnalysis).length'), 0);
});
