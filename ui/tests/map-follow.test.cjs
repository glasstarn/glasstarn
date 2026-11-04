// Narrowing the subway map to one line follows that item type on every page,
// so Variants and Items agree with the map; widening it again goes back.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

function app() {
  const context = vm.createContext({});
  vm.runInContext(source, context);
  const run = code => vm.runInContext(code, context);
  run(`followTypes = ['Order', 'Invoice', 'Delivery'];
    currentFollow = { type: 'Order' };
    currentFilter = { period: '2026' };
    followed = [];
    renders = 0;
    followType = async (type, filter, keep) => { followed.push([type, filter.period, keep]); currentFollow = { type }; subwayFocus = null; return true; };
    renderSubway = () => { renders++; };`);
  return run;
}

test('picking one line follows it, with filters kept, and the line stays picked', async () => {
  const run = app();
  run(`subwayFocus = 'Invoice'`);
  await run('syncFollowWithMap()');
  assert.deepEqual(JSON.parse(run('JSON.stringify(followed)')), [['Invoice', '2026', true]]);
  assert.equal(run('subwayFocus'), 'Invoice');
  assert.equal(run('renders'), 1);
});

test('showing all lines again goes back to what was followed before', async () => {
  const run = app();
  run(`subwayFocus = 'Invoice'`);
  await run('syncFollowWithMap()');
  run(`subwayFocus = 'Delivery'`);
  await run('syncFollowWithMap()');
  run(`subwayFocus = null`);
  await run('syncFollowWithMap()');
  assert.deepEqual(JSON.parse(run('JSON.stringify(followed.map(f => f[0]))')), ['Invoice', 'Delivery', 'Order']);
  assert.equal(run('currentFollow.type'), 'Order');
});

test('ticking a single item type in Objects and events does the same', async () => {
  const run = app();
  run(`subwayFocus = null; subwayShownTypes = new Set(['Invoice'])`);
  await run('syncFollowWithMap()');
  assert.equal(run('currentFollow.type'), 'Invoice');
  run(`subwayShownTypes = new Set(['Invoice', 'Order'])`);
  await run('syncFollowWithMap()');
  assert.equal(run('currentFollow.type'), 'Order');
});

test('several lines, or the type already followed, change nothing', async () => {
  const run = app();
  run(`subwayFocus = null; subwayShownTypes = new Set(['Invoice', 'Delivery'])`);
  await run('syncFollowWithMap()');
  run(`subwayFocus = 'Order'`);
  await run('syncFollowWithMap()');
  assert.equal(run('followed.length'), 0);
});
