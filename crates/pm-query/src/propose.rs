//! Proposing a field mapping from column names.
//!
//! Standard 1 says the three-column upload must be excellent, and every
//! question that could have been inferred is a defect. So this guesses, and
//! guesses well — but the guess is always shown for confirmation rather than
//! applied silently.

use pm_ingest::Mapping;

/// Column names that commonly hold each required field, best first.
const CASE_NAMES: &[&str] = &[
    "case_id",
    "caseid",
    "case",
    "case id",
    "process_instance",
    "instance_id",
    "ticket_id",
    "order_id",
    "incident_id",
    "claim_id",
    "request_id",
    "id",
];

const ACTIVITY_NAMES: &[&str] = &[
    "activity",
    "activity_name",
    "event",
    "event_name",
    "action",
    "task",
    "step",
    "status",
    "operation",
    "activity name",
];

const TIMESTAMP_NAMES: &[&str] = &[
    "timestamp",
    "time",
    "datetime",
    "date_time",
    "event_time",
    "eventtime",
    "occurred_at",
    "created_at",
    "date",
    "start_time",
    "time_stamp",
];

/// Proposes a mapping from the available column names.
///
/// Returns `None` for a field whose column cannot be guessed, so the caller
/// can ask about exactly that one rather than all three.
pub fn propose_mapping(headers: &[String]) -> Option<Mapping> {
    let mut mapping = Mapping::new(
        &best_match(headers, CASE_NAMES)?,
        &best_match(headers, ACTIVITY_NAMES)?,
        &best_match(headers, TIMESTAMP_NAMES)?,
    );

    // Optional columns are proposed too, since each unlocks an analysis the
    // user would otherwise not know was available.
    if let Some(c) = best_match(headers, RESOURCE_NAMES) {
        mapping = mapping.with_resource(&c);
    }
    if let Some(c) = best_match(headers, ORDER_NAMES) {
        mapping = mapping.with_event_order(&c);
    }
    if let Some(c) = best_match(headers, LIFECYCLE_NAMES) {
        mapping = mapping.with_lifecycle(&c);
    }
    // "Spend area text" or "Cost centre" name a kind of spend, not an amount.
    let amounts = headers
        .iter()
        .filter(|h| {
            let lower = h.to_lowercase();
            !NOT_AMOUNT_WORDS.iter().any(|w| lower.contains(w))
        })
        .cloned()
        .collect::<Vec<_>>();
    if let Some(c) = best_match(&amounts, COST_NAMES) {
        mapping = mapping.with_cost(&c);
    }
    if let Some(c) = best_match(headers, END_TIMESTAMP_NAMES) {
        // Only if it is not already the main timestamp.
        if c != mapping.timestamp {
            mapping = mapping.with_end_timestamp(&c);
        }
    }

    // Everything left over becomes a case attribute. A real export has many
    // more columns than the three required ones, and discarding them would
    // throw away exactly what cohort comparison and filtering need.
    let assigned: Vec<&str> = mapping.mapped_columns();
    let attributes: Vec<String> = headers
        .iter()
        .filter(|h| !assigned.contains(&h.as_str()))
        .cloned()
        .collect();
    Some(mapping.with_case_attributes(attributes))
}

const RESOURCE_NAMES: &[&str] = &[
    "resource",
    "user",
    "performer",
    "assignee",
    "agent",
    "operator",
    "handler",
    "handled_by",
    "owner",
    "team",
];

/// Names that mean the cost of a step. Amounts, prices and values usually
/// belong to the case (an invoice's amount repeated on each of its rows), so
/// they stay case attributes and are counted once per case, not once per step.
const COST_NAMES: &[&str] = &["cost", "spend", "charge"];

/// Words that make a cost-like name a label rather than an amount.
const NOT_AMOUNT_WORDS: &[&str] = &[
    "text", "area", "class", "categor", "type", "name", "centre", "center", "code", "desc",
];

const END_TIMESTAMP_NAMES: &[&str] = &[
    "end_time",
    "end_timestamp",
    "finished_at",
    "completed_at",
    "complete_time",
    "end_date",
];

const ORDER_NAMES: &[&str] = &[
    "event_order",
    "sequence",
    "seq",
    "step_number",
    "order_index",
];

const LIFECYCLE_NAMES: &[&str] = &[
    "lifecycle",
    "lifecycle_transition",
    "event_type",
    "transition",
];

/// Finds the header best matching `candidates`.
///
/// Exact matches win over partial ones, and earlier candidates win over later,
/// so `case_id` beats `id` when both exist.
fn best_match(headers: &[String], candidates: &[&str]) -> Option<String> {
    let normalized: Vec<(usize, String)> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| (i, h.trim().to_lowercase().replace([' ', '-'], "_")))
        .collect();

    for candidate in candidates {
        let want = candidate.replace([' ', '-'], "_");
        if let Some((i, _)) = normalized.iter().find(|(_, h)| *h == want) {
            return Some(headers[*i].clone());
        }
    }
    for candidate in candidates {
        let want = candidate.replace([' ', '-'], "_");
        if let Some((i, _)) = normalized.iter().find(|(_, h)| h.contains(&want)) {
            return Some(headers[*i].clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn proposes_the_obvious_mapping() {
        let m = propose_mapping(&headers(&["case_id", "activity", "timestamp"])).unwrap();
        assert_eq!(m.case_id, "case_id");
        assert_eq!(m.activity, "activity");
        assert_eq!(m.timestamp, "timestamp");
    }

    #[test]
    fn handles_capitalisation_and_spacing() {
        let m = propose_mapping(&headers(&["Case ID", "Activity Name", "Time Stamp"])).unwrap();
        assert_eq!(m.case_id, "Case ID");
        assert_eq!(m.activity, "Activity Name");
        assert_eq!(m.timestamp, "Time Stamp");
    }

    #[test]
    fn recognises_domain_specific_identifiers() {
        let m = propose_mapping(&headers(&["ticket_id", "status", "created_at"])).unwrap();
        assert_eq!(m.case_id, "ticket_id");
        assert_eq!(m.activity, "status");
        assert_eq!(m.timestamp, "created_at");
    }

    /// A specific name must beat a generic one, or a table with both `id` and
    /// `order_id` maps the wrong column.
    #[test]
    fn prefers_a_specific_identifier_over_a_generic_one() {
        let m = propose_mapping(&headers(&["id", "order_id", "activity", "timestamp"])).unwrap();
        assert_eq!(m.case_id, "order_id");
    }

    #[test]
    fn prefers_an_exact_match_over_a_partial_one() {
        let m = propose_mapping(&headers(&[
            "related_case_id",
            "case_id",
            "activity",
            "timestamp",
        ]))
        .unwrap();
        assert_eq!(m.case_id, "case_id");
    }

    #[test]
    fn falls_back_to_a_partial_match() {
        let m = propose_mapping(&headers(&[
            "incident_case_id",
            "activity_name",
            "event_time",
        ]))
        .unwrap();
        assert_eq!(m.case_id, "incident_case_id");
    }

    /// Wide-format tables have no activity column, so no mapping is possible
    /// and the caller must run the unpivot step instead.
    #[test]
    fn declines_when_no_activity_column_exists() {
        assert!(propose_mapping(&headers(&["ticket_id", "created_at", "resolved_at"])).is_none());
    }

    #[test]
    fn declines_on_unrecognisable_headers() {
        assert!(propose_mapping(&headers(&["col_a", "col_b", "col_c"])).is_none());
    }

    #[test]
    fn proposes_optional_columns_too() {
        let m = propose_mapping(&headers(&[
            "case_id",
            "activity",
            "timestamp",
            "resource",
            "sequence",
        ]))
        .unwrap();
        assert_eq!(m.resource.as_deref(), Some("resource"));
        assert_eq!(m.event_order.as_deref(), Some("sequence"));
    }

    #[test]
    fn leaves_optional_columns_unset_when_absent() {
        let m = propose_mapping(&headers(&["case_id", "activity", "timestamp"])).unwrap();
        assert!(m.resource.is_none());
        assert!(m.event_order.is_none());
        assert!(m.lifecycle.is_none());
    }

    /// `order_id` is a case identifier, not an event-order column. Matching it
    /// as one would silently reorder every case.
    #[test]
    fn does_not_mistake_an_order_id_for_an_event_order() {
        let m = propose_mapping(&headers(&["order_id", "activity", "timestamp"])).unwrap();
        assert_eq!(m.case_id, "order_id");
        assert!(m.event_order.is_none());
    }

    /// Extra columns must survive as case attributes, not be discarded.
    #[test]
    fn carries_unmapped_columns_through_as_case_attributes() {
        let m = propose_mapping(&headers(&[
            "case_id",
            "activity",
            "timestamp",
            "region",
            "customer",
            "product",
        ]))
        .unwrap();
        assert_eq!(m.case_attributes, vec!["region", "customer", "product"]);
    }

    #[test]
    fn a_mapped_column_is_not_also_an_attribute() {
        let m =
            propose_mapping(&headers(&["case_id", "activity", "timestamp", "resource"])).unwrap();
        assert_eq!(m.resource.as_deref(), Some("resource"));
        assert!(m.case_attributes.is_empty());
    }

    #[test]
    fn a_kind_of_spend_is_not_a_cost() {
        let headers = [
            "case_id",
            "activity",
            "timestamp",
            "Spend area text",
            "Cost center",
        ]
        .map(String::from);
        assert_eq!(propose_mapping(&headers).unwrap().cost, None);
    }

    #[test]
    fn proposes_resource_and_cost_from_common_names() {
        let m = propose_mapping(&headers(&[
            "order_id",
            "activity",
            "event_time",
            "handler",
            "step_cost",
        ]))
        .unwrap();
        assert_eq!(m.resource.as_deref(), Some("handler"));
        assert_eq!(m.cost.as_deref(), Some("step_cost"));
        assert!(m.case_attributes.is_empty());
    }

    /// An invoice amount repeats on every row of its case; read as a step
    /// cost it would be counted once per step.
    #[test]
    fn amounts_stay_case_attributes() {
        let m = propose_mapping(&headers(&["case_id", "activity", "timestamp", "amount"])).unwrap();
        assert_eq!(m.cost, None);
        assert_eq!(m.case_attributes, vec!["amount"]);
    }

    /// The end-timestamp guess must never duplicate the main timestamp.
    #[test]
    fn does_not_map_one_column_as_both_timestamps() {
        let m = propose_mapping(&headers(&["case_id", "activity", "end_time"])).unwrap();
        assert_ne!(m.end_timestamp.as_deref(), Some(m.timestamp.as_str()));
    }

    #[test]
    fn is_deterministic() {
        let h = headers(&["case_id", "activity", "timestamp"]);
        assert_eq!(propose_mapping(&h), propose_mapping(&h));
    }
}
