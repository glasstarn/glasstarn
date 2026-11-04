// The analysis workspace renders shared engine queries. No browser-side mining.
let studioContext=null, studioRevision=0, studioReport=null, studioQuery=null;
let studioChoicesRevision=0, studioValues=new Set(), studioMissing=false;
let studioRangeOverride=null;
let studioNativeFilter=null;
let studioDurationUnit=86400000000;
let mapControlsOpen=false;
let studioPicking=null, studioOpen=false, studioTab="time";
let studioScope=null;
const studioElement=id=>document.querySelector('#studio-'+id);
function studioOption(value,label=value){const option=reportElement('option',label);option.value=value;return option;}
function prepareAnalysisStudio(){
  if(!currentAnalysis || !studioElement('mode'))return;
  if(studioContext===currentAnalysis)return;
  studioContext=currentAnalysis;studioRevision++;studioReport=null;studioQuery=null;studioRangeOverride=null;studioNativeFilter=null;
  studioElement('apply').disabled=true;
  studioChoicesRevision++;studioElement('choices').replaceChildren();studioElement('values-note').textContent='';
  const activities=(filterCatalog?.activities || currentAnalysis.nodes?.map(node=>node.activity) || []);
  if(studioElement('mode').value!=='linked')for(const id of ['from','to']){const select=studioElement(id),previous=select.value;select.replaceChildren(studioOption('','Choose a step…'),...activities.map(name=>studioOption(name)));if(activities.includes(previous))select.value=previous;}

  const group=studioElement('group'),previous=group.value;
  group.replaceChildren(studioOption('','Choose a field…'),...(filterCatalog?.attributes || currentAnalysis.attributes || []).map(name=>studioOption(name)));
  if([...group.options].some(option=>option.value===previous))group.value=previous;
  const segment=currentFilter.segment;
  if(segment){studioElement('mode').value='segment';for(const id of ['from','to'])studioElement(id).value=segment[id];for(const id of ['from','to'])studioElement(id+'-occurrence').value=segment[id+'_occurrence'] || 'first';}
  const unit=Number(studioElement('unit').value);
  if(segment || currentFilter.min_duration!=null || studioElement('mode').value!=='linked')studioElement('min').value=(segment?.min_micros ?? currentFilter.min_duration)!=null?(segment?.min_micros ?? currentFilter.min_duration)/unit:'';
  if(segment || currentFilter.max_duration!=null || studioElement('mode').value!=='linked')studioElement('max').value=(segment?.max_micros ?? currentFilter.max_duration)!=null?(segment?.max_micros ?? currentFilter.max_duration)/unit:'';
  studioValues=new Set([...(currentFilter.attribute_in?.[group.value] || []),...(currentFilter.attribute_equals || []).filter(pair=>pair[0]===group.value).map(pair=>pair[1])]);studioMissing=(currentFilter.attribute_missing || []).includes(group.value);
  const linkedOption=studioElement('mode').querySelector('option[value="linked"]');if(linkedOption)linkedOption.disabled=!currentFollow;
  if(!currentFollow && studioElement('mode').value==='linked')studioElement('mode').value='whole';
  for(const id of ['from-type','to-type']){const select=studioElement(id),previous=select.value;select.replaceChildren(...(followTypes || []).map(type=>studioOption(type,itemTypeName(type))));select.value=followTypes.includes(previous)?previous:currentFollow?.type || '';}
  const connected=studioElement('connected');connected.hidden=!currentFollow;
  if(currentFollow){const follow=studioElement('follow');follow.replaceChildren(...followTypes.filter(type=>type!==currentFollow.type).map(type=>studioOption(type,itemTypeName(type))));}
  studioDefinition();
}
function studioDefinition(){
  const linked=studioElement('mode').value==='linked';
  const segment=studioElement('mode').value!=='whole';
  for(const button of document.querySelectorAll('[data-studio-mode]')){
    button.setAttribute('aria-pressed',String(button.dataset.studioMode===studioElement('mode').value));
    if(button.dataset.studioMode==='linked')button.hidden=!currentFollow;
  }
  for(const label of document.querySelectorAll('.studio-endpoint'))label.hidden=!segment;
  // The slowest-items tables measure whole histories, so they show only for Start to end.
  const supporting=document.querySelector('.studio-supporting');if(supporting)supporting.hidden=segment;
  for(const label of document.querySelectorAll('.studio-linked-type'))label.hidden=!linked;
  {
    const names=linked?(currentOcelAnalysis?.graphs?.activities || []).map(item=>item.activity):(filterCatalog?.activities || currentAnalysis?.nodes?.map(node=>node.activity) || []);
    for(const id of ['from','to']){const select=studioElement(id),previous=select.value;select.replaceChildren(studioOption('','Choose a step…'),...names.map(name=>studioOption(name)));if(names.includes(previous))select.value=previous;}
  }
  const scope=currentFollow?`each ${currentFollow.singular}`:'each item';
  studioElement('title').textContent=segment?'Time between steps':'Recorded history time';
  studioElement('definition').textContent=linked
    ?`One measurement per ${currentFollow?.singular || 'item'}, using directly linked records.`
    :segment?`${studioElement('from-occurrence').value==='first'?'First':'Last'} start → ${studioElement('to-occurrence').value==='first'?'first':'last'} finish, within ${scope}. Steps in between are included.`
    :`First to last recorded event for ${scope}. This may span several transactions.`;
  if(resultsRouteFocus || connectedVariantFocus)studioElement('definition').textContent+=' Uses all items in the active filters; route highlights only change the drawing.';
  studioElement('definition').hidden=segment && !resultsRouteFocus && !connectedVariantFocus;

}
function studioRequest(){
  const bound=id=>{const value=studioElement(id).value;if(value==='')return null;const number=Number(value);if(!Number.isFinite(number)||number<0)throw Error('Enter a non-negative duration.');return Math.round(number*Number(studioElement('unit').value));};
  const min=studioRangeOverride?.min ?? bound('min'),max=studioRangeOverride?.max ?? bound('max');
  if(min!==null && max!==null && min>max)throw Error('The lower duration must not exceed the upper duration.');
  const group_by=studioElement('group').value || null;
  if(studioElement('mode').value!=='whole' && (!studioElement('from').value || !studioElement('to').value))throw Error('Choose both steps on the map or in the selectors.');
  if(studioElement('mode').value==='linked')return {linked:{root_type:currentFollow.type,from_type:studioElement('from-type').value,to_type:studioElement('to-type').value,group_by,segment:{from:studioElement('from').value,to:studioElement('to').value,from_occurrence:studioElement('from-occurrence').value,to_occurrence:studioElement('to-occurrence').value,min_micros:min,max_micros:max}}};
  return studioElement('mode').value==='segment'?{group_by,segment:{from:studioElement('from').value,to:studioElement('to').value,from_occurrence:studioElement('from-occurrence').value,to_occurrence:studioElement('to-occurrence').value,min_micros:min,max_micros:max}}:{group_by,min_micros:min,max_micros:max};
}
async function openAnalysisStudio(){openMapAnalysis('whole');}
async function runAnalysisStudio(){
  if(!currentAnalysis || !currentProfile || !currentMapping || !studioElement('status'))return;
  const revision=++studioRevision,analysis=currentAnalysis,profile=currentProfile;
  studioReport=null;studioQuery=null;studioElement('apply').disabled=true;studioElement('save').disabled=true;
  studioChoicesRevision++;studioElement('choices').replaceChildren();studioElement('values-apply').disabled=true;studioElement('values-note').textContent='';
  for(const id of ['cards','histogram','groups','group-chart','contrasts','evidence'])studioElement(id).replaceChildren();
  studioElement('selection').hidden=true;studioElement('comparison').hidden=true;
  studioElement('status').textContent='Calculating time for the current selection…';studioElement('run').disabled=true;
  try{
    const exploration=studioRequest();
    const filter={...currentFilter};delete filter.segment;
    if(!exploration.segment && !exploration.linked){delete filter.min_duration;delete filter.max_duration;}
    if(exploration.linked && connectedScopeRequest)await connectedScopeRequest;
    if(revision!==studioRevision || analysis!==currentAnalysis || profile!==currentProfile)return;
    const response=await projectFetch(exploration.linked?'/api/ocel/explore':'/api/explore',{method:'POST',headers:{Authorization:`Bearer ${launchToken}`,'Content-Type':'application/json'},body:JSON.stringify(exploration.linked?{ocel_upload_id:currentOcelUpload,filter:currentOcelAnalysis.filter,case_view:comparisonRequestBody({filter}),interval:exploration.linked}:comparisonRequestBody({filter,exploration})),cache:'no-store'});
    const payload=await response.json();const report=exploration.linked?payload.report:payload;
    if(revision!==studioRevision || analysis!==currentAnalysis || profile!==currentProfile)return;
    if(!response.ok)throw Error(payload.error || 'Analysis could not be calculated.');
    studioNativeFilter=exploration.linked?payload.selected_filter:null;
    studioReport=report;studioQuery=exploration;studioScope=studioCurrentScope();renderAnalysisStudio(report);studioElement('save').disabled=false;
    void loadStudioChoices();
  }catch(error){if(revision===studioRevision)studioElement('status').textContent=plainErrorMessage(error.message);}
  finally{if(revision===studioRevision)studioElement('run').disabled=false;}
}
function studioDuration(value){return value==null?'Not measured':formatDuration(value);}
function studioReadableDuration(value){return value==null?'Not measured':formatDuration(value).replace(/([dhms])$/,unit=>({d:value===86400?' day':' days',h:value===3600?' hour':' hours',m:value===60?' minute':' minutes',s:value===1?' second':' seconds'})[unit]);}
function studioInterval(query=studioQuery){return query?.linked?.segment || query?.segment || query || {};}
function studioHasRange(query=studioQuery){const interval=studioInterval(query);return interval.min_micros!=null || interval.max_micros!=null;}
function studioRangeLabel(){
  const interval=studioInterval(),min=interval.min_micros,max=interval.max_micros;
  if(min==null)return `up to ${formatDuration(max/1e6)}`;
  if(max==null)return `${formatDuration(min/1e6)} or longer`;
  return `${formatDuration(min/1e6)}–${formatDuration((studioRangeOverride?max+1:max)/1e6)}`;
}
function renderStudioGroups(report=studioReport){
  if(!report)return;
  const field=studioElement('group').value,metric=studioElement('group-metric').value || 'median_seconds';
  studioElement('breakdown-empty').hidden=!!field;
  studioElement('breakdown-results').hidden=!field;
  studioElement('values-summary').textContent=`Filter specific ${field || 'field'} values`;
  const groups=report.groups.slice(0,8),max=Math.max(1,...groups.map(group=>group.population[metric] || 0));
  studioElement('groups-note').textContent=(groups.length?`${groups.length} largest groups of ${report.groups_total.toLocaleString()}. Select a group to filter it.`:'No groups in this selection.')+(report.groups_overlap?' This field changes between steps, so an item that had several values counts in each of their groups.':'');
  studioElement('group-chart').replaceChildren(...groups.map(group=>{
    const p=group.population,value=p[metric],row=reportElement('button',undefined,'studio-group-row');row.type='button';
    row.setAttribute('aria-label',`Select ${group.value??'Missing value'} for filtering`);
    const label=reportElement('span',group.value??'Missing value');label.append(reportElement('small',`${p.items.toLocaleString()} items · ${p.measurable.toLocaleString()} measured`));
    const track=reportElement('i'),fill=reportElement('i');fill.style.width=`${(value || 0)/max*100}%`;track.append(fill);
    row.append(label,track,reportElement('b',metric==='items'?value.toLocaleString():studioDuration(value)));
    row.addEventListener('click',()=>{studioValues=new Set(group.value==null?[]:[group.value]);studioMissing=group.value==null;studioElement('search').value='';studioElement('values-editor').open=true;studioElement('values-editor').scrollIntoView?.({block:'nearest',behavior:'smooth'});studioElement('search').focus?.({preventScroll:true});void loadStudioChoices();});
    return row;
  }));
  studioElement('groups').replaceChildren(...report.groups.map(group=>{const row=reportElement('tr');for(const text of [group.value??'Missing value',group.population.items.toLocaleString(),group.population.measurable.toLocaleString(),studioDuration(group.population.mean_seconds),studioDuration(group.population.median_seconds),studioDuration(group.population.p90_seconds),group.selected_items.toLocaleString()])row.append(reportElement('td',text));return row;}));
}
function renderAnalysisStudio(report){
  const p=report.overall,hasRange=studioHasRange(),interval=studioInterval();
  studioElement('status').textContent=`${p.measurable.toLocaleString()} of ${p.items.toLocaleString()} ${itemLabels().plural.toLowerCase()} measured${p.missing_endpoints_or_incomplete?` · ${p.missing_endpoints_or_incomplete.toLocaleString()} left out because ${studioElement('mode').value==='whole'?'they have no end yet':'a step is missing or out of order'}`:''}.`;
  const cards=[['Median',studioReadableDuration(p.median_seconds),''],['Average',studioReadableDuration(p.mean_seconds),''],['90% within',studioReadableDuration(p.p90_seconds),'']];
  studioElement('cards').replaceChildren(...cards.map(([label,value,note])=>{const card=reportElement('article');card.append(reportElement('span',label),reportElement('strong',value),reportElement('small',note));return card;}));
  studioElement('chart-unit').textContent='Items by elapsed time';
  const max=Math.max(1,...report.bins.map(bin=>bin.items)),tickEvery=Math.ceil(report.bins.length/4);
  studioElement('histogram').replaceChildren(...report.bins.map((bin,index)=>{
    const button=reportElement('button');button.type='button';button.className='studio-bin';
    const range=`${formatDuration(bin.from_micros/1e6)}${bin.to_micros==null?'+':` to <${formatDuration(bin.to_micros/1e6)}`}`;
    button.setAttribute('aria-label',`${range}: ${bin.items.toLocaleString()} items`);button.title=`${range} · ${bin.items.toLocaleString()} items`;
    const overlap=(interval.max_micros==null || bin.from_micros<=interval.max_micros) && (interval.min_micros==null || bin.to_micros==null || bin.to_micros>interval.min_micros);
    const partial=overlap && ((interval.min_micros!=null && interval.min_micros>bin.from_micros) || (interval.max_micros!=null && (bin.to_micros==null || interval.max_micros<bin.to_micros-1)));
    button.classList.toggle('outside-range',hasRange&&!overlap);button.classList.toggle('partial-range',hasRange&&partial);
    const bar=reportElement('i');bar.style.height=`${Math.max(2,bin.items/max*74)}px`;
    button.append(reportElement('small',bin.items.toLocaleString()+' items'),bar,reportElement('span',index%tickEvery===0?formatDuration(bin.from_micros/1e6):''));
    button.addEventListener('click',()=>{const unit=Number(studioElement('unit').value);studioRangeOverride={min:bin.from_micros,max:bin.to_micros===null?null:bin.to_micros-1};studioElement('min').value=bin.from_micros/unit;studioElement('max').value=bin.to_micros===null?'':bin.to_micros/unit;studioElement('range-editor').open=true;void runAnalysisStudio();});return button;
  }));
  if(!report.bins.length)studioElement('histogram').append(reportElement('p','No complete intervals to measure. Try other steps or remove a filter.'));
  studioElement('selection').hidden=!hasRange;
  studioElement('selection-title').textContent=hasRange?`${report.selected.measurable.toLocaleString()} ${itemLabels().plural.toLowerCase()} took ${studioRangeLabel()}${report.selected_value?` · ${formatAmount(report.selected_value[1])}`:''}`:'';
  renderStudioDrivers(report);
  studioElement('selection-note').textContent=hasRange?`${p.measurable?(report.selected.measurable/p.measurable*100).toFixed(1):0}% of those measured. Apply the filter to see only these everywhere.`:'';
  studioElement('comparison').hidden=!hasRange;
  studioElement('contrast-note').textContent=`Activities recorded in ${report.selected.measurable.toLocaleString()} items in this range, compared with ${report.remainder.measurable.toLocaleString()} other measured items.`;
  const contrastRows=report.contrasts.filter(item=>Math.abs(item.difference_points)>=0.05).map(item=>{
    const row=reportElement('div',undefined,'studio-contrast');row.append(reportElement('strong',item.activity),reportElement('span',`${item.selected_percent.toFixed(1)}% in range · ${item.remainder_percent.toFixed(1)}% outside range`),reportElement('b',`${Math.abs(item.difference_points).toFixed(1)} percentage points ${item.difference_points>=0?'more':'less'} common`));const bars=reportElement('div',undefined,'studio-contrast-bars');for(const [name,value] of [['selected',item.selected_percent],['remaining',item.remainder_percent]]){const track=reportElement('div');const fill=reportElement('i');fill.className=name;fill.style.width=`${value}%`;track.append(fill);bars.append(track);}row.append(bars);return row;
  });
  studioElement('contrasts').replaceChildren(...contrastRows.slice(0,4));
  if(contrastRows.length>4){const more=reportElement('details',undefined,'studio-more-patterns');more.append(reportElement('summary',`Show ${contrastRows.length-4} more patterns`));const body=reportElement('div');body.append(...contrastRows.slice(4));more.append(body);studioElement('contrasts').append(more);}
  if(!report.contrasts.length)studioElement('contrasts').append(reportElement('p',!report.selected.measurable?'No measured items in this range. Try a wider range.':!report.remainder.measurable?'This range contains every measured item. Narrow it to compare with the rest.':'No activity differences returned for this range.','studio-note'));
  renderStudioGroups(report);
  studioElement('semantics').replaceChildren(...studioPlainMethod(studioElement('mode').value).map(text=>reportElement('span',text)));
  studioElement('evidence').replaceChildren();
  for(const [title,population] of (hasRange?[['Items in this range',report.selected],['Items outside this range',report.remainder]]:[['Example items',report.selected]])){
    const group=reportElement('div');group.append(reportElement('h4',title));
    for(const id of population.examples){const button=reportElement('button',id,'case-chip');button.type='button';button.addEventListener('click',()=>void openCase(id));group.append(button);}studioElement('evidence').append(group);
  }
  studioElement('apply').disabled=!report.selected.items || !report.overall.items || !hasRange;
  studioElement('range-note').textContent=studioRangeOverride?'This bar starts at its first value and stops just short of the next bar.':'Items that take exactly the From or To time are included. Leave either box empty for no limit.';
}
async function clearStudioRange(){
  studioElement('min').value='';studioElement('max').value='';studioRangeOverride=null;studioElement('range-editor').open=false;
  const filter={...currentFilter};
  if(studioElement('mode').value!=='linked' && (filter.segment || filter.min_duration!=null || filter.max_duration!=null)){
    delete filter.segment;delete filter.min_duration;delete filter.max_duration;
    await analyzeMapping(currentMapping,filter,'filter');
  }else await runAnalysisStudio();
}
async function loadStudioChoices(){
  const field=studioElement('group').value,revision=++studioChoicesRevision,analysis=currentAnalysis;
  const list=studioElement('choices');list.replaceChildren();studioElement('values-apply').disabled=true;
  studioElement('search').placeholder=field?`Search ${field} values…`:'Choose a breakdown field…';
  if(!field){studioElement('values-note').textContent='Choose a breakdown field to select its recorded values.';return;}
  const filter={...currentFilter,attribute_equals:(currentFilter.attribute_equals||[]).filter(pair=>pair[0]!==field),attribute_in:{...currentFilter.attribute_in},attribute_missing:(currentFilter.attribute_missing||[]).filter(name=>name!==field)};delete filter.attribute_in[field];delete filter.segment;
  if(studioElement('mode').value==='whole'){delete filter.min_duration;delete filter.max_duration;}
  studioElement('values-note').textContent='Loading recorded values…';
  try{
    const response=await projectFetch('/api/filter/options',{method:'POST',headers:{Authorization:`Bearer ${launchToken}`,'Content-Type':'application/json'},body:JSON.stringify(comparisonRequestBody({filter,attribute:field,search:studioElement('search').value})),cache:'no-store'});
    const choices=await response.json();if(revision!==studioChoicesRevision || analysis!==currentAnalysis)return;
    if(!response.ok)throw Error(choices.error || 'Values unavailable');
    const note=()=>{studioElement('values-apply').disabled=!studioMissing && !studioValues.size;studioElement('values-note').textContent=`${choices.matching_values.toLocaleString()} matching recorded values${choices.truncated?' · top 100 shown; search to find others':''}. ${studioMissing?'Missing values':studioValues.size+' value'+(studioValues.size===1?'':'s')} selected.`;};
    const option=(value,count,missing=false)=>{const label=reportElement('label');const checkbox=reportElement('input');checkbox.type='checkbox';checkbox.checked=missing?studioMissing:studioValues.has(value);checkbox.setAttribute('aria-label',missing?'Include missing values':`Include ${value}`);checkbox.dataset.choice=value;checkbox.dataset.missing=String(missing);checkbox.addEventListener('change',()=>{if(missing){studioMissing=checkbox.checked;if(studioMissing)studioValues.clear();}else{studioMissing=false;checkbox.checked?studioValues.add(value):studioValues.delete(value);}for(const input of list.querySelectorAll('input'))input.checked=input.dataset.missing==='true'?studioMissing:studioValues.has(input.dataset.choice);note();});label.append(checkbox,reportElement('span',value),reportElement('b',count.toLocaleString()));return label;};
    list.replaceChildren(...choices.values.map(choice=>option(choice.value,choice.cases)));
    if(choices.missing_cases)list.append(option('Missing value',choices.missing_cases,true));
    note();
  }catch(error){if(revision===studioChoicesRevision)studioElement('values-note').textContent=plainErrorMessage(error.message);}
}
async function applyStudioSelection(kind){
  if(!currentMapping)return;
  const filter={...currentFilter};
  if(kind==='range'){
    if(!studioReport || !studioQuery || JSON.stringify(studioRequest())!==JSON.stringify(studioQuery)){studioElement('status').textContent='Analyze the updated definition before applying it.';return;}
    if(studioQuery.linked){if(studioNativeFilter){await applyOcelFilter(studioNativeFilter);showView('map');}return;}
    if(studioQuery.segment)filter.segment=studioQuery.segment;else{delete filter.segment;filter.min_duration=studioQuery.min_micros;filter.max_duration=studioQuery.max_micros;}
  }else{
    const field=studioElement('group').value;if(!field)return;
    filter.attribute_in={...filter.attribute_in};delete filter.attribute_in[field];
    filter.attribute_missing=(filter.attribute_missing||[]).filter(name=>name!==field);
    filter.attribute_equals=(filter.attribute_equals||[]).filter(pair=>pair[0]!==field);
    if(kind!=='clear'){if(studioMissing)filter.attribute_missing.push(field);else if(studioValues.size)filter.attribute_in[field]=[...studioValues].sort();else{studioElement('status').textContent='Select at least one recorded value.';return;}}
  }
  await analyzeMapping(currentMapping,filter,'filter');
  showView('map');
}
function appendStudioActions(activity){
  const panel=document.querySelector('#results-detail');if(!panel || !studioElement('from'))return;
  const actions=reportElement('div',undefined,'studio-step-actions');
  const button=reportElement('button','Measure to…');button.type='button';button.addEventListener('click',()=>beginMapMeasurement(activity));actions.append(button);panel.append(actions);
}
// How the measurement works, in plain words for the panel's "How this is
// measured" section.
function studioPlainMethod(mode){
  return [
    mode==='whole'?'Time runs from each item’s first recorded step to its last.':'Time runs from step A to step B, as recorded in the data.',
    'It is time on the clock, not hands-on work.',
    'Each item counts once.',
    mode==='whole'?'Items with no end yet are left out.':'Items missing A or B, or where B comes before A, are left out.',
    'Steps recorded at the same moment count as no time.',
    'When you compare a range with the rest, the differences show what is more common, not what causes the delay.',
  ];
}
// What goes with the slowest fifth of this measure: field values, most
// telling first. Clicking one filters every view to those items.
function renderStudioDrivers(report){
  const box=document.querySelector('#studio-drivers'),list=document.querySelector('#studio-driver-list');if(!box||!list)return;
  const drivers=report?.drivers?.drivers||[];
  box.hidden=!drivers.length;
  list.replaceChildren(...drivers.map(driver=>{
    const row=reportElement('button',undefined,'studio-driver');row.type='button';
    const name=driver.field==='Handled by'?`Handled by ${driver.value}`:driver.at_least!=null?`${driver.field} ${driver.value}`:`${driver.field} = ${driver.value}`;
    row.append(reportElement('strong',name),reportElement('span',`${driver.lift.toFixed(1)}× as likely to be slow · ${Math.round(driver.share_of_slow)}% of the slowest fifth`),reportElement('small',`median ${formatDuration(driver.median_seconds)} against ${formatDuration(driver.others_median_seconds)}`));
    row.addEventListener('click',()=>void openDriverItems({field:driver.field,value:driver.value,at_least:driver.at_least,at_most:driver.at_most}));
    return row;
  }));
}
function changeStudioDurationUnit(){
  const next=Number(studioElement('unit').value);
  for(const id of ['min','max']){const input=studioElement(id);if(input.value!=='')input.value=Number(input.value)*studioDurationUnit/next;}
  studioDurationUnit=next;studioElement('apply').disabled=true;
}
function toggleMapControls(open){
  mapControlsOpen=open;
  const subway=activeMapStyle()==='subway';
  document.querySelector('#map-view-controls').hidden=!subway || !open;
  document.querySelector('#process-controls').hidden=subway;
  document.querySelector('#map-controls-toggle').setAttribute('aria-expanded',String(open));
  fitExplorer('process');
}
// Measuring between steps waits for the missing step to be clicked on the
// map: the start first, then the end.
function studioPickingFor(mode,from,to){return mode==='whole'?null:!from?'from':!to?'to':null;}
function initializeAnalysisStudio(){
  if(!studioElement('mode'))return;
  initializeMapAnalysis();
  for(const button of document.querySelectorAll('[data-studio-mode]'))button.addEventListener('click',()=>{
    studioElement('mode').value=button.dataset.studioMode;studioRangeOverride=null;studioElement('min').value='';studioElement('max').value='';studioDefinition();studioPicking=studioPickingFor(button.dataset.studioMode,studioElement('from').value,studioElement('to').value);updateMeasurementHighlight();if(studioPicking){studioElement('status').textContent='';clearStudioAnswer();}else void runAnalysisStudio();
  });
  studioElement('range-clear').addEventListener('click',clearStudioRange);
  studioElement('group-metric').addEventListener('change',()=>renderStudioGroups());
  studioElement('run').addEventListener('click',()=>void runAnalysisStudio());studioElement('preview').addEventListener('click',async()=>{await runAnalysisStudio();studioElement('selection').scrollIntoView?.({behavior:'smooth',block:'nearest'});});
  studioElement('apply').addEventListener('click',()=>void applyStudioSelection('range'));
  studioElement('values-apply').addEventListener('click',()=>void applyStudioSelection('values'));studioElement('values-clear').addEventListener('click',()=>void applyStudioSelection('clear'));
  studioElement('compare').addEventListener('click',()=>{showView('compare');void loadComparisonOptions();});
  for(const id of ['mode','from','to','from-type','to-type','from-occurrence','to-occurrence','group'])studioElement(id).addEventListener('change',()=>{studioElement('apply').disabled=true;studioPicking=null;studioDefinition();updateMeasurementHighlight();if(id==='group'){studioValues=new Set([...(currentFilter.attribute_in?.[studioElement('group').value] || []),...(currentFilter.attribute_equals || []).filter(pair=>pair[0]===studioElement('group').value).map(pair=>pair[1])]);studioMissing=(currentFilter.attribute_missing || []).includes(studioElement('group').value);studioElement('search').value='';studioElement('values-editor').open=false;}void runAnalysisStudio();});
  for(const id of ['min','max'])studioElement(id).addEventListener('input',()=>{studioRangeOverride=null;studioElement('apply').disabled=true;});
  studioDurationUnit=Number(studioElement('unit').value);
  studioElement('unit').addEventListener('change',()=>{changeStudioDurationUnit();void runAnalysisStudio();});
  document.querySelector('#map-controls-toggle')?.addEventListener('click',()=>toggleMapControls(!mapControlsOpen));
  document.querySelector('#map-controls-close')?.addEventListener('click',()=>toggleMapControls(false));
  document.querySelector('#map-view-search')?.addEventListener('input',()=>renderSubway());
  for(const button of document.querySelectorAll('[data-subway-direction]'))button.addEventListener('click',()=>{subwayOrientation=button.dataset.subwayDirection;subwayOrientationChosen=true;renderSubway();if(typeof fitExplorer==='function')fitExplorer('process');});
  // Select all / Clear all for the object and event lists.
  for(const button of document.querySelectorAll('[data-map-select]'))button.addEventListener('click',()=>{
    const all=button.dataset.all==='true';
    if(button.dataset.mapSelect==='objects')subwayShownTypes=all?new Set((currentOcelAnalysis?.variants || []).map(item=>item.object_type)):new Set();
    else if(all)subwayHiddenEvents.clear();
    else for(const name of subwayModel(currentOcelAnalysis).events.keys())subwayHiddenEvents.add(name);
    renderSubway();
    if(button.dataset.mapSelect==='objects')void syncFollowWithMap();
  });
  let searchTimer;studioElement('search').addEventListener('input',()=>{clearTimeout(searchTimer);searchTimer=setTimeout(()=>void loadStudioChoices(),200);});
  studioElement('reset-linked').addEventListener('click',()=>void applyOcelFilter({}));
  studioElement('follow-apply').addEventListener('click',async()=>{const type=studioElement('follow').value;if(type && await followType(type,{},true)){studioContext=null;await openAnalysisStudio();}});
}

let mapViewControlContext=null;
function renderMapViewControls(model){
  const objects=document.querySelector('#map-object-list'),events=document.querySelector('#map-event-list');if(!objects||!events)return;
  const search=(document.querySelector('#map-view-search').value || '').toLowerCase();
  const types=[...new Set((currentOcelAnalysis?.variants || []).map(item=>item.object_type))];
  const shown=new Set(model.lines.map(line=>line.type));
  const row=(label,count,colour,checked,onChange)=>{const item=reportElement('label');const input=reportElement('input');input.type='checkbox';input.checked=checked;input.setAttribute('aria-label',label);input.addEventListener('change',()=>onChange(input.checked));const dot=reportElement('i');dot.style.background=`var(${colour})`;item.append(input,dot,reportElement('span',label),reportElement('b',count));return item;};
  objects.replaceChildren(...types.filter(type=>itemTypeName(type).toLowerCase().includes(search)).map(type=>{
    const top=currentOcelAnalysis.variants.find(item=>item.object_type===type&&item.rank===1);const count=top?Math.round(top.objects/Math.max((top.share_millionths||0)/1e6,1e-6)).toLocaleString():'';
    return row(itemTypeName(type),count,subwayTypeColour(type),shown.has(type),checked=>{if(subwayShownTypes===null)subwayShownTypes=new Set(shown);checked?subwayShownTypes.add(type):subwayShownTypes.delete(type);renderSubway();void syncFollowWithMap();});
  }));
  events.replaceChildren(...[...model.events].filter(([name])=>name.toLowerCase().includes(search)).sort((a,b)=>b[1]-a[1] || a[0].localeCompare(b[0])).map(([name,count])=>{
    const type=model.lines.find(line=>line.route.includes(name))?.type;
    return row(name,count.toLocaleString(),type?subwayTypeColour(type):'--muted',!subwayHiddenEvents.has(name),checked=>{checked?subwayHiddenEvents.delete(name):subwayHiddenEvents.add(name);renderSubway();});
  }));
}

