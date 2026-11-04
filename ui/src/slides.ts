// Slides: presentation of the saved investigation with cited evidence.

function renderSavedEvidence(snapshot) {
  const container = document.querySelector('#report-evidence-summary');
  container.replaceChildren();
  const result = snapshot.result;
  if (snapshot.request?.type === 'cohorts') {
    const table = reportElement('table', undefined, 'brief-evidence-table');
    table.append(reportElement('caption', 'Timing of the recorded groups'));
    const head = reportElement('thead'); const headings = reportElement('tr');
    ['Group', itemLabels().plural, 'Average time', 'Median time'].forEach(text => headings.append(reportElement('th', text)));
    head.append(headings); table.append(head);
    const body = reportElement('tbody');
    [result.left, result.right].forEach(group => {
      const row = reportElement('tr');
      [group.value, group.cases, readableReportTime(measuredDuration(group.mean_throughput_seconds, group.duration_eligible_cases)), readableReportTime(measuredDuration(group.median_throughput_seconds, group.duration_eligible_cases))].forEach(text => row.append(reportElement('td', text)));
      body.append(row);
    }); table.append(body); container.append(table);
    container.append(reportElement('p', 'These are elapsed times, not measured effort. Differences between groups do not establish a cause.'));
  } else if (snapshot.request?.type === 'case') {
    container.append(reportElement('h4', result.case));
    const timeline = reportElement('ol', undefined, 'brief-evidence-timeline');
    (result.events || []).forEach(event => {
      const row = reportElement('li');
      if (event.since_previous_seconds != null) row.append(reportElement('small', `${readableReportTime(formatDuration(event.since_previous_seconds))} ${result.gap_is_waiting ? 'waiting' : 'since previous step'}`));
      row.append(reportElement('strong', event.activity), reportElement('span', event.timestamp)); timeline.append(row);
    }); container.append(timeline);
  } else if (snapshot.request?.type === 'transition' && Array.isArray(result.cases)) {
    container.append(reportElement('p', `${result.matching_cases} ${itemLabels().plural.toLowerCase()} have this recorded transition${result.truncated ? '; the examples below are a partial list' : '; all matching items are listed below'}.`));
    const examples = reportElement('div', undefined, 'case-evidence');
    result.cases.forEach(id => {
      const button = reportElement('button', id, 'case-chip'); button.type = 'button';
      button.addEventListener('click', () => void openCase(id)); examples.append(button);
    }); container.append(examples);
  }
}

let evidenceReturnView = "report";
function citationButton(reference) {
  const refs = Object.keys(reportEvidence);
  const index = refs.filter(ref => ref !== 'engine:analysis').indexOf(reference) + 1;
  const snapshot = reportEvidence[reference];
  const names = { cohorts: 'Group comparison', transition: 'Step history', case: 'Item timeline' };
  const name = snapshot?.request?.type === 'case' ? `${snapshot.result?.case || 'Item'} history` : names[snapshot?.request?.type] || 'Evidence';
  const label = reference === 'engine:analysis' ? 'Calculated summary' : `Source ${index} · ${name}`;
  const button = reportElement('button', label, 'slide-citation');
  button.type = 'button'; button.disabled = !snapshot;
  button.addEventListener('click', () => {
    evidenceReturnView = document.body?.dataset.view || 'report';
    if (evidenceReturnView !== 'report') showView('report');
    const close = document.querySelector('#report-evidence-close');
    if (close) close.textContent = evidenceReturnView === 'report' ? 'Close' : 'Back to ' + viewNames[evidenceReturnView];
    const panel = document.querySelector('#report-evidence'); panel.hidden = false;
    document.querySelector('#report-evidence-title').textContent = label;
    document.querySelector('#report-evidence-description').textContent = reference === 'engine:analysis' ? 'Calculated analysis for the active project view.' : 'Saved calculation supporting this finding. The original query and scope are available below.';
    renderSavedEvidence(snapshot);
    document.querySelector('#report-evidence-json').textContent = JSON.stringify(snapshot, null, 2);
    const caseButton = document.querySelector('#report-evidence-case');
    caseButton.hidden = snapshot.request?.type !== 'case';
    caseButton.onclick = () => void openCase(snapshot.request.id);
    panel.scrollIntoView({ behavior: 'smooth', block: 'start' });
  });
  return button;
}

// The evidence panel a citation opens: closing it returns to the page it came from.
function initializeEvidencePanel() {
  document.querySelector('#report-refresh')?.addEventListener('click', () => void refreshExecutiveReport());
  document.querySelector('#report-evidence-close')?.addEventListener('click', () => { document.querySelector('#report-evidence').hidden = true; if (evidenceReturnView !== 'report') showView(evidenceReturnView); });
}
