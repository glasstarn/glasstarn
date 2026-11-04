//! Wide-to-long event building.
//!
//! # The shape real exports have
//!
//! Process mining needs one row per event. Most operational systems export one
//! row per *case*, with a timestamp column for each milestone:
//!
//! ```text
//! ticket_id,priority,created_at,assigned_at,escalated_at,resolved_at
//! T-1001,High,2026-03-01 09:12,2026-03-01 11:40,,2026-03-04 16:02
//! ```
//!
//! Every process-mining tool assumes the user already unpivoted this. For an
//! analyst with a ticketing export and no data engineer, that assumption is a
//! wall. Turning the table above into events is a small amount of work here
//! and likely serves more first-time users than a second mining algorithm
//! would.
//!
//! A blank cell means **the step did not happen**, so it produces no event.
//! That is unambiguous in wide layout, and deliberately different from a blank
//! in a long-format log, where it could equally mean missing data. Because the
//! same symbol means different things in different layouts, the mapping is
//! always shown for confirmation rather than applied silently.

use crate::read::{Row, Schema};
use crate::timestamp;
use std::rc::Rc;

/// Suffixes stripped from a column name when deriving an activity label.
const SUFFIXES: &[&str] = &[
    "_at",
    "_date",
    "_time",
    "_timestamp",
    "_on",
    "_dt",
    "at",
    "date",
    "timestamp",
];

/// How to turn one wide row into several events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WideSpec {
    /// Column holding the case identifier.
    pub case_column: String,
    /// Timestamp columns, each with the activity label it becomes.
    pub events: Vec<EventColumn>,
    /// Columns carried through as case attributes.
    pub attributes: Vec<String>,
}

/// One timestamp column and the activity it represents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventColumn {
    /// The source column name.
    pub column: String,
    /// The activity label it becomes. Editable by the user.
    pub activity: String,
}

impl WideSpec {
    /// The schema of the long-format rows this spec produces.
    pub fn output_schema(&self) -> Rc<Schema> {
        let mut headers = vec![
            "case_id".to_owned(),
            "activity".to_owned(),
            "timestamp".to_owned(),
        ];
        headers.extend(self.attributes.iter().cloned());
        Rc::new(Schema::new(headers))
    }

    /// The mapping that applies to the produced rows.
    pub fn output_mapping(&self) -> crate::validate::Mapping {
        crate::validate::Mapping::new("case_id", "activity", "timestamp")
    }
}

/// Derives a readable activity label from a column name.
///
/// `created_at` becomes `Created`; `first_response_time` becomes
/// `First Response`. The user can always override it.
pub fn activity_label(column: &str) -> String {
    let lowered = column.trim().to_lowercase().replace(['-', ' '], "_");
    let mut base = lowered.as_str();
    for suffix in SUFFIXES {
        if let Some(stripped) = base.strip_suffix(suffix)
            && !stripped.is_empty()
            && stripped != "_"
        {
            base = stripped;
            break;
        }
    }
    let base = base.trim_matches('_');
    if base.is_empty() {
        return column.trim().to_owned();
    }

    base.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How many rows to inspect when deciding whether a table is wide.
const DETECT_ROWS: usize = 50;

/// A column must parse as a timestamp in at least this share of its non-blank
/// sampled values to count as an event column.
const TIMESTAMP_CONFIDENCE: f64 = 0.9;

/// Proposes a wide-to-long spec, if the table looks like wide format.
///
/// Returns `None` when fewer than two timestamp columns are found — one
/// timestamp column is a long-format log, not a wide one.
pub fn detect(schema: &Schema, sample: &[Row]) -> Option<WideSpec> {
    let mut timestamp_columns = Vec::new();
    let mut other_columns = Vec::new();

    for (index, header) in schema.headers().iter().enumerate() {
        let values: Vec<&str> = sample
            .iter()
            .take(DETECT_ROWS)
            .map(|r| r.at(index))
            .filter(|v| !v.trim().is_empty())
            .collect();

        if values.is_empty() {
            // An entirely blank column in the sample is not evidence either
            // way; treat it as an attribute rather than guessing.
            other_columns.push(header.clone());
            continue;
        }

        let parsed = values
            .iter()
            .filter(|v| timestamp::all().iter().any(|f| f.parse(v).is_some()))
            .count();

        if parsed as f64 / values.len() as f64 >= TIMESTAMP_CONFIDENCE {
            timestamp_columns.push(header.clone());
        } else {
            other_columns.push(header.clone());
        }
    }

    if timestamp_columns.len() < 2 {
        return None;
    }

    let case_column = pick_case_column(&other_columns, sample, schema)?;
    let attributes = other_columns
        .into_iter()
        .filter(|c| *c != case_column)
        .collect();

    Some(WideSpec {
        case_column,
        events: timestamp_columns
            .into_iter()
            .map(|column| EventColumn {
                activity: activity_label(&column),
                column,
            })
            .collect(),
        attributes,
    })
}

/// Chooses the case identifier column.
///
/// Prefers a name that reads like an identifier; otherwise the column whose
/// values are most nearly unique, since a case id is one per row here.
fn pick_case_column(candidates: &[String], sample: &[Row], schema: &Schema) -> Option<String> {
    const ID_HINTS: &[&str] = &["_id", "id", "number", "key", "ref", "no"];

    if let Some(named) = candidates.iter().find(|c| {
        let lower = c.to_lowercase();
        ID_HINTS.iter().any(|h| lower.ends_with(h))
    }) {
        return Some(named.clone());
    }

    candidates
        .iter()
        .max_by_key(|c| {
            let Some(index) = schema.position(c) else {
                return 0;
            };
            sample
                .iter()
                .take(DETECT_ROWS)
                .map(|r| r.at(index))
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        })
        .cloned()
}

/// Turns one wide row into its events.
///
/// Blank timestamps produce no event. Source row numbers are preserved so
/// evidence still points at the original spreadsheet row.
pub fn unpivot(row: &Row, spec: &WideSpec, out_schema: &Rc<Schema>) -> Vec<Row> {
    let case = row.get(&spec.case_column).to_owned();
    let attributes: Vec<String> = spec
        .attributes
        .iter()
        .map(|c| row.get(c).to_owned())
        .collect();

    spec.events
        .iter()
        .filter_map(|event| {
            let stamp = row.get(&event.column);
            if stamp.trim().is_empty() {
                return None;
            }
            let mut values = Vec::with_capacity(3 + attributes.len());
            values.push(case.clone());
            values.push(event.activity.clone());
            values.push(stamp.to_owned());
            values.extend(attributes.iter().cloned());
            Some(Row::new(Rc::clone(out_schema), values, row.source_row))
        })
        .collect()
}

/// Turns a whole wide table into long-format rows.
pub fn unpivot_all(rows: &[Row], spec: &WideSpec) -> Vec<Row> {
    let schema = spec.output_schema();
    rows.iter()
        .flat_map(|r| unpivot(r, spec, &schema))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(headers: &[&str], data: &[&[&str]]) -> (Rc<Schema>, Vec<Row>) {
        let schema = Rc::new(Schema::new(
            headers.iter().map(|h| (*h).to_owned()).collect(),
        ));
        let rows = data
            .iter()
            .enumerate()
            .map(|(i, values)| {
                Row::new(
                    Rc::clone(&schema),
                    values.iter().map(|v| (*v).to_owned()).collect(),
                    i + 2,
                )
            })
            .collect();
        (schema, rows)
    }

    /// Mirrors `testdata/canonical/14-wide-format.csv`.
    fn tickets() -> (Rc<Schema>, Vec<Row>) {
        table(
            &[
                "ticket_id",
                "priority",
                "created_at",
                "assigned_at",
                "escalated_at",
                "resolved_at",
                "closed_at",
            ],
            &[
                &[
                    "T-1001",
                    "High",
                    "2026-03-01 09:12:00",
                    "2026-03-01 11:40:00",
                    "",
                    "2026-03-04 16:02:00",
                    "2026-03-05 09:00:00",
                ],
                &[
                    "T-1002",
                    "Low",
                    "2026-03-01 10:05:00",
                    "2026-03-02 09:15:00",
                    "",
                    "2026-03-02 15:30:00",
                    "2026-03-03 08:00:00",
                ],
                &[
                    "T-1003",
                    "High",
                    "2026-03-01 14:22:00",
                    "2026-03-01 15:00:00",
                    "2026-03-03 09:00:00",
                    "2026-03-06 11:45:00",
                    "2026-03-06 17:00:00",
                ],
                &[
                    "T-1004",
                    "Medium",
                    "2026-03-02 08:30:00",
                    "2026-03-02 13:10:00",
                    "",
                    "",
                    "",
                ],
                &["T-1005", "Low", "2026-03-02 09:45:00", "", "", "", ""],
            ],
        )
    }

    #[test]
    fn derives_readable_activity_labels() {
        assert_eq!(activity_label("created_at"), "Created");
        assert_eq!(activity_label("assigned_at"), "Assigned");
        assert_eq!(activity_label("resolved_at"), "Resolved");
        assert_eq!(activity_label("first_response_time"), "First Response");
        assert_eq!(activity_label("Order Date"), "Order");
        assert_eq!(activity_label("shipped"), "Shipped");
    }

    /// A column whose whole name is a suffix must still yield a usable label
    /// rather than an empty string.
    #[test]
    fn does_not_strip_a_label_to_nothing() {
        for name in ["date", "_at", "timestamp", "at", "time"] {
            let label = activity_label(name);
            assert!(
                !label.trim().is_empty(),
                "{name:?} produced an empty activity label"
            );
        }
        assert_eq!(activity_label("date"), "Date");
    }

    #[test]
    fn detects_a_wide_table() {
        let (schema, rows) = tickets();
        let spec = detect(&schema, &rows).expect("should detect wide format");
        assert_eq!(spec.case_column, "ticket_id");
        assert_eq!(spec.attributes, vec!["priority"]);
        assert_eq!(
            spec.events
                .iter()
                .map(|e| e.activity.as_str())
                .collect::<Vec<_>>(),
            vec!["Created", "Assigned", "Escalated", "Resolved", "Closed"]
        );
    }

    /// A long-format log has exactly one timestamp column and must not be
    /// mistaken for a wide one.
    #[test]
    fn declines_a_long_format_table() {
        let (schema, rows) = table(
            &["case_id", "activity", "timestamp"],
            &[
                &["C1", "Start", "2026-03-02 09:00:00"],
                &["C1", "End", "2026-03-02 10:00:00"],
            ],
        );
        assert!(detect(&schema, &rows).is_none());
    }

    #[test]
    fn declines_a_table_with_no_timestamps() {
        let (schema, rows) = table(&["a", "b"], &[&["1", "2"], &["3", "4"]]);
        assert!(detect(&schema, &rows).is_none());
    }

    /// The hand-derived expectation from the canonical README: 16 events
    /// across 5 cases, with blanks producing nothing.
    #[test]
    fn produces_the_hand_derived_event_count() {
        let (schema, rows) = tickets();
        let spec = detect(&schema, &rows).unwrap();
        let events = unpivot_all(&rows, &spec);
        assert_eq!(events.len(), 16);
    }

    #[test]
    fn blank_cells_produce_no_event() {
        let (schema, rows) = tickets();
        let spec = detect(&schema, &rows).unwrap();
        let events = unpivot_all(&rows, &spec);

        let t1004: Vec<&Row> = events
            .iter()
            .filter(|r| r.get("case_id") == "T-1004")
            .collect();
        assert_eq!(t1004.len(), 2, "T-1004 is still open");
        let t1005: Vec<&Row> = events
            .iter()
            .filter(|r| r.get("case_id") == "T-1005")
            .collect();
        assert_eq!(t1005.len(), 1, "T-1005 has only been created");
    }

    #[test]
    fn carries_case_attributes_onto_every_event() {
        let (schema, rows) = tickets();
        let spec = detect(&schema, &rows).unwrap();
        let events = unpivot_all(&rows, &spec);
        for event in events.iter().filter(|r| r.get("case_id") == "T-1001") {
            assert_eq!(event.get("priority"), "High");
        }
    }

    /// Evidence must still point at the original spreadsheet row.
    #[test]
    fn preserves_the_source_row_number() {
        let (schema, rows) = tickets();
        let spec = detect(&schema, &rows).unwrap();
        let events = unpivot_all(&rows, &spec);
        let first: Vec<&Row> = events
            .iter()
            .filter(|r| r.get("case_id") == "T-1001")
            .collect();
        assert!(first.iter().all(|r| r.source_row == 2));
    }

    /// The produced rows must be directly usable by the normal pipeline.
    #[test]
    fn output_flows_through_validation_and_normalization() {
        use crate::finding::Policy;
        let (schema, rows) = tickets();
        let spec = detect(&schema, &rows).unwrap();
        let events = unpivot_all(&rows, &spec);
        let mapping = spec.output_mapping();

        let report = crate::validate::validate(&events, &mapping);
        let normalized =
            crate::normalize::normalize(&events, &mapping, &report, &Policy::new()).unwrap();

        assert_eq!(normalized.log.event_count(), 16);
        assert_eq!(normalized.log.case_count(), 5);
        assert_eq!(normalized.log.single_event_cases(), 1);

        // Four distinct paths, per the hand-derived expectation.
        let variants = pm_core::variants::Variants::build(&normalized.log);
        assert_eq!(variants.len(), 4);
    }

    #[test]
    fn user_can_rename_an_activity() {
        let (schema, rows) = tickets();
        let mut spec = detect(&schema, &rows).unwrap();
        spec.events[0].activity = "Ticket Raised".to_owned();
        let events = unpivot_all(&rows, &spec);
        assert!(events.iter().any(|r| r.get("activity") == "Ticket Raised"));
        assert!(!events.iter().any(|r| r.get("activity") == "Created"));
    }

    #[test]
    fn is_deterministic() {
        let (schema, rows) = tickets();
        let spec = detect(&schema, &rows).unwrap();
        assert_eq!(unpivot_all(&rows, &spec), unpivot_all(&rows, &spec));
    }

    /// Falls back to the most nearly unique column when no name looks like an
    /// identifier.
    #[test]
    fn picks_a_case_column_by_uniqueness_when_names_do_not_help() {
        let (schema, rows) = table(
            &["region", "reference", "opened", "closed"],
            &[
                &["North", "AAA", "2026-03-01 09:00:00", "2026-03-02 09:00:00"],
                &["North", "BBB", "2026-03-01 10:00:00", "2026-03-02 10:00:00"],
                &["South", "CCC", "2026-03-01 11:00:00", "2026-03-02 11:00:00"],
            ],
        );
        let spec = detect(&schema, &rows).unwrap();
        assert_eq!(spec.case_column, "reference");
    }
}
