// Subway map: which item types become lines, where interchanges and spurs
// sit, and that lines never run over each other's private stretches.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({});
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);
const json = code => JSON.parse(vm.runInContext(`JSON.stringify(${code})`, context));

const variant = (object_type, objects, share, steps) => ({ object_type, rank: 1, objects, share_millionths: share * 1e6, layers: steps.map(activity => ({ activities: [activity] })) });
const flow = (object_type, source, target, objects, days = 0) => ({ object_type, source, target, objects, occurrences: objects, mean_elapsed_micros: days * 86400e6 });
const order = ['Receive Order', 'Credit Check', 'Approve Order', 'Pick List', 'Shipping Label', 'Ship Order', 'Generate Invoice'];
const analysis = {
  variants: [
    variant('Order', 1340, 0.67, order),
    variant('Invoice', 1465, 0.81, ['Generate Invoice', 'Send Invoice', 'Receive Payment', 'Apply Payment']),
    variant('Shipment', 1809, 1, ['Shipping Label', 'Ship Order']),
    variant('Payment', 1465, 1, ['Receive Payment', 'Apply Payment']),
    variant('Return', 103, 1, ['Return Request', 'Approve Return', 'Credit Note']),
    variant('Customer', 180, 0.21, ['Receive Order', 'Credit Check']),
    variant('CreditNote', 103, 1, ['Credit Note']),
  ],
  graphs: {
    activities: [{ activity: 'Generate Invoice', events: 1809 }],
    directly_follows: [
      ...order.slice(1).map((step, index) => flow('Order', order[index], step, 1809)),
      flow('Order', 'Credit Check', 'Reject Order', 191),
      flow('Order', 'Generate Invoice', 'Return Request', 97, 28),
      flow('Invoice', 'Generate Invoice', 'Send Invoice', 1809),
      flow('Invoice', 'Send Invoice', 'Receive Payment', 1465, 8),
      flow('Invoice', 'Receive Payment', 'Apply Payment', 1465),
      flow('Invoice', 'Send Invoice', 'Payment Reminder', 170, 12),
      flow('Invoice', 'Send Invoice', 'Rare Step', 20),
    ],
  },
};
run(`globalThis.model = subwayModel(${JSON.stringify(analysis)}); globalThis.layout = subwayLayout(model);`);

test('a type becomes a line when its common route covers a quarter of its items', () => {
  const types = run('model.lines.map(line => line.type)');
  assert.deepEqual([...types].sort(), ['Invoice', 'Order', 'Payment', 'Return', 'Shipment']);
  assert.equal(run("model.lines.find(line => line.type === 'Order').objects"), 2000);
});

test('common side steps become spurs, rare ones do not', () => {
  const spurs = run('model.spurs.map(spur => spur.to)');
  assert.ok(spurs.includes('Reject Order'));
  assert.ok(spurs.includes('Payment Reminder'));
  assert.ok(!spurs.includes('Rare Step'));
  assert.ok(!spurs.includes('Return Request'), 'a step on another line is not a spur');
});

test('shared steps are interchanges and the biggest line runs straight', () => {
  const at = name => run(`({ ...layout.stations.get(${JSON.stringify(name)}) })`);
  assert.deepEqual([...at('Ship Order').lines].sort(), ['Order', 'Shipment']);
  const ys = new Set(order.map(step => at(step).y));
  assert.equal(ys.size, 1);
  for (let index = 1; index < order.length; index += 1) assert.ok(at(order[index]).x > at(order[index - 1]).x);
  assert.ok(at('Send Invoice').x > at('Generate Invoice').x);
});

test('no two stations share a spot', () => {
  const spots = run('[...layout.stations.values()].map(station => `${station.x},${station.y}`)');
  assert.equal(new Set(spots).size, spots.length);
});

test('returns follow the invoice they come after, not the start of the map', () => {
  const at = name => run(`layout.stations.get(${JSON.stringify(name)}).x`);
  assert.ok(at('Return Request') > at('Generate Invoice'));
});

test('lines bend at 45 degrees between rows', () => {
  const points = json('subwayRoutePoints([[0, 0], [300, 100]])');
  assert.deepEqual(points, [[0, 0], [100, 0], [200, 100], [300, 100]]);
});

test('station names wrap onto at most three short lines', () => {
  assert.deepEqual(json("subwayLabelLines('Schedule Order Fulfillment')"), ['Schedule', 'Order', 'Fulfillment']);
  assert.deepEqual(json("subwayLabelLines('Ship Order')"), ['Ship Order']);
});

test('each line keeps one track along a row, so it never jogs', () => {
  const y = (name, type) => run(`layout.stations.get(${JSON.stringify(name)}).y + layout.track(layout.stations.get(${JSON.stringify(name)}), ${JSON.stringify(type)})`);
  assert.equal(y('Generate Invoice', 'Invoice'), y('Send Invoice', 'Invoice'));
  assert.notEqual(y('Generate Invoice', 'Invoice'), y('Generate Invoice', 'Order'));
  assert.notEqual(y('Receive Payment', 'Invoice'), y('Receive Payment', 'Payment'));
});

test('a spur sits one column after the step it branches from', () => {
  const column = name => run(`layout.stations.get(${JSON.stringify(name)}).column`);
  assert.equal(column('Reject Order'), column('Credit Check') + 1);
  assert.equal(column('Payment Reminder'), column('Send Invoice') + 1);
});

test('a hand-off into another line without a shared step is a dotted transfer', () => {
  const transfers = json('model.transfers');
  assert.deepEqual(transfers.map(item => `${item.from}>${item.to}`), ['Generate Invoice>Return Request']);
  assert.ok(json('layout.paths.filter(path => path.transfer).length') === 1);
});

test('a dotted transfer leaves at 45 degrees, then runs level', () => {
  assert.deepEqual(json('subwayRoutePoints([[0, 0], [300, 100]], true)'), [[0, 0], [100, 100], [300, 100]]);
});

test('How many labels every stretch with its count, How long with its time', () => {
  run('globalThis.formatDuration = seconds => `${Math.round(seconds / 86400)}d`;');
  const counts = json('subwayLabels(model, layout, "count")');
  const times = json('subwayLabels(model, layout, "time")');
  assert.ok(counts.some(label => label.text === '1,465'));
  assert.ok(counts.some(label => label.text === '97 returns' || label.text === '97 orders'));
  assert.ok(times.some(label => label.text === '8d' && label.slow));
  assert.ok(times.every(label => !/^\\d[\\d,]*$/.test(label.text)), 'no bare counts in time mode');
  assert.ok(!counts.some(label => label.slow), 'red is for time only');
  assert.notDeepEqual(counts.map(label => label.text), times.map(label => label.text));
});

test('every stretch far above the typical wait is red in How long, spurs included', () => {
  const times = json('subwayLabels(model, layout, "time")');
  assert.ok(times.find(label => label.text === '12d').slow, 'the 12-day reminder spur');
  assert.ok(!times.find(label => label.text === '0d').slow);
});

test('a station gains one full click target after its hidden map becomes visible', () => {
  run(`{
    let visible=false, hit=null;
    const group={querySelector:()=>hit,getBBox:()=>visible?{x:20,y:30,width:160,height:36}:{width:0},insertBefore:element=>{hit=element},firstChild:null};
    const svg={querySelectorAll:()=>[group]};
    globalThis.svgElement=(tag,attributes)=>({tag,...attributes});
    ensureSubwayHitAreas(svg);
    globalThis.hiddenHit=hit;
    visible=true;ensureSubwayHitAreas(svg);globalThis.visibleHit=hit;
    ensureSubwayHitAreas(svg);globalThis.sameHit=hit===visibleHit;
  }`);
  assert.equal(run('hiddenHit'),null);
  assert.deepEqual(json('visibleHit'),{tag:'rect',class:'subway-hit',x:15,y:25,width:170,height:46,fill:'transparent','pointer-events':'all'});
  assert.equal(run('sameHit'),true);
});

test('the subway runs along the longer side of the canvas until the person picks', () => {
  assert.equal(run('subwayAutoOrientation(1100, 700)'), 'horizontal');
  assert.equal(run('subwayAutoOrientation(500, 900)'), 'vertical');
});

test('lines side by side share one label, so none lands on the station names', () => {
  const counts = json('subwayLabels(model, layout, "count")');
  const shipping = counts.filter(label => label.key === 'Shipping Label\u0001Ship Order');
  assert.equal(shipping.length, 1);
  const station = run("layout.stations.get('Shipping Label').y");
  assert.ok(shipping[0].y < station, 'the shared label sits above the line');
});

test('station names avoid the lines, the numbers and each other', () => {
  const tags = 'subwayLabels(model, layout, "count")';
  const spots = json(`[...subwayNamePlacements(layout, ${tags}, station => subwayLabelLines(station.activity)).entries()]`);
  assert.equal(spots.length, json('[...layout.stations.keys()]').length);
  const boxes = spots.map(([activity, spot]) => {
    const lines = json(`subwayLabelLines(${JSON.stringify(activity)})`);
    const width = Math.max(...lines.map(line => line.length)) * 7.6;
    const x1 = spot.anchor === 'middle' ? spot.x - width / 2 : spot.anchor === 'end' ? spot.x - width : spot.x;
    return { activity, x1, x2: x1 + width, y1: spot.y - 12, y2: spot.y - 12 + lines.length * 15 };
  });
  for (const a of boxes) for (const b of boxes) if (a !== b) assert.ok(!(a.x1 < b.x2 && a.x2 > b.x1 && a.y1 < b.y2 && a.y2 > b.y1), `${a.activity} overlaps ${b.activity}`);
});
