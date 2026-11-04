// Subway after the Celonis comparison: side paths rejoin their line, lines
// start with ▶ and stop with ■, thickness follows volume, and a detail switch
// lets more side paths through.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({});
vm.runInContext(source, context);
const json = code => JSON.parse(vm.runInContext(`JSON.stringify(${code})`, context));

const variant = (object_type, objects, share, steps) => ({ object_type, rank: 1, objects, share_millionths: share * 1e6, layers: steps.map(activity => ({ activities: [activity] })) });
const flow = (object_type, source, target, objects) => ({ object_type, source, target, objects, occurrences: objects, mean_elapsed_micros: 0 });
const order = ['Receive', 'Check', 'Approve', 'Invoice', 'Close'];
const analysis = {
  variants: [variant('Order', 1000, 1, order)],
  graphs: {
    activities: [],
    directly_follows: [
      ...order.slice(1).map((step, index) => flow('Order', order[index], step, 1000)),
      flow('Order', 'Check', 'Reject', 100),            // 10%: ends there
      flow('Order', 'Approve', 'Discount', 80),         // 8%: comes back
      flow('Order', 'Discount', 'Invoice', 75),
      flow('Order', 'Check', 'Escalate', 30),           // 3%: only with more detail
      flow('Order', 'Check', 'Recheck', 8),             // 0.8%: only at Most
    ],
  },
};
const model = (level) => json(`(subwayDetailLevel = ${level}, subwayModel(${JSON.stringify(analysis)}))`);

test('a side path whose items carry on rejoins the line; one that stops does not', () => {
  const spurs = model(0).spurs;
  assert.equal(spurs.find(spur => spur.to === 'Discount').rejoins, 'Invoice');
  assert.equal(spurs.find(spur => spur.to === 'Reject').rejoins, null);
  const layout = json(`(subwayDetailLevel = 0, subwayLayout(subwayModel(${JSON.stringify(analysis)})))`);
  const discount = layout.paths.find(item => item.spur && item.rejoins);
  assert.ok(discount, 'the detour is drawn back to the line');
  assert.ok(layout.paths.some(item => item.spur && !item.rejoins), 'the dead end stays a dead end');
});

test('the detail switch lets smaller side paths through', () => {
  const names = level => model(level).spurs.map(spur => spur.to).sort();
  assert.deepEqual(names(0), ['Discount', 'Reject']);
  assert.deepEqual(names(1), ['Discount', 'Escalate', 'Reject']);
  assert.deepEqual(names(2), ['Discount', 'Escalate', 'Recheck', 'Reject']);
});

test('each stretch is drawn on its own with its volume, first and last marked', () => {
  const layout = json(`(subwayDetailLevel = 0, subwayLayout(subwayModel(${JSON.stringify(analysis)})))`);
  const stretches = layout.paths.filter(item => !item.spur);
  assert.equal(stretches.length, order.length - 1);
  assert.ok(stretches[0].first && !stretches[0].last);
  assert.ok(stretches.at(-1).last);
  assert.ok(stretches.every(item => item.objects === 1000));
});

test('thickness grows with volume but a small path stays visible', () => {
  assert.equal(json('subwayStrokeWidth(1000, 1000)'), 11);
  assert.equal(json('subwayStrokeWidth(0, 1000)'), 3);
  assert.ok(json('subwayStrokeWidth(10, 1000)') > 3.5);
  assert.ok(json('subwayStrokeWidth(250, 1000)') < json('subwayStrokeWidth(1000, 1000)'));
});

test('the start and end stops sit beyond the station, away from the line', () => {
  assert.deepEqual(json('subwayMarkPoint([100, 50], [200, 50])'), [74, 50]);
  assert.deepEqual(json('subwayMarkPoint([300, 50], [200, 50])'), [326, 50]);
});

test('the switch is on the map and only shows for the subway', () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'index.html'), 'utf8');
  assert.match(html, /id="subway-detail"[^>]*hidden>.*data-subway-detail="0" aria-pressed="true">Main lines<.*>More side paths<.*>Most</);
  assert.match(fs.readFileSync(path.join(__dirname, '..', 'src/subway.ts'), 'utf8'), /\["#subway-detail", !subway\]/);
});

test('levels that would add nothing are known, so they can be greyed out', () => {
  assert.deepEqual(json(`subwayDetailCounts(${JSON.stringify(analysis)})`), [2, 3, 4, 4]);
  const clean = { ...analysis, graphs: { ...analysis.graphs, directly_follows: analysis.graphs.directly_follows.filter(item => !['Escalate', 'Recheck'].includes(item.target)) } };
  assert.deepEqual(json(`subwayDetailCounts(${JSON.stringify(clean)})`), [2, 2, 2, 2]);
});

test('a steep climb runs between columns, not along the column it leaves', () => {
  assert.deepEqual(json('subwayRoutePoints([[0, 0], [80, -256]])'), [[0, 0], [40, -40], [40, -216], [80, -256]]);
});

test('a line running under another gets its own numbers below its track', () => {
  const two = {
    variants: [variant('Order', 500, 1, ['A', 'B', 'C', 'D']), variant('Delivery', 600, 1, ['B', 'X', 'Y', 'C'])],
    graphs: { activities: [], directly_follows: [
      flow('Order', 'A', 'B', 500), flow('Order', 'B', 'C', 500), flow('Order', 'C', 'D', 500),
      flow('Delivery', 'B', 'X', 600), flow('Delivery', 'X', 'Y', 600), flow('Delivery', 'Y', 'C', 600),
    ] },
  };
  const labels = json(`(subwayDetailLevel = 0, (() => { const m = subwayModel(${JSON.stringify(two)}); return subwayLabels(m, subwayLayout(m), "count"); })())`);
  const order = labels.find(label => label.type === 'Order' && label.text === '500' && label.key === 'B\u0001C');
  const delivery = labels.find(label => label.type === 'Delivery' && label.key === 'X\u0001Y');
  assert.ok(order && delivery, JSON.stringify(labels));
  assert.ok(order.y > delivery.y, `order ${order.y} should sit below delivery ${delivery.y}`);
});

test('two lines on the same stretch with different figures each get their own number', () => {
  const two = {
    variants: [variant('Order', 500, 1, ['A', 'B', 'C']), variant('Delivery', 600, 1, ['B', 'C'])],
    graphs: { activities: [], directly_follows: [
      flow('Order', 'A', 'B', 500), flow('Order', 'B', 'C', 500), flow('Delivery', 'B', 'C', 600),
    ] },
  };
  const labels = json(`(subwayDetailLevel = 0, (() => { const m = subwayModel(${JSON.stringify(two)}); return subwayLabels(m, subwayLayout(m), "count"); })())`);
  const stretch = labels.filter(label => label.key === 'B\u0001C');
  assert.equal(stretch.length, 2, JSON.stringify(stretch));
  assert.ok(!stretch.some(label => label.text.includes(' / ')));
  const order = stretch.find(label => label.type === 'Order'), delivery = stretch.find(label => label.type === 'Delivery');
  assert.ok(order.y > delivery.y);
});

test('a detour arch runs level over its stop, so the stop sits on the line', () => {
  const layout = json(`(subwayDetailLevel = 0, subwayLayout(subwayModel(${JSON.stringify(analysis)})))`);
  const arch = layout.paths.find(item => item.spur && item.rejoins);
  const stop = arch.points.findIndex((point, index) => index > 0 && index < arch.points.length - 1 && arch.points[index - 1][1] === point[1] && arch.points[index + 1][1] === point[1]);
  assert.ok(stop > 0, JSON.stringify(arch.points));
});

test('vertical names keep clear of the numbers on the lines', () => {
  const placed = json(`(subwayDetailLevel = 2, (() => {
    const m = subwayModel(${JSON.stringify(analysis)}), l = subwayLayout(m);
    const t = ([x, y]) => [y + 30, x * .78];
    const tags = subwayLabels(m, l, "count").map(raw => ({ x: raw.y + 30, y: raw.x * .78, text: raw.text }));
    const spots = subwayVerticalNames(l, t, tags, s => [s.activity], lines => lines[0].length * 7);
    const boxes = [...spots].map(([name, spot]) => { const w = name.length * 7; const x1 = spot.anchor === "end" ? spot.x - w : spot.x; return { name, x1, x2: x1 + w, y1: spot.top, y2: spot.top + 18 }; });
    const hits = [];
    for (const b of boxes) for (const tag of tags) { const w = tag.text.length * 6.9 + 14; if (b.x1 < tag.x + w / 2 && b.x2 > tag.x - w / 2 && b.y1 < tag.y + 9 && b.y2 > tag.y - 9) hits.push(b.name + " on " + tag.text); }
    return hits;
  })())`);
  assert.deepEqual(placed, []);
  assert.match(fs.readFileSync(path.join(__dirname, '..', 'src/subway.ts'), 'utf8'), /cx:x\+station\.top,cy:y,r:7/);
});

test('All adds skips forward and loops back between a line\'s own stops', () => {
  const withSkips = { ...analysis, graphs: { ...analysis.graphs, directly_follows: [...analysis.graphs.directly_follows, flow('Order', 'Check', 'Invoice', 60), flow('Order', 'Invoice', 'Approve', 40)] } };
  const most = json(`(subwayDetailLevel = 2, subwayModel(${JSON.stringify(withSkips)}))`);
  assert.equal(most.skips.length, 0);
  const all = json(`(subwayDetailLevel = 3, subwayModel(${JSON.stringify(withSkips)}))`);
  assert.deepEqual(all.skips.map(skip => [skip.from, skip.to, skip.back]), [['Check', 'Invoice', false], ['Invoice', 'Approve', true]]);
  // All means all: even a tiny skip is drawn.
  const tiny = { ...withSkips, graphs: { ...withSkips.graphs, directly_follows: [...withSkips.graphs.directly_follows, flow('Order', 'Receive', 'Close', 5)] } };
  assert.equal(json(`(subwayDetailLevel = 3, subwayModel(${JSON.stringify(tiny)}))`).skips.length, 3);
  assert.match(fs.readFileSync(path.join(__dirname, '..', 'index.html'), 'utf8'), /data-subway-detail="3" aria-pressed="false">All</);
  assert.match(fs.readFileSync(path.join(__dirname, '..', 'src/subway.ts'), 'utf8'), /async function openLineInClassic\(type\)/);
});
