const assert=require('node:assert/strict');
const vm=require('node:vm');
const {test}=require('node:test');
const fs=require('node:fs');
const path=require('node:path');
const source=require('./app-source.cjs');
const fixture=JSON.parse(fs.readFileSync(path.join(__dirname,'../../testdata/consultant/invoice-process.json'),'utf8'));
function app() {
  function element(tag) { return {tag,children:[],attributes:{},listeners:{},style:{},className:'',textContent:'',
    append(...items){this.children.push(...items)},replaceChildren(...items){this.children=items},
    setAttribute(k,v){this.attributes[k]=v},addEventListener(k,v){this.listeners[k]=v},classList:{add(){}},
    set innerHTML(_){throw Error('untrusted HTML')},
  }; }
  const parent=element('main');
  const ctx=vm.createContext({document:{createElement:element,createElementNS:(_,tag)=>element(tag)},parent,model:structuredClone(fixture)});
  vm.runInContext(source,ctx);
  vm.runInContext(`citationButton=reference=>reportElement('button','Evidence: '+reference);renderWorkingProcess(parent,{process_model:model,intention:'improve'});`,ctx);
  const text=node=>(node.textContent||'')+' '+(node.children||[]).map(text).join(' ');
  return {parent,ctx,text};
}
test('combined process shows explicit sources and unknown manual timing without changing recorded history',()=>{
  const {parent,text}=app();const section=parent.children[0];
  assert.match(text(section),/COMBINED PROCESS UNDERSTANDING/);
  const controls=section.children.find(child=>child.className==='process-step-controls');
  assert.equal(controls.children.length,3);
  controls.children[1].listeners.click();
  const detail=section.children.find(child=>child.className==='process-step-detail');
  assert.match(text(detail),/Synthetic invoice SOP.*v2/);
  assert.match(text(detail),/Finance lead's account/);
  // Suggestions stay in the chat: the browser shows sources, not treatments.
  assert.doesNotMatch(text(detail),/Proposed treatment|Agent-assisted/);
  assert.match(text(section),/No timestamps establish this/);
  assert.equal(controls.children[1].attributes['aria-pressed'],'true');
  assert.equal(controls.children[0].attributes['aria-pressed'],'false');
});
test('diagram arrows represent only supplied relationships and origin styles',()=>{
  const {parent}=app(); const section=parent.children[0];
  const svg=section.children.find(child=>child.tag==='svg');
  assert.equal(svg.attributes.role,'img');
  const connections=svg.children.filter(child=>child.tag==='path');
  assert.equal(connections.length,2);
  assert.equal(connections[0].attributes.class,'working-connection documented');
  assert.equal(connections[1].attributes.class,'working-connection reported');
  const groups=svg.children.filter(child=>child.tag==='g');
  assert.equal(groups.length,3);
  assert.equal(groups[1].attributes.class,'working-step reported');
});
test('labels and source text stay inert',()=>{
  const {parent,ctx,text}=app();
  vm.runInContext(`model.steps[1].label='<img src=x onerror=run()> ';parent.replaceChildren();renderWorkingProcess(parent,{process_model:model});`,ctx);
  assert.match(text(parent),/<img src=x onerror=run\(\)>/);
});

test('a proposed future process is not shown in the browser: improvement stays in the chat',()=>{
  const {parent,ctx,text}=app();
  vm.runInContext(`model.view='proposed_future';parent.replaceChildren();renderWorkingProcess(parent,{process_model:model});`,ctx);
  assert.equal(parent.children.length,0);
  assert.equal(vm.runInContext(`typeof renderImprovementProposal`,ctx),'undefined');
  assert.equal(vm.runInContext(`viewNames.improve`,ctx),undefined);
});
