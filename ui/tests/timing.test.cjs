const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

function app() {
  const element = (tag) => ({ tag, dataset: {}, style: {}, attributes: {}, children: [], textContent: '', className: '',
    replaceChildren(...children) { this.children = children; },
    append(...children) { this.children.push(...children); },
    setAttribute(key, value) { this.attributes[key] = value; },
    addEventListener() {},
  });
  const elements = new Map();
  const context = vm.createContext({ document: {
    querySelector(selector) { if (!elements.has(selector)) elements.set(selector, element('div')); return elements.get(selector); },
    createElement: element,
    createElementNS: (_, tag) => element(tag),
  } });
  vm.runInContext(source, context);
  vm.runInContext(`currentMapping = { case_id: 'invoice_id' };
    globalThis.analysis = { measures_true_waiting: false, bottlenecks: [], timing: {
      duration_eligible_cases: 3, incomplete_cases: 1, median_seconds: 172800, p90_seconds: 777600, bin_width_seconds: 86400,
      bins: [{ from_seconds: 0, to_seconds: 86400, cases: 1 }, { from_seconds: 86400, to_seconds: 172800, cases: 1 }, { from_seconds: 777600, to_seconds: null, cases: 1 }],
      slowest_cases: [
        { case: 'INV-9', started_at_micros: 0, duration_seconds: 777600, events: 6, route_rank: 2, repeats_step: true },
        { case: 'INV-1', started_at_micros: 0, duration_seconds: 172800, events: 4, route_rank: 1, repeats_step: false },
      ] } };`, context);
  return { run: code => vm.runInContext(code, context), elements };
}

test('the performance tab restates the engine distribution', () => {
  const { run, elements } = app();
  run('currentAnalysis = analysis; renderResultsPerformance()');
  assert.equal(elements.get('#results-hist-hint').textContent, '3 invoices by first-to-last time · median 2d · P90 9d');
  const bars = elements.get('#results-hist').children.filter(child => child.attributes.class?.startsWith('histogram-bar'));
  assert.deepEqual(bars.map(bar => bar.attributes.class), ['histogram-bar', 'histogram-bar', 'histogram-bar slow']);
});

test('the cases tab lists the slowest engine items in order with status pills', () => {
  const { run, elements } = app();
  run('currentAnalysis = analysis; renderResultsCases()');
  const rows = elements.get('#results-cases').children;
  assert.equal(rows.length, 2);
  assert.equal(rows[0].children[0].textContent, 'INV-9');
  assert.equal(rows[0].children[2].textContent, '9d');
  assert.equal(rows[0].children[5].children[0].textContent, 'Repeats a step');
});
