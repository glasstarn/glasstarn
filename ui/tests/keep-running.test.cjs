// 1.0.1: an open tab keeps glasstarn running, and deleting the project you are
// in takes you to the project list instead of a page that has stopped.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

function load(extra = {}) {
  const timers = [];
  const calls = [];
  const notes = [];
  const context = vm.createContext({
    launchToken: null,
    document: { hidden: true, querySelector: () => null, body: { append: (n) => notes.push(n), dataset: {} } },
    setInterval: (fn, ms) => { timers.push({ fn, ms }); return timers.length; },
    clearInterval: () => {},
    fetch: async (path) => { calls.push(path); if (extra.fail) throw new Error('down'); return { ok: true, json: async () => ({ status: 'ok' }) }; },
    ...extra.globals,
  });
  vm.runInContext(source, context);
  return { context, timers, calls, notes };
}

test('every tab sends a heartbeat about every half minute, even in the background', async () => {
  const { context, timers, calls } = load();
  vm.runInContext('startHeartbeat()', context);
  await new Promise((r) => setImmediate(r));
  assert.deepEqual(calls, ['/api/heartbeat']);
  const beat = timers.find((t) => t.ms === 30000);
  assert.ok(beat, 'a 30 second heartbeat');
  await beat.fn();
  assert.equal(calls.length, 2, 'it beats while the tab is hidden too');
});

test('when glasstarn is not running, the note says so plainly, with no talk of timing out', () => {
  const text = source.slice(source.indexOf('function showServiceStopped'), source.indexOf('function showServiceStopped') + 600);
  assert.match(text, /glasstarn isn’t running\./);
  assert.match(text, /Ask your agent to open it again, or run glasstarn in Terminal\./);
  assert.doesNotMatch(source, /stops after a while unused/);
});

test('a page leaving on purpose never shows the not-running note', async () => {
  const shown = [];
  const { context, timers } = load({ fail: true });
  context.showServiceStopped = () => shown.push(1);
  vm.runInContext('reportWatchStopped = true; startHeartbeat()', context);
  const beat = timers.find((t) => t.ms === 30000);
  await beat.fn(); await beat.fn(); await beat.fn();
  assert.equal(shown.length, 0);
});

test('deleting the project you are in goes to the project list with a Trash note', () => {
  const text = source.slice(source.indexOf('async function deleteProject'), source.indexOf('async function openWorkspace'));
  assert.match(text, /deleted\.list_url/);
  assert.match(text, /window\.location\.assign\(`\$\{deleted\.list_url\}\?trashed=/);
  const init = source.slice(source.indexOf('async function initializeWorkspaces'), source.indexOf('async function initializeWorkspaces') + 700);
  assert.match(init, /get\("trashed"\)/);
  assert.match(init, /is in the Trash\./);
});
