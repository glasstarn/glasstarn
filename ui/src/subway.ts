// Subway map for connected processes: one coloured line per item type, drawn
// through that type's most common route. A step two types share is an
// interchange; a common side step (a rejection, a reminder) is a short spur.
// Lines, stations and counts come from the engine's connected analysis; this
// file only chooses which to draw and lays them out.

const SUBWAY_COLUMN = 88;
const SUBWAY_LANE = 128;
const SUBWAY_TRACK = 12;
const SUBWAY_MIN_SHARE = 0.25;
const SUBWAY_TRANSFER_SHARE = 0.02;
const SUBWAY_MAX_LINES = 7;
// How much the subway shows: Main routes, More side paths, Most side paths.
// Each level lowers the share of a line's items a side path needs and lets
// more side paths per line through.
// The last level, All, takes every side step and also draws skips and
// loop-backs between stops already on a line, as faint dotted arcs.
const SUBWAY_DETAIL = [{ share: 0.05, per: 3 }, { share: 0.02, per: 6 }, { share: 0.005, per: 10 }, { share: 0, per: Infinity, all: true }];
// A side path rejoins the line when at least this share of its items carry
// on to a later step of the line.
const SUBWAY_REJOIN_SHARE = 0.3;
let subwayDetailLevel = 0;
// A clicked connection (a stretch of line, a side path or a dotted arc):
// { type, from, to }, shown in the step panel like a step.
let subwayConnection = null;
const SUBWAY_COLOURS = ["--l1", "--l2", "--l3", "--l4", "--l5", "--l6", "--l7"];
let mapStyle = "subway";
let subwayFocus = null;
let subwaySelection = null;
let subwayOrientation = "vertical";
let subwayOrientationChosen = false;

// Until the person picks a direction, the map runs along the longer side of
// the canvas so it fills the space instead of a narrow strip.
function subwayAutoOrientation(width, height) {
  return width > height * 1.2 ? "horizontal" : "vertical";
}
let subwayShownTypes = null;
let subwayHiddenEvents = new Set();
let subwayColourTypes = new Map();
let subwayColourProject = null;
function subwayTypeColour(type){
  if(subwayColourProject!==currentWorkspace?.id){subwayColourTypes.clear();subwayColourProject=currentWorkspace?.id;}
  if(!subwayColourTypes.has(type))subwayColourTypes.set(type,`--l${subwayColourTypes.size%12+1}`);
  return subwayColourTypes.get(type);
}

// The route a type's items most often take, each step once, in order.
function subwayRoute(variant) {
  const route = [];
  for (const layer of variant.layers || []) for (const activity of layer.activities || []) if (!route.includes(activity)) route.push(activity);
  return route;
}

// Which lines and spurs to draw. A type gets a line when its most common
// route covers at least a quarter of its items and has two or more steps.
function subwayModel(analysis) {
  const variants = analysis?.variants || [];
  for(const type of [...new Set(variants.map(item=>item.object_type))].sort())subwayTypeColour(type);
  const flows = analysis?.graphs?.directly_follows || [];
  const candidates = [];
  for (const variant of variants) {
    if (variant.rank !== 1) continue;
    const share = (variant.share_millionths || 0) / 1e6;
    const route = subwayRoute(variant);
    if ((share < SUBWAY_MIN_SHARE || route.length < 2) && !subwayShownTypes?.has(variant.object_type)) continue;
    const total = Math.round(variant.objects / Math.max(share, 1e-6));
    candidates.push({ type: variant.object_type, objects: total, route, share });
  }
  candidates.sort((a, b) => b.objects - a.objects || a.type.localeCompare(b.type));
  const shown=subwayShownTypes===null?candidates.slice(0,SUBWAY_MAX_LINES):candidates.filter(line=>subwayShownTypes.has(line.type));
  const lines = shown.map(line=>({...line,colour:subwayTypeColour(line.type)}));
  const onLine = new Set(lines.flatMap((line) => line.route));
  const spurs = [];
  const detail = SUBWAY_DETAIL[subwayDetailLevel] || SUBWAY_DETAIL[0];
  for (const line of lines) {
    const own = flows
      .filter((flow) => flow.object_type === line.type && line.route.includes(flow.source) && !onLine.has(flow.target) && flow.objects >= line.objects * detail.share)
      .sort((a, b) => b.objects - a.objects)
      .slice(0, detail.per);
    for (const flow of own) {
      if (spurs.some((spur) => spur.to === flow.target)) continue;
      // Where most of the side path's items go next: back onto a later step
      // of the line (a detour), or nowhere drawn (they end there).
      const from = line.route.indexOf(flow.source);
      const back = flows
        .filter((next) => next.object_type === line.type && next.source === flow.target && line.route.indexOf(next.target) > from && next.objects >= flow.objects * SUBWAY_REJOIN_SHARE)
        .sort((a, b) => b.objects - a.objects)[0];
      spurs.push({ type: line.type, from: flow.source, to: flow.target, objects: flow.objects, mean_seconds: flow.mean_elapsed_micros / 1e6, rejoins: back?.target || null });
    }
  }
  // At All: connections between two stops of the same line that are not
  // next to each other on it: a skip forward, or a loop back.
  const skips = [];
  if (detail.all) {
    for (const line of lines) {
      for (const flow of flows) {
        if (flow.object_type !== line.type || !flow.objects) continue;
        const from = line.route.indexOf(flow.source), to = line.route.indexOf(flow.target);
        if (from < 0 || to < 0 || to === from + 1 || to === from) continue;
        skips.push({ type: line.type, from: flow.source, to: flow.target, objects: flow.objects, back: to <= from });
      }
    }
  }
  // Hand-offs into another line that do not pass through a shared step,
  // such as an order that later starts a return: drawn as a dotted link.
  const firsts = new Map(lines.map((line) => [line.route[0], line]));
  const transfers = [];
  for (const line of lines) {
    for (const flow of flows) {
      const target = firsts.get(flow.target);
      if (flow.object_type !== line.type || !target || target.type === line.type || line.route.includes(flow.target) || !line.route.includes(flow.source)) continue;
      if (flow.objects < line.objects * SUBWAY_TRANSFER_SHARE || transfers.some((item) => item.to === flow.target)) continue;
      transfers.push({ type: line.type, from: flow.source, to: flow.target, objects: flow.objects, mean_seconds: flow.mean_elapsed_micros / 1e6 });
    }
  }
  const segments = [];
  for (const line of lines) {
    for (let index = 1; index < line.route.length; index += 1) {
      const flow = flows.find((item) => item.object_type === line.type && item.source === line.route[index - 1] && item.target === line.route[index]);
      segments.push({ type: line.type, from: line.route[index - 1], to: line.route[index], objects: flow?.objects || 0, mean_seconds: flow ? flow.mean_elapsed_micros / 1e6 : null });
    }
  }
  const events = new Map((analysis?.graphs?.activities || []).map((item) => [item.activity, item.events]));
  return { lines, spurs, transfers, segments, skips, events, flows, typesAvailable: new Set(variants.map(variant => variant.object_type)).size };
}

// Columns follow the order of steps across every line (and any recorded
// hand-off between them); rows give each line's own stretch a free lane.
function subwayLayout(model) {
  const stations = new Map();
  const add = (activity) => { if (!stations.has(activity)) stations.set(activity, { activity, lines: [], column: 0, lane: null, spur: false }); return stations.get(activity); };
  for (const line of model.lines) for (const activity of line.route) add(activity).lines.push(line.type);
  for (const spur of model.spurs) { const station = add(spur.to); station.lines.push(spur.type); station.spur = true; }

  // Longest-path columns over every ordering the lines and hand-offs give.
  const after = new Map([...stations.keys()].map((key) => [key, new Set()]));
  const reaches = (from, to, seen = new Set()) => {
    if (from === to) return true;
    if (seen.has(from)) return false;
    seen.add(from);
    return [...after.get(from)].some((next) => reaches(next, to, seen));
  };
  const link = (from, to) => { if (from !== to && stations.has(from) && stations.has(to) && !reaches(to, from)) after.get(from).add(to); };
  for (const line of model.lines) for (let index = 1; index < line.route.length; index += 1) link(line.route[index - 1], line.route[index]);
  for (const spur of model.spurs) link(spur.from, spur.to);
  for (const flow of model.flows) link(flow.source, flow.target);
  const column = new Map();
  const depth = (activity, trail = new Set()) => {
    if (column.has(activity)) return column.get(activity);
    trail.add(activity);
    let best = 0;
    for (const [from, targets] of after) if (targets.has(activity) && !trail.has(from)) best = Math.max(best, depth(from, trail) + 1);
    trail.delete(activity);
    column.set(activity, best);
    return best;
  };
  for (const activity of stations.keys()) stations.get(activity).column = depth(activity);

  // Lanes: the biggest line runs along lane 0; each other stretch takes the
  // nearest lane that is free across the columns it spans, with a column of
  // room on either side. A line's spurs are placed right after the line, so
  // later lines keep clear of them.
  const taken = new Map();
  const free = (lane, from, to, self = null) => { for (let c = from; c <= to; c += 1) { const owner = taken.get(`${lane}:${c}`); if (owner && owner !== self) return false; } return true; };
  const take = (lane, from, to, owner) => { for (let c = from; c <= to; c += 1) taken.set(`${lane}:${c}`, owner); };
  const lanes = [0, 1, -1, 2, -2, 3, -3, 4, -4];
  for (const line of model.lines) {
    let run = [];
    const place = () => {
      if (!run.length) return;
      const columns = run.map((station) => station.column);
      const from = Math.min(...columns), to = Math.max(...columns);
      const lane = lanes.find((candidate) => free(candidate, from - 1, to + 1, line.type)) ?? lanes.at(-1);
      for (const station of run) station.lane = lane;
      take(lane, from, to, line.type);
      run = [];
    };
    for (const activity of line.route) {
      const station = stations.get(activity);
      if (station.lane === null) run.push(station);
      else { take(station.lane, station.column, station.column, line.type); place(); }
    }
    place();
    // Detours take the lanes nearest the line, so the way back never crosses
    // a dead-end stop; side paths that end go further out.
    for (const spur of model.spurs.filter((item) => item.type === line.type).sort((a, b) => Number(!!b.rejoins) - Number(!!a.rejoins))) {
      const station = stations.get(spur.to);
      if (station.lane !== null) continue;
      const source = stations.get(spur.from);
      station.column = source.column + 1;
      const lane = [-1, 1, -2, 2, -3, 3].map((offset) => source.lane + offset).find((candidate) => free(candidate, station.column, station.column)) ?? source.lane - 4;
      station.lane = lane;
      station.above = lane < source.lane;
      take(lane, station.column, station.column, `${line.type} spur`);
    }
  }

  const lanesUsed = [...stations.values()].map((station) => station.lane);
  const top = Math.min(...lanesUsed), bottom = Math.max(...lanesUsed);
  const columns = [...stations.values()].map((station) => station.column);
  const left = 70, upper = 90;
  for (const station of stations.values()) {
    station.x = left + station.column * SUBWAY_COLUMN;
    station.y = upper + (station.lane - top) * SUBWAY_LANE;
  }
  // A detour sits halfway between where it leaves the line and where it
  // rejoins, so out and back make one even arch.
  for (const spur of model.spurs) {
    const out = stations.get(spur.from), stop = stations.get(spur.to), back = spur.rejoins && stations.get(spur.rejoins);
    if (back && back.x > out.x && stop.spur) stop.x = (out.x + back.x) / 2;
  }
  // Lines sharing a station run side by side, one track apart. Each line
  // keeps one track along a row, so it never jogs between stations: the
  // biggest line takes the first track, every other line the first track
  // no line it shares a station with on that row already uses.
  const offsets = new Map();
  for (const line of model.lines) {
    const rows = new Set(line.route.map((activity) => stations.get(activity).lane));
    for (const lane of rows) {
      const neighbours = new Set();
      for (const activity of line.route) {
        const station = stations.get(activity);
        if (station.lane !== lane) continue;
        for (const other of station.lines) if (other !== line.type && offsets.has(`${other}:${lane}`)) neighbours.add(offsets.get(`${other}:${lane}`));
      }
      let offset = 0;
      while (neighbours.has(offset)) offset += SUBWAY_TRACK;
      offsets.set(`${line.type}:${lane}`, offset);
    }
  }
  const track = (station, type) => station.spur ? 0 : offsets.get(`${type}:${station.lane}`) || 0;
  for (const station of stations.values()) {
    const tracks = station.lines.map((type) => track(station, type));
    station.top = Math.min(...tracks);
    station.bottom = Math.max(...tracks);
  }
  const point = (activity, type) => { const station = stations.get(activity); return [station.x, station.y + track(station, type)]; };
  const paths = [];
  // Each stretch of a line is its own path, so its thickness can follow
  // how many items take it.
  for (const line of model.lines) {
    for (let index = 1; index < line.route.length; index += 1) {
      const from = line.route[index - 1], to = line.route[index];
      const objects = model.segments.find((segment) => segment.type === line.type && segment.from === from && segment.to === to)?.objects || 0;
      paths.push({ type: line.type, colour: line.colour, spur: false, objects, from, to, first: index === 1, last: index === line.route.length - 1, points: subwayRoutePoints([point(from, line.type), point(to, line.type)]) });
    }
  }
  for (const spur of model.spurs) {
    const line = model.lines.find((item) => item.type === spur.type);
    const stops = [point(spur.from, spur.type), point(spur.to, spur.type)];
    if (spur.rejoins && stations.get(spur.rejoins).x >= stations.get(spur.to).x) stops.push(point(spur.rejoins, spur.type));
    let points = subwayRoutePoints(stops);
    if (stops.length > 2) {
      // The top of a detour's arch runs level for a moment, so the rounded
      // corners never pull the line off its stop.
      const [x, y] = stops[1], at = points.findIndex(([px, py]) => px === x && py === y);
      if (at > 0 && at < points.length - 1) points = [...points.slice(0, at), [x - 12, y], [x, y], [x + 12, y], ...points.slice(at + 1)];
    }
    paths.push({ type: spur.type, colour: line.colour, spur: true, objects: spur.objects, from: spur.from, to: spur.to, rejoins: stops.length > 2, points });
  }
  for (const transfer of model.transfers || []) {
    paths.push({ type: transfer.type, colour: "--edge-ink", spur: true, transfer: true, points: subwayRoutePoints([point(transfer.from, transfer.type), point(transfer.to, model.lines.find((line) => line.route[0] === transfer.to)?.type)], true) });
  }
  return {
    stations, paths, track,
    // Extra room on the right for a last station's name set beside it.
    width: left * 2 + 60 + (Math.max(...columns) - Math.min(...columns)) * SUBWAY_COLUMN,
    height: upper + (bottom - top) * SUBWAY_LANE + 110,
  };
}

// Between two stations on different rows a line runs level, turns at 45°
// across the gap, then runs level again, like a transit map.
function subwayRoutePoints(stops, diagonalFirst = false) {
  const points = [stops[0]];
  for (let index = 1; index < stops.length; index += 1) {
    const [x1, y1] = stops[index - 1], [x2, y2] = stops[index];
    if (diagonalFirst && y1 !== y2 && x2 - x1 > Math.abs(y2 - y1)) {
      points.push([x1 + Math.abs(y2 - y1), y2]);
    } else if (y1 !== y2 && x2 > x1) {
      const rise = Math.abs(y2 - y1);
      const half = Math.min(rise / 2, (x2 - x1) / 2);
      const middle = (x1 + x2) / 2;
      if (x2 - x1 >= rise) points.push([middle - half, y1], [middle + half, y2]);
      // A steep climb runs up between the two columns, never along a column,
      // where it could cross another stop.
      else { const h = (x2 - x1) / 2, sign = Math.sign(y2 - y1); points.push([x1 + h, y1 + sign * h], [x1 + h, y2 - sign * h]); }
    }
    points.push([x2, y2]);
  }
  return points.filter((p, index) => index === 0 || p[0] !== points[index - 1][0] || p[1] !== points[index - 1][1]);
}

function subwayPath(points, radius = 22) {
  let d = `M${points[0][0]},${points[0][1]}`;
  for (let index = 1; index < points.length - 1; index += 1) {
    const [x0, y0] = points[index - 1], [x1, y1] = points[index], [x2, y2] = points[index + 1];
    const l1 = Math.hypot(x1 - x0, y1 - y0), l2 = Math.hypot(x2 - x1, y2 - y1);
    const r = Math.min(radius, l1 / 2, l2 / 2);
    d += ` L${(x1 - (x1 - x0) / l1 * r).toFixed(1)},${(y1 - (y1 - y0) / l1 * r).toFixed(1)} Q${x1},${y1} ${(x1 + (x2 - x1) / l2 * r).toFixed(1)},${(y1 + (y2 - y1) / l2 * r).toFixed(1)}`;
  }
  const last = points[points.length - 1];
  return `${d} L${last[0]},${last[1]}`;
}

// Station names wrap onto short lines so neighbours never collide.
function subwayLabelLines(text, width = 11) {
  const lines = [];
  let current = "";
  for (const word of String(text).split(/\s+/)) {
    if (current && `${current} ${word}`.length > width) { lines.push(current); current = word; }
    else current = current ? `${current} ${word}` : word;
  }
  if (current) lines.push(current);
  return lines.slice(0, 3);
}

// A stretch is slow when it takes at least ten times the typical stretch on
// the lines, and at least an hour.
function subwaySlowThreshold(model) {
  const times = model.segments.filter((segment) => segment.mean_seconds !== null && segment.objects > 0).map((segment) => segment.mean_seconds).sort((a, b) => a - b);
  if (!times.length) return Infinity;
  return Math.max(3600, times[Math.floor(times.length / 2)] * 10);
}

// The label on each stretch: how many items took it ("How many"), or the
// average time it took ("How long", the slowest stretches in red). Labels sit
// just above a level stretch; where two lines run side by side, the lower
// line's label goes below it. Dotted hand-offs say what moved on.
function subwayLabels(model, layout, measure) {
  const threshold = subwaySlowThreshold(model);
  const items = [
    ...model.segments.map((item) => ({ ...item, kind: "segment" })),
    ...model.spurs.map((item) => ({ ...item, kind: "spur" })),
    ...(model.transfers || []).map((item) => ({ ...item, kind: "transfer" })),
  ];
  const labels = [];
  for (const item of items) {
    const a = layout.stations.get(item.from), b = layout.stations.get(item.to);
    if (!a || !b) continue;
    let text;
    if (measure === "time") {
      if (item.mean_seconds === null || item.mean_seconds === undefined || !item.objects) continue;
      text = formatDuration(item.mean_seconds);
    } else {
      if (!item.objects) continue;
      text = item.objects.toLocaleString();
    }
    if (item.kind === "transfer") text = measure === "time" ? `${text} later` : `${text} ${itemTypeName(item.type).toLowerCase()}`;
    const ya = a.y + layout.track(a, item.type), yb = b.y + layout.track(b, item.type);
    // Lines running side by side share one label above the top track, so
    // nothing lands on the station names below; different figures read "a / b".
    if (item.kind === "segment" && ya === yb) {
      const key = `${item.from}\u0001${item.to}`;
      const slow = measure === "time" && item.mean_seconds >= threshold;
      const top = Math.min(...a.lines.filter((type) => b.lines.includes(type)).map((type) => layout.track(a, type)));
      const own = layout.track(a, item.type);
      // Each number sits on the line it counts: the top line's above it, a
      // line running underneath just below its own track. Where both lines
      // share the stretch and the figure is the same, one number serves both.
      const same = labels.find((label) => label.key === key && label.text === text);
      if (same) { same.slow ||= slow; continue; }
      // side and base let a top-to-bottom map set the number beside its own
      // track, clear of the neighbouring line's number.
      const side = own > top ? 1 : -1, base = a.y + (side > 0 ? own : top);
      labels.push({ key, type: item.type, x: (a.x + b.x) / 2, y: base + side * 16, side, base, text, slow });
      continue;
    }
    // Branches and hand-offs carry their label near the far end, clear of
    // the names around the station they leave.
    // A number never sits on a stop: slide it along its line to a clear spot.
    const preferred = item.kind === "spur" ? 0.72 : 0.5;
    const at = (along) => [a.x + (b.x - a.x) * along, ya + (yb - ya) * along];
    const width = text.length * 6.9 + 14;
    const onStop = ([x, y]) => [...layout.stations.values()].some((station) => Math.abs(station.x - x) < width / 2 + 10 && y > station.y + station.top - 19 && y < station.y + station.bottom + 19);
    const along = [preferred, 0.35, 0.65, 0.25, 0.8].find((value) => !onStop(at(value))) ?? preferred;
    const [x, y] = at(along);
    labels.push({ type: item.type, x, y, text, slow: measure === "time" && item.mean_seconds >= threshold });
  }
  return labels;
}

// Where each station's name goes on a left-to-right map: under the station
// if that spot is clear of lines, numbers and names already placed, else to
// its left, its right, or above it. Returns {anchor, x, y} per station,
// where y is the first line's baseline.
const SUBWAY_CHAR = 7.6, SUBWAY_LINE_HEIGHT = 15;
// A number's place on a top-to-bottom map: beside its own track, on the side
// its stretch calls for, unless another line runs there; then the other side.
function subwayVerticalTag(raw, layout, transpose) {
  if (!raw.side) return { ...raw, x: raw.y + 30, y: raw.x * .78 };
  const half = (raw.text.length * 6.9 + 14) / 2, middle = raw.x * .78;
  const at = (side) => raw.base + 30 + side * (half + 8);
  const crossed = (x, y) => layout.paths.some((path) => path.type !== raw.type && path.points.map(transpose).some(([px, py], index, all) => {
    if (index === 0) return false;
    const [qx, qy] = all[index - 1];
    if (Math.max(py, qy) < y - 22 || Math.min(py, qy) > y + 22) return false;
    const t = qy === py ? 0.5 : (y - qy) / (py - qy), lx = qx + (px - qx) * Math.max(0, Math.min(1, t));
    return lx > x - half - 14 && lx < x + half + 14;
  }));
  // Its own side first, then the other; then a little up or down the track.
  for (const shift of [0, -26, 26, -44, 44]) {
    for (const side of [raw.side, -raw.side]) {
      if (!crossed(at(side), middle + shift)) return { ...raw, x: at(side), y: middle + shift };
    }
  }
  return { ...raw, x: at(raw.side), y: middle };
}

// Where each station's name goes on a top-to-bottom map: beside the stop on
// whichever side, and at whichever height, is clear of numbers, stops, lines
// and names already placed; else the spot that collides least.
function subwayVerticalNames(layout, transpose, tags, linesOf, measureWidth) {
  const boxes = tags.map((tag) => { const width = tag.text.length * 6.9 + 14; return { x1: tag.x - width / 2, x2: tag.x + width / 2, y1: tag.y - 9, y2: tag.y + 9 }; });
  const points = [];
  for (const path of layout.paths) {
    const drawn = path.points.map(transpose);
    for (let index = 1; index < drawn.length; index += 1) {
      const [x1, y1] = drawn[index - 1], [x2, y2] = drawn[index];
      const steps = Math.max(1, Math.ceil(Math.hypot(x2 - x1, y2 - y1) / 5));
      for (let step = 0; step <= steps; step += 1) points.push([x1 + (x2 - x1) * step / steps, y1 + (y2 - y1) * step / steps]);
    }
  }
  const stations = [...layout.stations.values()].map((station) => { const [x, y] = transpose([station.x, station.y]); return { station, x, y }; }).sort((a, b) => a.y - b.y || a.x - b.x);
  for (const { station, x, y } of stations) boxes.push({ x1: x + station.top - 10, x2: x + station.bottom + 10, y1: y - 10, y2: y + 10 });
  // Covering another name weighs most, a number or stop less, and a line
  // running behind the letters least: the name's halo keeps it readable.
  const clash = (box) => boxes.reduce((sum, other) => sum + (other.w || 4) * Math.max(0, Math.min(box.x2, other.x2) - Math.max(box.x1, other.x1)) * Math.max(0, Math.min(box.y2, other.y2) - Math.max(box.y1, other.y1)), 0)
    + 10 * points.filter(([x, y]) => x > box.x1 && x < box.x2 && y > box.y1 && y < box.y2).length;
  const placements = new Map();
  for (const { station, x, y } of stations) {
    const lines = linesOf(station);
    const width = measureWidth(lines), height = lines.length * 14 + 4;
    const right = x + station.bottom + 16, left = x + station.top - 16;
    // A side step drawn out to the left of its line names itself on the left,
    // outside its arch, rather than inside it.
    const leftFirst = station.spur && station.above;
    const options = [
      ...(leftFirst ? [{ anchor: "end", x: left, top: y - height / 2 }] : []),
      { anchor: "start", x: right, top: y - height / 2 },
      { anchor: "end", x: left, top: y - height / 2 },
      { anchor: "start", x: right, top: y + 10 },
      { anchor: "end", x: left, top: y + 10 },
      { anchor: "start", x: right, top: y - height - 10 },
      { anchor: "end", x: left, top: y - height - 10 },
    ];
    const box = (option) => { const x1 = option.anchor === "end" ? option.x - width : option.x; return { x1: x1 - 2, x2: x1 + width + 2, y1: option.top, y2: option.top + height }; };
    let chosen = options[0], least = Infinity;
    for (const option of options) { const score = clash(box(option)); if (score === 0) { chosen = option; break; } if (score < least) { least = score; chosen = option; } }
    // A placed name keeps a little room around it, so the next name never
    // touches it.
    const placed = box(chosen);
    boxes.push({ x1: placed.x1 - 6, x2: placed.x2 + 6, y1: placed.y1 - 4, y2: placed.y2 + 4, w: 10 });
    placements.set(station.activity, chosen);
  }
  return placements;
}

function subwayNamePlacements(layout, tags, linesOf, measureWidth = (lines) => Math.max(...lines.map((line) => line.length)) * SUBWAY_CHAR) {
  const boxes = tags.map((tag) => { const width = tag.text.length * 6.9 + 14; return { x1: tag.x - width / 2, x2: tag.x + width / 2, y1: tag.y - 9, y2: tag.y + 9 }; });
  // Every drawn rail, sampled every few pixels.
  const points = [];
  for (const path of layout.paths) {
    for (let index = 1; index < path.points.length; index += 1) {
      const [x1, y1] = path.points[index - 1], [x2, y2] = path.points[index];
      const steps = Math.max(1, Math.ceil(Math.hypot(x2 - x1, y2 - y1) / 5));
      for (let step = 0; step <= steps; step += 1) points.push([x1 + (x2 - x1) * step / steps, y1 + (y2 - y1) * step / steps]);
    }
  }
  const stations = [...layout.stations.values()].sort((a, b) => a.x - b.x || a.y - b.y);
  for (const station of stations) boxes.push({ x1: station.x - 10, x2: station.x + 10, y1: station.y + station.top - 10, y2: station.y + station.bottom + 10 });
  // How badly a spot collides: overlapped area plus rail points it covers.
  // Covering another name weighs most, a number or stop less, and a line
  // running behind the letters least: the name's halo keeps it readable.
  const clash = (box) => boxes.reduce((sum, other) => sum + (other.w || 4) * Math.max(0, Math.min(box.x2, other.x2) - Math.max(box.x1, other.x1)) * Math.max(0, Math.min(box.y2, other.y2) - Math.max(box.y1, other.y1)), 0)
    + 10 * points.filter(([x, y]) => x > box.x1 && x < box.x2 && y > box.y1 && y < box.y2).length;
  const placements = new Map();
  for (const station of stations) {
    const lines = linesOf(station);
    const width = measureWidth(lines);
    const height = lines.length * SUBWAY_LINE_HEIGHT;
    const below = station.y + station.bottom + 30, above = station.y + station.top - 22 - (lines.length - 1) * SUBWAY_LINE_HEIGHT;
    const options = [
      ...(station.above ? [{ anchor: "middle", x: station.x, y: above }] : []),
      { anchor: "middle", x: station.x, y: below },
      { anchor: "end", x: station.x - 14, y: station.y + station.bottom + 22 },
      { anchor: "start", x: station.x + 14, y: station.y + station.bottom + 22 },
      { anchor: "middle", x: station.x, y: above },
      { anchor: "middle", x: station.x, y: below + 26 },
      { anchor: "end", x: station.x - 14, y: station.y + station.bottom + 48 },
      { anchor: "start", x: station.x + 14, y: station.y + station.bottom + 48 },
      { anchor: "middle", x: station.x, y: above - 26 },
      { anchor: "middle", x: station.x, y: below + 52 },
      { anchor: "end", x: station.x - 14, y: station.y + station.bottom + 74 },
      { anchor: "start", x: station.x + 14, y: station.y + station.bottom + 74 },
      { anchor: "middle", x: station.x, y: above - 52 },
    ];
    const box = (option) => {
      const x1 = option.anchor === "middle" ? option.x - width / 2 : option.anchor === "end" ? option.x - width : option.x;
      return { x1: x1 - 2, x2: x1 + width + 2, y1: option.y - 12, y2: option.y - 12 + height };
    };
    // The first clear spot, or else the one that collides least.
    let chosen = null, least = Infinity;
    for (const option of options) { const score = clash(box(option)); if (score === 0) { chosen = option; break; } if (score < least) { least = score; chosen = option; } }
    boxes.push({ ...box(chosen), w: 10 });
    placements.set(station.activity, chosen);
  }
  return placements;
}

function drawSubwayMap(svg, analysis, measure = "count") {
  const model = subwayModel(analysis);
  svg.replaceChildren();
  svg.classList?.add("subway-map");
  if (!model.lines.length) {
    svg.setAttribute("viewBox", "0 0 700 160");
    const message = svgElement("text", { x: 350, y: 80, "text-anchor": "middle", class: "subway-empty" });
    message.textContent = "No item type has a common enough route to draw as a line";
    svg.append(message);
    return model;
  }
  const layout = subwayLayout(model);
  const vertical=subwayOrientation==="vertical";
  const transpose=([x,y])=>vertical?[y+30,x*.78]:[x,y];
  const canvasWidth=vertical?layout.height+230:layout.width;
  const canvasHeight=vertical?layout.width*.78+80:layout.height;
  svg.setAttribute("viewBox", `0 0 ${canvasWidth} ${canvasHeight}`);
  svg.dataset.width = `${canvasWidth}px`;
  svg.classList?.toggle("vertical",vertical);
  // Thickness follows volume: the busiest stretch on the map is the widest.
  const maxObjects = Math.max(1, ...layout.paths.filter((path) => !path.transfer).map((path) => path.objects || 0));
  const lineLayer = svgElement("g"), markLayer = svgElement("g"), waitLayer = svgElement("g"), stationLayer = svgElement("g"), iconLayer = svgElement("g");
  svg.append(lineLayer, markLayer, waitLayer, stationLayer, iconLayer);
  // Lines draw in the first time this map appears, not on every redraw.
  const signature = layout.paths.map((path) => `${path.type}:${path.points.length}`).join("|") + `|${subwayOrientation}`;
  const firstDraw = svg.dataset.drawn !== signature;
  svg.dataset.drawn = signature;
  // Casings first, then rails, so a stretch's white edge never cuts
  // across the line it joins.
  const casingLayer = svgElement("g"), railLayer = svgElement("g");
  lineLayer.append(casingLayer, railLayer);
  for (const path of [...layout.paths].reverse()) {
    const d = subwayPath(path.points.map(transpose));
    const width = subwayStrokeWidth(path.objects || 0, maxObjects);
    if (!path.transfer) casingLayer.append(svgElement("path", { d, class: "casing", "data-lines": path.type, "stroke-width": (width + 5).toFixed(1) }));
    const group = svgElement("g", { class: "subway-line", "data-lines": path.type });
    const stroke = svgElement("path", { d, class: `rail${path.transfer ? " transfer" : ""}`, stroke: `var(${path.colour})`, "stroke-width": path.transfer ? "2" : width.toFixed(1) });
    group.append(stroke);
    // A wide, invisible stroke makes the stretch easy to click.
    if (!path.transfer && path.from && path.to) {
      group.append(svgElement("path", { d, class: "subway-hit-line", stroke: "transparent", "stroke-width": "16", fill: "none" }));
      group.classList.add("pickable");
      group.addEventListener("click", (event) => { event.stopPropagation(); pickSubwayConnection(path.type, path.from, path.to); });
    }
    railLayer.append(group);
    if (path.transfer) continue;
    const length = stroke.getTotalLength?.();
    if (length && firstDraw) { stroke.style.strokeDasharray = `${length}`; stroke.style.strokeDashoffset = `${length}`; stroke.classList.add("draw"); }
    // Where a line begins (▶) and where items stop: its last step, or a
    // side path that does not rejoin (■).
    const points = path.points.map(transpose);
    // Where another line also passes, or on a side path's own stop, the mark
    // goes inside the stop instead, so it never sits on another line or a name.
    // A line ending on a diagonal also keeps its mark inside the stop, where
    // a mark set beyond it would land on the station's name.
    const inside = (activity, at, toward) => path.spur || (layout.stations.get(activity)?.lines.length || 0) > 1 || (Math.abs(at[0] - toward[0]) > 1 && Math.abs(at[1] - toward[1]) > 1);
    const mark = (kind, at, toward, activity) => { const within = inside(activity, at, toward); (within ? iconLayer : markLayer).append(subwayMark(kind, at, toward, path, within)); };
    if (path.first) mark("start", points[0], points[1], path.from);
    if (path.last || (path.spur && !path.rejoins)) mark("end", points.at(-1), points.at(-2), path.to);
  }
  // At All, skips and loop-backs between a line's own stops: faint dotted
  // arcs over the line (forward) or under it (back), counts on hover.
  const skipLayer = svgElement("g", { class: "subway-skips" });
  for (const skip of model.skips || []) {
    const a = layout.stations.get(skip.from), b = layout.stations.get(skip.to);
    if (!a || !b) continue;
    const ya = a.y + layout.track(a, skip.type), yb = b.y + layout.track(b, skip.type);
    const lift = (skip.back ? 1 : -1) * (26 + Math.abs(b.x - a.x) * 0.18);
    const [p1, c, p2] = [[a.x, ya], [(a.x + b.x) / 2, Math.min(ya, yb) + lift], [b.x, yb]].map(transpose);
    const line = model.lines.find((item) => item.type === skip.type);
    const d = `M${p1[0]},${p1[1]} Q${c[0]},${c[1]} ${p2[0]},${p2[1]}`;
    const arc = svgElement("path", { d, class: "subway-skip", "data-lines": skip.type, stroke: `var(${line?.colour || "--faint"})` });
    const title = svgElement("title");
    title.textContent = `${skip.from} → ${skip.to}${skip.back ? " (back)" : " (skips steps)"}: ${skip.objects.toLocaleString()} ${itemTypeName(skip.type).toLowerCase()}`;
    const group = svgElement("g", { class: "subway-skip-group pickable", "data-lines": skip.type });
    group.append(arc, svgElement("path", { d, class: "subway-hit-line", stroke: "transparent", "stroke-width": "12", fill: "none" }), title);
    group.addEventListener("click", (event) => { event.stopPropagation(); pickSubwayConnection(skip.type, skip.from, skip.to); });
    skipLayer.append(group);
  }
  lineLayer.prepend(skipLayer);
  const tagList = subwayLabels(model, layout, measure);
  // Names are measured as drawn, so placement knows their real width.
  const rulerGroup = svgElement("g", { class: "subway-station" }), ruler = svgElement("text", { class: "name", x: -9999, y: -9999 }); rulerGroup.append(ruler); stationLayer.append(rulerGroup);
  const measureWidth = (lines) => Math.max(0, ...lines.map((line) => { ruler.textContent = line; return ruler.getComputedTextLength?.() || line.length * SUBWAY_CHAR; }));
  const namePlacements = vertical ? new Map() : subwayNamePlacements(layout, tagList, (station) => subwayHiddenEvents.has(station.activity) ? [""] : subwayLabelLines(station.activity), measureWidth);
  const verticalNames = vertical ? subwayVerticalNames(layout, transpose, tagList.map((raw) => subwayVerticalTag(raw, layout, transpose)), (station) => subwayHiddenEvents.has(station.activity) ? [""] : subwayLabelLines(station.activity, 23), measureWidth) : new Map();
  rulerGroup.remove();
  for (const raw of tagList) {
    const tag=vertical?subwayVerticalTag(raw,layout,transpose):raw;
    const group = svgElement("g", { class: `subway-tag${tag.slow ? " slow" : ""}`, "data-lines": tag.type });
    const width = tag.text.length * 6.9 + 14;
    group.append(svgElement("rect", { x: tag.x - width / 2, y: tag.y - 9, width, height: 18, rx: 9 }));
    const label = svgElement("text", { x: tag.x, y: tag.y + 4, "text-anchor": "middle" });
    label.textContent = tag.text;
    group.append(label);
    waitLayer.append(group);
  }
  for (const station of layout.stations.values()) {
    const through = station.spur ? 1 : station.lines.filter((type) => model.lines.some((line) => line.type === type)).length;
    const group = svgElement("g", { class: `subway-station${station.activity === subwaySelection ? " sel" : ""}`, tabindex: 0, role: "button", "data-lines": station.lines.join("\u0001"), "data-activity": station.activity, "aria-label": `${station.activity}, ${station.lines.join(" and ")}` });
    const [x,y]=transpose([station.x,station.y]);
    if(vertical){
      if(through>1)group.append(svgElement("rect",{class:"stop",x:x+station.top-8,y:y-8,width:16+station.bottom-station.top,height:16,rx:8}));
      else group.append(svgElement("circle",{class:"stop",cx:x+station.top,cy:y,r:7}));
      if(!subwayHiddenEvents.has(station.activity)){
        // Names go left or right of the line, wherever is clear of numbers,
        // stops, lines and other names. The count is on the line itself.
        const labels=subwayLabelLines(station.activity,23),spot=verticalNames.get(station.activity);
        labels.forEach((text,index)=>{const name=svgElement("text",{x:spot.x,y:spot.top+11+index*14,class:"name","text-anchor":spot.anchor});name.textContent=text;group.append(name);});
      }
    }else{
      if (through > 1) group.append(svgElement("rect", { class: "stop", x: station.x - 9, y: station.y + station.top - 9, width: 18, height: 18 + station.bottom - station.top, rx: 9 }));
      else group.append(svgElement("circle", { class: "stop", cx: station.x, cy: station.y + station.top, r: 7 }));
      if(!subwayHiddenEvents.has(station.activity)){
        const labels=subwayLabelLines(station.activity),spot=namePlacements.get(station.activity);
        labels.forEach((text,index)=>{const name=svgElement("text",{x:spot.x,y:spot.y+index*SUBWAY_LINE_HEIGHT,class:"name","text-anchor":spot.anchor});name.textContent=text;group.append(name);});
      }
    }
    if(subwayHiddenEvents.has(station.activity))group.classList.add("label-hidden");
    const pick = () => { if(pickMapMeasurement(station.activity,station.lines))return; subwaySelection = station.activity; subwayConnection = null; showMapStepDetail(); renderSubway(); };
    group.addEventListener("click", pick);
    group.addEventListener("keydown", (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); pick(); } });
    group.addEventListener("mouseenter", () => focusSubwayLine(svg, station.lines.length === 1 ? station.lines[0] : null));
    group.addEventListener("mouseleave", () => focusSubwayLine(svg, subwayFocus));
    stationLayer.append(group);
  }
  focusSubwayLine(svg, subwayFocus);
  return model;
}

// Line thickness for a stretch: 3px for the quietest, up to 11px for the
// busiest, on a square-root scale so small paths stay visible.
function subwayStrokeWidth(objects, maxObjects) {
  return 3 + 8 * Math.sqrt(Math.max(0, Math.min(1, objects / Math.max(1, maxObjects))));
}

// A line's start and end are small terminal stops set 26px beyond its first
// and last stations: a ring holding ▶ where it begins, ■ where items stop,
// joined to the station by a short dotted stub.
function subwayMarkPoint(at, toward, gap = 26) {
  const dx = toward[0] - at[0], dy = toward[1] - at[1];
  const length = Math.hypot(dx, dy) || 1;
  return [at[0] - dx / length * gap, at[1] - dy / length * gap];
}

function subwayMark(kind, at, toward, path, inside = false) {
  const group = svgElement("g", { class: `subway-mark ${kind}${inside ? " inside" : ""}`, "data-lines": path.type, "aria-hidden": "true" });
  const [x, y] = inside ? at : subwayMarkPoint(at, toward);
  const colour = `var(${path.colour})`;
  if (!inside) {
    const [sx, sy] = subwayMarkPoint(at, toward, 9);
    const [ex, ey] = subwayMarkPoint(at, toward, 19);
    group.append(svgElement("line", { x1: sx.toFixed(1), y1: sy.toFixed(1), x2: ex.toFixed(1), y2: ey.toFixed(1), class: "stub", stroke: colour }));
    group.append(svgElement("circle", { cx: x.toFixed(1), cy: y.toFixed(1), r: 7.5, class: "ring", stroke: colour }));
  }
  if (kind === "end") {
    group.append(svgElement("rect", { x: (x - 3).toFixed(1), y: (y - 3).toFixed(1), width: 6, height: 6, rx: 1, fill: colour }));
  } else {
    const angle = Math.atan2(toward[1] - at[1], toward[0] - at[0]) * 180 / Math.PI;
    group.append(svgElement("path", { d: "M-2.4,-3.6 L3.6,0 L-2.4,3.6 Z", transform: `translate(${x.toFixed(1)},${y.toFixed(1)}) rotate(${angle.toFixed(1)})`, fill: colour }));
  }
  return group;
}

// Main routes | More side paths | Most: redraws the subway at that level.
function setSubwayDetail(level) {
  subwayDetailLevel = Math.max(0, Math.min(SUBWAY_DETAIL.length - 1, Number(level) || 0));
  if (SUBWAY_DETAIL[subwayDetailLevel].all && typeof showMapNote === "function") {
    showMapNote("Everything recorded: dotted arcs are skips and loop-backs. Click any line for its numbers. For one record type in full, use ↗ on its line.");
  }
  for (const button of document.querySelectorAll?.("[data-subway-detail]") || []) button.setAttribute("aria-pressed", String(Number(button.dataset.subwayDetail) === subwayDetailLevel));
  renderSubway();
  if (typeof fitExplorer === "function") fitExplorer("process");
}

// A level that would add no side path in this data is greyed out and says so,
// rather than appearing to do nothing.
function subwayDetailCounts(analysis) {
  const chosen = subwayDetailLevel;
  try { return SUBWAY_DETAIL.map((_, level) => { subwayDetailLevel = level; const model = subwayModel(analysis); return model.spurs.length + model.skips.length; }); }
  finally { subwayDetailLevel = chosen; }
}

function syncSubwayDetail(analysis) {
  const counts = subwayDetailCounts(analysis);
  for (const button of document.querySelectorAll?.("[data-subway-detail]") || []) {
    const level = Number(button.dataset.subwayDetail);
    const adds = level === 0 || counts[level] > counts[level - 1];
    button.disabled = !adds && level !== subwayDetailLevel;
    button.title = adds ? "" : "No further side paths in this data";
    button.setAttribute("aria-pressed", String(level === subwayDetailLevel));
  }
}

function focusSubwayLine(svg, type) {
  for (const element of svg.querySelectorAll("[data-lines]")) {
    element.classList.toggle("dim", !!type && !element.dataset.lines.split("\u0001").includes(type));
  }
}

// One line's full detail: follow that record type, switch to the classic map
// and show every step and connection.
async function openLineInClassic(type) {
  if (typeof currentFollow !== "undefined" && currentFollow?.type !== type && typeof followType === "function") {
    if (!await followType(type, currentFilter || {}, true)) return;
  }
  subwayFocus = null;
  mapStyle = "classic";
  if (typeof renderMapStyle === "function") renderMapStyle();
  await loadMapView({ mode: "limits", activities: currentAnalysis?.summary?.activities || 100, connections: currentAnalysis?.edges?.length || 200 });
  if (typeof showMapNote === "function") showMapNote(`Every path of ${itemTypeName(type).toLowerCase()}: Main flow on the map detail card simplifies it again.`);
}

// Narrowing the map to one line follows that item type on every page, so
// Variants and Items agree with the map; widening it again goes back to what
// was followed before. Filters are kept either way.
let subwayFollowReturn;
async function syncFollowWithMap() {
  if (typeof followType !== "function" || typeof followTypes === "undefined" || !followTypes.length) return;
  const only = subwayFocus || (subwayShownTypes?.size === 1 ? [...subwayShownTypes][0] : null);
  let target = null;
  if (only && followTypes.includes(only)) {
    if (currentFollow?.type === only) return;
    if (subwayFollowReturn === undefined) subwayFollowReturn = currentFollow?.type ?? null;
    target = only;
  } else if (subwayFollowReturn !== undefined) {
    target = subwayFollowReturn;
    subwayFollowReturn = undefined;
    if (!target || currentFollow?.type === target) return;
  } else return;
  // Following rebuilds the pages, which forgets the map's line choice.
  const focus = subwayFocus, shown = subwayShownTypes;
  if (!await followType(target, currentFilter || {}, true)) return;
  subwayFocus = focus; subwayShownTypes = shown;
  renderSubway();
}

function subwayLegend(model) {
  const legend = document.querySelector("#subway-legend");
  if (!legend) return;
  legend.replaceChildren(...model.lines.map((line) => {
    const chip = document.createElement("span");
    chip.className = "subway-chip";
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute("aria-pressed", String(subwayFocus === line.type));
    const swatch = document.createElement("i"); swatch.style.background = `var(${line.colour})`;
    const count = document.createElement("span"); count.className = "num"; count.textContent = line.objects.toLocaleString();
    button.append(swatch, itemTypeName(line.type), count);
    button.addEventListener("click", () => { subwayFocus = subwayFocus === line.type ? null : line.type; renderSubway(); void syncFollowWithMap(); });
    button.addEventListener("mouseenter", () => focusSubwayLine(document.querySelector("#subway-map"), line.type));
    button.addEventListener("mouseleave", () => focusSubwayLine(document.querySelector("#subway-map"), subwayFocus));
    // Every path this record type took, on the classic map.
    const full = document.createElement("button");
    full.type = "button";
    full.className = "subway-open-classic";
    full.textContent = "↗";
    full.title = `Every path of ${itemTypeName(line.type).toLowerCase()} on the classic map`;
    full.setAttribute("aria-label", full.title);
    full.addEventListener("click", () => void openLineInClassic(line.type));
    chip.append(button, full);
    return chip;
  }));
  if (model.transfers?.length) {
    const key = document.createElement("span");
    key.className = "subway-key";
    key.append(document.createElement("i"), "Moved on to another line");
    legend.append(key);
  }
  legend.classList.toggle("focused", !!subwayFocus);
}

// The line under the subway map, in the process's own words:
// "Orders with invoices, shipments and payments · 20 steps, 5 where they meet".
function subwaySummaryText(names, steps, meeting) {
  const [first, ...others] = names;
  const lower = others.map((name) => name.toLowerCase());
  const joined = lower.length > 1 ? `${lower.slice(0, -1).join(", ")} and ${lower.at(-1)}` : lower[0];
  const lead = others.length ? `${first} with ${joined}` : first;
  const where = meeting ? `, ${meeting} where they meet` : "";
  return [lead, ` · ${steps} ${steps === 1 ? "step" : "steps"}${others.length ? where : ""}`];
}

// A stretch of line, side path or dotted arc, clicked: how many items took
// it and how long it took, in the step panel.
function pickSubwayConnection(type, from, to) {
  subwayConnection = { type, from, to };
  subwaySelection = null;
  showMapStepDetail();
  renderSubway();
}

function subwayConnectionDetail(model, pick) {
  const flow = model.flows.find((item) => item.object_type === pick.type && item.source === pick.from && item.target === pick.to);
  const line = model.lines.find((item) => item.type === pick.type);
  if (!flow) return false;
  const route = line?.route || [];
  const a = route.indexOf(pick.from), b = route.indexOf(pick.to);
  const kind = a >= 0 && b >= 0 ? (b === a + 1 ? "Next step on the line" : b > a ? "Skips steps on the line" : "Goes back on the line") : "Side path";
  const plural = itemTypeName(pick.type).toLowerCase();
  const facts = [
    [itemTypeName(pick.type), flow.objects.toLocaleString()],
    ["Average time", formatDuration(flow.mean_elapsed_micros / 1e6)],
    ["Times recorded", flow.occurrences.toLocaleString()],
    ["Share of the line", line?.objects ? `${Math.round(flow.objects / line.objects * 1000) / 10}% of ${plural}` : "—"],
    ["Fastest · slowest", `${formatDuration(flow.min_elapsed_micros / 1e6)} · ${formatDuration(flow.max_elapsed_micros / 1e6)}`],
  ];
  resultsFacts(`${pick.from} → ${pick.to}`, facts, [], [], kind);
  return true;
}

function subwayDetail(model, activity) {
  const lines = model.lines.filter((line) => line.route.includes(activity)).map((line) => itemTypeName(line.type));
  const spur = model.spurs.find((item) => item.to === activity);
  const next = model.flows
    .filter((flow) => flow.source === activity && model.lines.some((line) => line.type === flow.object_type))
    .sort((a, b) => b.objects - a.objects)
    .slice(0, 4)
    .map((flow) => ({ name: `${flow.target} (${itemTypeName(flow.object_type).toLowerCase()})`, count: flow.objects }));
  const facts = [
    ["Times recorded", (model.events.get(activity) || 0).toLocaleString()],
    [lines.length > 1 ? "Used by" : "Part of", lines.join(", ") || (spur ? itemTypeName(spur.type) : "—")],
  ];
  if (lines.length > 1) facts.push(["Shared step", "Several item types record a step with this name. That alone doesn’t show they were handed over to each other; Overview shows the measured links."]);
  if (spur) facts.push(["Branches from", spur.from]);
  resultsFacts(activity, facts, [], next, lines.length > 1 ? "Shared step" : "Step");
  if(typeof appendStudioActions === "function")appendStudioActions(activity);
}

function renderSubway() {
  const svg = document.querySelector("#subway-map");
  if (!svg || !currentOcelAnalysis) return;
  const wrap = svg.parentElement;
  // The canvas may still be hidden on the first draw; redraw once it has a
  // size, and when a resize makes the other direction fit better.
  if (wrap && !wrap.dataset.watched && typeof ResizeObserver === "function") {
    wrap.dataset.watched = "true";
    new ResizeObserver(() => {
      if (subwayOrientationChosen || activeMapStyle() !== "subway" || !wrap.clientWidth) return;
      if (subwayAutoOrientation(wrap.clientWidth, wrap.clientHeight) !== subwayOrientation) renderSubway();
    }).observe(wrap);
  }
  if (!subwayOrientationChosen && wrap?.clientWidth && wrap?.clientHeight) {
    subwayOrientation = subwayAutoOrientation(wrap.clientWidth, wrap.clientHeight);
  }
  for (const button of document.querySelectorAll?.("[data-subway-direction]") || []) button.setAttribute("aria-pressed", String(button.dataset.subwayDirection === subwayOrientation));
  const model = drawSubwayMap(svg, currentOcelAnalysis, currentOverlay === "performance" ? "time" : "count");
  subwayLegend(model);
  syncSubwayDetail(currentOcelAnalysis);
  if(typeof renderMapViewControls==="function")renderMapViewControls(model);
  const coverage = document.querySelector("#results-coverage");
  if (coverage && model.lines.length) {
    const stops = new Set([...model.lines.flatMap((line) => line.route), ...model.spurs.map((spur) => spur.to)]);
    const shared = [...stops].filter((activity) => model.lines.filter((line) => line.route.includes(activity)).length > 1).length;
    const [lead, rest] = subwaySummaryText(model.lines.map((line) => itemTypeName(line.type)), stops.size, shared);
    const bold = document.createElement("b"); bold.textContent = lead;
    coverage.replaceChildren(bold, rest);
    const omitted = Math.max(0, model.typesAvailable - model.lines.length);
    coverage.dataset.detail = `${omitted ? `${omitted} other record ${omitted === 1 ? "type is" : "types are"} left off. ` : ""}Each line follows its most common route. Where lines meet, several records go through a step with the same name; Overview shows how they are actually linked.`;
  }
  if (coverage && !model.lines.length) coverage.textContent = "No common multi-step route meets the subway display threshold in this selection. Use Classic to inspect the followed items.";
  if (coverage && currentOcelAnalysis.filter?.cohort) { const cohort=currentOcelAnalysis.filter.cohort; coverage.append(` · linked to ${cohort.roots.length.toLocaleString()} ${itemTypeName(cohort.root_type).toLowerCase()}`); }
  if (subwaySelection && !model.lines.some((line) => line.route.includes(subwaySelection)) && !model.spurs.some((spur) => spur.to === subwaySelection)) subwaySelection = null;
  if (subwaySelection) subwayDetail(model, subwaySelection);
  else if (subwayConnection && !subwayConnectionDetail(model, subwayConnection)) subwayConnection = null;
  if (typeof renderMapScope === "function") renderMapScope();
  // Size comes from the canvas fit (fitExplorer), not the drawing's own width.
  svg.style.minWidth = "";
  if (typeof updateProcessExplorer === "function") updateProcessExplorer(currentMapView);
}

// Connected projects choose between the classic map of one followed type and
// the subway map of every type together.
function activeMapStyle() {
  if (typeof resultsRouteFocus !== "undefined" && resultsRouteFocus) return "classic";
  return typeof currentOcelAnalysis !== "undefined" && currentOcelAnalysis ? mapStyle : "classic";
}

function renderMapStyle() {
  const connected = !!currentOcelAnalysis;
  const style = activeMapStyle();
  const switcher = document.querySelector("#map-style");
  if (switcher) switcher.hidden = !connected;
  for (const button of document.querySelectorAll?.("[data-map-style]") || []) button.setAttribute("aria-pressed", String(button.dataset.mapStyle === style));
  const subway = style === "subway";
  const exportButton = document.querySelector("#export-bpmn");
  if (exportButton) { exportButton.disabled = subway || currentAnalysis?.summary?.cases === 0; exportButton.dataset.tip = subway ? "Switch to Classic to export as BPMN" : "Export this map as BPMN"; exportButton.removeAttribute("title"); }
  for (const [selector, hide] of [["#subway-map", !subway], ["#subway-legend", !subway], ["#subway-detail", !subway], ["#results-map", subway], ["#results-legend", subway], ["#results-stepper", subway], ["[data-results-detail]", subway]]) {
    // SVG elements have no `hidden` property, so set the attribute itself.
    for (const element of document.querySelectorAll?.(selector) || []) element.toggleAttribute("hidden", hide);
  }
  document.querySelector(".mapgrid")?.classList.toggle("subway-on", subway);
  const controls = document.querySelector("#process-controls"); if (controls) controls.hidden = subway;
  const detailSwitch = document.querySelector("[data-results-detail]")?.parentElement;
  if (detailSwitch) detailSwitch.hidden = subway;
  return style;
}

// Maps are initially rendered while Overview is visible. Hit areas need the
// station's real SVG bounds once Process is shown.
function ensureSubwayHitAreas(svg){
  for(const group of svg.querySelectorAll?.('.subway-station') || []){
    if(group.querySelector('.subway-hit'))continue;
    const box=group.getBBox?.();
    if(box?.width)group.insertBefore(svgElement('rect',{class:'subway-hit',x:box.x-5,y:box.y-5,width:box.width+10,height:box.height+10,fill:'transparent','pointer-events':'all'}),group.firstChild);
  }
}
