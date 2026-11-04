// Every user workflow, start to finish, the way an agent and a person go
// through it: the agent runs glasstarn commands, the person looks at the page.
// Runs against the release build with a scratch home folder and its own
// headless Chrome, so nothing on this computer is touched.
//
//   cargo build --release -p pm-cli && node tests/journeys/journeys.mjs
//
// Prints one line per check and exits non-zero if any fails.
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const binary = path.join(repo, "target/release/glasstarn");
const data = path.join(repo, "testdata/journeys");
const chromePath = process.env.CHROME || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const home = fs.mkdtempSync(path.join(os.tmpdir(), "glasstarn-journeys-"));
const work = path.join(home, "Documents", "claims-review");
fs.mkdirSync(work, { recursive: true });
fs.copyFileSync(path.join(data, "claims.csv"), path.join(work, "claims.csv"));
fs.copyFileSync(path.join(data, "claims.xlsx"), path.join(work, "claims.xlsx"));
fs.copyFileSync(path.join(data, "claims-sop.model.json"), path.join(work, "sop.json"));
const projects = path.join(work, "glasstarn");

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: Boolean(ok), detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${ok || !detail ? "" : `  (${detail})`}`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const env = { ...process.env, HOME: home, GLASSTARN_NO_BROWSER: "1", GLASSTARN_NO_UPDATE_CHECK: "1" };
function gl(args, { json = true } = {}) {
  const out = execFileSync(binary, args, { env, cwd: work, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  return json ? JSON.parse(out) : out;
}
function glTry(args) {
  try { return { ok: true, value: gl(args) }; } catch (error) { return { ok: false, error: String(error.stderr || error.message) }; }
}
function writeJson(name, value) {
  const file = path.join(work, name);
  fs.writeFileSync(file, JSON.stringify(value));
  return file;
}
function stopPages() {
  try { execFileSync("pkill", ["-f", `glasstarn-journeys`], { stdio: "ignore" }); } catch { /* none running */ }
}

// --- headless Chrome --------------------------------------------------------
const port = 9400 + Math.floor(Math.random() * 400);
// Outside the scratch home, so stopping glasstarn's pages never stops Chrome.
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "journeys-browser-"));
const chrome = spawn(chromePath, [
  "--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, "--no-first-run",
  "--no-default-browser-check", "--disable-background-timer-throttling", "--disable-renderer-backgrounding",
  "--disable-backgrounding-occluded-windows", "about:blank",
], { stdio: "ignore" });
for (let i = 0; i < 40; i++) {
  try { await fetch(`http://127.0.0.1:${port}/json/version`); break; } catch { await sleep(250); }
}
async function openTab(url, { width = 1440, height = 900, fresh = false } = {}) {
  let contextId;
  const browser = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  if (fresh) {
    const b = await connect(browser.webSocketDebuggerUrl);
    contextId = (await b.send("Target.createBrowserContext")).result.browserContextId;
    const target = (await b.send("Target.createTarget", { url: "about:blank", browserContextId: contextId })).result.targetId;
    b.close();
    const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    return tab(list.find((t) => t.id === target), url, width, height);
  }
  const created = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" })).json();
  return tab(created, url, width, height);
}
async function connect(wsUrl) {
  const ws = new WebSocket(wsUrl);
  await new Promise((r) => ws.addEventListener("open", r, { once: true }));
  let id = 0;
  const pending = new Map();
  ws.addEventListener("message", (e) => {
    const m = JSON.parse(e.data);
    if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
  });
  return {
    send: (method, params = {}) => new Promise((r) => { const n = ++id; pending.set(n, r); ws.send(JSON.stringify({ id: n, method, params })); }),
    close: () => ws.close(),
  };
}
async function tab(target, url, width, height) {
  const t = await connect(target.webSocketDebuggerUrl);
  await t.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });
  await t.send("Page.navigate", { url });
  await sleep(8000);
  return {
    async js(code) {
      const res = await t.send("Runtime.evaluate", { expression: `(async()=>{${code}})()`, awaitPromise: true, returnByValue: true });
      if (res.result?.exceptionDetails) return { error: res.result.exceptionDetails.exception?.description };
      return res.result?.result?.value;
    },
    async go(next) { await t.send("Page.navigate", { url: next }); await sleep(8000); },
    async front() { await t.send("Page.bringToFront"); await sleep(6000); },
    close: () => t.close(),
  };
}
const pageState = `return {
  title: document.title, view: document.body.dataset.view,
  chips: document.querySelector('#filter-chip-list')?.textContent.trim() || '',
  findings: document.querySelectorAll('.summary-list .summary-card').length,
  lead: document.querySelector('.summary-list .summary-card .summary-card-title')?.textContent || '',
  earlier: document.querySelector('.answer-history')?.textContent || '',
  conformanceTab: getComputedStyle(document.querySelector('#conformance-nav')).display !== 'none',
  items: typeof currentAnalysis !== 'undefined' ? currentAnalysis?.summary?.cases : null,
  wide: document.documentElement.scrollWidth > innerWidth + 2,
  error: document.querySelector('#access-error:not([hidden]), #service-stopped') ? 'error banner' : '',
};`;

try {
  // J1: first analysis in a folder, as an agent does it.
  const created = gl(["workspace", "create", "--name", "Claims review", "--workspace-dir", projects]);
  const id = created.project.id;
  const imported = gl(["workspace", "import", id, "claims.csv", "--workspace-dir", projects]);
  const titles = imported.headline.findings.map((f) => f.title).join(" | ");
  check("first analysis finds the slow documents wait", /Request documents → Documents received is the longest wait/.test(titles), titles);
  check("first analysis finds South slow to start", /Region = South/.test(titles), titles);
  check("first analysis finds Home claims slower", /Claim Type = Home/.test(titles), titles);
  check("first analysis finds the large-amount band", /Claim Amount/.test(titles), titles);
  check("first analysis finds payments without approval, one adjuster", /41 cases reached Pay claim without Approve payment/.test(titles) && /Leo Grant/.test(imported.headline.findings.map((f) => f.detail).join(" ")), titles);
  gl(["workspace", "record", id, "--request", writeJson("r1.json", { version: 1, question: "Health check", answer: "Document waits and South's slow start drive most delay.", findings: [] }), "--workspace-dir", projects]);
  const opened = gl(["open", "--workspace-dir", projects, "--project", id]);
  check("results link is a plain address", /^http:\/\/127\.0\.0\.1:\d+\/\?project=[0-9a-f]{32}$/.test(opened.url), opened.url);

  // The page, the way the person sees it.
  const page = await openTab(opened.url);
  await page.js(`window.__errs = []; addEventListener('error', e => __errs.push(String(e.message))); addEventListener('unhandledrejection', e => __errs.push(String(e.reason?.message || e.reason))); return 1;`);
  let s = await page.js(pageState);
  check("page opens on the Overview", s.view === "report", JSON.stringify(s));
  check("Overview leads with the agent's answer", /Document waits/.test(s.lead), s.lead);
  check("Overview lists the findings", s.findings >= 8, `${s.findings}`);
  check("Overview is unfiltered", !s.chips && s.items === 1200, `${s.chips} ${s.items}`);
  check("page fits the window", !s.wide);
  check("a plain analysis has no Conformance tab", !s.conformanceTab);

  // J2: the same link in another browser, with nothing remembered.
  const other = await openTab(opened.url, { fresh: true });
  s = await other.js(pageState);
  check("the link works in a second browser", s.view === "report" && s.findings >= 8 && !s.error, JSON.stringify(s));
  other.close();

  // J3: a follow-up while the page is open shows up there by itself.
  gl(["workspace", "record", id, "--request", writeJson("r2.json", { version: 1, question: "What could be automated?", answer: "Document chasing is the best place to start automating.", findings: [] }), "--workspace-dir", projects]);
  await sleep(8000);
  s = await page.js(pageState);
  check("a follow-up answer appears in the open page", /Document chasing/.test(s.lead) && /Health check/.test(s.earlier), `${s.lead} / ${s.earlier.slice(0, 80)}`);
  const again = gl(["open", "--workspace-dir", projects, "--project", id]);
  check("asking to open again reuses the running page", again.already_running === true, JSON.stringify(again));
  check("while the page is showing, opening again opens nothing new", again.page_showing === true && again.opened_in_browser === false, JSON.stringify(again));

  // J4: findings lead to their items; the Overview never keeps a filter.
  const leaks = await page.js(`
    const out = [];
    const cards = () => [...document.querySelectorAll('.summary-list .summary-card')];
    for (let i = 1; i < Math.min(cards().length, 7); i++) {
      const c = cards()[i]; c.open = true;
      const b = c.querySelector('.finding-open'); if (!b) continue;
      b.click(); await new Promise(r => setTimeout(r, 3500));
      const away = document.querySelector('#filter-chip-list')?.textContent.trim();
      document.querySelector('#process-reset')?.click(); await new Promise(r => setTimeout(r, 800));
      document.querySelector('.nav-item[data-view=report]').click(); await new Promise(r => setTimeout(r, 3500));
      out.push({ away, back: document.querySelector('#filter-chip-list')?.textContent.trim(), items: currentAnalysis.summary.cases });
    }
    return out;`);
  check("each finding opens with its own filter", leaks.every((l) => l.away), JSON.stringify(leaks));
  check("back on the Overview, no filter is left on", leaks.every((l) => !l.back && l.items === 1200), JSON.stringify(leaks));

  // J5: a Variants visit from a filtered finding shows all its items.
  const variants = await page.js(`
    const c = [...document.querySelectorAll('.summary-list .summary-card')].find(c => /main variants/.test(c.textContent));
    c.open = true; c.querySelector('.finding-open').click(); await new Promise(r => setTimeout(r, 3500));
    document.querySelector('.nav-item[data-view=variants]').click(); await new Promise(r => setTimeout(r, 4500));
    const text = document.querySelector('#variant-coverage')?.textContent;
    document.querySelector('.nav-item[data-view=report]').click(); await new Promise(r => setTimeout(r, 3000));
    return text;`);
  check("Variants from a finding shows every path in it", /^(\d+) selected of \1 variants/.test(variants || ""), variants);

  // J5b: "show me the three main variants" in the agent moves the open page.
  gl(["workspace", "record", id, "--request", writeJson("show.json", { version: 1, question: "Show me the three main variants", answer: "The three main variants cover most claims.", findings: [], show: { view: "variants", filter: { top_variants: 3 } } }), "--workspace-dir", projects]);
  await sleep(9000);
  const shownPage = await page.js(`return { view: document.body.dataset.view, chips: document.querySelector('#filter-chip-list')?.textContent.trim() };`);
  check("asking the agent to show the three main variants moves the open page there, with its filter", shownPage.view === "variants" && /Top 3 variants/.test(shownPage.chips || ""), JSON.stringify(shownPage));
  // The agent's link clicked afterwards opens a page on the same view, filter kept.
  const linked = await openTab(gl(["open", "--workspace-dir", projects, "--project", id]).url);
  const linkedPage = await linked.js(`return { view: document.body.dataset.view, chips: document.querySelector('#filter-chip-list')?.textContent.trim() };`);
  linked.close();
  // Back to the person's one page, as they would be.
  await page.front();
  check("clicking the agent's link afterwards shows the same view and filter, not a reset page", linkedPage.view === "variants" && /Top 3 variants/.test(linkedPage.chips || ""), JSON.stringify(linkedPage));
  gl(["workspace", "record", id, "--request", writeJson("show2.json", { version: 1, question: "Show me the main path", answer: "The main path.", findings: [], show: { view: "process", detail: "main", filter: {} } }), "--workspace-dir", projects]);
  await sleep(9000);
  const mainPath = await page.js(`const out = { view: document.body.dataset.view, chips: document.querySelector('#filter-chip-list')?.textContent.trim() }; document.querySelector('.nav-item[data-view=report]').click(); await new Promise(r => setTimeout(r, 3000)); return out;`);
  check("asking for the main path moves the page to Process and clears the filter", mainPath.view === "map" && !mainPath.chips, JSON.stringify(mainPath));
  // Back to the follow-up answer the later journeys look for.
  gl(["workspace", "record", id, "--request", path.join(work, "r2.json"), "--workspace-dir", projects]);
  await sleep(6000);

  // J6: an SOP check while the page is open shows on its Conformance tab.
  const sopCheck = glTry(["workspace", "query", id, "--request", writeJson("check.json", { type: "conformance", model: path.join(work, "sop.json"), algorithm: "token", limit: 5 }), "--workspace-dir", projects]);
  check("an SOP check runs while the page is open", sopCheck.ok, sopCheck.error);
  const bySteps = sopCheck.ok ? sopCheck.value.response.result.by_step.map((d) => `${d.cases} ${d.activity} ${d.kind}`).join(" | ") : "";
  // The SOP has optional steps, so the check is the exact one whatever was
  // asked: 628 claims follow it, 553 skipped notifying, 41 skipped approval.
  check("the SOP check is exact for an SOP with optional steps", sopCheck.ok && sopCheck.value.response.result.algorithm === "astar_alignment" && sopCheck.value.response.result.fit_cases === 628, sopCheck.ok ? `${sopCheck.value.response.result.algorithm} ${sopCheck.value.response.result.fit_cases}` : "");
  check("the SOP check counts payments without approval", /41 Approve payment skipped/.test(bySteps) && /553 Notify customer skipped/.test(bySteps), bySteps);
  await sleep(8000);
  const conf = await page.js(`
    const tab = getComputedStyle(document.querySelector('#conformance-nav')).display !== 'none';
    document.querySelector('#conformance-nav').click(); await new Promise(r => setTimeout(r, 6000));
    return { tab, headline: document.querySelector('#conf-headline')?.textContent, groups: document.querySelector('#conf-groups')?.innerText,
      line: document.querySelector('#conf-line')?.innerText, controls: Boolean(document.querySelector('#case-conformance-algorithm, #case-conformance-limit')), wide: document.documentElement.scrollWidth > innerWidth + 2 };`);
  check("the open page gets a Conformance tab", conf.tab, JSON.stringify(conf));
  check("the Conformance tab leads with one plain answer", /^572 of 1,200 claims did not follow /.test(conf.headline || ""), JSON.stringify(conf));
  check("the Conformance tab lists what broke the SOP, one row each", /Skipped “Notify customer”\s+553\s/.test(conf.groups || "") && /Skipped “Approve payment”\s+41\s/.test(conf.groups || ""), JSON.stringify(conf));
  check("the Conformance tab shows the SOP as steps", /Approve payment\s+41 skipped/.test(conf.line || ""), JSON.stringify(conf));
  check("the Conformance tab has no method controls on top and fits the window", !conf.controls && !conf.wide, JSON.stringify(conf));
  const opened41 = await page.js(`
    [...document.querySelectorAll('#conf-groups tr')].find(c => /Approve payment/.test(c.textContent)).click();
    await new Promise(r => setTimeout(r, 5000));
    const out = { view: document.body.dataset.view, items: currentAnalysis.summary.cases, chips: document.querySelector('#filter-chip-list')?.textContent.trim() };
    document.querySelector('.nav-item[data-view=report]').click(); await new Promise(r => setTimeout(r, 3000));
    return out;`);
  check("a Conformance row opens exactly its claims under one chip", opened41.view === "cases" && opened41.items === 41 && /^Skipped “Approve payment”/.test(opened41.chips || "") && !/Variant/.test(opened41.chips || ""), JSON.stringify(opened41));
  page.close();

  // J7: after a restart, the old link is gone and a fresh one has everything.
  stopPages();
  await sleep(1500);
  const deadOk = await fetch(opened.url).then(() => true, () => false);
  check("after a restart the old page is closed", !deadOk);
  const reopened = gl(["open", "--workspace-dir", projects, "--project", id]);
  const page2 = await openTab(reopened.url);
  s = await page2.js(pageState);
  check("reopened project keeps every answer", /Document chasing/.test(s.lead) && /Health check/.test(s.earlier), `${s.lead} / ${s.earlier.slice(0, 60)}`);
  check("reopened project keeps its Conformance tab", s.conformanceTab);
  page2.close();

  // J8: a new chat finds the project from anywhere.
  const listed = gl(["workspace", "list"]);
  check("the project is listed from glasstarn's home (folder remembered)", JSON.stringify(listed).includes("Claims review"), JSON.stringify(listed).slice(0, 200));

  const shown = glTry(["workspace", "show", id]);
  check("a new chat can open the project by id from anywhere", shown.ok && shown.value.project?.name === "Claims review", shown.error);
  check("a new chat learns the project's SOP and when it was set", shown.ok && /Claims Handling SOP/.test(shown.value.expected_process?.name || "") && /^\d{4}-\d{2}-\d{2}$/.test(shown.value.expected_process?.set_on || ""), JSON.stringify(shown.value?.expected_process));

  // J8b: an SOP check with no page open shows when the page next opens.
  stopPages();
  await sleep(1000);
  const quiet = gl(["workspace", "create", "--name", "Quiet check", "--workspace-dir", projects]).project.id;
  gl(["workspace", "import", quiet, "claims.csv", "--workspace-dir", projects]);
  const qpage = await openTab(gl(["open", "--workspace-dir", projects, "--project", quiet]).url);
  // The agent's later link (the running page, no key in it) in the same tab.
  await qpage.go(gl(["open", "--workspace-dir", projects, "--project", quiet]).url);
  const quietCheck = glTry(["workspace", "query", quiet, "--request", path.join(work, "check.json"), "--workspace-dir", projects]);
  check("an SOP check runs while the page opened from the agent's link is showing", quietCheck.ok, quietCheck.error);
  await sleep(8000);
  const qconf = await qpage.js(`
    const tab = getComputedStyle(document.querySelector('#conformance-nav')).display !== 'none';
    document.querySelector('#conformance-nav').click(); await new Promise(r => setTimeout(r, 6000));
    return { tab, steps: document.querySelector('#conf-groups')?.innerText };`);
  check("that page shows the check on its Conformance tab", qconf.tab && /Approve payment”\s+41\s/.test(qconf.steps || ""), JSON.stringify(qconf));
  qpage.close();

  // J9: the same export as Excel gives the same findings.
  const x = gl(["workspace", "create", "--name", "Claims from Excel", "--workspace-dir", projects]).project.id;
  const xi = gl(["workspace", "import", x, "claims.xlsx", "--workspace-dir", projects]);
  check("an Excel export gives the same findings as the CSV", xi.headline.findings.length === imported.headline.findings.length && xi.headline.summary.cases === 1200, `${xi.headline.findings.length} vs ${imported.headline.findings.length}`);

  // J10: two projects in one folder, both listed and openable.
  const home2 = gl(["open", "--workspace-dir", projects]);
  const list = await openTab(home2.url);
  const rows = await list.js(`return [...document.querySelectorAll('#workspace-list .project-row strong')].map(e => e.textContent)`);
  check("one folder holds both projects on the projects page", rows.includes("Claims review") && rows.includes("Claims from Excel"), JSON.stringify(rows));

  // J11: rename and delete from the projects page, even after opening.
  const renamed = await list.js(`
    window.prompt = () => 'Excel claims'; window.confirm = () => true;
    const p = (await refreshWorkspaces()).projects.find(p => p.name === 'Claims from Excel');
    await workspaceApi('/api/workspaces/open', { id: p.id });
    await renameProject(p);
    const after = (await refreshWorkspaces()).projects.find(q => q.id === p.id)?.name;
    await deleteProject({ ...p, name: after });
    const gone = !(await refreshWorkspaces()).projects.some(q => q.id === p.id);
    return { after, gone };`);
  check("rename works on a project that was opened", renamed.after === "Excel claims", JSON.stringify(renamed));
  check("delete moves it to the Trash", renamed.gone && fs.existsSync(path.join(home, process.platform === "darwin" ? ".Trash" : ".local/share/Trash/files", "Excel claims (glasstarn project)")), JSON.stringify(renamed));
  list.close();

  // J12: a project started before there were files moves into the folder.
  const early = gl(["workspace", "create", "--name", "Purchase orders"]).project.id;
  gl(["workspace", "record", early, "--request", writeJson("plan.json", { version: 1, question: "Why are purchase orders slow?", answer: "Waiting for the SAP export.", findings: [] })]);
  const moved = glTry(["workspace", "move", early, "--to", projects]);
  check("a project started without files moves into the person's folder", moved.ok && fs.existsSync(path.join(projects, early, "workspace.json")), moved.error);

  // J13: connected data, subway map, following another item type.
  const conn = gl(["demo", "connected", "--workspace-dir", projects, "--json"]).project.id;
  const cpage = await openTab(gl(["open", "--workspace-dir", projects, "--project", conn]).url);
  const connected = await cpage.js(`
    document.querySelector('.nav-item[data-view=map]').click(); await new Promise(r => setTimeout(r, 3000));
    document.querySelector('[data-map-style=subway]')?.click(); await new Promise(r => setTimeout(r, 3000));
    const lines = document.querySelectorAll('#subway-legend button').length;
    const before = currentFollow?.type;
    [...document.querySelectorAll('#follow-types button')].find(b => b.getAttribute('aria-pressed') !== 'true')?.click();
    await new Promise(r => setTimeout(r, 6000));
    document.querySelector('.nav-item[data-view=report]').click(); await new Promise(r => setTimeout(r, 3000));
    return { lines, before, after: currentFollow?.type, chips: document.querySelector('#filter-chip-list')?.textContent.trim(), findings: document.querySelectorAll('.summary-list .summary-card').length };`);
  check("connected data draws a subway map", connected.lines >= 3, JSON.stringify(connected));
  check("following another item type works and the Overview follows it", connected.after && connected.after !== connected.before && connected.findings > 0 && !connected.chips, JSON.stringify(connected));
  cpage.close();

  // J14: half-width window (an agent's side browser).
  const half = await openTab(gl(["open", "--workspace-dir", projects, "--project", id]).url, { width: 720 });
  s = await half.js(pageState);
  check("half-width window fits without spilling sideways", !s.wide && s.findings >= 8, JSON.stringify(s));
  half.close();

  // J15: an unused page stops by itself.
  const idle = spawn(binary, ["serve", "--no-open", "--workspace-dir", projects], { env: { ...env, GLASSTARN_IDLE_SECONDS: "2" }, stdio: "ignore" });
  const stopped = await Promise.race([new Promise((r) => idle.on("exit", () => r(true))), sleep(15000).then(() => false)]);
  check("an unused page stops by itself", stopped);
  if (!stopped) idle.kill();
} catch (error) {
  check("journeys ran to the end", false, error.stack || String(error));
} finally {
  stopPages();
  chrome.kill();
}
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length} of ${results.length} checks passed.`);
process.exit(failed.length ? 1 : 0);
