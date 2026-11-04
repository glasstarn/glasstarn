const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

function app() {
  const element = () => ({ dataset: {}, style: {}, children: [], textContent: '', className: '', listeners: {},
    replaceChildren(...children) { this.children = children; },
    append(...children) { this.children.push(...children); },
    addEventListener(name, fn) { this.listeners[name] = fn; },
    setAttribute() {},
  });
  const elements = new Map();
  const context = vm.createContext({ document: {
    querySelector(selector) { if (!elements.has(selector)) elements.set(selector, element()); return elements.get(selector); },
    createElement: element,
  } });
  vm.runInContext(source, context);
  const text = node => typeof node === 'string' ? node : [node.textContent, ...(node.children || []).map(text)].filter(Boolean).join(' ');
  return { run: code => vm.runInContext(code, context), elements, text };
}

const analysis = `({ summary: { cases: 334, events: 1679 }, key_findings: [
  { kind: 'working_well', severity: 'good', title: 'A → B is quick', detail: '', tags: ['Working well'], view: 'map', reach_percent: 90 },
  { kind: 'rework', severity: 'medium', title: '13% of {items} repeat a step', detail: 'In 44 {items}.', tags: ['Rework', 'Proposal'], view: 'map', reach_percent: 13 },
  { kind: 'bottleneck', severity: 'high', title: 'Deal created → Order created takes 34 days on average', detail: '', tags: ['Bottleneck', 'x'], view: 'timing', reach_percent: 40 } ] })`;
const built = `({ slides: [{ title: 'fallback' }] })`;

test('without an agent answer, Summary is ranked engine cards, all closed', () => {
  const { run, elements, text } = app();
  run(`currentFollow = { singular: 'deal', plural: 'Deals' }; currentJourney = { findings: [
    { kind: 'drop_out', severity: 'high', title: '23 {type:invoice} with “Issued” have no “Paid” yet · 1.3M in amount', detail: '', tags: ['Drop-out'], view: 'journey', reach_percent: 15 } ] };
    renderInvestigationAnswer(null, ${built}, ${analysis}, []);`);
  const [cards] = elements.get('#investigation-answer').children;
  assert.equal(text(cards.children[0]), 'What stands out Most important first · based on 334 deals and 1,679 recorded steps.');
  const rows = cards.children[1].children.filter(child => child.className === 'summary-card');
  assert.equal(rows.length, 4);
  assert.match(text(rows[0]), /^Bottleneck Deal created → Order created takes 34 days on average/);
  assert.notEqual(rows[0].open, true);
  assert.notEqual(rows[1].open, true);
  assert.match(text(rows[1]), /^Drop-out 23 invoices with/);
  assert.match(text(rows[2]), /^Repeated steps 13% of deals repeat a step In 44 deals\. Proposal/);
  assert.match(text(rows[3]), /Working well/);
  assert.doesNotMatch(text(cards), /Answer|Your process, explained|Calculated by the engine/);
});

test('an agent answer leads as the first card, its findings follow, earlier questions are kept', () => {
  const { run, elements, text } = app();
  run(`currentFollow = null; currentMapping = { case_id: 'invoice_id' };
    renderInvestigationAnswer({ note: { question: 'Why are invoices paid late?', answer: 'Approval waits 6 days and drives most late payments.',
      process: 'Purchase-to-pay; the goal is paying on time.', next_questions: ['Shall I check which approvers hold invoices longest?'],
      findings: [{ kind: 'observed', title: 'Approval waits 6 days', statement: 'Median wait before approval.', evidence: [] },
                 { kind: 'hypothesis', statement: 'Approver holidays may explain the slow weeks.', evidence: [] }] } },
      ${built}, ${analysis}, [{ question: 'Health check', answer: 'Most time is spent before approval.', findings: [{ title: 'Approval is slow' }], recorded_at: '2026-10-10T20:00:00Z', current: true }]);`);
  const [cards, history] = elements.get('#investigation-answer').children;
  const rows = cards.children[1].children.filter(child => child.className === 'summary-card');
  assert.match(text(rows[0]), /^Your agent Approval waits 6 days and drives most late payments\. Your question: “Why are invoices paid late\?” Purchase-to-pay/);
  assert.match(text(rows[0]), /Worth asking next Shall I check which approvers hold invoices longest\?/);
  assert.notEqual(rows[0].open, true);
  // The agent's restated observation gives way to the engine's measured
  // findings; its idea to check follows them, in one list.
  assert.doesNotMatch(text(cards), /Your agent Approval waits 6 days Median wait/);
  assert.match(text(rows[rows.length - 1]), /^Idea to check Approver holidays/);
  assert.equal(rows.length, 1 + 3 + 1, 'answer, three engine facts, one idea');
  assert.match(text(history), /^Earlier questions \(1\) Health check Most time is spent before approval\. Approval is slow/);
});

test('ask-next questions come from the agent, or from what the engine found', () => {
  const { run } = app();
  run(`currentFollow = null; currentMapping = { case_id: 'invoice_id' };`);
  assert.deepEqual([...run(`askNextQuestions({ next_questions: ['A?', 'B?', 'C?', 'D?'] }, [])`)], ['A?', 'B?', 'C?']);
  const asks = [...run(`askNextQuestions(null, [
    { kind: 'bottleneck', tags: ['Bottleneck', 'Receive → Approve'] },
    { kind: 'rework', tags: ['Rework', 'Approve'] },
    { kind: 'working_well', tags: ['Working well'] }])`)];
  assert.deepEqual(asks, ['Why does Receive → Approve take so long?', 'Why do invoices repeat “Approve”?']);
});

test('the recognised process frames the ranking and the time figure', () => {
  const { run, elements, text } = app();
  run(`currentFollow = null; currentMapping = { case_id: 'invoice_id' }; lastReport = null;
    renderInvestigationAnswer(null, ${built}, Object.assign(${analysis}, { process: { kind: 'it_tickets', label: 'IT or service tickets', goal: 'Fast first resolution', time_label: 'Time to resolve', emphasis: ['rework', 'bottleneck'], source: 'engine' } }), []);`);
  const [cards] = elements.get('#investigation-answer').children;
  assert.match(text(cards.children[0]), /^What stands out Most important first · based on 334 invoices/);
  assert.match(text(cards.children[0]), /Looks like IT or service tickets, from the step names/);
  const rows = cards.children[1].children.filter(child => child.className === 'summary-card');
  assert.match(text(rows[0]), /^Bottleneck/, 'high severity still comes first');
  assert.match(text(rows[1]), /^Repeated steps/, 'then what matters for the goal');
});

test('Overview retains important milestone findings with endpoints and an intermediate-work caveat',()=>{
  const {run,elements,text}=app();
  run(`currentFollow={singular:'order',plural:'Orders'};currentJourney={findings:[
    {kind:'drop_out',severity:'high',title:'344 invoices have no recorded payment',detail:'Invoice issued → Payment received.',tags:['Drop-out'],view:'journey',reach_percent:19,
      target:{kind:'handoff',object_type:'invoice',from:'Invoice issued',to:'Payment received'}}]};
    renderInvestigationAnswer(null,${built},{summary:{cases:1809,events:4000},key_findings:[]},[]);`);
  const rendered=text(elements.get('#investigation-answer'));
  assert.match(rendered,/344 invoices have no recorded payment/);
  assert.match(rendered,/Invoice issued → Payment received/);
  assert.match(rendered,/can include intermediate work/);
  assert.match(rendered,/work in progress or incomplete coverage/);
  assert.match(rendered,/Open affected Items/);
});

test('Overview preserves every available calculated finding alongside an agent answer',()=>{
  const {run,elements,text}=app();
  run(`currentFollow=null;currentJourney=null;
    renderInvestigationAnswer({note:{answer:'Review this process.',findings:[{kind:'hypothesis',statement:'Check missing approval history.'}],assumptions:['Payment means posted payment.'],missing_data:['Approval transitions are missing.']}},${built},
    {summary:{cases:10,events:100},key_findings:Array.from({length:13},(_,i)=>({kind:'bottleneck',severity:'high',title:'Finding '+i,detail:'Measured evidence',tags:['Bottleneck'],view:'map',reach_percent:90-i}))},[]);`);
  const result=elements.get('#investigation-answer');
  for(let i=0;i<13;i++)assert.match(text(result),new RegExp('Finding '+i));
  assert.match(text(result),/More calculated findings \(5\)/);
  // Assumptions and open questions sit in "How this data was built".
  const notes=text(elements.get('#data-built-notes'));
  assert.match(notes,/Payment means posted payment/);
  assert.match(notes,/Approval transitions are missing/);
  assert.doesNotMatch(text(result),/Assumptions and missing evidence/);
});
