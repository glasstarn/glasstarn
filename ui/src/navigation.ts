// Navigation between views; view changes never alter analysis state.

function showView(view) {
  if (!document.body || !viewNames[view]) return;
  if (view === "conformance" && currentWorkspace && !auditRequested(currentWorkspace)) return;
  if(view === "timing"){showView("map");void openAnalysisStudio();return;}
  // Overview is always the whole project. Any filter, whether a finding put
  // it on or it was chosen on another page, comes off on the way here, however
  // you get here (the side menu, a button, the browser).
  if (view === "report" && currentMapping && typeof filterChips === "function" && filterChips(currentFilter || {}).length) {
    if (typeof mapFindingFocus !== "undefined") mapFindingFocus = null;
    void analyzeMapping(currentMapping, {}, "filter");
  }
  // The projects page starts tidy: new-project and settings sections closed.
  if (view === "home") for (const section of document.querySelectorAll(".browser-create, .project-settings")) section.open = false;
  document.body.dataset.view = view;
  rememberProjectView(view);
  if (typeof placeFilterChips === "function") placeFilterChips(view);
  const title = document.querySelector("#view-title");
  if (title) title.textContent = viewNames[view];
  const heading = document.querySelector("#page-title");
  if (heading) heading.textContent = viewNames[view];
  for (const link of document.querySelectorAll(".nav-item[data-view]")) {
    const active = link.dataset.view === view || (view === "import" && link.dataset.view === "home");
    link.classList.toggle("active", active);
    if (active) link.setAttribute("aria-current", "page"); else link.removeAttribute("aria-current");
  }
  if (view === "variants" && typeof ensureVariantView === "function") void ensureVariantView();
  // A check the agent already ran shows its results straight away.
  if (view === "conformance" && typeof checkCaseConformance === "function" && currentCaseBaseline && !currentCaseConformance) void checkCaseConformance();
  if (view === "timing" && typeof openAnalysisStudio === "function") void openAnalysisStudio();
  if (view === "report" && currentOcelAnalysis) document.querySelector("#ocel-panel").dataset.objectView = "flow";
  requestAnimationFrame(() => { fitProcessMaps(); if (typeof fitExplorers === "function") fitExplorers(); });
}

function fitProcessMaps() {
  for (const id of ["#process-map", "#ocel-process-map"]) {
    const svg = document.querySelector(id);
    if (!svg || !svg.viewBox?.baseVal.width || !svg.parentElement.clientWidth) continue;
    const box = svg.viewBox.baseVal;
    const scale = mapActualSize ? 1 : Math.min(1, Math.max(1, svg.parentElement.clientWidth - 32) / box.width);
    svg.style.width = `${box.width * scale}px`;
    svg.style.height = `${box.height * scale}px`;
  }
}

function showOcelEvidence(open = true) {
  const panel = document.querySelector("#ocel-evidence");
  if (!panel) return;
  panel.dataset.open = String(open);
  document.querySelector("#ocel-evidence-toggle")?.setAttribute("aria-expanded", String(open));
  if (document.body) requestAnimationFrame(fitProcessMaps);
}

function setNavigationPinned(pinned){
  document.querySelector('.project-shell')?.classList.toggle('navigation-pinned',pinned);
  const button=document.querySelector('#navigation-pin');
  if(button){button.setAttribute('aria-pressed',String(pinned));button.setAttribute('aria-label',pinned?'Unpin navigation':'Pin navigation open');button.title=pinned?'Unpin navigation':'Pin navigation open';button.querySelector('.nav-label').textContent=pinned?'Unpin menu':'Pin menu open';}
}

function initializeNavigation() {
  try{setNavigationPinned(localStorage.getItem('glasstarn-navigation-pinned')==='true');}catch(_error){setNavigationPinned(false);}
  document.querySelector('#navigation-pin')?.addEventListener('click',()=>{
    const pinned=document.querySelector('#navigation-pin').getAttribute('aria-pressed')!=='true';setNavigationPinned(pinned);
    try{localStorage.setItem('glasstarn-navigation-pinned',String(pinned));}catch(_error){/* This session still keeps the choice. */}
    requestAnimationFrame(()=>{fitProcessMaps();if(typeof fitExplorers==='function')fitExplorers();});
  });
  for(const link of document.querySelectorAll('.side-nav .nav-item')){
    const name=link.querySelector('.nav-label')?.textContent || link.textContent.trim();link.setAttribute('aria-label',name);link.title=name;
  }
  document.querySelector(".brand")?.addEventListener("click", event => { event.preventDefault(); showView("home"); });
  for (const link of document.querySelectorAll(".nav-item[data-view]")) {
    link.addEventListener("click", event => {
      event.preventDefault();
      if (link.getAttribute("aria-disabled") === "true") return;
      showView(link.dataset.view);
      window.scrollTo({ top: 0, behavior: "instant" });
    });
  }
  document.querySelector("#ocel-map-fit")?.addEventListener("click", () => { mapActualSize = false; fitProcessMaps(); });
  document.querySelector("#ocel-map-actual")?.addEventListener("click", () => { mapActualSize = true; fitProcessMaps(); });
  document.querySelector("#ocel-evidence-toggle")?.addEventListener("click", () => showOcelEvidence(document.querySelector("#ocel-evidence")?.dataset.open !== "true"));
  for (const button of document.querySelectorAll("button[data-object-view]")) {
    button.addEventListener("click", () => {
      document.querySelector("#ocel-panel").dataset.objectView = button.dataset.objectView;
      for (const sibling of document.querySelectorAll("button[data-object-view]")) sibling.setAttribute("aria-pressed", String(sibling === button));
      requestAnimationFrame(fitProcessMaps);
    });
  }
  new ResizeObserver(fitProcessMaps).observe(document.querySelector("#main"));
}

// One page per project: reopening it (the agent's link, a closed tab opened
// again) comes back on the page you were on, with the filters you had.
function projectViewKey() { return typeof currentWorkspace !== "undefined" && currentWorkspace?.id ? `glasstarn-view:${currentWorkspace.id}` : null; }
function rememberProjectView(view) {
  const key = projectViewKey();
  if (!key || ["home", "import"].includes(view)) return;
  try { localStorage.setItem(key, view); } catch (_error) { /* storage off: start on the Overview */ }
}
function rememberedProjectView() {
  const key = projectViewKey();
  if (!key) return null;
  try { return localStorage.getItem(key); } catch (_error) { return null; }
}
