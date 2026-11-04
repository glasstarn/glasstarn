// Polished dropdowns: the menu lists the <select>'s own options and skips
// ones that cannot be chosen.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({});
vm.runInContext(source, context);
const json = code => JSON.parse(vm.runInContext(`JSON.stringify(${code})`, context));

const select = { selectedIndex: 1, options: [
  { value: 'cases', label: 'Unique items', disabled: false, hidden: false },
  { value: 'frequency', label: '', textContent: 'Event occurrences', disabled: true, hidden: false },
  { value: 'performance', label: 'Average elapsed time', disabled: false, hidden: false },
] };
context.sample = select;

test('the menu lists each option with its label, state and the current choice', () => {
  const items = json('selectMenuItems(sample)');
  assert.deepEqual(items.map(item => item.label), ['Unique items', 'Event occurrences', 'Average elapsed time']);
  assert.deepEqual(items.map(item => item.selected), [false, true, false]);
  assert.equal(items[1].disabled, true);
});

test('arrow keys skip options that cannot be chosen and wrap around', () => {
  const items = 'selectMenuItems(sample)';
  assert.equal(json(`nextSelectItem(${items}, 0, 1)`), 2);
  assert.equal(json(`nextSelectItem(${items}, 2, 1)`), 0);
  assert.equal(json(`nextSelectItem(${items}, 0, -1)`), 2);
});

test('a multiple-choice list says how many are selected', () => {
  assert.equal(json("multiSelectSummary([{ selected: true }, { selected: false }, { selected: true }])"), '2 selected');
  assert.equal(json("multiSelectSummary([{ selected: false }])"), 'None selected');
});
