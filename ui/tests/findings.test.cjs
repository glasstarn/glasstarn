const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({});
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

test('saved findings carry the standard claim labels from their cited evidence', () => {
  run(`globalThis.record = { supporting_evidence: {
    'exports/query-1.json': { request: { type: 'cohorts' } },
    'exports/query-2.json': { request: { type: 'case' } },
  } };`);
  assert.equal(run("findingLabel({ kind: 'observed', evidence: ['exports/query-1.json'] }, record).text"), 'Statistical association');
  assert.equal(run("findingLabel({ kind: 'observed', evidence: ['exports/query-2.json'] }, record).text"), 'Calculated fact');
  assert.equal(run("findingLabel({ kind: 'hypothesis', evidence: [] }, record).text"), 'Hypothesis');
});
