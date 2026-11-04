const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({ document: { querySelector() { return null; } } });
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

test('a long route shows its first steps, how many were left out, and its last step', () => {
  run(`variant = { layers: [{ activities: ['Create'] }, { activities: ['Pay', 'Post'] }, { activities: ['Close'] }], layers_omitted: 1200 };`);
  assert.equal(run('ocelVariantPath(variant)'), 'Create → [Pay + Post] → … 1,200 more steps … → Close');
  run(`variant = { layers: [{ activities: ['Pay', 'Pay'], omitted: 300 }] };`);
  assert.equal(run('ocelVariantPath(variant)'), '[Pay + Pay + 300 more]');
  run(`variant = { layers: [{ activities: ['Create'] }, { activities: ['Close'] }] };`);
  assert.equal(run('ocelVariantPath(variant)'), 'Create → Close');
});
