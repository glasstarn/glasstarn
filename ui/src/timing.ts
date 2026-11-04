// Performance: draws the engine's completion-time distribution.

function drawDurationHistogram(svg, timing) {
  const width = 640, height = 270, left = 44, right = 12, top = 28, bottom = 40;
  svg.replaceChildren();
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  if (!timing.bins.length) return;
  const most = Math.max(1, ...timing.bins.map((bin) => bin.cases));
  const plotWidth = width - left - right;
  const plotHeight = height - top - bottom;
  const band = plotWidth / timing.bins.length;
  const y = (count) => top + plotHeight - (count / most) * plotHeight;
  for (const step of [0, 0.5, 1]) {
    const value = Math.round(most * step);
    svg.append(svgElement("line", { x1: left, x2: width - right, y1: y(value), y2: y(value), class: "histogram-grid" }));
    const label = svgElement("text", { x: left - 8, y: y(value) + 4, "text-anchor": "end", class: "histogram-axis" });
    label.textContent = value.toLocaleString();
    svg.append(label);
  }
  const labelEvery = Math.ceil(timing.bins.length / 7);
  timing.bins.forEach((bin, index) => {
    const x = left + index * band + 3;
    const slow = timing.p90_seconds > 0 && bin.from_seconds >= timing.p90_seconds;
    const bar = svgElement("rect", { x, y: y(bin.cases), width: Math.max(1, band - 6), height: Math.max(0, top + plotHeight - y(bin.cases)), rx: 2, class: slow ? "histogram-bar slow" : "histogram-bar" });
    const title = svgElement("title");
    title.textContent = `${bin.to_seconds === null ? `${formatDuration(bin.from_seconds)} or longer` : `${formatDuration(bin.from_seconds)} to ${formatDuration(bin.to_seconds)}`}: ${bin.cases.toLocaleString()}`;
    bar.append(title);
    svg.append(bar);
    if (index % labelEvery === 0 || bin.to_seconds === null) {
      const tick = svgElement("text", { x: x + (band - 6) / 2, y: height - bottom + 18, "text-anchor": "middle", class: "histogram-axis" });
      tick.textContent = bin.to_seconds === null ? `${formatDuration(bin.from_seconds)}+` : formatDuration(bin.from_seconds);
      svg.append(tick);
    }
  });
  if (timing.duration_eligible_cases) {
    const position = left + Math.min(timing.bins.length, timing.median_seconds / timing.bin_width_seconds) * band;
    svg.append(svgElement("line", { x1: position, x2: position, y1: top, y2: top + plotHeight, class: "histogram-median" }));
    const label = svgElement("text", { x: position, y: top - 10, "text-anchor": "middle", class: "histogram-median-label" });
    label.textContent = `median ${formatDuration(timing.median_seconds)}`;
    svg.append(label);
  }
}
