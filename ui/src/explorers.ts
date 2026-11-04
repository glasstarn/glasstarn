// Two complementary explorers share project scope, not diagram-detail state.
// Every number comes from /api/map or the active analysis; no browser mining.
let processLimitSelection = {mode:"limits",activities:0,connections:0};
let variantSelection = [1];
let variantView = null;
let variantViewAnalysis = null;
let variantCatalog = [];
let variantCatalogAnalysis = null;
let variantError = "";
let variantRequest = 0;
let variantLoading = false;
let variantListed = 10;
let variantSelectedStep = null;
// Whether the person clicked a step: only then does the map fade the rest.
// The step panel still describes the first step, but a whole route stays lit.
let variantStepPicked = false;
let variantMetric = "cases";
let connectedVariantFocus = null;
const explorerZoom = {process:1,variant:1};
// The routes on screen in the Variants list, for Select all and Clear.
let variantVisibleRanks = [];
// A route's steps in one short line: the first and last two steps, with the
// count of steps between when it is long.
function variantStepsText(sequence) {
  if (!sequence?.length) return "";
  if (sequence.length <= 4) return sequence.join(" → ");
  return `${sequence.slice(0, 2).join(" → ")} → ${sequence.length - 4} more → ${sequence.slice(-2).join(" → ")}`;
}

function resetExplorers() {
  variantRequest++; variantLoading=false; variantView=null; variantViewAnalysis=null;variantCatalog=[];variantCatalogAnalysis=null;variantError="";
  // With a filter on (often from a finding), every path in it is ticked, so
  // the map shows all the filtered items; otherwise the most common path.
  const filtered = typeof filterChips === "function" && filterChips(currentFilter || {}).length > 0;
  const paths = currentAnalysis?.summary?.variants || currentAnalysis?.variants?.length || 0;
  variantSelection = filtered && paths > 1 ? Array.from({ length: Math.min(paths, 200) }, (_, i) => i + 1) : [1];
  variantListed=10; variantSelectedStep=null; connectedVariantFocus=null;
  if(typeof subwayFocus!=="undefined")subwayFocus=null;
  if(typeof mapViewControlContext!=="undefined" && mapViewControlContext!==currentWorkspace?.id){subwayShownTypes=null;subwayHiddenEvents.clear();mapViewControlContext=currentWorkspace?.id;}
  processLimitSelection={mode:"limits",activities:0,connections:0};
  explorerZoom.process=1; explorerZoom.variant=1;
  const search=document.querySelector("#variant-search");if(search)search.value="";
  const focus=document.querySelector("#process-variant-focus");if(focus){focus.hidden=true;focus.replaceChildren();}
  const canvas=document.querySelector("#variant-map");canvas?.replaceChildren();
}

function scopedVariantRows() {
  if (variantCatalogAnalysis === currentAnalysis) return variantCatalog;
  return (currentAnalysis?.variants || []).map((item,index)=>({...item,rank:index+1}));
}

async function ensureVariantView() {
  if (!currentAnalysis || !currentProfile || !currentMapping || launchToken === null || variantLoading) return;
  if (!currentAnalysis.summary.cases) {renderVariantExplorer();return;}
  if (variantView && variantViewAnalysis === currentAnalysis && JSON.stringify(variantView.selection.variants) === JSON.stringify(variantSelection)) {renderVariantExplorer();return;}
  const request=++variantRequest, analysis=currentAnalysis, project=currentWorkspace, followed=currentFollow?.type;
  variantLoading=true; variantError=""; renderVariantExplorer();
  try {
    const response=await projectFetch("/api/map",{method:"POST",headers:{Authorization:`Bearer ${launchToken}`,"Content-Type":"application/json"},body:JSON.stringify(comparisonRequestBody({selection:{mode:"routes",variants:variantSelection}})),cache:"no-store"});
    const payload=await response.json();
    if(request!==variantRequest || analysis!==currentAnalysis || project!==currentWorkspace || followed!==currentFollow?.type)return;
    if(!response.ok)throw Error(payload.error || "The selected variants could not be loaded.");
    variantView=payload;variantViewAnalysis=analysis;variantCatalog=payload.variants;variantCatalogAnalysis=analysis;variantSelection=[...payload.selection.variants];
  } catch(error) {
    if(request!==variantRequest || analysis!==currentAnalysis)return;
    variantView=null;variantError=error.message;
    const status=document.querySelector("#variant-coverage");if(status)status.textContent=error.message;
  } finally {
    if(request===variantRequest){variantLoading=false;renderVariantExplorer();}
  }
}

function selectVariants(ranks) {
  const selected=[...new Set(ranks)].sort((a,b)=>a-b);
  if(!selected.length)return;
  variantSelection=selected;variantSelectedStep=null;variantStepPicked=false;
  // Invalidate in-flight results rather than displaying counts for old routes.
  variantRequest++;variantLoading=false;variantView=null;
  void ensureVariantView();
}

function variantScopeText() {
  return currentFollow ? `Variants of ${currentFollow.plural.toLowerCase()}` : "Variants";
}

function renderVariantExplorer() {
  const svg=document.querySelector("#variant-map");
  if(!svg || typeof document.createElementNS!=="function")return;
  const status=document.querySelector("#variant-coverage");
  const heading=document.querySelector("#variant-scope");if(heading)heading.textContent=variantScopeText();
  if(document.body?.dataset.view === "variants"){const title=document.querySelector("#page-title");if(title)title.textContent=variantScopeText();}
  const note=document.querySelector("#variant-scope-note");
  if(note)note.textContent=currentFollow ? `One ${currentFollow.singular} history defines a route. Linked record types remain separate; concurrent shipments or invoices are not flattened into one sequence.` : "Select one route or combine several. The diagram and counts follow your selection; project filters remain unchanged.";
  const rows=scopedVariantRows(), chosen=new Set(variantSelection);
  const search=(document.querySelector("#variant-search")?.value || "").trim().toLowerCase();
  const sorting=document.querySelector("#variant-sort")?.value || "frequency";
  const listed=rows.filter(item=>!search || item.sequence.some(step=>step.toLowerCase().includes(search))).slice();
  if(sorting!=="frequency")listed.sort((a,b)=>{
    if(!a.duration_eligible_cases || !b.duration_eligible_cases)return Number(!!b.duration_eligible_cases)-Number(!!a.duration_eligible_cases) || a.rank-b.rank;
    return (sorting==="slowest" ? -1 : 1)*(a.mean_throughput_seconds-b.mean_throughput_seconds) || a.rank-b.rank;
  });
  const list=document.querySelector("#variant-list");
  if(list){
    list.replaceChildren(...listed.slice(0,variantListed).map(item=>{
      const card=reportElement("div",undefined,"variant-option"+(chosen.has(item.rank)?" selected":""));
      const label=reportElement("label",undefined,"variant-check");
      const number=variantNumber(item.sequence,item.rank);
      const checkbox=reportElement("input");checkbox.type="checkbox";checkbox.checked=chosen.has(item.rank);checkbox.setAttribute("aria-label",`Include variant V${number}`);
      checkbox.addEventListener("change",()=>{if(!checkbox.checked && chosen.size===1){checkbox.checked=true;return;}selectVariants(checkbox.checked?[...variantSelection,item.rank]:variantSelection.filter(rank=>rank!==item.rank));});
      label.append(checkbox,reportElement("span",`V${number}`));
      const choose=reportElement("button",undefined,"variant-pick");choose.type="button";choose.setAttribute("aria-label",`Select only variant V${number}`);
      const title=reportElement("span",`${item.percent.toLocaleString(undefined,{maximumFractionDigits:1})}%`,"variant-share");
      const track=reportElement("span",undefined,"variant-track"), fill=reportElement("i");fill.style.width=`${Math.max(0,Math.min(100,item.percent))}%`;track.append(fill);
      const facts=reportElement("span",`${item.cases.toLocaleString()} ${itemLabels().plural.toLowerCase()} · ${measuredDuration(item.mean_throughput_seconds,item.duration_eligible_cases)} avg`,"variant-facts");
      // The route's own steps, so a row says what V4 is without opening it.
      const steps=reportElement("span",variantStepsText(item.sequence),"variant-steps");steps.title=item.sequence.join(" → ");
      choose.append(title,track,facts,steps);choose.addEventListener("click",()=>selectVariants([item.rank]));
      card.append(label,choose);return card;
    }));
    variantVisibleRanks=listed.slice(0,variantListed).map(item=>item.rank);
    if(!listed.length)list.append(reportElement("p",currentAnalysis?.summary?.cases===0?"No items match the active filters.":"No listed variant contains this activity.","empty"));
  }
  const count=document.querySelector("#variant-count");if(count)count.textContent=`${chosen.size.toLocaleString()} selected`;
  const more=document.querySelector("#variant-more");if(more){more.hidden=listed.length<=variantListed;more.disabled=variantLoading;}
  const listedNote=document.querySelector("#variant-list-note");
  if(listedNote){const total=currentAnalysis?.summary?.variants || 0;listedNote.textContent=`${Math.min(variantListed,listed.length)} of ${listed.length} matching listed variants. ${Math.max(0,total-rows.length)} further variants are outside this ranked list. Times describe first-to-last recorded elapsed time, not work effort.`;}
  const link=document.querySelector("#variant-open-process");if(link)link.disabled=!variantView || variantLoading;
  if(!variantView || variantViewAnalysis!==currentAnalysis || variantLoading){
    const message=variantError || (variantLoading?"Loading the selected variants…":currentAnalysis?.summary?.cases===0?"No items match these filters.":"Select a variant to inspect its recorded flow.");
    svg.replaceChildren();if(status)status.textContent=message;
    document.querySelector("#variant-detail")?.replaceChildren(reportElement("p",message));
    return;
  }
  if(status)status.textContent=`${variantView.variants_shown.toLocaleString()} selected of ${variantView.variants_total.toLocaleString()} variants · ${variantView.cases_covered.toLocaleString()} of ${variantView.cases_total.toLocaleString()} ${itemLabels().plural.toLowerCase()} (${variantView.percent_covered.toLocaleString(undefined,{maximumFractionDigits:1})}%)`;
  const view=variantView;
  if(!variantSelectedStep || !view.nodes.some(node=>node.activity===variantSelectedStep))variantSelectedStep=view.top_route[0];
  drawResultsMap(svg,view,{duration:variantMetric==="performance",metric:variantMetric,selection:variantStepPicked?variantSelectedStep:null,prefix:"variant",onNode:activity=>{variantSelectedStep=activity;variantStepPicked=true;renderVariantExplorer();},onEdge:edge=>renderVariantDetail(null,edge)});
  renderVariantDetail(view.nodes.find(node=>node.activity===variantSelectedStep));
  fitExplorer("variant");
}

function renderVariantDetail(node,edge=null) {
  const panel=document.querySelector("#variant-detail");if(!panel)return;
  panel.replaceChildren(reportElement("h3",edge?`${edge.from} → ${edge.to}`:node?.activity || "Select a step"));
  if(edge)panel.append(reportElement("p",`${edge.case_frequency.toLocaleString()} unique items · ${edge.frequency.toLocaleString()} occurrences · ${measuredDuration(edge.mean_seconds,edge.duration_eligible_occurrences)} average recorded interval`));
  else if(node)panel.append(reportElement("p",`${node.case_frequency.toLocaleString()} unique items · ${node.frequency.toLocaleString()} event occurrences`));
  for(const id of (edge?.example_cases || node?.example_cases || []).slice(0,3)){const button=reportElement("button",`Open ${id}`,"case-chip");button.type="button";button.addEventListener("click",()=>void openCase(id));panel.append(button);}
  const selected=scopedVariantRows().filter(item=>variantSelection.includes(item.rank));
  if(selected.length===1){const sequence=reportElement("details");sequence.append(reportElement("summary",`Exact recorded sequence · ${selected[0].sequence.length} steps`),reportElement("p",selected[0].sequence.join(" → ")));panel.append(sequence);}
}

// The process map runs to the bottom of the window, whatever sits above it
// (a header that wraps, chips on two lines): its height is what is left.
function sizeMapCanvas() {
  const canvas = document.querySelector(".map-workspace .mapwrap");
  if (!canvas || !canvas.getBoundingClientRect) return;
  const top = canvas.getBoundingClientRect().top;
  if (top <= 0) return;
  canvas.style.height = `${Math.max(420, Math.round(window.innerHeight - top - 10))}px`;
}

function fitExplorer(which) {
  if (which === "process") sizeMapCanvas();
  const ids=which==="variant"?["#variant-map"]:[activeMapStyle()==="subway"?"#subway-map":"#results-map"];
  for(const id of ids){const svg=document.querySelector(id),box=svg?.viewBox?.baseVal;if(!box?.width || !svg.parentElement?.clientWidth)continue;const availableHeight=Math.max(320,(svg.parentElement.clientHeight>200?svg.parentElement.clientHeight:window.innerHeight-260)-96);const base=explorerFitWidth(box.width,box.height,svg.parentElement.clientWidth,availableHeight,which==="process" && activeMapStyle()==="subway"?subwayOrientation:null);const width=base*explorerZoom[which];svg.style.maxWidth="none";svg.style.width=`${width}px`;svg.style.height=`${width/box.width*box.height}px`;if(id==="#subway-map")ensureSubwayHitAreas(svg);}
}
// The height kept free (96px) leaves room for the legend and summary that
// float over the top and bottom of the map.
// The drawn width that fits the canvas: the whole map in view, enlarged a
// little when there is room (the subway more than the classic map, whose
// pills would grow too big). A vertical subway is drawn at its natural size,
// never enlarged, and scrolls down.
function explorerFitWidth(width,height,canvasWidth,canvasHeight,subway){
  const byWidth=Math.max(120,canvasWidth-32);
  if(subway==="vertical")return Math.min(width,byWidth);
  const byHeight=canvasHeight/height*width;
  const fit=Math.min(width*(subway?1.6:1.15),byWidth,byHeight);
  // A subway never shrinks below 78%, so station names stay readable; a
  // wider map scrolls sideways instead.
  return subway?Math.max(fit,width*.78):fit;
}
function fitExplorers(){fitExplorer("process");fitExplorer("variant");}

function updateProcessExplorer(view) {
  const subway=activeMapStyle()==="subway";
  const title=document.querySelector("#process-scope");if(title)title.textContent=subway?"Connected process":currentFollow?`${currentFollow.plural} · recorded flow`:"Recorded process";
  const metric=document.querySelector("#process-metric");
  if(metric){const events=metric.querySelector?.('option[value="frequency"]');if(events)events.disabled=subway;if(subway && currentOverlay==="frequency")currentOverlay="cases";metric.value=currentOverlay;if(typeof syncMapMetric==="function")syncMapMetric();}
  const limits=document.querySelector("#process-controls");if(limits)limits.hidden=subway;
  const views=document.querySelector("#map-view-controls");if(views)views.hidden=!subway || !mapControlsOpen;
  const toggle=document.querySelector("#map-controls-toggle");if(toggle){toggle.hidden=!subway;toggle.setAttribute("aria-expanded",String(mapControlsOpen));}
  const banner=document.querySelector("#process-variant-focus");
  if(banner){banner.replaceChildren();banner.hidden=!connectedVariantFocus && !resultsRouteFocus && !(typeof mapFindingFocus!=="undefined" && mapFindingFocus);
    if(!connectedVariantFocus && !resultsRouteFocus && typeof mapFindingFocus!=="undefined" && mapFindingFocus){
      banner.append(reportElement("strong",`From Overview: ${mapFindingFocus.title}`),reportElement("p",mapFindingFocus.previous?"Only the items this finding is about are shown. Show the usual map to see them all again.":"Every step is drawn, with the one this finding is about picked out."));
      const overview=reportElement("button","Back to Overview","text-action");overview.type="button";overview.addEventListener("click",()=>showView("report"));
      const usual=reportElement("button","Show the usual map","text-action");usual.type="button";usual.addEventListener("click",backToUsualMap);
      banner.append(overview,usual);
    }
    if(connectedVariantFocus || resultsRouteFocus){
      const item=resultsRouteFocus?.item;
      banner.append(reportElement("strong",connectedVariantFocus?`${variantScopeText()} · selected route context`:item?`Path of ${itemLabels().singular} ${item}`:`Selected variants · ${variantSelection.map(rank=>"V"+rank).join(", ")}`),reportElement("p",connectedVariantFocus?"The selected object line and matching activity labels are highlighted. Surrounding lines retain the active project scope; this schematic does not reproduce every selected route. Open Variants for the exact flow.":item?(resultsRouteFocus.rank?"The map shows only this route; every item on it is counted.":"Every step is drawn; this item's path is lit."):"This map contains the selected variants only. Return to Main flow to explore activities and connections independently."));
      const back=reportElement("button",item?"Back to the item":"Back to Variants","text-action");back.type="button";back.addEventListener("click",()=>item?void openCase(item):showView("variants"));
      const clear=reportElement("button","Clear highlight","text-action");clear.type="button";clear.addEventListener("click",()=>{connectedVariantFocus=null;resultsRouteFocus=null;subwayFocus=null;void loadMapView(processLimitSelection);renderMap();});banner.append(back,clear);
    }
  }
  if(subway && connectedVariantFocus){const svg=document.querySelector("#subway-map");for(const station of svg?.querySelectorAll?.(".subway-station") || [])station.classList.toggle("variant-focus",connectedVariantFocus.activities.includes(station.dataset.activity) && station.dataset.lines.split("\u0001").includes(connectedVariantFocus.type));}
  if(view && !subway && view.selection.mode==="limits")processLimitSelection={...view.selection};
  if(typeof updateMeasurementHighlight === "function")updateMeasurementHighlight();
  fitExplorer("process");
}

async function showSelectedVariantsInProcess() {
  if(!variantView || variantViewAnalysis!==currentAnalysis || variantLoading)return;
  if(currentFollow && currentOcelAnalysis){
    resultsRouteFocus=null;mapStyle="subway";subwayFocus=currentFollow.type;
    connectedVariantFocus={type:currentFollow.type,activities:[...new Set(scopedVariantRows().filter(item=>variantSelection.includes(item.rank)).flatMap(item=>item.sequence))]};
    showView("map");renderMapStyle();renderSubway();return;
  }
  connectedVariantFocus=null;
  resultsRouteFocus={ranks:[...variantSelection],sequence:variantView.top_route,cases:variantView.cases_covered};resultsSelection=null;
  showView("map");await loadMapView({mode:"routes",variants:[...variantSelection]});
}

async function openExplorerVariant(rank) {
  const analysis=currentAnalysis;
  variantRequest++;variantLoading=false;variantSelection=[rank];variantView=null;
  await ensureVariantView();
  if(analysis===currentAnalysis && variantSelection.length===1 && variantSelection[0]===rank)await showSelectedVariantsInProcess();
}

function initializeExplorers() {
  document.querySelector("#process-metric")?.addEventListener("change",event=>{currentOverlay=event.target.value;renderMap();});
  document.querySelector("#variant-metric")?.addEventListener("change",event=>{variantMetric=event.target.value;renderVariantExplorer();});
  for(const id of ["variant-search","variant-sort"])document.querySelector("#"+id)?.addEventListener("input",()=>renderVariantExplorer());
  document.querySelector("#variant-more")?.addEventListener("click",()=>{variantListed+=10;renderVariantExplorer();});
  // Clear goes back to the most common route on the list: the map always shows at least one.
  document.querySelector("#variant-reset")?.addEventListener("click",()=>selectVariants([variantVisibleRanks[0] || 1]));
  document.querySelector("#variant-all")?.addEventListener("click",()=>selectVariants(variantVisibleRanks.length?[...variantVisibleRanks]:scopedVariantRows().map(item=>item.rank)));
  document.querySelector("#variant-open-process")?.addEventListener("click",()=>void showSelectedVariantsInProcess());
  for(const [id,all] of [["process-reset",false],["process-all",true]])document.querySelector("#"+id)?.addEventListener("click",()=>{resultsRouteFocus=null;connectedVariantFocus=null;if(typeof mapFindingFocus!=="undefined")mapFindingFocus=null;void loadMapView({mode:"limits",activities:all?(currentMapView?.activities_total || currentAnalysis?.summary?.activities || 0):0,connections:all?(currentMapView?.connections_total || 0):0});});
  for(const button of document.querySelectorAll("[data-explorer-zoom]"))button.addEventListener("click",()=>{const target=button.dataset.explorerZoom;explorerZoom[target]=button.dataset.zoom==="fit"?1:Math.max(.5,Math.min(4,explorerZoom[target]*(button.dataset.zoom==="in"?1.25:.8)));fitExplorer(target);});
  window.addEventListener("resize",fitExplorers);
}
