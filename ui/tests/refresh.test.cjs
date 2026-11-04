// The 2026-10-17 refresh: lowercase name, the glacier-lake mark, and the
// tokens the new surfaces use in both themes.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');

const html = fs.readFileSync(path.join(__dirname, '../index.html'), 'utf8');
const css = fs.readFileSync(path.join(__dirname, '../src/app.css'), 'utf8');

test('the name is lowercase everywhere a person reads it', () => {
  assert.match(html, /<title>glasstarn<\/title>/);
  assert.match(html, /<span class="brand-word">glasstarn<\/span>/);
  const visible = html.replace(/<[^>]+>/g, ' ');
  assert.doesNotMatch(visible, /Glasstarn/);
});

test('the header carries the peak-and-reflection mark', () => {
  const mark = html.match(/<svg class="brand-mark"[\s\S]*?<\/svg>/)?.[0] || '';
  assert.match(mark, /fill="var\(--ink\)"/);
  assert.match(mark, /fill="var\(--glacier\)"/);
});

test('refresh tokens exist in light and in both dark blocks', () => {
  for (const token of ['--ink', '--ink-fg', '--glacier', '--edge-ink']) {
    const count = (css.match(new RegExp(`${token}:`, 'g')) || []).length;
    assert.ok(count >= 3, `${token} is defined ${count} times`);
  }
});

test('findings are numbered in order', () => {
  assert.match(css, /summary-list > \.summary-card > summary::before \{ content: counter\(finding, decimal-leading-zero\)/);
});
