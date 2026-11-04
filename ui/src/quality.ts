// Data quality report.

function renderQuality(quality) {
  showView("quality");
  const panel = document.querySelector("#quality-panel");
  const summary = document.querySelector("#quality-summary");
  const findings = document.querySelector("#quality-findings");
  if (!panel || !summary || !findings) return;
  summary.textContent = `${quality.summary.open_decisions.toLocaleString()} open ${quality.summary.open_decisions === 1 ? "decision" : "decisions"} · ${quality.summary.warnings.toLocaleString()} warnings`;
  findings.replaceChildren(...quality.findings.map((finding) => {
    const article = document.createElement("article"); article.className = "quality-finding"; article.dataset.severity = finding.severity;
    const heading = document.createElement("div"); heading.className = "finding-heading";
    const title = document.createElement("h3"); title.textContent = finding.title;
    const severity = document.createElement("span"); severity.className = "severity-badge"; severity.textContent = finding.severity;
    heading.append(title, severity);
    const detail = document.createElement("p"); detail.textContent = finding.detail;
    article.append(heading, detail);
    if (finding.examples.length) { const examples = document.createElement("div"); examples.className = "finding-examples"; examples.textContent = finding.examples.join(" · "); article.append(examples); }
    if (finding.decision) {
      const fieldset = document.createElement("fieldset"); fieldset.className = "decision-fieldset"; fieldset.dataset.decisionKind = finding.kind; fieldset.dataset.required = String(finding.decision.required);
      const legend = document.createElement("legend"); legend.textContent = finding.decision.question;
      const options = document.createElement("div"); options.className = "decision-options";
      if (!finding.decision.required) {
        const unchanged = document.createElement("label"); unchanged.className = "decision-option";
        const radio = document.createElement("input"); radio.type = "radio"; radio.name = `decision-${finding.kind}`; radio.value = ""; radio.checked = currentPolicy[finding.kind] === undefined;
        const text = document.createElement("span"); text.textContent = "Leave unchanged"; unchanged.append(radio, text); options.append(unchanged);
      }
      for (const option of finding.decision.options) {
        const label = document.createElement("label"); label.className = "decision-option";
        const radio = document.createElement("input"); radio.type = "radio"; radio.name = `decision-${finding.kind}`; radio.value = option.id; radio.checked = currentPolicy[finding.kind] === option.id;
        const text = document.createElement("span"); text.textContent = option.label;
        const effect = document.createElement("small"); effect.textContent = option.effect;
        label.append(radio, text, effect); options.append(label);
      }
      const risk = document.createElement("div"); risk.className = "decision-risk"; risk.textContent = `Why this matters: ${finding.decision.risk}`;
      fieldset.append(legend, options, risk); article.append(fieldset);
    }
    return article;
  }));
  const navigation = document.querySelector("#quality-nav");
  navigation?.classList.remove("unavailable"); navigation?.removeAttribute("aria-disabled"); navigation?.removeAttribute("tabindex");
  panel.hidden = false;
  panel.scrollIntoView({ behavior: "smooth", block: "start" });
}
