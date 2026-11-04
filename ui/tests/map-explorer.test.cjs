const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({});
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

const edge = (from, to, frequency) => ({ from, to, frequency });
run(`globalThis.view = {
  top_route: ['Receive', 'Check', 'Approve', 'Ship'],
  nodes: ['Receive', 'Check', 'Approve', 'Ship', 'Request info', 'Reject', 'Escalate'].map(activity => ({ activity })),
  edges: ${JSON.stringify([
    edge('Receive', 'Check', 10), edge('Check', 'Approve', 8), edge('Approve', 'Ship', 8),
    edge('Check', 'Request info', 3), edge('Request info', 'Check', 3), edge('Check', 'Reject', 2),
    edge('Approve', 'Escalate', 1), edge('Escalate', 'Ship', 1),
  ])},
  activity_ranking: ['Receive', 'Check', 'Approve', 'Ship', 'Request info', 'Reject', 'Escalate'].map((activity, index) => ({ activity, rank: index + 1 })),
};
globalThis.layout = explorerLayout(view);
globalThis.points = Object.fromEntries([...layout.positions].map(([activity, point]) => [activity, point]));`);

test('the most common route runs straight down one column in route order', () => {
  const spine = run("view.top_route.map(activity => points[activity])");
  assert.equal(new Set(spine.map(point => point.x)).size, 1);
  for (let index = 1; index < spine.length; index += 1) assert.ok(spine[index].y > spine[index - 1].y);
});

test('other steps sit beside the steps they connect and never overlap', () => {
  const spineX = run("points.Receive.x");
  for (const activity of ['Request info', 'Reject', 'Escalate']) assert.notEqual(run(`points[${JSON.stringify(activity)}].x`), spineX);
  // Escalate sits between Approve and Ship, the steps it connects.
  assert.ok(run("points.Escalate.y > points.Approve.y && points.Escalate.y < points.Ship.y"));
  const placed = run("Object.values(points).map(point => [point.x, point.y])");
  for (const [index, [x, y]] of placed.entries()) {
    for (const [otherX, otherY] of placed.slice(index + 1)) {
      assert.ok(x !== otherX || Math.abs(y - otherY) >= 100, 'steps in one column keep a full row apart');
    }
  }
  assert.ok(run("Object.values(points).every(point => point.x - 100 >= 0 && point.x + 100 <= layout.width)"));
});

test('the drawn connections run straight down the main route and loop back marked', () => {
  run(`globalThis.drawn = resultsLayout(view); globalThis.at = name => drawn.positions.get(name);`);
  const down = run("resultsEdgeGeometry(at('Check'), at('Approve'), 0)");
  assert.equal(down.back, false);
  assert.match(down.d, /^M[\d.]+,[\d.]+ L[\d.]+,[\d.]+$/);
  assert.equal(run("resultsEdgeGeometry(at('Request info'), at('Check'), 0).back"), true);
  assert.equal(run("at('Receive').x === at('Ship').x"), true);
});
