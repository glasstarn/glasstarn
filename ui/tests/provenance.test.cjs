const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({ document: { querySelector() { return null; } } });
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

test('the evidence restates extracts, matches, following and data checks in plain words', () => {
  run(`currentOcelAnalysis = { source: 'C:/x/prepared.jsonocel', quality: { findings: [
      { severity: 'warning', title: 'Links to objects missing from the file', affected: 89 },
      { severity: 'information', title: 'Ignored', affected: 1 } ] } };
    currentFollow = { summary: { object_type: 'order', cases: 3, shared_events: 1, shared_event_copies: 1, excluded_events: 4 } };
    currentProvenance = { audit_log: true,
      extractions: [{ system: 'salesforce', window: { start: '2026-01-01T00:00:00Z', end_exclusive: '2026-02-01T00:00:00Z' },
        approved_by: 'Pat', approved_at: '2026-10-10T00:00:00Z', objects: [{ name: 'Opportunity', rows: 2 }] }],
      preparation: { tables: ['orders', 'deals'], links: [{ from: 'order', to: 'opportunity', on: ['deal_id'], meaning: 'from deal', matched: 3, unmatched: 1 }],
        totals: { items: 4, steps: 3, item_links: 3, step_links: 3, issues: 0 } } };`);
  const facts = Object.fromEntries([...run('provenanceFacts()')].map(([term, value]) => [term, value]));
  assert.equal(facts['Source file'], 'prepared.jsonocel');
  assert.match(facts['Salesforce extract'], /^Opportunity \(2 rows\) · .+ · approved by Pat on /);
  assert.equal(facts['Orders matched to opportunities'], 'on deal_id (“from deal”): 3 matched, 1 unmatched');
  assert.equal(facts.Result, '4 items, 3 steps, 6 links · no unresolved issues');
  assert.match(facts.Following, /1 step involves several orders and is counted once for each \(1 extra\)\. 4 steps involve no order and are left out\.$/);
  assert.equal(facts['Data checks'], 'Links to objects missing from the file (89)');
});

test('a process pack and each confirmed answer are listed with who confirmed them', () => {
  run(`currentOcelAnalysis = null; currentFollow = null;
    currentProvenance = { audit_log: true, extractions: [], preparation: null, packs: [{ id: 'sap-purchase-to-pay', status: 'untested', open_questions: 1,
      answers: [{ question: 'Is ZZAPPR_DAT a real step?', answer: 'Yes, show it as a step (name the step)', value: 'Approve purchase order',
        confirmed_by: 'Purchasing analyst', at: '2026-10-12T00:41:26Z' }] }] };`);
  const facts = [...run('provenanceFacts()')];
  assert.equal(facts[0][0], 'Process pack');
  assert.equal(facts[0][1], 'sap-purchase-to-pay (not yet tested on a real system) · 1 answer saved, 1 open');
  assert.match(facts[1][0], /^Confirmed by Purchasing analyst, /);
  assert.equal(facts[1][1], 'Is ZZAPPR_DAT a real step? Answer: Yes, show it as a step (name the step): Approve purchase order');
});
