//! Bounded, searchable field choices shared by every presentation surface.
use pm_core::EventLog;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct AttributeChoice {
    pub value: String,
    pub cases: usize,
}

#[derive(Debug, Serialize)]
pub struct AttributeChoices {
    pub attribute: String,
    pub values: Vec<AttributeChoice>,
    pub distinct_values: usize,
    pub matching_values: usize,
    pub missing_cases: usize,
    pub truncated: bool,
}

pub fn attribute_choices(
    log: &EventLog,
    attribute: &str,
    search: &str,
    limit: usize,
) -> Result<AttributeChoices, String> {
    let index = log
        .attribute_index(attribute)
        .ok_or_else(|| format!("Unknown field: {attribute}"))?;
    let mut counts = BTreeMap::new();
    let mut missing = 0;
    // An item counts once under every value it had; a column that changes
    // between steps gives an item several.
    for trace in &log.traces {
        let values = log.case_values(trace, index);
        if values.is_empty() {
            missing += 1;
        }
        for value in values
            .into_iter()
            .filter_map(|id| log.attribute_values.label(id))
        {
            *counts.entry(value).or_insert(0usize) += 1;
        }
    }
    let distinct_values = counts.len();
    let search = search.to_lowercase();
    let mut matches = counts
        .into_iter()
        .filter(|(value, _)| value.to_lowercase().contains(&search))
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let matching_values = matches.len();
    let values = matches
        .into_iter()
        .take(limit.min(100))
        .map(|(value, cases)| AttributeChoice {
            value: value.to_owned(),
            cases,
        })
        .collect::<Vec<_>>();
    Ok(AttributeChoices {
        attribute: attribute.to_owned(),
        truncated: values.len() < matching_values,
        values,
        distinct_values,
        matching_values,
        missing_cases: missing,
    })
}
