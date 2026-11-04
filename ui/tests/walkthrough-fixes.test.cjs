// Fixes from the 2026-10-22 user walk-through, checked in the shipped files.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');
const read = (file) => fs.readFileSync(path.join(__dirname, '..', file), 'utf8');
const html = read('index.html'), css = read('src/app.css'), results = read('src/results.ts');

test('Compare runs from a primary button and variants reset says what it does', () => {
  assert.match(html, /class="primary-action" id="compare-cohorts"/);
  assert.match(html, /class="primary-action" id="compare-periods"/);
  assert.match(html, /id="variant-reset">Clear</);
});

test('the date notice is a tinted note, not a coloured edge', () => {
  assert.match(css, /\.results \.data-context\.unusual \{ border: 0;/);
});

test('Import file is the main button; connecting an agent is optional and hidden until asked', () => {
  assert.match(html, /class="btn primary home-only" type="button" id="app-import">Import file</);
  assert.match(html, /class="btn home-only" type="button" id="app-agent">Connect an agent</);
  assert.match(css, /\.agent-journey:not\(\.expanded\) \{ display: none; \}/);
});

test('connecting an agent is three short numbered steps with one copy button each', () => {
  for (const step of ['Choose what your agent can see', 'Add glasstarn to your agent', 'Paste this in your agent']) assert.ok(html.includes(`<h3>${step}</h3>`), step);
  assert.equal((html.match(/class="agent-step"/g) || []).length, 3);
  assert.match(css, /\.agent-step::before \{ content: counter\(agent-step\)/);
  assert.doesNotMatch(html, /Process Mining running|Agent Host sessions/);
  assert.doesNotMatch(html, /results link|map, slides/);
  assert.doesNotMatch(html, /Rust/, 'people never see the engine’s programming language');
});

test('Connect an agent on the home page shows the three steps', () => {
  assert.match(results, /journey\.classList\.add\("expanded", "spotlight"\)/);
  assert.match(css, /\.agent-journey\.expanded ol \{ display: grid;/);
});

test('the Measure panel words repeats plainly and hides method text by default', () => {
  assert.match(html, /<summary>When a step repeats<\/summary>/);
  assert.match(html, /<summary>How this is measured<\/summary><p id="studio-semantics">/);
});
