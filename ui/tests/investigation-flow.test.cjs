const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');
function app() {
  const elements = new Map();
  function element() { return {dataset:{},style:{},children:[],listeners:{},attributes:{},textContent:'',
    append(...items){this.children.push(...items)},replaceChildren(...items){this.children=items},
    setAttribute(k,v){this.attributes[k]=v},removeAttribute(k){delete this.attributes[k]},toggleAttribute(k,v){if(v)this.attributes[k]="";else delete this.attributes[k]},
    addEventListener(k,v){this.listeners[k]=v},classList:{toggle(){},remove(){}},
  }; }
  const context=vm.createContext({document:{querySelector(selector){if(!elements.has(selector))elements.set(selector,element());return elements.get(selector)},querySelectorAll(){return[]},createElement:element,createTextNode:text=>({textContent:text})}});
  vm.runInContext(source,context);
  const run=code=>vm.runInContext(code,context);
  const text=node=>typeof node==='string'?node:node.textContent||(node.children||[]).map(text).join(' ');
  return {run,elements,text};
}
test('empty map removes old diagram, evidence and route controls before a subway redraw',()=>{
  const {run,elements,text}=app();
  run(`currentAnalysis={summary:{cases:0}}; currentOcelAnalysis={}; currentOverlay='performance'; document.querySelector('#results-detail').textContent=''; document.querySelector('#results-detail').append(document.createTextNode('2000 old orders')); renderResultsMap();`);
  assert.match(text(elements.get('#results-detail')),/No items match/);
  assert.doesNotMatch(text(elements.get('#results-detail')),/2000/);
  assert.equal(elements.get('#results-stepper').hidden,true);
  assert.equal(elements.get('#subway-map').children.length,0);
});
test('route selection opens its actual sequence in classic view without changing the preferred map style',async()=>{
  const {run}=app();
  run(`currentOcelAnalysis={}; mapStyle='subway'; currentAnalysis={variants:[{sequence:['A','B']},{sequence:['A','C','A','D']}]}; showView=view=>globalThis.opened=view; renderMapStyle=()=>{}; loadMapView=async selection=>{globalThis.requested=selection};`);
  await run('openResultsRoute(2)');
  assert.equal(run('opened'),'map'); assert.equal(run('activeMapStyle()'),'classic');
  assert.equal(run('mapStyle'),'subway'); assert.equal(run('requested.variants[0]'),2);
  assert.deepEqual([...run('resultsRouteFocus.sequence')],['A','C','A','D']);
});
test('a finding opens the named step even when it is absent from the top route',async()=>{
  const {run}=app();
  run(`currentAnalysis={summary:{activities:30},edges:Array(60)}; showView=()=>{}; renderMapStyle=()=>{}; loadMapView=async selection=>{globalThis.requested=selection}; resultsSelection='Top route step';`);
  await run(`openFindingEvidence({view:'map',target:{kind:'step',activity:'Record Service Entry Sheet'}})`);
  assert.equal(run('resultsSelection'),'Record Service Entry Sheet');
  assert.equal(run('requested.mode'),'limits'); assert.equal(run('requested.activities'),30);
});
test('strict numeric filters and unreached evidence stay legible and removable',()=>{
  const {run}=app();
  const chips=run(`filterChips({attribute_between:[{attribute:'amount',greater_than:10,less_than:30}],not_reached:[['Issue','Pay']]})`);
  assert.deepEqual([...chips].map(chip=>chip.text),['Issue without a later Pay','amount > 10 and < 30']);
  assert.equal(chips[0].next.attribute_between.length,1);
});
test('saved charts render calculated rows as text, retain citations and open the example',async()=>{
  const {run,elements,text}=app();
  run(`citationButton=reference=>{const button=reportElement('button',reference);return button}; globalThis.opened=null; openCase=async id=>{opened=id};
    renderInvestigationVisuals(document.querySelector('#charts'),{visuals:[{title:'Amount by customer',kind:'bar',metric:'total',field:'amount',evidence:'exports/query-1.json',request:{type:'values'},rows:[{label:'<img onerror=bad>',value:120,cases:2,cases_with_value:2,example_case:'INV-1'}]}]});`);
  const chart=elements.get('#charts'); assert.match(text(chart),/120/);assert.match(text(chart),/exports\/query-1.json/);
  assert.match(text(chart),/<img onerror=bad>/);
  const row=chart.children[0].children[2].children[0];
  await row.children.at(-1).listeners.click();
  assert.equal(run('opened'),'INV-1');
});
test('connected following starts with the source identifier before any case profile exists',async()=>{
  const {run}=app();
  run(`currentProfile=null; currentOcelUpload='connected-source'; launchToken='local'; followTypes=['Order'];
    renderFollowBar=()=>{}; resetFilterControls=()=>{}; loadProvenance=async()=>{};
    projectFetch=async(path,options)=>{globalThis.followBody=JSON.parse(options.body); return {ok:true,json:async()=>({upload_id:'followed',mapping:{case_id:'case_id'},follow:{cases:2}})}};
    analyzeMapping=async(mapping,filter)=>{globalThis.followFilter=filter;return true};`);
  assert.equal(await run(`followType('Order')`),true);
  assert.equal(run('followBody.upload_id'),'connected-source');
  assert.equal(run('followBody.preserve_scope'),false);
  assert.equal(run('currentProfile.upload_id'),'followed');
  assert.equal(await run(`followType('Invoice',{not_reached:[['Issue','Pay']]},true)`),true);
  assert.equal(run('followBody.preserve_scope'),true);
  assert.equal(run('followFilter.not_reached[0][1]'),'Pay');
});
test('late connected scope responses cannot replace the latest filtered journey',async()=>{
  const {run}=app();
  run(`globalThis.requests=[]; currentProfile={upload_id:'followed'};currentOcelUpload='source';currentFollow={type:'Order'};launchToken='local';currentFilter={};
    comparisonRequestBody=()=>({upload_id:currentProfile.upload_id,filter:currentFilter});
   rerenderSummary=()=>{};renderResultsMap=()=>{};
    projectFetch=(path,options)=>new Promise(resolve=>requests.push({resolve,body:JSON.parse(options.body)}));`);
  const old=run(`loadJourney('Order')`);
  run(`currentFilter={attribute_equals:[['customer','ABC']]};`);
  const latest=run(`loadJourney('Order')`);
  run(`requests[1].resolve({ok:true,json:async()=>({journey:{stages:[],findings:[]},analysis:{scope:{objects_shown:0}}})});`);
  await latest;
  run(`requests[0].resolve({ok:true,json:async()=>({journey:{stages:[{},{}],findings:[{title:'Old global finding'}]},analysis:{scope:{objects_shown:2000}}})});`);
  await old;
  assert.equal(run('currentOcelAnalysis.scope.objects_shown'),0);
  assert.equal(run('currentJourney.findings.length'),0);
  assert.equal(run('requests[1].body.case_view.filter.attribute_equals[0][1]'),'ABC');
});
test('the metric toggle render path updates the subway pill and never redraws an empty cohort',()=>{
  const {run}=app();
  run(`globalThis.buttons=[{dataset:{resultsMetric:'frequency'},setAttribute(k,v){this.pressed=v}},{dataset:{resultsMetric:'performance'},setAttribute(k,v){this.pressed=v}}];
    document.querySelectorAll=selector=>selector==='[data-results-metric]'?buttons:[];
    renderMapStyle=()=> 'subway'; globalThis.draws=0;renderSubway=()=>draws++;
    currentOcelAnalysis={};currentAnalysis={summary:{cases:1}};currentOverlay='frequency';currentModelView='observed';renderMap();`);
  assert.equal(run('buttons[0].pressed'),'true');assert.equal(run('buttons[1].pressed'),'false');assert.equal(run('draws'),1);
  run(`currentOverlay='performance';renderMap();`);
  assert.equal(run('buttons[0].pressed'),'false');assert.equal(run('buttons[1].pressed'),'true');assert.equal(run('draws'),2);
  run(`currentAnalysis.summary.cases=0;renderMap();`);
  assert.equal(run('draws'),2,'an empty selection must not repopulate the previous connected diagram');
});
test('saved agent updates are observed while Improve is open, without navigation or reimport',async()=>{
  const {run,elements,text}=app();
  run(`document.body={dataset:{view:'improve'}};document.hidden=false;currentWorkspace={id:'same-project'};lastReport={artifact:'old'};
    globalThis.refreshed=0;globalThis.poll=null;clearInterval=()=>{};setInterval=callback=>{poll=callback;return 1};
    workspaceApi=async()=>({artifact:'new'});refreshExecutiveReport=async()=>{refreshed++;lastReport.artifact='new'};watchForAnswers();`);
  await run('poll()');
  assert.equal(run('refreshed'),1);
  assert.equal(run('document.body.dataset.view'),'improve');
  assert.match(text(elements.get('#consultant-update')),/Updated from your agent: its latest answer is now at the top/);
  await run('poll()');
  assert.equal(run('refreshed'),1,'unchanged records do not redraw the proposal');
});
