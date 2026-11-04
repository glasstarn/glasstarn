const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');
function app(hash) {
  const nodes = new Map(); const saved = new Map(); const navigations = [];
  const context = vm.createContext({URLSearchParams, window: {location:{hash,pathname:'/',search:'',assign:url=>navigations.push(url)},addEventListener(){}},history:{replaceState(){}},sessionStorage:{setItem:(k,v)=>saved.set(k,v),getItem:k=>saved.get(k)}, document:{body:{dataset:{}},querySelector:id=>{if(!nodes.has(id)) nodes.set(id,{dataset:{},addEventListener(){}});return nodes.get(id);}}});
  const run = code => vm.runInContext(code, context); run(source);
  run('refreshWorkspaces = async () => ({current:null,projects:[]})');
  return {run,nodes,navigations};
}
test('authenticated project link goes straight to that isolated session', async()=>{
  const id='1'.repeat(32); const {run,navigations}=app(`#token=${'a'.repeat(64)}&project=${id}`);
  run(`tokenFromLaunchUrl(); workspaceApi = async (route, body) => { if(route !== '/api/workspaces/open' || body.id !== '${id}') throw Error('wrong project'); return {url:'http://127.0.0.1:1234/#token=child'}; };`);
  await run('initializeWorkspaces()');
  assert.deepEqual(navigations,['http://127.0.0.1:1234/#token=child']);
  assert.equal(run('requestedWorkspaceId'),null);
});
test('a project link needs a valid identifier, and works without a key',()=>{
  const {run:bad}=app(`#token=${'a'.repeat(64)}&project=../other`); bad('tokenFromLaunchUrl()'); assert.equal(bad('requestedWorkspaceId'),null);
  const {run:plain}=app(`#project=${'1'.repeat(32)}`); plain('tokenFromLaunchUrl()'); assert.equal(plain('requestedWorkspaceId'),'1'.repeat(32));
});
test('failed direct opening stays on chooser and explains the failure',async()=>{
  const {run,nodes,navigations}=app(`#token=${'a'.repeat(64)}&project=${'1'.repeat(32)}`);
  run("tokenFromLaunchUrl(); workspaceApi = async () => { throw Error('Project is in use'); }");
  await run('initializeWorkspaces()');
  assert.equal(navigations.length,0);
  assert.match(nodes.get('#workspace-status').textContent,/Project is in use.*Choose a project/);
});
test('a plain address opens the page in any browser, with no key',()=>{
  const id='3'.repeat(32);
  const context = vm.createContext({URLSearchParams, window:{location:{hash:'',pathname:'/',search:`?project=${id}`},addEventListener(){}},history:{replaceState(){}},sessionStorage:{setItem(){},getItem:()=>null},document:{body:{dataset:{}},querySelector:()=>({dataset:{},addEventListener(){}})}});
  vm.runInContext(source, context);
  assert.equal(vm.runInContext('tokenFromLaunchUrl()', context), null);
  assert.equal(vm.runInContext('requestedWorkspaceId', context), id);
});
test('a link with the key and project in the query, as glasstarn open prints it, opens that project',()=>{
  const token='c'.repeat(64), id='2'.repeat(32);
  const context = vm.createContext({URLSearchParams, window:{location:{hash:'',pathname:'/',search:`?token=${token}&project=${id}&view=map`},addEventListener(){}},history:{replaceState(){}},sessionStorage:{setItem(){},getItem:()=>null},document:{body:{dataset:{}},querySelector:()=>({dataset:{},addEventListener(){}})}});
  vm.runInContext(source, context);
  assert.equal(vm.runInContext('tokenFromLaunchUrl()', context), token);
  assert.equal(vm.runInContext('requestedWorkspaceId', context), id);
  assert.equal(vm.runInContext('requestedView', context), 'map');
});
