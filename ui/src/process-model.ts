// Working process understanding supplements the measured map without editing it.
// Saved evidence labels are presentation data; no process figures are inferred.
function processEvidenceKind(evidence) {
  const kinds = new Set((evidence || []).map(item => item.kind));
  return ["proposed","reported","documented","observed"].find(kind => kinds.has(kind)) || "proposed";
}

function processEvidenceLabel(kind) {
  return {observed:"Recorded evidence",documented:"Documented expectation",reported:"Reported practice",proposed:"Proposed"}[kind] || "Unverified";
}

function renderWorkingProcess(parent, record, instance = "working") {
  const model = record?.process_model;
  // The browser shows what was recorded, documented or reported. A proposed
  // future process is a suggestion, and suggestions stay in the chat.
  if (!model?.steps?.length || model.view === "proposed_future") return;
  const sources = new Map((model.sources || []).map(source => [source.id,source]));
  const steps = model.steps.slice(0,40);
  const labels = new Map(steps.map(step => [step.id,step.label]));
  const section = reportElement("section",undefined,"process-understanding");
  section.append(reportElement("p","COMBINED PROCESS UNDERSTANDING","process-eyebrow"),
    reportElement("h2",model.title),
    reportElement("p","Working interpretation. Recorded, documented, reported and proposed work stay distinct. Missing manual timings remain unknown.","process-model-note"));
  const meta = [];
  if (record.intention) meta.push({understand:"Understand",improve:"Improve",check:"Check"}[record.intention] || record.intention);
  if (record.process_model_recorded_at) meta.push("Model revised " + new Date(record.process_model_recorded_at).toLocaleString());
  if (meta.length) section.append(reportElement("p",meta.join(" · "),"process-model-note"));
  if (model.supersedes) section.append(reportElement("p","This revision replaces an earlier working model. Earlier evidence is retained.","process-model-note"));
  const legend = reportElement("div",undefined,"process-evidence-legend");
  for (const kind of ["observed","documented","reported","proposed"]) legend.append(reportElement("span",processEvidenceLabel(kind),"process-basis " + kind));
  section.append(legend);
  const details = reportElement("div",undefined,"process-step-detail");
  function evidenceItems(parent, evidence) {
    for (const item of evidence || []) {
      const row = reportElement("div",undefined,"process-evidence-item");
      row.append(reportElement("span",processEvidenceLabel(item.kind),"process-basis " + item.kind));
      if (item.kind === "observed") row.append(citationButton(item.query));
      else if (item.kind === "proposed") row.append(reportElement("p",item.rationale));
      else {
        const source = sources.get(item.source);
        row.append(reportElement("p",source ? [source.title,source.revision,item.section,source.reference].filter(Boolean).join(" · ") : "Source reference unavailable"));
      }
      parent.append(row);
    }
  }
  const controls = new Map();
  function select(step) {
    details.replaceChildren(reportElement("h3",step.label),reportElement("p",step.owner ? "Owner: " + step.owner : "Owner not established","process-model-note"));
    evidenceItems(details,step.evidence);
    for (const [id,button] of controls) button.setAttribute("aria-pressed",String(id === step.id));
  }
  const inventory = reportElement("div",undefined,"process-step-controls");
  for (const step of steps) {
    const button = reportElement("button",step.label,"process-step-choice " + processEvidenceKind(step.evidence));
    button.type = "button"; button.setAttribute("aria-pressed","false");
    button.addEventListener("click",()=>select(step));
    controls.set(step.id,button); inventory.append(button);
  }
  section.append(inventory);
  // Layout alone: supplied connections define every arrow. Array order is not
  // interpreted as an observed sequence, duration or causal relationship.
  if (steps.length <= 9 && typeof document.createElementNS === "function") {
    const ns = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(ns,"svg");
    const columns = Math.min(3,steps.length), rows = Math.ceil(steps.length / columns);
    svg.setAttribute("viewBox","0 0 " + (columns * 300 + 20) + " " + (rows * 150 + 20));
    svg.setAttribute("role","img"); svg.setAttribute("aria-label","Working process connections; select a named step for its evidence");
    svg.classList.add("working-process-map");
    const positions = new Map(steps.map((step,index)=>[step.id,{x:20+(index%columns)*300,y:20+Math.floor(index/columns)*150}]));
    const defs = document.createElementNS(ns,"defs"), marker = document.createElementNS(ns,"marker"), arrow = document.createElementNS(ns,"path");
    marker.setAttribute("id","working-process-arrow-"+instance); marker.setAttribute("viewBox","0 0 10 10"); marker.setAttribute("refX","9"); marker.setAttribute("refY","5"); marker.setAttribute("markerWidth","6"); marker.setAttribute("markerHeight","6"); marker.setAttribute("orient","auto-start-reverse");
    arrow.setAttribute("d","M 0 0 L 10 5 L 0 10 z"); arrow.setAttribute("fill","currentColor"); marker.append(arrow); defs.append(marker); svg.append(defs);
    for (const edge of (model.connections || []).slice(0,100)) {
      const from = positions.get(edge.from), to = positions.get(edge.to);
      if (!from || !to) continue;
      const path = document.createElementNS(ns,"path");
      const rightward = to.x > from.x && to.y === from.y;
      const x1 = from.x + (rightward ? 250 : 125), y1 = from.y + (rightward ? 44 : 88);
      const x2 = to.x + (rightward ? 0 : 125), y2 = to.y + (rightward ? 44 : 0);
      const curve = edge.from === edge.to ? "M "+x1+" "+y1+" c 100 60 130 -110 0 -88" : "M "+x1+" "+y1+" C "+(x1+30)+" "+y1+", "+(x2-30)+" "+y2+", "+x2+" "+y2;
      path.setAttribute("d",curve); path.setAttribute("class","working-connection " + processEvidenceKind(edge.evidence)); path.setAttribute("marker-end","url(#working-process-arrow-"+instance+")");
      const title = document.createElementNS(ns,"title"); title.textContent = labels.get(edge.from) + " → " + labels.get(edge.to); path.append(title); svg.append(path);
    }
    for (const step of steps) {
      const {x,y} = positions.get(step.id), group = document.createElementNS(ns,"g"), rect = document.createElementNS(ns,"rect");
      group.setAttribute("class","working-step " + processEvidenceKind(step.evidence));
      rect.setAttribute("x",String(x)); rect.setAttribute("y",String(y)); rect.setAttribute("width","250"); rect.setAttribute("height","88"); rect.setAttribute("rx","10"); group.append(rect);
      const words = step.label.split(/\s+/), lines = []; let line = "";
      for (const word of words) { if ((line+" "+word).trim().length > 29 && line) { lines.push(line); line=word; } else line=(line+" "+word).trim(); }
      if (line) lines.push(line);
      lines.slice(0,2).forEach((line,index)=>{ const label=document.createElementNS(ns,"text"); label.setAttribute("x",String(x+14)); label.setAttribute("y",String(y+29+index*20)); label.textContent=line.slice(0,29)+((line.length>29 || (index===1&&lines.length>2))?"…":""); group.append(label); });
      const basis=document.createElementNS(ns,"text"); basis.setAttribute("x",String(x+14)); basis.setAttribute("y",String(y+75)); basis.setAttribute("class","working-basis"); basis.textContent=processEvidenceLabel(processEvidenceKind(step.evidence)); group.append(basis);
      svg.append(group);
    }
    section.append(svg);
  }
  section.append(reportElement("p","Arrows use supplied relationship evidence; placement is for readability. Select a named step to inspect its sources.","process-model-note"),details);
  select(steps[0]);
  const links = reportElement("details",undefined,"process-connections");
  links.append(reportElement("summary","Connection evidence (" + (model.connections || []).length + ")"));
  for (const edge of model.connections || []) {
    const row = reportElement("div",undefined,"process-connection-detail");
    row.append(reportElement("h4",(labels.get(edge.from)||edge.from)+" → "+(labels.get(edge.to)||edge.to)));
    if (edge.condition) row.append(reportElement("p","Condition: "+edge.condition));
    evidenceItems(row,edge.evidence); links.append(row);
  }
  section.append(links);
  if (model.open_questions?.length) {
    section.append(reportElement("h3","Still to establish"));
    const questions=reportElement("ul"); model.open_questions.forEach(item=>questions.append(reportElement("li",item))); section.append(questions);
  }
  parent.append(section);
}
