const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');
function app() {
  const context = vm.createContext({});
  vm.runInContext(source,context);
  return code => vm.runInContext(code,context);
}

test('milestone evidence opens the exact affected records without a separate progression view',async()=>{
  const run=app();
  run(`currentJourney={handoffs:[
    {from_activity:'Order received',to_activity:'Shipment created',via_object_type:'order'},
    {from_activity:'Invoice issued',to_activity:'Payment received',via_object_type:'invoice'}]};
    globalThis.followed=null;globalThis.opened=null;
    followType=async(type,filter,preserveScope)=>{followed={type,filter,preserveScope};return true};
    showView=view=>opened=view;`);
  await run(`openFindingEvidence({view:'journey',target:{kind:'handoff',object_type:'invoice',from:'Invoice issued',to:'Payment received'}})`);
  assert.equal(run('opened'),'cases');
  assert.equal(run('followed.type'),'invoice');
  assert.equal(run('followed.preserveScope'),true);
  assert.deepEqual(JSON.parse(run('JSON.stringify(followed.filter)')),{not_reached:[['Invoice issued','Payment received']]});
});

test('an unavailable milestone target returns to the process map',async()=>{
  const run=app();
  run(`currentJourney={handoffs:[]};globalThis.opened=null;showView=view=>opened=view;`);
  await run(`openFindingEvidence({view:'journey',target:{kind:'handoff',object_type:'invoice',from:'Issue',to:'Pay'}})`);
  assert.equal(run('opened'),'map');
  await run(`openFindingEvidence({view:'journey'})`);
  assert.equal(run('opened'),'map');
});
