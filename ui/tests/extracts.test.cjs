const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

const context = vm.createContext({ document: { querySelector() { return null; } } });
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);

test('a Jira extract shows its two files, the JQL and the endpoints for approval', () => {
  run(`preview = { system: 'jira', site: 'https://acme.atlassian.net', issue_fields: ['key', 'status'],
    history_fields: ['status'], history_gaps: [], jql: 'project in ("OPS")', endpoints: ['GET /rest/api/3/search/jql'] };`);
  const rows = JSON.parse(run('JSON.stringify(extractObjectRows(preview))'));
  assert.deepEqual(rows[0], ['JiraIssue', 'Records', 'key, status', 'created', 'None', 'None stated']);
  assert.equal(rows[1][2], 'status');
  assert.deepEqual(JSON.parse(run('JSON.stringify(extractQueries(preview))')), ['project in ("OPS")', 'GET /rest/api/3/search/jql']);
  assert.equal(run("extractSystemName('jira')"), 'Jira');
});

test('a Salesforce extract still lists its objects and SOQL', () => {
  run(`preview = { system: 'salesforce', objects: [{ name: 'Case', kind: 'records', fields: ['Id'], time_field: 'CreatedDate', history_gaps: [] }],
    exports: [{ soql: 'SELECT Id FROM Case' }] };`);
  const rows = JSON.parse(run('JSON.stringify(extractObjectRows(preview))'));
  assert.deepEqual(rows[0], ['Case', 'Records', 'Id', 'CreatedDate', 'None', 'None stated']);
  assert.deepEqual(JSON.parse(run('JSON.stringify(extractQueries(preview))')), ['SELECT Id FROM Case']);
});

test('a ServiceNow extract shows a file per table, the audit file and the endpoints', () => {
  run(`preview = { system: 'servicenow', instance: 'https://acme.service-now.com', history_fields: 'all', history_gaps: [],
    tables: [{ table: 'change_request', file: 'ServiceNowChangeRequest', fields: ['sys_id', 'state'], scope: 'records with activity in the window' },
             { table: 'change_task', file: 'ServiceNowChangeTask', fields: ['sys_id'], scope: 'records whose change_request is one of the change_request records read' }],
    endpoints: ['GET /api/now/table/sys_audit (field history)'] };`);
  const rows = JSON.parse(run('JSON.stringify(extractObjectRows(preview))'));
  assert.deepEqual(rows[0], ['ServiceNowChangeRequest', 'Records (change_request)', 'sys_id, state', 'sys_created_on', 'records with activity in the window', 'None stated']);
  assert.equal(rows[2][0], 'ServiceNowAudit');
  assert.equal(rows[2][2], 'All audited fields');
  assert.equal(JSON.parse(run('JSON.stringify(extractQueries(preview))')).length, 3);
  assert.equal(run("extractSystemName('servicenow')"), 'ServiceNow');
});

test('HubSpot quotes show their own creation field in both old and new previews', () => {
  for (const created of [undefined, 'hs_createdate']) {
    context.preview = { system: 'hubspot', objects: [
      { object: 'contacts', file: 'HubSpotContact', properties: ['createdate'], history: [], scope: 'test' },
      { object: 'quotes', file: 'HubSpotQuote', properties: ['hs_createdate'], history: [], created, scope: 'test' },
    ] };
    const rows = JSON.parse(run('JSON.stringify(extractObjectRows(preview))'));
    assert.equal(rows[0][3], 'createdate');
    assert.equal(rows[1][3], 'hs_createdate');
  }
});
