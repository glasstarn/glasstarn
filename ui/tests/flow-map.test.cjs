// Classic process map after the refresh: pills sized to their names, and
// curved connections that meet each step at its own edge.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({});
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

test('steps are pills wide enough for their name, within the column', () => {
  const short = run("resultsNodeWidth('Pack', 12, true)");
  const long = run("resultsNodeWidth('Create Purchase Order Item', 81084, true)");
  assert.ok(long > short);
  assert.ok(long >= 'Create Purchase Order Item'.length * 7.3);
  assert.ok(run("resultsNodeWidth('A very long step name that would never fit in any column at all', 1, false)") <= run('RESULTS_SIDE_WIDTH'));
  assert.equal(run('RESULTS_NODE_HEIGHT'), 38);
});

test('a connection that skips steps curves out and ends at the target pill edge', () => {
  const a = { x: 300, y: 100, width: 300, column: 0 };
  const b = { x: 300, y: 400, width: 200, column: 0 };
  const geometry = run(`resultsEdgeGeometry(${JSON.stringify(a)}, ${JSON.stringify(b)}, 0)`);
  const numbers = geometry.d.match(/-?[\d.]+/g).map(Number);
  assert.equal(numbers[0], 150);
  assert.equal(numbers.at(-2), 197);
  assert.equal(numbers.at(-1), 400);
});

test('only the ends of the main route draw lines to Start and End', () => {
  run(`globalThis.flowLayout = { centre: 100, positions: new Map([
    ['A', { x: 100, y: 10 }], ['B', { x: 100, y: 80 }], ['C', { x: 100, y: 150 }], ['Side', { x: 400, y: 80 }],
  ]) };`);
  const allowed = (name, start) => run(`terminalLineAllowed(flowLayout, flowLayout.positions.get('${name}'), ${start})`);
  assert.equal(allowed('A', true), true);
  assert.equal(allowed('B', true), false);
  assert.equal(allowed('B', false), false);
  assert.equal(allowed('C', false), true);
  assert.equal(allowed('Side', false), true);
});

test('a BPMN export is named after the project, safely', () => {
  assert.equal(run("bpmnFileName('Order to cash (connected)')"), 'Order to cash connected - process map.bpmn');
  assert.equal(run("bpmnFileName('../../etc/passwd')"), '.. .. etc passwd - process map.bpmn');
  assert.equal(run("bpmnFileName('')"), 'glasstarn - process map.bpmn');
});

test('maps fill the canvas, growing a little when there is room', () => {
  assert.equal(run('explorerFitWidth(1000, 400, 1400, 900, "horizontal")'), 1368);
  assert.equal(run('explorerFitWidth(600, 900, 1400, 700, null)'), 700 / 900 * 600);
  assert.equal(Math.round(run('explorerFitWidth(400, 300, 1400, 900, null)')), 460);
  assert.equal(run('explorerFitWidth(400, 3000, 1000, 700, "vertical")'), 400);
});

test('a wide subway keeps readable names and scrolls instead of shrinking', () => {
  assert.equal(Math.round(run('explorerFitWidth(1600, 500, 1000, 900, "horizontal")')), 1248);
});

test('the scope chip says how many items and filters the map uses', () => {
  assert.equal(run("mapScopeText(251734, 'work items', 0)"), '251,734 work items');
  assert.equal(run("mapScopeText(1200, 'orders', 2)"), '1,200 orders · 2 filters');
  assert.equal(run("mapScopeText(1200, 'orders', 1)"), '1,200 orders · 1 filter');
});

test('the legend names three kinds of arrow, slow only when timing', () => {
  run('globalThis.formatDuration = s => `${s}s`;');
  assert.deepEqual(JSON.parse(run('JSON.stringify(mapLegendItems(false, {slow_threshold_seconds: 9}).map(i => i[1]))')), ['Forward', 'Goes back']);
  assert.deepEqual(JSON.parse(run('JSON.stringify(mapLegendItems(true, {slow_threshold_seconds: 9}).map(i => i[1]))')), ['Forward', 'Goes back', 'Slow']);
});

test('the step panel opens only once a step is picked on the map shown', () => {
  run('mapStyle = "classic"; currentOcelAnalysis = null; resultsSelection = null;');
  assert.equal(run('mapHasSelection()'), false);
  run('resultsSelection = "Pay";');
  assert.equal(run('mapHasSelection()'), true);
  run('currentOcelAnalysis = {}; mapStyle = "subway"; subwaySelection = null;');
  assert.equal(run('mapHasSelection()'), false, 'a classic pick does not open the subway panel');
  run('currentOcelAnalysis = null; mapStyle = "classic"; resultsSelection = null;');
});

test('numbers sit halfway along their arrow', () => {
  const point = JSON.parse(run('JSON.stringify(edgeLabelPoint({ getTotalLength: () => 200, getPointAtLength: (at) => ({ x: at, y: 7 }) }, { lx: 1, ly: 2 }))'));
  assert.deepEqual(point, [100, 7]);
  assert.deepEqual(JSON.parse(run('JSON.stringify(edgeLabelPoint({}, { lx: 1, ly: 2 }))')), [1, 2]);
});

test('measuring between steps waits for the missing step on the map', () => {
  assert.equal(run("studioPickingFor('segment', '', '')"), 'from');
  assert.equal(run("studioPickingFor('segment', 'A', '')"), 'to');
  assert.equal(run("studioPickingFor('segment', 'A', 'B')"), null);
  assert.equal(run("studioPickingFor('whole', '', '')"), null);
});

test('the step panel counts every way out, not only the arrows drawn', () => {
  const nodes = [{ activity: 'Receipt', ends: 20 }, { activity: 'Invoice' }, { activity: 'Vendor' }];
  const edges = [
    { from: 'Receipt', to: 'Invoice', case_frequency: 110, duration_eligible_occurrences: 1, mean_seconds: 5 },
    { from: 'Receipt', to: 'Vendor', case_frequency: 80, duration_eligible_occurrences: 1, mean_seconds: 5 },
    { from: 'Vendor', to: 'Receipt', case_frequency: 90, duration_eligible_occurrences: 9, mean_seconds: 900 },
  ];
  const result = JSON.parse(run(`JSON.stringify(stepFlows('Receipt', ${JSON.stringify(nodes)}, ${JSON.stringify(edges)}))`));
  assert.deepEqual(result.next.map(item => item.name), ['Invoice', 'Vendor', 'Ends here']);
  assert.equal(result.slowestBefore.from, 'Vendor');
});
