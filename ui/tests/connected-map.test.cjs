const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({});
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

// Two invoice routes: three go issued → paid; one is paid twice.
run(`globalThis.analysis = {
  variants: [
    { object_type: 'invoice', rank: 2, objects: 1, layers: [{ activities: ['Invoice issued'] }, { activities: ['Payment recorded'] }, { activities: ['Payment recorded'] }] },
    { object_type: 'invoice', rank: 1, objects: 3, layers: [{ activities: ['Invoice issued'] }, { activities: ['Payment recorded'] }] },
    { object_type: 'order', rank: 1, objects: 2, layers: [{ activities: ['Order created'] }] },
  ],
  graphs: { directly_follows: [
    { object_type: 'invoice', source: 'Payment recorded', target: 'Payment recorded', occurrences: 1, objects: 1, mean_elapsed_micros: 86400000000 },
    { object_type: 'invoice', source: 'Invoice issued', target: 'Payment recorded', occurrences: 4, objects: 4, mean_elapsed_micros: 172800000000 },
    { object_type: 'order', source: 'Order created', target: 'Invoice issued', occurrences: 2, objects: 2, mean_elapsed_micros: 1 },
  ] },
};`);

test('the connected map follows one item type with engine counts', () => {
  const view = run("ocelMapView(analysis, 'invoice', 50)");
  assert.deepEqual([...view.top_route], ['Invoice issued', 'Payment recorded']);
  const nodes = Object.fromEntries(view.nodes.map(node => [node.activity, node]));
  assert.equal(nodes['Payment recorded'].frequency, 5);
  assert.equal(nodes['Invoice issued'].starts, 4);
  assert.equal(nodes['Payment recorded'].ends, 4);
  assert.deepEqual([...view.edges.map(edge => `${edge.from}>${edge.to}:${edge.frequency}`)], ['Invoice issued>Payment recorded:4', 'Payment recorded>Payment recorded:1']);
  assert.equal(view.edges[0].mean_seconds, 172800);
});

test('the connection limit keeps the most frequent connections', () => {
  const view = run("ocelMapView(analysis, 'invoice', 1)");
  assert.equal(view.edges.length, 1);
  assert.equal(view.available, 2);
});

test('the map opens on the item type involved in the most events', () => {
  run(`analysis.graphs.participation = [
    { object_type: 'customers', occurrences: 2000 },
    { object_type: 'orders', occurrences: 2000 }, { object_type: 'orders', occurrences: 4000 },
    { object_type: 'items', occurrences: 5000 },
  ];`);
  assert.equal(run("mainObjectType(analysis, ['customers', 'items', 'orders'])"), 'orders');
  assert.equal(run("mainObjectType({ graphs: { participation: [] } }, ['b', 'a'])"), 'a');
});

test('item types read as plain plural names', () => {
  assert.equal(run("itemTypeName('opportunity')"), 'Opportunities');
  assert.equal(run("itemTypeName('Order')"), 'Orders');
  assert.equal(run("itemTypeName('CreditNote')"), 'Credit notes');
  assert.equal(run("itemTypeName('goods receipt')"), 'Goods receipts');
  assert.equal(run("itemTypeName('invoice', false)"), 'Invoice');
  // Types already named in the plural are not pluralised again.
  assert.equal(run("itemTypeName('items')"), 'Items');
  assert.equal(run("itemTypeName('packages', false)"), 'Package');
  assert.equal(run("itemTypeName('deliveries', false)"), 'Delivery');
  assert.equal(run("itemTypeName('boxes', false)"), 'Box');
  assert.equal(run("itemTypeName('address')"), 'Addresses');
  assert.equal(run("itemTypeName('status')"), 'Statuses');
});

test('every item type that takes part in a step can be followed, busiest first', () => {
  const order = run(`followCandidates({ graphs: { participation: [
    { object_type: 'payment', occurrences: 3 }, { object_type: 'order', occurrences: 5 }, { object_type: 'order', occurrences: 2 },
  ] }, catalog: { object_types: ['payment', 'order', 'unused'] } })`);
  assert.deepEqual([...order], ['order', 'payment']);
});

test('a project business name replaces the source name', () => {
  run("currentItemNames = { opportunity: 'Deals', 'goods receipt': 'Goods receipts' };");
  assert.equal(run("itemTypeName('opportunity')"), 'Deals');
  assert.equal(run("itemTypeName('opportunity', false)"), 'Deal');
  assert.equal(run("itemTypeName('goods receipt', false)"), 'Goods receipt');
  assert.equal(run("itemTypeName('order')"), 'Orders');
  run("currentItemNames = {};");
});
