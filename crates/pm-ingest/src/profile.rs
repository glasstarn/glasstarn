//! Column profiling.
//!
//! Before mapping three columns, a user needs to see what is actually in the
//! file. A proposed mapping the user cannot check is a guess wearing a
//! confident face — so profiling shows real sample values, what each column
//! looks like, and which role it might fill.
//!
//! Profiling reads a **sample**, not the whole file, so it stays instant on a
//! large log. Every count is therefore reported as "within the sample" rather
//! than presented as a fact about the file.

use crate::read::{Row, Schema};
use crate::timestamp;
use std::collections::BTreeSet;

/// How many rows profiling inspects.
pub const PROFILE_ROWS: usize = 200;

/// How many example values to show per column.
const EXAMPLES: usize = 4;

/// A column is treated as an identifier when nearly every value is distinct.
const IDENTIFIER_RATIO: f64 = 0.9;

/// Above this many distinct values, a column is not a useful activity label.
const CATEGORY_LIMIT: usize = 200;

/// A required field must be present on essentially every event, so a column
/// with more than this share of blanks cannot fill one.
const MAX_BLANK_FOR_REQUIRED: f64 = 2.0;

/// What a column appears to contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnKind {
    /// Parses as a date or time. Carries the format identifier when one
    /// explains the whole sample.
    Timestamp(Option<String>),
    /// Whole numbers.
    Integer,
    /// Numbers with a fractional part.
    Decimal,
    /// Nearly every value distinct — looks like an identifier.
    Identifier,
    /// A modest set of repeating values.
    Category,
    /// Free text.
    Text,
    /// Every sampled value was blank.
    Empty,
}

impl ColumnKind {
    /// A short human description.
    pub fn describe(&self) -> String {
        match self {
            ColumnKind::Timestamp(Some(f)) => format!("date/time ({f})"),
            ColumnKind::Timestamp(None) => "date/time (mixed formats)".to_owned(),
            ColumnKind::Integer => "whole number".to_owned(),
            ColumnKind::Decimal => "number".to_owned(),
            ColumnKind::Identifier => "identifier".to_owned(),
            ColumnKind::Category => "category".to_owned(),
            ColumnKind::Text => "text".to_owned(),
            ColumnKind::Empty => "empty".to_owned(),
        }
    }
}

/// A field a column could be mapped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    CaseId,
    Activity,
    Timestamp,
    EndTimestamp,
    EventOrder,
    Resource,
    Lifecycle,
    Cost,
}

impl Role {
    /// The field name as the user sees it.
    pub fn label(&self) -> &'static str {
        match self {
            Role::CaseId => "case id",
            Role::Activity => "activity",
            Role::Timestamp => "timestamp",
            Role::EndTimestamp => "end timestamp",
            Role::EventOrder => "event order",
            Role::Resource => "resource",
            Role::Lifecycle => "lifecycle",
            Role::Cost => "cost",
        }
    }

    /// Whether this field is required for any analysis.
    pub fn required(&self) -> bool {
        matches!(self, Role::CaseId | Role::Activity | Role::Timestamp)
    }
}

/// What one column contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub index: usize,
    /// Non-blank values inspected.
    pub sampled: usize,
    /// Blank values inspected.
    pub blank: usize,
    /// Distinct values seen **within the sample**.
    pub distinct: usize,
    /// Real values from the file, so the user can recognise the column.
    pub examples: Vec<String>,
    pub kind: ColumnKind,
    /// Fields this column could fill, most likely first.
    pub suggests: Vec<Role>,
}

/// How well a column fits a role. Higher is better.
///
/// Used to rank candidates rather than to exclude them, because on a file with
/// forty columns an unranked list of twenty possibilities is no more useful
/// than none.
fn fit(column: &Column, role: Role) -> Option<i32> {
    // A required field cannot come from a column that is often blank.
    if role.required() && column.blank_percent() > MAX_BLANK_FOR_REQUIRED {
        return None;
    }
    if !column.suggests.contains(&role) {
        return None;
    }

    let lower = column.name.to_lowercase();
    let hints = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    let ratio = column.distinct as f64 / column.sampled.max(1) as f64;

    let mut score = match column.suggests.iter().position(|r| *r == role) {
        Some(0) => 20,
        Some(1) => 10,
        _ => 0,
    };

    match role {
        // A case identifier names many things; an activity names few. In a
        // long-format log the case column has far more distinct values than
        // the activity column, so cardinality separates them — but only once
        // the sample is large enough to show it, which is why name hints carry
        // real weight here.
        Role::CaseId => {
            if hints(&[
                "_id", "id", "number", "_no", "key", "ref", "case", "ticket", "order",
            ]) {
                score += 60;
            }
            score += (ratio * 40.0) as i32;
        }
        Role::Activity => {
            if hints(&[
                "activity", "event", "action", "status", "step", "task", "state",
            ]) {
                score += 60;
            }
            // Two values is a flag, not a process; hundreds is unreadable.
            if (3..=60).contains(&column.distinct) {
                score += 20;
            }
        }
        Role::Resource => {
            if hints(&[
                "resource",
                "user",
                "assignee",
                "agent",
                "owner",
                "team",
                "handler",
                "performer",
                "operator",
            ]) {
                score += 60;
            }
        }
        Role::Timestamp if hints(&["start", "begin", "created", "event"]) => {
            score += 20;
        }
        _ => {}
    }
    Some(score)
}

impl Column {
    /// Share of sampled values that were blank, as a percentage.
    pub fn blank_percent(&self) -> f64 {
        let total = self.sampled + self.blank;
        if total == 0 {
            0.0
        } else {
            self.blank as f64 / total as f64 * 100.0
        }
    }
}

/// A profile of every column in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub columns: Vec<Column>,
    /// Rows inspected.
    pub rows_sampled: usize,
}

impl Profile {
    /// Profiles a sample of rows.
    pub fn build(schema: &Schema, sample: &[Row]) -> Profile {
        let rows = sample.len().min(PROFILE_ROWS);
        let columns = schema
            .headers()
            .iter()
            .enumerate()
            .map(|(index, name)| profile_column(name, index, &sample[..rows]))
            .collect();
        Profile {
            columns,
            rows_sampled: rows,
        }
    }

    /// Columns that could fill `role`, best fit first.
    pub fn candidates(&self, role: Role) -> Vec<&Column> {
        let mut scored: Vec<(i32, &Column)> = self
            .columns
            .iter()
            .filter_map(|c| fit(c, role).map(|score| (score, c)))
            .collect();
        // Best score first; ties broken by file order so the result is stable.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.index.cmp(&b.1.index)));
        scored.into_iter().map(|(_, c)| c).collect()
    }

    /// The best candidate for `role`, if any column suits it.
    pub fn best(&self, role: Role) -> Option<&Column> {
        self.candidates(role).into_iter().next()
    }

    /// Required fields with no candidate column.
    ///
    /// This is what turns "we could not map your file" into a specific,
    /// actionable message.
    pub fn unfillable_roles(&self) -> Vec<Role> {
        [Role::CaseId, Role::Activity, Role::Timestamp]
            .into_iter()
            .filter(|r| self.candidates(*r).is_empty())
            .collect()
    }
}

fn profile_column(name: &str, index: usize, sample: &[Row]) -> Column {
    let values: Vec<&str> = sample.iter().map(|r| r.at(index)).collect();
    let (blank, present): (Vec<&&str>, Vec<&&str>) =
        values.iter().partition(|v| v.trim().is_empty());

    let distinct: BTreeSet<&str> = present.iter().map(|v| **v).collect();
    let examples: Vec<String> = distinct
        .iter()
        .take(EXAMPLES)
        .map(|v| (*v).to_owned())
        .collect();

    let kind = infer_kind(&present, distinct.len());
    let suggests = suggest_roles(name, &kind, distinct.len(), present.len());

    Column {
        name: name.to_owned(),
        index,
        sampled: present.len(),
        blank: blank.len(),
        distinct: distinct.len(),
        examples,
        kind,
        suggests,
    }
}

fn infer_kind(present: &[&&str], distinct: usize) -> ColumnKind {
    if present.is_empty() {
        return ColumnKind::Empty;
    }

    let detection =
        timestamp::detect(&present.iter().map(|v| (**v).to_owned()).collect::<Vec<_>>());
    if !detection.candidates.is_empty() {
        return ColumnKind::Timestamp(detection.resolved().map(|f| f.id.to_string()));
    }
    if !detection.mixed.is_empty() && detection.unparseable.is_empty() {
        return ColumnKind::Timestamp(None);
    }

    if present.iter().all(|v| v.trim().parse::<i64>().is_ok()) {
        return ColumnKind::Integer;
    }
    if present.iter().all(|v| v.trim().parse::<f64>().is_ok()) {
        return ColumnKind::Decimal;
    }

    let ratio = distinct as f64 / present.len() as f64;
    if ratio >= IDENTIFIER_RATIO {
        ColumnKind::Identifier
    } else if distinct <= CATEGORY_LIMIT {
        ColumnKind::Category
    } else {
        ColumnKind::Text
    }
}

/// Suggests the fields a column could fill.
///
/// Shape decides what is *possible*; the column name only reorders what is
/// already plausible. A column named `activity` holding a hundred thousand
/// distinct values is still not an activity column.
fn suggest_roles(name: &str, kind: &ColumnKind, distinct: usize, present: usize) -> Vec<Role> {
    let lower = name.to_lowercase();
    let hints = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    let mut roles = Vec::new();

    match kind {
        ColumnKind::Timestamp(_) => {
            if hints(&["end", "finish", "complete", "resolved", "closed"]) {
                roles.push(Role::EndTimestamp);
                roles.push(Role::Timestamp);
            } else {
                roles.push(Role::Timestamp);
                roles.push(Role::EndTimestamp);
            }
        }
        ColumnKind::Integer => {
            if hints(&["seq", "order", "step", "index", "position"]) {
                roles.push(Role::EventOrder);
            }
            if hints(&["cost", "amount", "price", "value", "spend"]) {
                roles.push(Role::Cost);
            }
            // A numeric column with mostly-distinct values is plausibly a case
            // identifier, even without a helpful name.
            if distinct as f64 / present.max(1) as f64 >= IDENTIFIER_RATIO {
                roles.push(Role::CaseId);
            }
        }
        ColumnKind::Decimal => {
            roles.push(Role::Cost);
        }
        ColumnKind::Identifier => {
            roles.push(Role::CaseId);
        }
        ColumnKind::Category => {
            if hints(&["lifecycle", "transition", "event_type"]) {
                roles.push(Role::Lifecycle);
            }
            if hints(&[
                "resource",
                "user",
                "assignee",
                "agent",
                "owner",
                "team",
                "performer",
            ]) {
                roles.push(Role::Resource);
                roles.push(Role::Activity);
            } else {
                roles.push(Role::Activity);
                roles.push(Role::Resource);
            }
            // In a long-format event log the case identifier *repeats* — once
            // per event — so it profiles as a category rather than an
            // identifier. Excluding CaseId here would make the case column
            // unfindable in exactly the commonest shape.
            roles.push(Role::CaseId);

            // A single value distinguishes nothing, so it fills no role. It is
            // still carried through as a case attribute.
            if distinct <= 1 {
                roles.clear();
            }
        }
        ColumnKind::Text | ColumnKind::Empty => {}
    }

    roles.dedup();
    roles
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn table(headers: &[&str], data: &[&[&str]]) -> (Schema, Vec<Row>) {
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
        ((*schema).clone(), rows)
    }

    fn sample() -> (Schema, Vec<Row>) {
        table(
            &["order_id", "activity", "timestamp", "handler", "amount"],
            &[
                &["ORD-1", "Receive", "2026-03-02 09:00:00", "alice", "120.50"],
                &["ORD-2", "Receive", "2026-03-02 10:00:00", "alice", "80.00"],
                &["ORD-3", "Approve", "2026-03-02 11:00:00", "bob", "300.25"],
                &["ORD-4", "Approve", "2026-03-02 12:00:00", "bob", "45.75"],
            ],
        )
    }

    fn column<'a>(p: &'a Profile, name: &str) -> &'a Column {
        p.columns.iter().find(|c| c.name == name).expect("column")
    }

    #[test]
    fn identifies_column_kinds() {
        let (schema, rows) = sample();
        let p = Profile::build(&schema, &rows);
        assert!(matches!(
            column(&p, "timestamp").kind,
            ColumnKind::Timestamp(Some(_))
        ));
        assert_eq!(column(&p, "order_id").kind, ColumnKind::Identifier);
        assert_eq!(column(&p, "activity").kind, ColumnKind::Category);
        assert_eq!(column(&p, "amount").kind, ColumnKind::Decimal);
    }

    /// Real values, so the user can recognise their own data.
    #[test]
    fn shows_real_example_values() {
        let (schema, rows) = sample();
        let p = Profile::build(&schema, &rows);
        assert!(
            column(&p, "activity")
                .examples
                .contains(&"Receive".to_owned())
        );
        assert!(
            column(&p, "order_id")
                .examples
                .contains(&"ORD-1".to_owned())
        );
    }

    #[test]
    fn counts_distinct_and_blank_values() {
        let (schema, rows) = table(&["a"], &[&["x"], &["x"], &[""], &["y"]]);
        let p = Profile::build(&schema, &rows);
        let c = column(&p, "a");
        assert_eq!(c.sampled, 3);
        assert_eq!(c.blank, 1);
        assert_eq!(c.distinct, 2);
        assert_eq!(c.blank_percent(), 25.0);
    }

    #[test]
    fn suggests_the_obvious_roles() {
        let (schema, rows) = sample();
        let p = Profile::build(&schema, &rows);
        assert!(column(&p, "order_id").suggests.contains(&Role::CaseId));
        assert!(column(&p, "activity").suggests.contains(&Role::Activity));
        assert!(column(&p, "timestamp").suggests.contains(&Role::Timestamp));
        assert!(column(&p, "handler").suggests.contains(&Role::Resource));
        assert!(column(&p, "amount").suggests.contains(&Role::Cost));
    }

    /// A name hint reorders plausible roles; it does not create them. A column
    /// named `activity` full of unique values is still not an activity column.
    #[test]
    fn shape_beats_the_column_name() {
        let (schema, rows) = table(&["activity"], &[&["ID-1"], &["ID-2"], &["ID-3"], &["ID-4"]]);
        let p = Profile::build(&schema, &rows);
        let c = column(&p, "activity");
        assert_eq!(c.kind, ColumnKind::Identifier);
        assert!(!c.suggests.contains(&Role::Activity));
        assert!(c.suggests.contains(&Role::CaseId));
    }

    /// An end-timestamp name is offered as an end timestamp first, but is
    /// still usable as the main timestamp.
    #[test]
    fn prefers_an_end_timestamp_role_for_an_end_named_column() {
        let (schema, rows) = table(
            &["resolved_at"],
            &[&["2026-03-02 09:00:00"], &["2026-03-02 10:00:00"]],
        );
        let p = Profile::build(&schema, &rows);
        let c = column(&p, "resolved_at");
        assert_eq!(c.suggests[0], Role::EndTimestamp);
        assert!(c.suggests.contains(&Role::Timestamp));
    }

    #[test]
    fn recognises_an_order_column() {
        let (schema, rows) = table(&["seq"], &[&["1"], &["2"], &["3"]]);
        let p = Profile::build(&schema, &rows);
        assert!(column(&p, "seq").suggests.contains(&Role::EventOrder));
    }

    #[test]
    fn reports_an_entirely_blank_column() {
        let (schema, rows) = table(&["unused"], &[&[""], &[""]]);
        let p = Profile::build(&schema, &rows);
        let c = column(&p, "unused");
        assert_eq!(c.kind, ColumnKind::Empty);
        assert!(c.suggests.is_empty());
        assert_eq!(c.blank_percent(), 100.0);
    }

    /// A single-valued column cannot distinguish anything, so it is not
    /// offered as an activity or resource.
    #[test]
    fn does_not_suggest_a_constant_column() {
        let (schema, rows) = table(&["region"], &[&["EU"], &["EU"], &["EU"], &["EU"]]);
        let p = Profile::build(&schema, &rows);
        assert!(column(&p, "region").suggests.is_empty());
    }

    /// The useful failure message: which required field could not be filled.
    #[test]
    fn names_the_required_fields_it_cannot_fill() {
        let (schema, rows) = table(
            &["ticket_id", "created_at", "resolved_at"],
            &[&["T-1", "2026-03-01 09:00:00", "2026-03-02 09:00:00"]],
        );
        let p = Profile::build(&schema, &rows);
        assert_eq!(p.unfillable_roles(), vec![Role::Activity]);
    }

    #[test]
    fn a_mappable_file_has_no_unfillable_roles() {
        let (schema, rows) = sample();
        assert!(Profile::build(&schema, &rows).unfillable_roles().is_empty());
    }

    #[test]
    fn lists_candidates_for_a_role() {
        let (schema, rows) = sample();
        let p = Profile::build(&schema, &rows);
        let names: Vec<&str> = p
            .candidates(Role::Timestamp)
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["timestamp"]);
    }

    /// A column that is mostly blank cannot be a required field — a case id
    /// that is missing on most events is not a case id.
    #[test]
    fn a_mostly_blank_column_cannot_fill_a_required_field() {
        let (schema, rows) = table(
            &["case_id", "activity", "timestamp", "notes"],
            &[
                &["C1", "A", "2026-03-02 09:00:00", "urgent"],
                &["C1", "B", "2026-03-02 10:00:00", ""],
                &["C2", "A", "2026-03-02 11:00:00", ""],
                &["C2", "B", "2026-03-02 12:00:00", ""],
            ],
        );
        let p = Profile::build(&schema, &rows);
        let names: Vec<&str> = p
            .candidates(Role::CaseId)
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(!names.contains(&"notes"), "got {names:?}");
    }

    /// With many columns an unranked list is useless, so the best fit must
    /// come first.
    #[test]
    fn ranks_the_right_column_first() {
        let (schema, rows) = table(
            &["region", "order_id", "status", "event_time", "handler"],
            &[
                &["EMEA", "ORD-1", "Receive", "2026-03-02 09:00:00", "intake"],
                &["EMEA", "ORD-1", "Approve", "2026-03-02 11:00:00", "credit"],
                &["AMER", "ORD-2", "Receive", "2026-03-02 10:00:00", "intake"],
                &["AMER", "ORD-2", "Approve", "2026-03-02 12:00:00", "credit"],
            ],
        );
        let p = Profile::build(&schema, &rows);
        assert_eq!(
            p.best(Role::CaseId).map(|c| c.name.as_str()),
            Some("order_id")
        );
        assert_eq!(
            p.best(Role::Activity).map(|c| c.name.as_str()),
            Some("status")
        );
        assert_eq!(
            p.best(Role::Timestamp).map(|c| c.name.as_str()),
            Some("event_time")
        );
        assert_eq!(
            p.best(Role::Resource).map(|c| c.name.as_str()),
            Some("handler")
        );
    }

    /// Ranking must not depend on where a column sits in the file.
    #[test]
    fn ranking_is_independent_of_column_order() {
        let forward = table(
            &["order_id", "status", "event_time"],
            &[&["ORD-1", "Receive", "2026-03-02 09:00:00"]],
        );
        let reversed = table(
            &["event_time", "status", "order_id"],
            &[&["2026-03-02 09:00:00", "Receive", "ORD-1"]],
        );
        let a = Profile::build(&forward.0, &forward.1);
        let b = Profile::build(&reversed.0, &reversed.1);
        assert_eq!(
            a.best(Role::CaseId).map(|c| c.name.as_str()),
            b.best(Role::CaseId).map(|c| c.name.as_str())
        );
    }

    #[test]
    fn is_deterministic() {
        let (schema, rows) = sample();
        assert_eq!(
            Profile::build(&schema, &rows),
            Profile::build(&schema, &rows)
        );
    }

    #[test]
    fn handles_a_file_with_no_rows() {
        let (schema, rows) = table(&["a", "b"], &[]);
        let p = Profile::build(&schema, &rows);
        assert_eq!(p.rows_sampled, 0);
        assert_eq!(p.columns.len(), 2);
        assert!(p.columns.iter().all(|c| c.kind == ColumnKind::Empty));
    }
}
