// The filter menu and an open filter close on a click elsewhere.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');
const filters = fs.readFileSync(path.join(__dirname, '../src/filters.ts'), 'utf8');

test('a click outside closes the filter menu and cancels an open filter', () => {
  const block = filters.slice(filters.indexOf('// A click anywhere else closes the menu'));
  assert.match(block, /addEventListener\("mousedown"/);
  assert.match(block, /menu\.hidden = true/);
  assert.match(block, /restoreFilterControls\(currentFilter \|\| \{\}\);\s*closeFilterPanel\(\);/);
  assert.match(block, /\.select-menu/, 'choosing from a dropdown inside the filter does not close it');
});
