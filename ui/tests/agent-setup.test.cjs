const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');
function app() {
  const nodes = new Map();
  const node = selector => { if (!nodes.has(selector)) nodes.set(selector,{textContent:'',value:'',disabled:false,hidden:false,dataset:{}});return nodes.get(selector); };
  const context = vm.createContext({document:{querySelector:node}});
  const run = code => vm.runInContext(code,context);
  run(source);
  node('#agent-client').value='codex';
  return {run,node,context};
}
const setup={enabled:true,project_name:'Audit',analysis_ready:false,policy:{activity_names:'alias'},prompt:'Start with evidence',contact:null,codex_command:'codex setup',codex_config:'toml',shell:'Terminal'};
test('enabling access does not claim an agent connected; contact needs protocol evidence',()=>{
  const {run,node,context}=app();context.fixture=setup;run('renderAgentSetup(fixture)');
  assert.equal(node('#agent-contact').textContent,'Not connected yet.');
  assert.match(node('#agent-access-status').textContent,/Import an event log/);
  assert.match(node('#agent-scope').textContent,/never your raw rows/);
  assert.equal(node('#agent-enable').hidden,true);
  context.fixture={...setup,contact:{initialized:true,client:'Test client',last_seen:1,tools_listed:true}};
  run('renderAgentSetup(fixture)');
  assert.match(node('#agent-contact').textContent,/^Connected to Test client\. Last contact /);
});
test('disabling access hides setup and permits a new choice',()=>{
  const {run,node,context}=app();context.fixture={...setup,enabled:false,policy:null};run('renderAgentSetup(fixture)');
  assert.equal(node('#agent-setup').hidden,true);
  assert.equal(node('#agent-enable').disabled,false);
  assert.equal(node('#agent-disable').disabled,true);
  assert.equal(node('#agent-preset').disabled,false);
  assert.equal(node('#agent-disable').hidden,true);
  assert.equal(node('#agent-contact').textContent,'');
});
test('a rejected enable shows an error without displaying a successful connection',async()=>{
  const {run,node}=app();
  run(`workspaceApi = async () => { throw new Error('Could not save access settings'); };`);
  await run('changeAgentAccess(true)');
  assert.match(node('#agent-access-status').textContent,/Could not save/);
  assert.equal(node('#agent-enable').disabled,false);
  assert.equal(run('agentSetup'),null);
});
test('the top button says Agent connected only once an agent has made contact',()=>{
  const {run,node,context}=app();context.fixture=setup;run('renderAgentSetup(fixture)');
  assert.equal(node('#app-agent').textContent,'Connect an agent');
  context.fixture={...setup,contact:{initialized:true,client:'Test client',last_seen:1,tools_listed:true}};
  run('renderAgentSetup(fixture)');
  assert.equal(node('#app-agent').textContent,'Agent connected');
  context.fixture={...setup,enabled:false,policy:null,contact:{initialized:true}};
  run('renderAgentSetup(fixture)');
  assert.equal(node('#app-agent').textContent,'Connect an agent');
});
