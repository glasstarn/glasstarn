const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({ document: { querySelector() { return null; } } });
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

test('the money field is chosen by name before by coverage', () => {
  assert.equal(run(`mainNumberField([{ name: 'customer_no', numbers: 90 }, { name: 'order_value', numbers: 40 }])`), 'order_value');
  assert.equal(run(`mainNumberField([{ name: 'a', numbers: 3 }, { name: 'b', numbers: 9 }])`), 'b');
  assert.equal(run('mainNumberField([])'), null);
});

test('headline facts restate the engine figures and count items without an amount', () => {
  run(`currentFollow = null; currentMapping = { case_id: 'invoice_id' };`);
  const facts = run(`valueFacts({ overall: { cases: 5, cases_with_value: 4, total: 2500000, mean: 625000, median: 400, p90: 2000000, max: 2000000, open_cases: 2, open_total: 52000 } })`);
  assert.deepEqual([...facts[0]], ['Total', (2500000).toLocaleString(undefined, { notation: 'compact', maximumFractionDigits: 1 }), '4 invoices · 1 without an amount']);
  assert.equal(facts[1][1], '400');
  assert.equal(facts.length, 4);
  assert.match(facts[3][2], /^2 invoices not at a finishing step$/);
  const unmeasured = run(`valueFacts({ overall: { cases: 1, cases_with_value: 1, total: 5, mean: 5, median: 5, p90: 5, max: 5, open_cases: null, open_total: null } })`);
  assert.equal(unmeasured.length, 3);
});

test('group rows carry their share of the total and a remainder row', () => {
  const rows = run(`valueRows({ groups: [{ label: 'Ship', stats: { total: 75, open_total: null, cases: 3 } }], groups_available: 3,
    other_groups: { total: 25, open_total: null, cases: 2 } }, 100)`);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].share, 75);
  assert.equal(rows[1].label, '2 more');
});

test('typed amount bounds read like the engine reads them', () => {
  assert.equal(run(`amountBound('50,000', 'x')`), 50000);
  assert.equal(run(`amountBound(' $1,234.5 ', 'x')`), 1234.5);
  assert.equal(run(`amountBound('', 'x')`), null);
  assert.throws(() => run(`amountBound('12,5', 'Amount at least')`), /Amount at least must be a number/);
  assert.throws(() => run(`amountBound('lots', 'x')`));
});

test('a saved filter without amount ranges still counts correctly', () => {
  const base = `{ with_activities: [], without_activities: [], starts_with: null, ends_with: null, directly_follows: [], eventually_follows: [],
    attribute_equals: [], started_after: null, started_before: null, min_duration: null, max_duration: null, top_variants: null, min_events: null, complete_only: false`;
  assert.equal(run(`activeFilterCount(${base} })`), 0);
  assert.equal(run(`activeFilterCount(${base}, attribute_between: [{ attribute: 'amount', at_least: 1, at_most: null }] })`), 1);
});

test('the note speaks of items and flags a field that is all zeros', () => {
  run(`currentFollow = { singular: 'order', plural: 'Orders' };`);
  const note = run(`valueNote({ field: 'order_value', overall: { cases: 3, cases_with_value: 2, min: 0, max: 0, open_total: null } })`);
  assert.match(note, /^Each order counts its order_value once/);
  assert.match(note, /1 order without an amount/);
  assert.match(note, /Every recorded order_value is 0/);
  assert.match(note, /Finishing steps are not confirmed/);
  run('currentFollow = null;');
});
