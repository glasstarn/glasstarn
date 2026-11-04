// The Overview reads in the person's words: their item name, money with its
// currency, and the engine's precise label for each finding.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');
const context = vm.createContext({});
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

test('items are named from the case column', () => {
  for (const [column, plural] of [['Claim ID', 'Claims'], ['application_id', 'Applications'], ['Shipment', 'Shipments'], ['case:concept:name', 'Work items'], ['Policy number', 'Policies']]) {
    assert.equal(run(`itemLabels({ case_id: ${JSON.stringify(column)} }).plural`), plural, column);
  }
});

test('money carries its currency or its column, never a bare number', () => {
  assert.equal(run(`moneyText(4500000, 'Claim Amount (USD)')`), '$4.5M');
  assert.equal(run(`moneyText(1200, 'Betrag (EUR)')`), '€1,200');
  assert.equal(run(`moneyText(2100000, 'order_amount')`), '2.1M order amount');
});

test('findings use the engine label for a longest wait, slow step or skipped step', () => {
  assert.equal(run(`findingTag({ kind: 'skipped_step', tags: ['Skipped step', 'Approve payment'] })`), 'Skipped step');
  assert.equal(run(`findingTag({ kind: 'bottleneck', tags: ['Longest wait', 'A → B'] })`), 'Longest wait');
  assert.equal(run(`findingTag({ kind: 'bottleneck', tags: ['Bottleneck', 'A → B'] })`), 'Bottleneck');
});
