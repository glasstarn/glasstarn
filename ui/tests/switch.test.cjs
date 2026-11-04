// Switches: the knob slides under whichever button is pressed.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const { test } = require('node:test');
const source = require('./app-source.cjs');

test('the knob takes the size and place of the pressed button', () => {
  const context = vm.createContext({});
  vm.runInContext(source, context);
  const knob = { style: {} };
  const pressed = { offsetWidth: 84, offsetLeft: 70 };
  const seg = { querySelector: (selector) => selector === '.seg-knob' ? knob : pressed };
  context.seg = seg;
  vm.runInContext('placeSegmentKnob(seg)', context);
  assert.equal(knob.style.width, '84px');
  assert.equal(knob.style.transform, 'translateX(70px)');
  assert.equal(knob.style.opacity, '1');
});
