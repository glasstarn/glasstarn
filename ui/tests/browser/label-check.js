// Map label check: paste into the browser console (or evaluate with
// Playwright) on the Process page. Returns a list of label problems:
// overlaps, numbers off their own line, cut-off names, stops off their line.
// An empty list means the map in view is clean.
// Runs in the page: returns label problems on the visible map.
(() => {
  const svgSub = document.querySelector('#subway-map:not([hidden])');
  const svgCls = document.querySelector('#results-map');
  const out = [];
  const R = (el) => el.getBoundingClientRect();
  const hit = (a, b, pad = 0) => Math.min(a.right, b.right) - Math.max(a.left, b.left) > pad && Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > pad;
  const union = (rs) => rs.reduce((u, r) => ({ left: Math.min(u.left, r.left), top: Math.min(u.top, r.top), right: Math.max(u.right, r.right), bottom: Math.max(u.bottom, r.bottom) }), { left: 1e9, top: 1e9, right: -1e9, bottom: -1e9 });
  const visible = (svg) => svg && svg.getBoundingClientRect().width > 0 && getComputedStyle(svg).display !== 'none';
  if (visible(svgSub)) {
    const svg = svgSub, box = R(svg);
    const tags = [...svg.querySelectorAll('g.subway-tag')].map(g => ({ type: g.dataset.lines, text: g.textContent.trim(), r: R(g.querySelector('rect')) }));
    const stations = [...svg.querySelectorAll('g.subway-station[data-activity]')].map(g => {
      const names = [...g.querySelectorAll('text.name, text.count')].filter(t => t.textContent.trim());
      return { name: g.dataset.activity, nameBox: names.length ? union(names.map(R)) : null, stop: g.querySelector('.stop') ? R(g.querySelector('.stop')) : null };
    });
    const rails = [...svg.querySelectorAll('g.subway-line path.rail')].map(p => {
      const type = p.closest('g.subway-line').dataset.lines, ctm = p.getScreenCTM(), len = p.getTotalLength(), pts = [];
      for (let d = 0; d <= len; d += 3) { const q = p.getPointAtLength(d); pts.push([q.x * ctm.a + ctm.e, q.y * ctm.d + ctm.f]); }
      return { type, transfer: p.classList.contains('transfer'), pts };
    });
    const dist = (r, type, sameType) => { let best = 1e9; for (const rail of rails) { if (rail.transfer) continue; if (sameType !== (rail.type === type)) continue; for (const [x, y] of rail.pts) { const dx = Math.max(r.left - x, 0, x - r.right), dy = Math.max(r.top - y, 0, y - r.bottom); best = Math.min(best, Math.hypot(dx, dy)); } } return best; };
    for (let i = 0; i < tags.length; i++) for (let j = i + 1; j < tags.length; j++) if (hit(tags[i].r, tags[j].r, 1)) out.push(`number on number: ${tags[i].text} / ${tags[j].text}`);
    for (const t of tags) {
      for (const s of stations) {
        if (s.nameBox && hit(t.r, s.nameBox, 1)) out.push(`number on name: ${t.text} on ${s.name}`);
        if (s.stop && hit(t.r, s.stop, 1)) out.push(`number on stop: ${t.text} on ${s.name}`);
      }
      if (t.text.includes(' / ')) continue;
      const own = dist(t.r, t.type, true), other = dist(t.r, t.type, false);
      if (own > 14) out.push(`number away from its line: ${t.text} (${t.type}) ${Math.round(own)}px`);
      else if (other < own - 2) out.push(`number nearer another line: ${t.text} (${t.type}) own ${Math.round(own)} other ${Math.round(other)}`);
      if (t.r.left < box.left - 1 || t.r.right > box.right + 1 || t.r.top < box.top - 1 || t.r.bottom > box.bottom + 1) out.push(`number cut off: ${t.text}`);
    }
    for (let i = 0; i < stations.length; i++) {
      const a = stations[i]; if (!a.nameBox) continue;
      if (a.nameBox.left < box.left - 1 || a.nameBox.right > box.right + 1 || a.nameBox.top < box.top - 1 || a.nameBox.bottom > box.bottom + 1) out.push(`name cut off: ${a.name}`);
      for (let j = 0; j < stations.length; j++) {
        if (i === j) continue; const b = stations[j];
        if (j > i && b.nameBox && hit(a.nameBox, b.nameBox, 1)) out.push(`name on name: ${a.name} / ${b.name}`);
        if (b.stop && hit(a.nameBox, b.stop, 1)) out.push(`name on stop: ${a.name} on ${b.name}`);
      }
      // A line running through the letters (the halo keeps it readable, but count it).
      let through = 0; for (const rail of rails) for (const [x, y] of rail.pts) if (x > a.nameBox.left + 2 && x < a.nameBox.right - 2 && y > a.nameBox.top + 2 && y < a.nameBox.bottom - 2) through++;
      if (through > 3) out.push(`line through name: ${a.name}`);
    }
    // Every stop sits on a line.
    for (const s of stations) { if (!s.stop) continue; const cx=(s.stop.left+s.stop.right)/2, cy=(s.stop.top+s.stop.bottom)/2; let best=1e9; for (const rail of rails) for (const [x,y] of rail.pts) best=Math.min(best, Math.hypot(x-cx,y-cy)); if (best > Math.max(6, Math.max(s.stop.bottom-s.stop.top, s.stop.right-s.stop.left)/2)) out.push(`stop off its line: ${s.name} ${Math.round(best)}px`); }
    for (const mark of svg.querySelectorAll('g.subway-mark:not(.inside)')) { const r = R(mark); for (const s of stations) if (s.nameBox && hit(r, s.nameBox, 1)) out.push(`end mark on name: ${s.name}`); for (const t of tags) if (hit(r, t.r, 1)) out.push(`end mark on number: ${t.text}`); }
  } else if (visible(svgCls)) {
    const svg = svgCls, box = R(svg);
    const pills = [...svg.querySelectorAll('.elabel-pill')].filter(g => getComputedStyle(g).opacity !== '0').map(g => ({ text: g.textContent.trim(), r: R(g.querySelector('rect')) }));
    const nodes = [...svg.querySelectorAll('.node')].map(g => ({ name: g.dataset.activity, r: R(g.querySelector('rect.pill') || g) }));
    for (let i = 0; i < pills.length; i++) for (let j = i + 1; j < pills.length; j++) if (hit(pills[i].r, pills[j].r, 1)) out.push(`number on number: ${pills[i].text} / ${pills[j].text}`);
    for (const p of pills) for (const n of nodes) if (hit(p.r, n.r, 1)) out.push(`number on step: ${p.text} on ${n.name}`);
    for (let i = 0; i < nodes.length; i++) for (let j = i + 1; j < nodes.length; j++) if (hit(nodes[i].r, nodes[j].r, 1)) out.push(`step on step: ${nodes[i].name} / ${nodes[j].name}`);
  } else out.push('no map visible');
  return [...new Set(out)];
})()
