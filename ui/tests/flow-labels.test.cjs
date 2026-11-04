const assert = require('node:assert/strict');
const vm = require('node:vm');
const {test} = require('node:test');
const source = require('./app-source.cjs');
const context = vm.createContext({});
vm.runInContext(source,context);
const run = code => vm.runInContext(code,context);
test('business nouns follow the mapped identifier with a neutral fallback',()=>{
  assert.equal(run('itemLabels({case_id:"invoice_id"}).plural'),'Invoices');
  assert.equal(run('itemLabels({case_id:"ticket_number"}).plural'),'Tickets');
  assert.equal(run('itemLabels({case_id:"case_id"}).plural'),'Work items');
});

test('aggregate duration with no eligible items remains unavailable',()=>{
  assert.equal(run('measuredDuration(0,0)'),'Time unavailable');
  assert.equal(run('measuredDuration(0,1)'),'0s');
});

