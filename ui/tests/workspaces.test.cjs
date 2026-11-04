const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');
function app(fetch) {
  const status = { textContent: '', dataset: {} };
  const context = vm.createContext({ fetch, document: { querySelector: () => status } });
  const run = code => vm.runInContext(code, context);
  run(source);
  run(`currentWorkspace = { id: 'first' }; launchToken = 'browser';`);
  return { run, status };
}
test('accepted mutations save before returning while read-only queries do not write', async () => {
  const calls = [];
  const { run } = app(async path => { calls.push(path); return {ok:true,json:async()=>({saved:true})}; });
  await run(`projectFetch('/api/model', {method:'POST'})`);
  assert.deepEqual(calls, ['/api/model', '/api/workspace/save']);
  await run(`projectFetch('/api/conformance', {method:'POST'})`);
  assert.deepEqual(calls, ['/api/model', '/api/workspace/save', '/api/conformance']);
  assert.equal(run('workspaceUnsaved'), false);
});
test('failed persistence keeps accepted state and blocks navigation until saved', async () => {
  const { run, status } = app(async path => ({ ok:path !== '/api/workspace/save',json:async()=>({error:'Disk full'}) }));
  const response = await run(`projectFetch('/api/model', {method:'POST'})`);
  assert.equal(response.ok, true);
  assert.equal(run('workspaceUnsaved'), true);
  assert.match(status.textContent, /Disk full/);
  await assert.rejects(run('saveWorkspace()'), /Disk full/);
});
test('switching projects is blocked while a mutation is in flight', async () => {
  let resolve;
  const { run, status } = app(() => new Promise(done => { resolve = done; }));
  const pending = run(`projectFetch('/api/profile', {method:'POST'})`);
  await run('openWorkspace(false)');
  assert.match(status.textContent, /Wait for the current operation/);
  resolve({ok:false});
  await pending;
  assert.equal(run('workspacePending'), 0);
});
