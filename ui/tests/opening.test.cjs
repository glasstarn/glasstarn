// Opening a saved project goes straight to its results; the column-mapping
// screen appears only when the saved project cannot be restored.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

function app(restored) {
  const calls = [];
  const plain = () => JSON.parse(JSON.stringify(calls));
  const nodes = new Map();
  const context = vm.createContext({ document: { body: { dataset: {} }, querySelector: (id) => { if (!nodes.has(id)) nodes.set(id, { dataset: {}, hidden: true }); return nodes.get(id); } } });
  vm.runInContext(source, context);
  vm.runInContext(`
    currentWorkspace = { name: 'Purchasing' };
    renderProfile = (profile, options = {}) => calls.push(['profile', !!options.quiet]);
    reopenPendingProject = async () => { calls.push(['reopen', document.body.dataset.opening === 'true']); return ${restored}; };
    showView = (view) => calls.push(['view', view]);
  `, Object.assign(context, { calls }));
  return { calls: { get value() { return plain(); } }, nodes, context };
}

test('a saved project opens without flashing the mapping screen', async () => {
  const { calls, nodes, context } = app(true);
  await vm.runInContext("restoreSavedProfile({ profile: {}, case_project: { name: 'x' } })", context);
  assert.deepEqual(calls.value, [['profile', true], ['reopen', true]]);
  assert.equal(nodes.get('#opening-title').textContent, 'Opening Purchasing…');
  assert.equal(nodes.get('#opening').hidden, true);
  assert.equal(context.document.body.dataset.opening, undefined);
});

test('a project that cannot be restored shows the mapping screen to fix it', async () => {
  const { calls, context } = app(false);
  await vm.runInContext("restoreSavedProfile({ profile: {}, case_project: { name: 'x' } })", context);
  assert.deepEqual(calls.value.at(-1), ['view', 'import']);
});

test('new data with no saved analysis goes to the mapping screen as before', async () => {
  const { calls, context } = app(true);
  await vm.runInContext('restoreSavedProfile({ profile: {} })', context);
  assert.deepEqual(calls.value, [['profile', false]]);
});
