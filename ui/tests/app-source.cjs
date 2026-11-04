// The browser loads ui/src as classic scripts sharing one global scope, in the
// order of ui/index.html. Tests evaluate the same files in that order, without
// main.ts, which only starts the application.
const fs = require('node:fs');
const path = require('node:path');

const html = fs.readFileSync(path.join(__dirname, '../index.html'), 'utf8');
const files = [...html.matchAll(/<script src="\/assets\/([\w-]+)\.js" defer><\/script>/g)].map(match => match[1]);
if (files.at(-1) !== 'main') throw new Error('ui/index.html must load main.js last');

module.exports = files.slice(0, -1)
  .map(name => fs.readFileSync(path.join(__dirname, `../src/${name}.ts`), 'utf8'))
  .join('\n');
