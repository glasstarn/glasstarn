const assert=require('node:assert/strict');
const vm=require('node:vm');
const {test}=require('node:test');
const source=require('./app-source.cjs');
const variants=[
 {rank:1,sequence:['Receive','Pay'],cases:6,percent:60,duration_eligible_cases:6,mean_throughput_seconds:86400,example_cases:['A1']},
 {rank:2,sequence:['Receive','Review','Pay'],cases:3,percent:30,duration_eligible_cases:3,mean_throughput_seconds:172800,example_cases:['B1']},
 {rank:3,sequence:['Receive','Reject'],cases:1,percent:10,duration_eligible_cases:0,mean_throughput_seconds:0,example_cases:['C1']}
];
function response(ranks){
 const chosen=variants.filter(v=>ranks.includes(v.rank)),cases=chosen.reduce((sum,v)=>sum+v.cases,0);
 const names=[...new Set(chosen.flatMap(v=>v.sequence))];
 return {selection:{mode:'routes',variants:ranks},variants:variants.map(v=>({...v,shown:ranks.includes(v.rank)})),top_route:chosen[0].sequence,nodes:names.map(activity=>({activity,frequency:cases,case_frequency:cases,example_cases:['B1']})),edges:[],cases_covered:cases,cases_total:10,percent_covered:cases*10,variants_shown:ranks.length,variants_total:3};
}
function app(){
 function element(tag='div'){return {tag,children:[],listeners:{},attributes:{},dataset:{},style:{},textContent:'',value:'',hidden:false,
 append(...items){this.children.push(...items)},replaceChildren(...items){this.children=items},setAttribute(k,v){this.attributes[k]=v},removeAttribute(k){delete this.attributes[k]},addEventListener(k,v){this.listeners[k]=v},classList:{add(){},toggle(){}},querySelectorAll(){return[]}};}
 const elements=new Map();
 const context=vm.createContext({document:{body:{dataset:{view:'variants'}},querySelector(selector){if(!elements.has(selector))elements.set(selector,element());return elements.get(selector)},querySelectorAll(){return[]},createElement:element,createElementNS:(_,tag)=>element(tag)},variants});
 vm.runInContext(source,context);
 const run=code=>vm.runInContext(code,context);
 run(`currentAnalysis={summary:{cases:10,events:23,variants:3},variants};currentProfile={upload_id:'same'};currentMapping={case_id:'invoice'};currentWorkspace={id:'project'};currentFollow=null;currentFilter={};launchToken='local';
   comparisonRequestBody=extra=>({upload_id:currentProfile.upload_id,filter:currentFilter,...extra});drawResultsMap=(svg,view,options)=>{globalThis.drawn={view,metric:options.metric}};`);
 const text=node=>typeof node==='string'?node:(node.textContent||'')+' '+(node.children||[]).map(text).join(' ');
 return {run,context,elements,text};
}

test('Variants is an independent exact selection with engine coverage and a ranked distribution',async()=>{
 const {run,context,elements,text}=app();context.payload=response([2]);
 run(`variantSelection=[2];projectFetch=async()=>({ok:true,json:async()=>payload});`);
 await run('ensureVariantView()');
 assert.equal(run('drawn.view.cases_covered'),3);
 assert.equal(run('currentMapSelection.mode'),'limits','Process controls are independent');
 assert.match(text(elements.get('#variant-coverage')),/3 of 10 invoices \(30%\)/);
 assert.match(text(elements.get('#variant-detail')),/Receive → Review → Pay/);
 const rows=elements.get('#variant-list').children;
 assert.equal(rows.length,3);
 assert.equal(rows[1].children[0].children[0].checked,true);
 assert.equal(rows[0].children[0].children[0].checked,false);
 assert.equal(rows[1].children[1].attributes['aria-label'],'Select only variant V2');
});

test('multiple selected variants use the engine union, not a sum of displayed graph nodes',async()=>{
 const {run,context,elements,text}=app();context.payload=response([1,2]);
 run(`variantSelection=[1,2];projectFetch=async()=>({ok:true,json:async()=>payload});`);
 await run('ensureVariantView()');
 assert.match(text(elements.get('#variant-coverage')),/2 selected of 3 variants · 9 of 10 invoices \(90%\)/);
 assert.equal(run('variantSelection.length'),2);
 run(`document.querySelector('#variant-search').value='Reject';renderVariantExplorer();`);
 assert.equal(elements.get('#variant-list').children.length,1);
 assert.match(text(elements.get('#variant-list')),/V3/);
});

test('a response from the previous filter or object selection cannot repopulate the diagram',async()=>{
 const {run,context}=app();context.payload=response([1]);
 run(`projectFetch=()=>new Promise(resolve=>globalThis.resolveOld=resolve);`);
 const pending=run('ensureVariantView()');
 run(`currentAnalysis={summary:{cases:0,events:0,variants:0},variants:[]};resetExplorers();resolveOld({ok:true,json:async()=>payload});`);
 await pending;
 assert.equal(run('variantView'),null);
 assert.equal(run('variantLoading'),false);
});

test('connected variant selection keeps the subway and labels surrounding context without changing filters',async()=>{
 const {run,context}=app();context.payload=response([2]);
 run(`variantView=payload;variantViewAnalysis=currentAnalysis;variantCatalog=payload.variants;variantCatalogAnalysis=currentAnalysis;variantSelection=[2];
 currentFollow={type:'invoice',singular:'invoice',plural:'Invoices'};currentOcelAnalysis={};currentFilter={attribute_equals:[['customer','ABC']]};
 globalThis.originalFilter=currentFilter;globalThis.opened=null;showView=view=>opened=view;renderMapStyle=()=>{};renderSubway=()=>{};`);
 await run('showSelectedVariantsInProcess()');
 assert.equal(run('opened'),'map');assert.equal(run('mapStyle'),'subway');
 assert.equal(run('connectedVariantFocus.type'),'invoice');
 assert.deepEqual([...run('connectedVariantFocus.activities')],['Receive','Review','Pay']);
 assert.equal(run('currentFilter===originalFilter'),true);
 assert.equal(run('resultsRouteFocus'),null);
});

test('Process coverage distinguishes complete visible routes from counts over the whole active selection',()=>{
 const {run,elements,text}=app();
 run(`renderResultsControls({selection:{mode:'limits',activities:7,connections:6},activities_shown:7,activities_total:12,connections_shown:6,connections_total:14,variants_shown:1,variants_total:5,variants:[],cases_covered:50,cases_total:100,percent_covered:50});`);
 assert.match(text(elements.get('#results-coverage')).replace(/\s+/g,' ').trim(),/50% of invoices follow a route shown here/);
 assert.match(elements.get('#results-coverage').dataset.detail,/counts use all 100 invoices/);
});

test('Unique items and event occurrences draw different values on repeated steps and connections',()=>{
 const {run,elements,text}=app();
 run(`drawResultsMap=undefined;`);
 // Re-load only the real drawer; the orchestration tests stub it separately.
 const drawing=source.slice(source.indexOf('function drawResultsMap('),source.indexOf('\n// Only side steps',source.indexOf('function drawResultsMap(')));
 run(drawing);
 run(`globalThis.repeated={top_route:['A','B'],activity_ranking:[{activity:'A',rank:1},{activity:'B',rank:2}],nodes:[{activity:'A',frequency:8,case_frequency:3,starts:3,ends:0},{activity:'B',frequency:7,case_frequency:3,starts:0,ends:3}],edges:[{from:'A',to:'B',frequency:7,case_frequency:3,duration_eligible_occurrences:7,mean_seconds:60}]};
 drawResultsMap(document.querySelector('#unique'),repeated,{duration:false,metric:'cases',prefix:'unique',onNode:()=>{},onEdge:()=>{}});
 drawResultsMap(document.querySelector('#events'),repeated,{duration:false,metric:'frequency',prefix:'events',onNode:()=>{},onEdge:()=>{}});`);
 const nodes=element=>element.children.find(child=>child.children?.some(grandchild=>grandchild.attributes?.class?.startsWith('node')))?.children || [];
 assert.equal(nodes(elements.get('#unique'))[0].children.at(-1).textContent,'3');
 assert.equal(nodes(elements.get('#events'))[0].children.at(-1).textContent,'8');
 assert.match(text(elements.get('#events')),/7/);
});

test('an empty filtered view clears the previous variant counts and examples',()=>{
 const a=app();
 a.run(`variantView=${JSON.stringify(response([1]))};variantViewAnalysis=currentAnalysis;renderVariantExplorer();`);
 assert.match(a.text(a.elements.get('#variant-detail')),/6 unique items/);
 a.run(`currentAnalysis={summary:{cases:0,events:0,variants:0},variants:[]};resetExplorers();renderVariantExplorer();`);
 const detail=a.text(a.elements.get('#variant-detail'));
 assert.match(detail,/No items match/);
 assert.doesNotMatch(detail,/6 unique items|Open B1/);
});

test('a new followed scope clears the line focus left by a selected variant',()=>{
 const a=app();
 a.run(`subwayFocus='order';connectedVariantFocus={type:'order',activities:['Receive']};resetExplorers();`);
 assert.equal(a.run('subwayFocus'),null);
 assert.equal(a.run('connectedVariantFocus'),null);
});
