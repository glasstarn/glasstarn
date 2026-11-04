const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({ document: { querySelector() { return null; } } });
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

test('every active condition is a plain chip, and removing one leaves the rest', () => {
  run(`currentFollow = null; currentMapping = { case_id: 'invoice_id' };`);
  const chips = run(`filterChips({ with_activities: ['Approve'], without_activities: [], starts_with: null, ends_with: 'Pay',
    directly_follows: [], eventually_follows: [['Receive', 'Pay']], attribute_equals: [['region', 'EU']],
    attribute_between: [{ attribute: 'amount', at_least: 50000, at_most: null }], started_after: null, started_before: null,
    min_duration: 172800000000, max_duration: null, top_variants: 3, min_events: null, complete_only: true, inside_window: null })`);
  assert.deepEqual([...chips].map(chip => chip.text), ['Includes Approve', 'Ends with Pay', 'Receive before Pay', 'region = EU',
    `amount ≥ ${(50000).toLocaleString()}`, 'Longer than 2d', 'Top 3 variants', 'Complete only']);
  const next = chips[0].next;
  assert.deepEqual([...next.with_activities], []);
  assert.equal(next.ends_with, 'Pay');
  assert.equal(chips[4].next.attribute_between.length, 0);
});

test('a variant filter keeps the number it was picked under', () => {
  run(`currentFollow = null; currentMapping = { case_id: 'invoice_id' }; rememberVariantRank(['Receive', 'Check', 'Pay'], 3);`);
  const chips = run(`filterChips({ variants: [['Receive', 'Check', 'Pay'], ['Receive', 'Pay']] })`);
  assert.deepEqual([...chips].map(chip => chip.text), ['Variant V3', 'Variant Receive → … → Pay (2 steps)']);
  assert.deepEqual(JSON.parse(JSON.stringify(chips[0].next.variants)), [['Receive', 'Pay']]);
});
