// Map interactions only. All durations, groups and eligibility come from pm-query.
function studioCurrentScope(){return JSON.stringify({filter:currentFilter,follow:currentFollow?.type || null,ocel:currentOcelAnalysis?.filter || null});}
function clearStudioAnswer(){
  studioRevision++;studioReport=null;studioQuery=null;
  for(const id of ['cards','histogram','groups','group-chart','contrasts','evidence'])studioElement(id).replaceChildren();
  studioElement('chart-unit').textContent='';studioElement('selection').hidden=true;studioElement('comparison').hidden=true;studioElement('apply').disabled=true;studioElement('save').disabled=true;
}
function setStudioTab(name){
  studioTab=name;
  for(const button of document.querySelectorAll('[data-studio-tab]'))button.setAttribute('aria-selected',String(button.dataset.studioTab===name));
  for(const pane of document.querySelectorAll('[data-studio-pane]'))pane.hidden=pane.dataset.studioPane!==name;
}
function showMapStepDetail(){
  studioOpen=false;studioPicking=null;studioRevision++;
  document.querySelector('#analysis-inspector').hidden=true;document.querySelector('#saved-views-panel').hidden=true;
  document.querySelector('#map-step-detail').hidden=!mapHasSelection();updateMeasurementHighlight();
}
function openMapAnalysis(mode=null){
  prepareAnalysisStudio();studioOpen=true;
  document.querySelector('#analysis-inspector').hidden=false;document.querySelector('#map-step-detail').hidden=true;document.querySelector('#saved-views-panel').hidden=true;
  if(mode){studioElement('mode').value=mode;studioPicking=null;studioRangeOverride=null;studioElement('min').value='';studioElement('max').value='';clearStudioAnswer();}
  studioDefinition();studioElement('title').textContent=studioElement('mode').value==='whole'?'Recorded history time':'Time between steps';
  setStudioTab(studioTab);updateMeasurementHighlight();fitExplorer('process');
  if(mode==='whole')void runAnalysisStudio();
}
function beginMapMeasurement(activity=null){
  openMapAnalysis(currentFollow && activeMapStyle()==='subway'?'linked':'segment');
  studioElement('from').value=activity || '';studioElement('to').value='';
  studioElement('from-occurrence').value='first';studioElement('to-occurrence').value='last';
  studioPicking=activity?'to':'from';setStudioTab('time');
  studioElement('status').textContent='';studioElement('range-editor').open=false;
  updateMeasurementHighlight();
}
function pickMapMeasurement(activity,types=[]){
  if(!studioOpen || !studioPicking)return false;
  const endpoint=studioPicking;studioElement(endpoint).value=activity;
  if(studioElement('mode').value==='linked' && types.length){
    const choice=types.includes(currentFollow?.type)?currentFollow.type:types[0];studioElement(endpoint+'-type').value=choice;
  }
  studioPicking=endpoint==='from'?'to':null;studioDefinition();updateMeasurementHighlight();
  if(!studioPicking)void runAnalysisStudio();
  return true;
}
function measurementBadgePoint(node){
  const stop=node.querySelector?.('.stop');
  if(stop && typeof stop.getBBox==='function'){const box=stop.getBBox();return [box.x+box.width/2,box.y+Math.min(box.height,box.width)/2];}
  const pill=node.querySelector?.('rect.pill');
  if(pill && typeof pill.getBBox==='function'){const box=pill.getBBox();return [box.x,box.y+box.height/2];}
  return null;
}
function updateMeasurementHighlight(){
  const from=studioOpen?studioElement('from')?.value:null,to=studioOpen?studioElement('to')?.value:null;
  const isInterval=studioElement('mode')?.value!=='whole';
  for(const node of document.querySelectorAll('#results-map [data-activity],#subway-map [data-activity]')){
    const which=isInterval && node.dataset.activity===from?'A':isInterval && node.dataset.activity===to?'B':'';
    node.classList.toggle('measurement-endpoint',!!which);if(node.classList.contains('node'))node.classList.toggle('sel',!studioOpen && node.dataset.activity===resultsSelection);node.setAttribute('data-measurement-endpoint',which);
    node.querySelector('.measurement-badge')?.remove();
    // The letter sits in a small filled disc on the step itself: on the
    // subway stop, or at the left end of a classic pill.
    if(which){
      const at=measurementBadgePoint(node);
      if(at){const badge=svgElement('g',{class:'measurement-badge'});badge.append(svgElement('circle',{cx:at[0],cy:at[1],r:from===to?13:10}));const label=svgElement('text',{x:at[0],y:at[1]+4,'text-anchor':'middle'});label.textContent=from===to?'A/B':which;badge.append(label);node.append(badge);}
    }
  }
  const picking=studioElement('picking');if(picking){picking.hidden=!studioPicking;picking.textContent=studioPicking==='from'?'Select the starting step on the map.':'Select the ending step on the map.';}
  document.querySelector('#measured-process-map')?.classList.toggle('measuring',!!studioPicking);
  // Until both steps are chosen there is nothing to show in the tabs.
  const inspector=document.querySelector('#analysis-inspector');if(inspector?.dataset){if(studioPicking)inspector.dataset.picking=studioPicking;else delete inspector.dataset.picking;}
}
async function refreshMapAnalysis(){
  if(!studioOpen)return;
  prepareAnalysisStudio();updateMeasurementHighlight();await runAnalysisStudio();
}
async function saveStudioView(){
  if(!currentWorkspace || !studioReport || !studioQuery)return;
  if(studioScope!==studioCurrentScope() || JSON.stringify(studioRequest())!==JSON.stringify(studioQuery)){
    studioElement('status').textContent='The selection changed. Refresh the measurement before saving.';return;
  }
  const segment=studioQuery.linked?.segment || studioQuery.segment;
  const title=segment?`${segment.from} → ${segment.to}`:'Whole recorded history';
  studioElement('save').disabled=true;
  try{
    await workspaceApi('/api/workspace/views',{title:title.slice(0,160),exploration:studioQuery,filter:currentFilter,object_type:currentFollow?.type || null});
    studioElement('status').textContent=`Saved “${title}”. Find it under Saved views (the bookmark icon).`;
  }catch(error){studioElement('status').textContent=`Could not save: ${error.message}`;}
  finally{studioElement('save').disabled=!studioReport;}
}
function renderSavedAgentVisuals(){
  const container=document.querySelector('#saved-agent-visuals');if(!container)return;
  container.replaceChildren();
  if(currentInvestigationRecord?.visuals?.length){container.append(reportElement('h4','From your agent'));renderInvestigationVisuals(container,currentInvestigationRecord);}
}
async function openSavedViews(){
  studioOpen=false;studioPicking=null;studioRevision++;
  document.querySelector('#analysis-inspector').hidden=true;document.querySelector('#map-step-detail').hidden=true;document.querySelector('#saved-views-panel').hidden=false;updateMeasurementHighlight();
  const list=document.querySelector('#saved-views-list'),status=document.querySelector('#saved-views-status'),project=currentWorkspace;
  list.replaceChildren();status.textContent='Loading saved views…';renderSavedAgentVisuals();
  try{
    const data=await workspaceApi('/api/workspace/views');if(project!==currentWorkspace)return;
    status.textContent=data.views.length?'Open one to bring back its filters and measure again.':'No saved views yet. Measure something, then choose Save view.';
    for(const view of data.views){
      const row=reportElement('div',undefined,'saved-view-row');
      const button=reportElement('button',view.title,'saved-view');button.type='button';button.disabled=!view.compatible;
      button.append(reportElement('small',view.compatible?'Open and measure again':view.same_source?'Made with earlier process settings':'Made from an earlier data file'));
      button.addEventListener('click',()=>void restoreStudioView(view));row.append(button);
      if(view.id){
        const remove=reportElement('button','×','saved-view-remove');remove.type='button';remove.setAttribute('aria-label',`Remove saved view: ${view.title}`);
        remove.addEventListener('click',()=>void removeSavedView(view,row));row.append(remove);
      }
      list.append(row);
    }
  }catch(error){status.textContent=error.message;}
}
async function removeSavedView(view,row){
  const status=document.querySelector('#saved-views-status');
  try{
    await workspaceApi('/api/workspace/views/remove',{id:view.id});
    row.remove();
    status.textContent=`Removed “${view.title}”.`;
    if(!document.querySelector('#saved-views-list')?.children.length)status.textContent='No saved views left. Measure something, then choose Save view.';
  }catch(error){status.textContent=`Could not remove this view: ${plainErrorMessage(error.message)}`;}
}
async function restoreStudioView(view){
  const status=document.querySelector('#saved-views-status');status.textContent='Restoring the saved selection…';
  try{
    if(view.state.followed_view){
      if(!await applyOcelFilter(view.state.followed_view.base_filter))return;
      if(!await followType(view.object_type,view.filter,true))return;
    }else if(!await analyzeMapping(currentMapping,view.filter,'filter'))return;
    studioContext=null;prepareAnalysisStudio();
    const query=view.exploration,segment=query.linked?.segment || query.segment;
    studioElement('mode').value=query.linked?'linked':segment?'segment':'whole';studioDefinition();
    if(segment)for(const id of ['from','to']){studioElement(id).value=segment[id];studioElement(id+'-occurrence').value=segment[id+'_occurrence'] || 'first';}
    if(query.linked)for(const id of ['from','to'])studioElement(id+'-type').value=query.linked[id+'_type'];
    studioElement('group').value=query.linked?.group_by || query.group_by || '';
    studioRangeOverride=null;
    const range=segment || query,unit=Number(studioElement('unit').value);
    studioElement('min').value=range.min_micros==null?'':range.min_micros/unit;studioElement('max').value=range.max_micros==null?'':range.max_micros/unit;
    showView('map');openMapAnalysis();await runAnalysisStudio();
  }catch(error){status.textContent=`Could not restore this view: ${error.message}`;}
}
function initializeMapAnalysis(){
  document.querySelector('#map-measure').addEventListener('click',()=>beginMapMeasurement());
  document.querySelector('#map-saved').addEventListener('click',()=>void openSavedViews());
  studioElement('close').addEventListener('click',showMapStepDetail);
  studioElement('save').addEventListener('click',()=>void saveStudioView());
  document.querySelector('#saved-views-close').addEventListener('click',showMapStepDetail);
  for(const button of document.querySelectorAll('[data-studio-tab]'))button.addEventListener('click',()=>setStudioTab(button.dataset.studioTab));
}
