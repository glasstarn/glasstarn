//! Deterministic construction of an event log from company-supplied CSV extracts.
//!
//! The recipe is deliberately bounded: it unions explicit event projections and
//! can enrich them through checked many-to-one lookups. It cannot execute code or
//! silently invent events. Every skipped row and unmatched lookup is reconciled.

use crate::read::{ReadError, stream_csv};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PreparationError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error("could not write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: csv::Error,
    },
    #[error("invalid preparation recipe: {0}")]
    Recipe(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct InventoryColumn {
    pub name: String,
    pub non_blank: u64,
    pub blank: u64,
    pub distinct_observed: u64,
    pub distinct_complete: bool,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InputInventory {
    pub rows: u64,
    pub columns: Vec<InventoryColumn>,
}

/// Read a complete CSV for exact row, blank, and distinct-value counts.
/// Examples remain bounded; full values never enter the result.
pub fn inventory_csv(path: impl AsRef<Path>) -> Result<InputInventory, PreparationError> {
    const DISTINCT_LIMIT: usize = 10_000;
    let (schema, rows) = stream_csv(path)?;
    let mut blanks = vec![0u64; schema.len()];
    let mut present = vec![0u64; schema.len()];
    let mut distinct = vec![HashSet::<String>::new(); schema.len()];
    let mut examples = vec![Vec::<String>::new(); schema.len()];
    let mut count = 0u64;
    for row in rows {
        let row = row?;
        count += 1;
        for index in 0..schema.len() {
            let value = row.at(index);
            if value.trim().is_empty() {
                blanks[index] += 1;
            } else {
                present[index] += 1;
                if distinct[index].len() < DISTINCT_LIMIT || distinct[index].contains(value) {
                    distinct[index].insert(value.to_owned());
                }
                if examples[index].len() < 4 && !examples[index].iter().any(|item| item == value) {
                    examples[index].push(value.to_owned());
                }
            }
        }
    }
    Ok(InputInventory {
        rows: count,
        columns: schema
            .headers()
            .iter()
            .enumerate()
            .map(|(index, name)| InventoryColumn {
                name: name.clone(),
                non_blank: present[index],
                blank: blanks[index],
                distinct_observed: distinct[index].len() as u64,
                distinct_complete: distinct[index].len() < DISTINCT_LIMIT,
                examples: examples[index].clone(),
            })
            .collect(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationRecipe {
    pub version: u32,
    pub inputs: Vec<InputReference>,
    pub events: Vec<EventRule>,
    #[serde(default)]
    pub lookups: Vec<LookupRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputReference {
    pub id: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub column: String,
    #[serde(rename = "as")]
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActivitySource {
    Column(ActivityColumn),
    Value(ActivityValue),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityColumn {
    pub column: String,
    /// Words put before the column's value, such as `Change task: ` before a
    /// task's status, so steps of different item types read apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

impl ActivityColumn {
    /// The step name for a row's value; blank stays blank.
    pub fn name<'a>(&self, value: &'a str) -> std::borrow::Cow<'a, str> {
        match &self.prefix {
            Some(prefix) if !value.trim().is_empty() => format!("{prefix}{value}").into(),
            _ => value.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityValue {
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventRule {
    pub input: String,
    pub case_id: String,
    pub activity: ActivitySource,
    pub timestamp: String,
    #[serde(default)]
    pub optional_timestamp: bool,
    pub end_timestamp: Option<String>,
    pub event_order: Option<String>,
    pub resource: Option<String>,
    pub lifecycle: Option<String>,
    pub cost: Option<String>,
    #[serde(default)]
    pub case_attributes: Vec<Field>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookupRule {
    pub event_input: String,
    pub event_key: String,
    pub input: String,
    pub lookup_key: String,
    pub fields: Vec<Field>,
    #[serde(default)]
    pub allow_unmatched: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EventRuleReport {
    pub input: String,
    pub rows_read: u64,
    pub events_emitted: u64,
    pub optional_timestamps_blank: u64,
    pub invalid_required_rows: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LookupReport {
    pub event_input: String,
    pub input: String,
    pub lookup_rows: u64,
    pub unique_keys: u64,
    pub duplicate_keys: u64,
    pub blank_lookup_keys: u64,
    pub matched_events: u64,
    pub unmatched_events: u64,
    pub blank_event_keys: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreparationReport {
    pub status: &'static str,
    pub output_rows: u64,
    pub output_cases: u64,
    pub exact_duplicate_events: u64,
    pub event_rules: Vec<EventRuleReport>,
    pub lookups: Vec<LookupReport>,
    pub issues: Vec<String>,
}

pub enum PreparationOutcome {
    Complete(PreparationReport),
    Blocked(PreparationReport),
}

struct LookupData {
    rule: LookupRule,
    values: HashMap<String, Vec<String>>,
    report: LookupReport,
}

pub fn prepare_csv(
    recipe: &PreparationRecipe,
    inputs: &BTreeMap<String, PathBuf>,
    output: impl AsRef<Path>,
) -> Result<PreparationOutcome, PreparationError> {
    validate_recipe(recipe, inputs)?;
    let output = output.as_ref();
    let mut headers = vec![
        "case_id".to_owned(),
        "activity".to_owned(),
        "timestamp".to_owned(),
    ];
    for (name, used) in [
        (
            "end_timestamp",
            recipe.events.iter().any(|r| r.end_timestamp.is_some()),
        ),
        (
            "event_order",
            recipe.events.iter().any(|r| r.event_order.is_some()),
        ),
        (
            "resource",
            recipe.events.iter().any(|r| r.resource.is_some()),
        ),
        (
            "lifecycle",
            recipe.events.iter().any(|r| r.lifecycle.is_some()),
        ),
        ("cost", recipe.events.iter().any(|r| r.cost.is_some())),
    ] {
        if used {
            headers.push(name.into());
        }
    }
    let mut custom = BTreeSet::new();
    for rule in &recipe.events {
        custom.extend(
            rule.case_attributes
                .iter()
                .map(|field| field.output.clone()),
        );
    }
    for lookup in &recipe.lookups {
        custom.extend(lookup.fields.iter().map(|field| field.output.clone()));
    }
    headers.extend(custom);
    headers.push("_source_input".into());
    headers.push("_source_row".into());

    let mut issues = Vec::new();
    let mut lookups = Vec::new();
    for rule in &recipe.lookups {
        let path = inputs.get(&rule.input).expect("validated input");
        let (schema, rows) = stream_csv(path)?;
        require_columns(
            &rule.input,
            schema.headers(),
            std::iter::once(&rule.lookup_key).chain(rule.fields.iter().map(|f| &f.column)),
        )?;
        let mut values = HashMap::new();
        let mut report = LookupReport {
            event_input: rule.event_input.clone(),
            input: rule.input.clone(),
            ..Default::default()
        };
        for row in rows {
            let row = row?;
            report.lookup_rows += 1;
            let key = row.get(&rule.lookup_key).to_owned();
            if key.trim().is_empty() {
                report.blank_lookup_keys += 1;
                continue;
            }
            let selected = rule
                .fields
                .iter()
                .map(|field| row.get(&field.column).to_owned())
                .collect();
            if values.insert(key.clone(), selected).is_some() {
                report.duplicate_keys += 1;
            }
        }
        report.unique_keys = values.len() as u64;
        if report.duplicate_keys > 0 {
            issues.push(format!(
                "lookup '{}' has {} duplicate '{}' keys; a many-to-one join is required",
                rule.input, report.duplicate_keys, rule.lookup_key
            ));
        }
        if report.blank_lookup_keys > 0 {
            issues.push(format!(
                "lookup '{}' has {} blank '{}' keys",
                rule.input, report.blank_lookup_keys, rule.lookup_key
            ));
        }
        lookups.push(LookupData {
            rule: rule.clone(),
            values,
            report,
        });
    }

    let mut writer = csv::WriterBuilder::new()
        .from_path(output)
        .map_err(|source| PreparationError::Write {
            path: output.display().to_string(),
            source,
        })?;
    writer
        .write_record(&headers)
        .map_err(|source| PreparationError::Write {
            path: output.display().to_string(),
            source,
        })?;
    let index: BTreeMap<String, usize> = headers
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), i))
        .collect();
    let mut rule_reports = Vec::new();
    let mut output_rows = 0u64;
    let mut cases = HashSet::new();
    let mut seen_events = HashSet::new();
    let mut duplicates = 0u64;

    for rule in &recipe.events {
        let path = inputs.get(&rule.input).expect("validated input");
        let (schema, rows) = stream_csv(path)?;
        let mut required = vec![&rule.case_id, &rule.timestamp];
        if let ActivitySource::Column(activity) = &rule.activity {
            required.push(&activity.column);
        }
        for value in [
            &rule.end_timestamp,
            &rule.event_order,
            &rule.resource,
            &rule.lifecycle,
            &rule.cost,
        ]
        .into_iter()
        .flatten()
        {
            required.push(value);
        }
        required.extend(rule.case_attributes.iter().map(|field| &field.column));
        for lookup in recipe
            .lookups
            .iter()
            .filter(|lookup| lookup.event_input == rule.input)
        {
            required.push(&lookup.event_key);
        }
        require_columns(&rule.input, schema.headers(), required.into_iter())?;
        let mut report = EventRuleReport {
            input: rule.input.clone(),
            ..Default::default()
        };
        for row in rows {
            let row = row?;
            report.rows_read += 1;
            let case = row.get(&rule.case_id);
            let activity = match &rule.activity {
                ActivitySource::Column(activity) => activity.name(row.get(&activity.column)),
                ActivitySource::Value(activity) => activity.value.as_str().into(),
            };
            let activity = activity.as_ref();
            let timestamp = row.get(&rule.timestamp);
            if timestamp.trim().is_empty() && rule.optional_timestamp {
                report.optional_timestamps_blank += 1;
                continue;
            }
            if case.trim().is_empty() || activity.trim().is_empty() || timestamp.trim().is_empty() {
                report.invalid_required_rows += 1;
                continue;
            }
            let mut out = vec![String::new(); headers.len()];
            set(&mut out, &index, "case_id", case);
            set(&mut out, &index, "activity", activity);
            set(&mut out, &index, "timestamp", timestamp);
            for (name, column) in [
                ("end_timestamp", &rule.end_timestamp),
                ("event_order", &rule.event_order),
                ("resource", &rule.resource),
                ("lifecycle", &rule.lifecycle),
                ("cost", &rule.cost),
            ] {
                if let Some(column) = column {
                    set(&mut out, &index, name, row.get(column));
                }
            }
            for field in &rule.case_attributes {
                set(&mut out, &index, &field.output, row.get(&field.column));
            }
            for lookup in lookups
                .iter_mut()
                .filter(|lookup| lookup.rule.event_input == rule.input)
            {
                let key = row.get(&lookup.rule.event_key);
                if key.trim().is_empty() {
                    lookup.report.blank_event_keys += 1;
                    lookup.report.unmatched_events += 1;
                } else if let Some(values) = lookup.values.get(key) {
                    lookup.report.matched_events += 1;
                    for (field, value) in lookup.rule.fields.iter().zip(values) {
                        set(&mut out, &index, &field.output, value);
                    }
                } else {
                    lookup.report.unmatched_events += 1;
                }
            }
            // Compare analytical values before adding lineage; otherwise two
            // duplicate source rows differ only by their spreadsheet row.
            if !seen_events.insert(out.clone()) {
                duplicates += 1;
            }
            set(&mut out, &index, "_source_input", &rule.input);
            set(&mut out, &index, "_source_row", &row.source_row.to_string());
            cases.insert(case.to_owned());
            writer
                .write_record(&out)
                .map_err(|source| PreparationError::Write {
                    path: output.display().to_string(),
                    source,
                })?;
            output_rows += 1;
            report.events_emitted += 1;
        }
        if report.invalid_required_rows > 0 {
            issues.push(format!(
                "event input '{}' has {} rows missing a required case, activity, or timestamp",
                rule.input, report.invalid_required_rows
            ));
        }
        rule_reports.push(report);
    }
    writer.flush().map_err(|source| PreparationError::Write {
        path: output.display().to_string(),
        source: source.into(),
    })?;
    for lookup in &lookups {
        if lookup.report.unmatched_events > 0 && !lookup.rule.allow_unmatched {
            issues.push(format!(
                "lookup '{}' left {} events unmatched; set allow_unmatched only after review",
                lookup.rule.input, lookup.report.unmatched_events
            ));
        }
    }
    if output_rows == 0 {
        issues.push("the recipe produced no events".into());
    }
    let blocked = !issues.is_empty();
    let report = PreparationReport {
        status: if blocked { "blocked" } else { "complete" },
        output_rows,
        output_cases: cases.len() as u64,
        exact_duplicate_events: duplicates,
        event_rules: rule_reports,
        lookups: lookups.into_iter().map(|lookup| lookup.report).collect(),
        issues,
    };
    Ok(if blocked {
        PreparationOutcome::Blocked(report)
    } else {
        PreparationOutcome::Complete(report)
    })
}

fn validate_recipe(
    recipe: &PreparationRecipe,
    inputs: &BTreeMap<String, PathBuf>,
) -> Result<(), PreparationError> {
    if recipe.version != 1 {
        return Err(PreparationError::Recipe("version must be 1".into()));
    }
    if recipe.events.is_empty() {
        return Err(PreparationError::Recipe(
            "at least one event rule is required".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    for input in &recipe.inputs {
        if input.id.trim().is_empty() || !ids.insert(input.id.clone()) {
            return Err(PreparationError::Recipe(
                "input ids must be non-empty and unique".into(),
            ));
        }
        if !inputs.contains_key(&input.id) {
            return Err(PreparationError::Recipe(format!(
                "input '{}' was not resolved",
                input.id
            )));
        }
    }
    for rule in &recipe.events {
        if !ids.contains(&rule.input) {
            return Err(PreparationError::Recipe(format!(
                "event input '{}' is not declared",
                rule.input
            )));
        }
    }
    for rule in &recipe.lookups {
        if !ids.contains(&rule.event_input) || !ids.contains(&rule.input) {
            return Err(PreparationError::Recipe(
                "every lookup input must be declared".into(),
            ));
        }
        if rule.fields.is_empty() {
            return Err(PreparationError::Recipe(
                "a lookup must select at least one field".into(),
            ));
        }
    }
    let reserved = [
        "case_id",
        "activity",
        "timestamp",
        "end_timestamp",
        "event_order",
        "resource",
        "lifecycle",
        "cost",
        "_source_input",
        "_source_row",
    ];
    for rule in &recipe.events {
        let mut outputs = BTreeSet::new();
        for field in &rule.case_attributes {
            if field.output.trim().is_empty()
                || reserved.contains(&field.output.as_str())
                || !outputs.insert(field.output.clone())
            {
                return Err(PreparationError::Recipe(format!(
                    "output field '{}' is empty, reserved, or duplicated within one event rule",
                    field.output
                )));
            }
        }
    }
    let mut lookup_outputs: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for rule in &recipe.lookups {
        let outputs = lookup_outputs.entry(&rule.event_input).or_default();
        for field in &rule.fields {
            let collides_with_event = recipe.events.iter().any(|event| {
                event.input == rule.event_input
                    && event
                        .case_attributes
                        .iter()
                        .any(|candidate| candidate.output == field.output)
            });
            if field.output.trim().is_empty()
                || reserved.contains(&field.output.as_str())
                || collides_with_event
                || !outputs.insert(&field.output)
            {
                return Err(PreparationError::Recipe(format!(
                    "lookup output field '{}' is empty, reserved, or would overwrite another field for input '{}'",
                    field.output, rule.event_input
                )));
            }
        }
    }
    Ok(())
}

fn require_columns<'a>(
    input: &str,
    headers: &[String],
    columns: impl Iterator<Item = &'a String>,
) -> Result<(), PreparationError> {
    for column in columns {
        if !headers.contains(column) {
            return Err(PreparationError::Recipe(format!(
                "input '{input}' has no column '{column}'"
            )));
        }
    }
    Ok(())
}

fn set(row: &mut [String], index: &BTreeMap<String, usize>, name: &str, value: &str) {
    if let Some(position) = index.get(name) {
        row[*position] = value.to_owned();
    }
}
