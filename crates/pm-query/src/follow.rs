//! Following one item type through a connected log.
//!
//! A connected (object-centric) log has no single case notion. To answer the
//! questions asked of any process (how long, which route, where it is slow,
//! which items stand out) for, say, orders, each order becomes a case: every
//! recorded step that involves it, in time order. The result is an ordinary
//! case log, so the same engine and every case view apply with the same
//! meaning they have for a single log.
//!
//! Following is a projection and says so. A step shared by several items of
//! the followed type appears once in each of their cases; the counts that
//! disclose this travel with the result.

use crate::ocel::OcelQueryError;
use pm_core::{AttributeValue, OcelFilter, OcelLog, OcelView, project_ocel};
use pm_ingest::validate::Mapping;
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

pub const OBJECT_HISTORY_SEMANTICS: &str = "Each object is a history of its recorded steps and may span separate business transactions. First-to-last spans are not necessarily business cycle times; repeated activity labels are not proof of rework. Shared steps appear in each participating object history. Use transaction-bound linked measurements for business hand-offs.";

/// Columns of the written case log.
pub const FOLLOW_CASE: &str = "case_id";
pub const FOLLOW_ACTIVITY: &str = "activity";
pub const FOLLOW_TIMESTAMP: &str = "timestamp";
/// Each step's position within its item. Two different recorded steps can
/// share an activity and a time (two payments on one day); the position keeps
/// them apart, so they are neither mistaken for duplicate rows nor left in an
/// undetermined order.
pub const FOLLOW_ORDER: &str = "step_number";

/// What following one item type produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FollowSummary {
    pub object_type: String,
    /// Items of the followed type with at least one recorded step.
    pub cases: u64,
    /// Steps written, counting a shared step once per item it involves.
    pub events: u64,
    /// Steps in the view that involve no item of the followed type.
    pub excluded_events: u64,
    /// Steps involving several items of the followed type.
    pub shared_events: u64,
    /// Extra copies those shared steps add to the case log.
    pub shared_event_copies: u64,
    /// Item details written as case attributes, as of each item's last step.
    pub case_attributes: Vec<String>,
}

impl FollowSummary {
    /// The mapping under which the written log reads as a case log.
    pub fn mapping(&self) -> Mapping {
        let mut mapping = Mapping::new(FOLLOW_CASE, FOLLOW_ACTIVITY, FOLLOW_TIMESTAMP)
            .with_event_order(FOLLOW_ORDER);
        mapping.case_attributes = self.case_attributes.clone();
        mapping.object_history = true;
        mapping
    }
}

/// Writes the case log of `object_type` for an OCEL file to `out` as CSV.
pub fn follow_ocel(
    path: impl AsRef<Path>,
    filter: &OcelFilter,
    object_type: &str,
    out: impl Write,
) -> Result<FollowSummary, OcelQueryError> {
    let path = path.as_ref();
    if from_disk(path) {
        return follow_ocel_disk_view(path, filter, object_type, out);
    }
    let log = pm_ingest::read_ocel(path)?;
    follow_ocel_log(&log, filter, object_type, out)
}

/// A saved connected log at least this large is followed from disk.
pub const DISK_FOLLOW_BYTES: u64 = 256 * 1024 * 1024;

/// The engine's choice for every question about a saved connected log: a
/// SQLite log of at least [`DISK_FOLLOW_BYTES`] is read in pieces from disk.
pub(crate) fn from_disk(path: &Path) -> bool {
    let sqlite = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("sqlite") || e.eq_ignore_ascii_case("sqlite3"));
    sqlite
        && (pm_ingest::force_disk()
            || std::fs::metadata(path).is_ok_and(|m| m.len() >= DISK_FOLLOW_BYTES))
}

/// Follows an item type in the kept disk copy of a large saved log, for any
/// filter; memory stays small however large the log.
pub fn follow_ocel_disk_view(
    path: &Path,
    filter: &OcelFilter,
    object_type: &str,
    out: impl Write,
) -> Result<FollowSummary, OcelQueryError> {
    let log = crate::ocel::open_disk(path)?;
    let type_id = log
        .object_types
        .iter()
        .position(|candidate| candidate == object_type)
        .map(|index| pm_core::ObjectTypeId(index as u32))
        .ok_or_else(|| OcelQueryError::UnknownObjectType(object_type.to_owned()))?;
    let view = log.view(filter)?;
    let written = |error: csv::Error| pm_ingest::OcelSqliteError::Data(error.to_string());
    let writer = std::cell::RefCell::new(csv::Writer::from_writer(out));
    let fixed = [FOLLOW_CASE, FOLLOW_ACTIVITY, FOLLOW_TIMESTAMP, FOLLOW_ORDER];
    let followed = log
        .follow(
            &view,
            type_id,
            &fixed,
            |details| {
                writer
                    .borrow_mut()
                    .write_record(
                        fixed
                            .iter()
                            .map(|f| (*f).to_owned())
                            .chain(details.iter().cloned()),
                    )
                    .map_err(written)
            },
            |step| {
                let mut writer = writer.borrow_mut();
                writer.write_field(step.case).map_err(written)?;
                writer.write_field(step.activity).map_err(written)?;
                writer.write_field(timestamp(step.at)).map_err(written)?;
                writer
                    .write_field(step.position.to_string())
                    .map_err(written)?;
                for detail in step.details {
                    writer.write_field(detail).map_err(written)?;
                }
                writer.write_record(None::<&[u8]>).map_err(written)
            },
        )
        .map_err(crate::ocel::disk_failed)?;
    writer
        .into_inner()
        .flush()
        .map_err(OcelQueryError::ModelIo)?;
    Ok(FollowSummary {
        object_type: object_type.to_owned(),
        cases: followed.cases,
        events: followed.events,
        excluded_events: followed.excluded_events,
        shared_events: followed.shared_events,
        shared_event_copies: followed.shared_event_copies,
        case_attributes: followed.attributes,
    })
}

/// Writes the case log of `object_type` for an already-read log.
pub fn follow_ocel_log(
    log: &OcelLog,
    filter: &OcelFilter,
    object_type: &str,
    out: impl Write,
) -> Result<FollowSummary, OcelQueryError> {
    let type_id = log
        .object_types
        .iter()
        .position(|candidate| candidate.name == object_type)
        .map(|index| pm_core::ObjectTypeId(index as u32))
        .ok_or_else(|| OcelQueryError::UnknownObjectType(object_type.to_owned()))?;
    let view = OcelView::apply(log, filter)?;
    let projection = project_ocel(log, &view, type_id);

    // Item details become case attributes. Names that would collide with the
    // three fixed columns, or with each other, are left out rather than renamed,
    // and so is the source lineage connected preparation records on every
    // item (`_source_input`, `_source_row`): it is evidence, not a detail.
    let fixed = [FOLLOW_CASE, FOLLOW_ACTIVITY, FOLLOW_TIMESTAMP, FOLLOW_ORDER];
    let mut seen = BTreeSet::new();
    let attributes = log.object_types[type_id.0 as usize]
        .attributes
        .iter()
        .filter_map(|definition| {
            let name = log.attribute_names.label(definition.name.0)?;
            (!fixed.contains(&name)
                && !name.starts_with("_source_")
                && seen.insert(name.to_owned()))
            .then(|| (definition.name, name.to_owned()))
        })
        .collect::<Vec<_>>();

    let mut writer = csv::Writer::from_writer(out);
    let header = fixed
        .iter()
        .map(|name| (*name).to_owned())
        .chain(attributes.iter().map(|(_, name)| name.clone()))
        .collect::<Vec<_>>();
    writer.write_record(&header).map_err(io)?;
    let labels = readable_labels(log, &projection.cases);
    // Each item's recorded details, gathered in one pass: looking them up per
    // item across the whole log would take time quadratic in its size.
    let mut details_of = std::collections::HashMap::<pm_core::ObjectId, Vec<usize>>::new();
    for (index, attribute) in log.object_attributes.iter().enumerate() {
        if log.objects[attribute.object.0 as usize].object_type == type_id {
            details_of.entry(attribute.object).or_default().push(index);
        }
    }
    let mut events = 0u64;
    let mut row = Vec::with_capacity(header.len());
    for (case, label) in projection.cases.iter().zip(&labels) {
        let last = case.layers.last().map_or(0, |layer| layer.at);
        let recorded = details_of
            .get(&case.case_object)
            .map_or(&[][..], Vec::as_slice);
        // As of the item's last step; on equal times the later record wins,
        // as in `OcelLog::object_attribute_at`.
        let details = attributes
            .iter()
            .map(|(name, _)| {
                recorded
                    .iter()
                    .map(|index| (*index, &log.object_attributes[*index]))
                    .filter(|(_, attribute)| attribute.name == *name && attribute.at <= last)
                    .max_by_key(|(index, attribute)| (attribute.at, *index))
                    .map(|(_, attribute)| text(log, attribute.value))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        let mut position = 0u64;
        for layer in &case.layers {
            let at = timestamp(layer.at);
            for event in &layer.events {
                position += 1;
                row.clear();
                row.push(label.to_owned());
                row.push(
                    log.event_type(event.event_type)
                        .map(|declared| declared.name.clone())
                        .unwrap_or_default(),
                );
                row.push(at.clone());
                row.push(position.to_string());
                row.extend(details.iter().cloned());
                writer.write_record(&row).map_err(io)?;
                events += 1;
            }
        }
    }
    writer.flush().map_err(OcelQueryError::ModelIo)?;
    Ok(FollowSummary {
        object_type: object_type.to_owned(),
        cases: projection.projected_cases,
        events,
        excluded_events: projection.excluded_events,
        shared_events: projection.convergence_events,
        shared_event_copies: projection.convergence_additional_occurrences,
        case_attributes: attributes.into_iter().map(|(_, name)| name).collect(),
    })
}

/// Case labels people can read. Connected preparation names objects with
/// exact JSON identity tuples such as `["erp:a","order",["A","0007"]]`; the
/// key values ("A 0007") identify the item within its type. The full identity
/// is kept whenever the short labels would not be unique.
fn readable_labels(log: &OcelLog, cases: &[pm_core::OcelProjectedCase]) -> Vec<String> {
    let full = cases
        .iter()
        .map(|case| {
            log.object_label(case.case_object)
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<Vec<_>>();
    let short = full
        .iter()
        .map(|id| {
            serde_json::from_str::<(String, String, Vec<String>)>(id)
                .ok()
                .map(|(_, _, keys)| keys.join(" "))
        })
        .collect::<Option<Vec<_>>>();
    match short {
        Some(short) if short.iter().collect::<BTreeSet<_>>().len() == short.len() => short,
        _ => full,
    }
}

fn io(error: csv::Error) -> OcelQueryError {
    OcelQueryError::ModelIo(std::io::Error::other(error))
}

fn timestamp(micros: i64) -> String {
    chrono::DateTime::from_timestamp_micros(micros)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
        .unwrap_or_default()
}

fn text(log: &OcelLog, value: AttributeValue) -> String {
    match value {
        AttributeValue::String(id) => log
            .attribute_values
            .label(id.0)
            .unwrap_or_default()
            .to_owned(),
        AttributeValue::Time(at) => timestamp(at),
        AttributeValue::Integer(number) => number.to_string(),
        AttributeValue::Float(number) => number.to_string(),
        AttributeValue::Boolean(flag) => flag.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Outcome, analyze_filtered};
    use pm_ingest::finding::Policy;

    const FIXTURE: &[u8] = include_bytes!("../../../testdata/ocel/order-management.jsonocel");

    fn followed(object_type: &str) -> (String, FollowSummary) {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        let mut out = Vec::new();
        let summary = follow_ocel_log(&log, &OcelFilter::default(), object_type, &mut out).unwrap();
        (String::from_utf8(out).unwrap(), summary)
    }

    #[test]
    fn each_item_becomes_a_case_of_its_own_steps_in_time_order() {
        let (csv, summary) = followed("item");
        let lines = csv.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "case_id,activity,timestamp,step_number,price");
        // Two items share the place-order and packing steps: each gets a copy.
        assert_eq!(summary.cases, 2);
        assert_eq!(summary.events, 4);
        assert_eq!(summary.shared_events, 2);
        assert_eq!(summary.shared_event_copies, 2);
        assert!(lines[1].starts_with("item-1,place order,1970-01-01T00:00:00Z,1,10.5"));
        assert_eq!(summary.case_attributes, ["price"]);
        assert!(summary.mapping().object_history);
    }

    #[test]
    fn the_written_log_runs_through_the_normal_case_engine() {
        let (csv, summary) = followed("order");
        let dir = std::env::temp_dir().join(format!("pm-follow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("orders.csv");
        std::fs::write(&path, csv).unwrap();
        let outcome = analyze_filtered(
            &path,
            Some(summary.mapping()),
            &Policy::new(),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let Outcome::Complete(analysis) = outcome else {
            panic!("blocked")
        };
        assert_eq!(analysis.summary.cases as u64, summary.cases);
        assert_eq!(analysis.summary.events as u64, summary.events);
        assert!(analysis.case_attributes.contains(&"status".to_owned()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepared_identity_tuples_read_as_their_keys_when_unique() {
        let mut builder = pm_core::OcelBuilder::new();
        builder.declare_object_type("order", Vec::new()).unwrap();
        builder.declare_event_type("place", Vec::new()).unwrap();
        for (id, at) in [
            (r#"["erp:a","order",["A","0007"]]"#, 1),
            (r#"["erp:b","order",["A","0007"]]"#, 2),
        ] {
            builder.add_object(id, "order").unwrap();
            builder.add_event(&format!("e{at}"), "place", at).unwrap();
            builder
                .relate_event_object(&format!("e{at}"), id, "placed")
                .unwrap();
        }
        let log = builder.build();
        let mut out = Vec::new();
        follow_ocel_log(&log, &OcelFilter::default(), "order", &mut out).unwrap();
        // Two namespaces share the key, so the full identities are kept.
        assert!(String::from_utf8(out).unwrap().contains("erp:b"));
    }

    #[test]
    fn prepared_identity_tuples_read_as_their_keys() {
        let mut builder = pm_core::OcelBuilder::new();
        builder.declare_object_type("order", Vec::new()).unwrap();
        builder.declare_event_type("place", Vec::new()).unwrap();
        for (id, at) in [
            (r#"["erp:a","order",["A","0007"]]"#, 1),
            (r#"["erp:a","order",["B","0007"]]"#, 2),
        ] {
            builder.add_object(id, "order").unwrap();
            builder.add_event(&format!("e{at}"), "place", at).unwrap();
            builder
                .relate_event_object(&format!("e{at}"), id, "placed")
                .unwrap();
        }
        let log = builder.build();
        let mut out = Vec::new();
        follow_ocel_log(&log, &OcelFilter::default(), "order", &mut out).unwrap();
        let csv = String::from_utf8(out).unwrap();
        assert!(
            csv.contains("\nA 0007,place,") && csv.contains("\nB 0007,place,"),
            "{csv}"
        );
    }

    #[test]
    fn source_lineage_is_not_an_item_detail() {
        let mut builder = pm_core::OcelBuilder::new();
        let row = builder.attribute_name("_source_row");
        let amount = builder.attribute_name("amount");
        let declare = |name| pm_core::AttributeDefinition {
            name,
            value_type: pm_core::AttributeType::Integer,
        };
        builder
            .declare_object_type("order", vec![declare(row), declare(amount)])
            .unwrap();
        builder.declare_event_type("place", Vec::new()).unwrap();
        builder.add_object("o1", "order").unwrap();
        for (name, value) in [(row, 7), (amount, 500)] {
            builder
                .add_object_attribute("o1", name, 0, AttributeValue::Integer(value))
                .unwrap();
        }
        builder.add_event("e1", "place", 0).unwrap();
        builder.relate_event_object("e1", "o1", "o").unwrap();
        let mut out = Vec::new();
        let summary =
            follow_ocel_log(&builder.build(), &OcelFilter::default(), "order", &mut out).unwrap();
        assert_eq!(summary.case_attributes, ["amount"]);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("case_id,activity,timestamp,step_number,amount\no1,place,")
        );
    }

    #[test]
    fn an_unknown_item_type_is_an_error() {
        let log = pm_ingest::parse_ocel_json(FIXTURE).unwrap();
        assert!(matches!(
            follow_ocel_log(&log, &OcelFilter::default(), "nope", Vec::new()),
            Err(OcelQueryError::UnknownObjectType(_))
        ));
    }

    /// Two different payments on the same day are two steps, not a
    /// duplicate row, and the projected log needs no decision to analyze.
    #[test]
    fn distinct_steps_at_the_same_time_are_not_duplicates() {
        let mut builder = pm_core::OcelBuilder::new();
        builder.declare_object_type("order", Vec::new()).unwrap();
        builder.declare_event_type("pay", Vec::new()).unwrap();
        builder.add_object("o1", "order").unwrap();
        for id in ["p1", "p2"] {
            builder.add_event(id, "pay", 0).unwrap();
            builder.relate_event_object(id, "o1", "o").unwrap();
        }
        let mut out = Vec::new();
        let summary =
            follow_ocel_log(&builder.build(), &OcelFilter::default(), "order", &mut out).unwrap();
        let dir = std::env::temp_dir().join(format!("pm-follow-same-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("o.csv");
        std::fs::write(&path, out).unwrap();
        let outcome = analyze_filtered(
            &path,
            Some(summary.mapping()),
            &Policy::new(),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let Outcome::Complete(analysis) = outcome else {
            panic!("two payments on one day should not need a decision")
        };
        assert_eq!(analysis.summary.events, 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Following from disk writes exactly what following the loaded log
    /// writes: same rows in the same order, same details, same counts.
    fn same_from_disk(path: &Path) {
        let log = pm_ingest::read_ocel(path).unwrap();
        for object_type in log.object_types.iter().map(|t| t.name.clone()) {
            let mut loaded = Vec::new();
            let expected =
                follow_ocel_log(&log, &OcelFilter::default(), &object_type, &mut loaded).unwrap();
            let mut streamed = Vec::new();
            let summary =
                follow_ocel_disk_view(path, &OcelFilter::default(), &object_type, &mut streamed)
                    .unwrap();
            assert_eq!(summary, expected, "{object_type}");
            assert_eq!(
                String::from_utf8(streamed).unwrap(),
                String::from_utf8(loaded).unwrap(),
                "{object_type}"
            );
        }
    }

    /// Decisions to check against both readings: a rule between the first
    /// two activities and the first performer confirmed as a system.
    fn improvement_decisions(log: &pm_core::OcelLog) -> pm_project::improvement::Improvement {
        use pm_project::improvement::{Said, SavedPerformer, SavedRule, Status};
        let said = Said {
            status: Status::Confirmed,
            by: "test".into(),
            at: String::new(),
            reason: None,
        };
        let mut decisions = pm_project::improvement::Improvement::default();
        if let [a, b, ..] = log.event_types.as_slice() {
            decisions.rules.push(SavedRule {
                template: pm_core::declare::Template::Response,
                a: a.name.clone(),
                b: Some(b.name.clone()),
                said: said.clone(),
            });
        }
        if let Some(first) = crate::improvement::ocel_performers(log).0.first() {
            decisions.performers.insert(
                first.clone(),
                SavedPerformer {
                    kind: pm_core::ResourceKind::System,
                    said,
                },
            );
        }
        decisions
    }

    /// The overview read in pieces from disk is the overview of the loaded
    /// log, for the whole log and for each kind of filter.
    fn overview_same_from_disk(path: &Path) {
        let log = pm_ingest::read_ocel_sqlite_skeleton(path).unwrap();
        let source = path.display().to_string();
        let mut filters = vec![OcelFilter::default()];
        for object_type in log.object_types.iter().take(2) {
            filters.push(OcelFilter {
                object_types: vec![object_type.name.clone()],
                ..Default::default()
            });
        }
        if let Some(qualifier) = log.event_object_relations.first() {
            filters.push(OcelFilter {
                event_object_qualifiers: vec![
                    log.qualifier_label(qualifier.qualifier).unwrap().into(),
                ],
                ..Default::default()
            });
        }
        if let Some(relation) = log.object_object_relations.first() {
            filters.push(OcelFilter {
                object_object_qualifiers: vec![
                    log.qualifier_label(relation.qualifier).unwrap().into(),
                ],
                ..Default::default()
            });
        }
        filters.push(OcelFilter {
            object_types: vec!["no such type".into()],
            ..Default::default()
        });
        fn same<T: PartialEq + std::fmt::Debug>(
            what: &str,
            loaded: Result<T, OcelQueryError>,
            disk: Result<T, OcelQueryError>,
        ) {
            match (loaded, disk) {
                (Ok(loaded), Ok(disk)) => assert_eq!(disk, loaded, "{what}"),
                (Err(loaded), Err(disk)) => {
                    assert_eq!(disk.to_string(), loaded.to_string(), "{what}")
                }
                (loaded, disk) => panic!("{what}: {loaded:?} vs {disk:?}"),
            }
        }
        let full = pm_ingest::read_ocel(path).unwrap();
        let activities = full
            .event_types
            .iter()
            .map(|t| t.name.clone())
            .collect::<Vec<_>>();
        let mut objects = full
            .object_ids
            .iter()
            .take(3)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        objects.extend(full.object_ids.iter().last().map(str::to_owned));
        objects.push("no such item".into());
        // Baseline models: from the whole log, and from each single item type,
        // so other types' steps deviate.
        let dir = std::env::temp_dir().join(format!(
            "pm-disk-models-{}-{}",
            std::process::id(),
            path.file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut models = Vec::new();
        for (index, base) in std::iter::once(OcelFilter::default())
            .chain(full.object_types.iter().map(|t| OcelFilter {
                object_types: vec![t.name.clone()],
                ..Default::default()
            }))
            .enumerate()
        {
            if let Ok(model) = crate::discover_ocel_model_log("m", &full, &base, Default::default())
            {
                let file = dir.join(format!("model-{index}.json"));
                std::fs::write(&file, serde_json::to_vec(&model).unwrap()).unwrap();
                models.push(file);
            }
        }
        assert_eq!(
            crate::ocel::names(path, true).unwrap(),
            crate::ocel::names(path, false).unwrap()
        );
        for object in &objects {
            let wanted = |candidate: &str| candidate == object;
            assert_eq!(
                crate::ocel::find_object(path, true, wanted).unwrap(),
                crate::ocel::find_object(path, false, wanted).unwrap()
            );
        }
        for filter in filters {
            let what = format!("{filter:?}");
            for model in &models {
                for limit in [0, 2, 1_000] {
                    same(
                        &format!("conformance {} {limit} {what}", model.display()),
                        crate::check_ocel_conformance_log(
                            &source,
                            &full,
                            &filter,
                            &model.display().to_string(),
                            &serde_json::from_slice(&std::fs::read(model).unwrap()).unwrap(),
                            limit,
                        ),
                        crate::check_ocel_conformance_disk(path, model, &filter, limit),
                    );
                    same(
                        &format!("alignment {} {limit} {what}", model.display()),
                        crate::align_ocel_conformance_log(
                            &source,
                            &full,
                            &filter,
                            &model.display().to_string(),
                            &serde_json::from_slice(&std::fs::read(model).unwrap()).unwrap(),
                            limit,
                        ),
                        crate::align_ocel_conformance_disk(path, model, &filter, limit),
                    );
                }
            }
            same(
                &format!("overview {what}"),
                crate::analyze_ocel_log(&source, &log, &filter),
                crate::analyze_ocel_disk(path, &filter),
            );
            same(
                &format!("synchronization {what}"),
                crate::analyze_ocel_synchronization_log(&source, &full, &filter),
                crate::analyze_ocel_synchronization_disk(path, &filter),
            );
            same(
                &format!("bottlenecks {what}"),
                crate::rank_ocel_bottlenecks_log(&source, &full, &filter),
                crate::rank_ocel_bottlenecks_disk(path, &filter),
            );
            for object in &objects {
                same(
                    &format!("item {object} {what}"),
                    crate::inspect_ocel_object_log(&source, &full, &filter, object),
                    crate::inspect_ocel_object_disk(path, &filter, object),
                );
            }
            for (index, from) in activities.iter().enumerate() {
                for to in activities.iter().skip(index % 2).step_by(2).take(4) {
                    let loaded =
                        crate::measure_ocel_time_between_log(&source, &full, &filter, from, to);
                    same(
                        &format!("{from} to {to} {what}"),
                        loaded,
                        crate::ocel::measure_time_between_disk(path, &filter, from, to, 3),
                    );
                }
            }
            for object_type in full
                .object_types
                .iter()
                .map(|t| t.name.clone())
                .chain(["no such type".to_owned()])
            {
                for (cases, events) in [(0, 0), (2, 3), (1_000_000, 1_000)] {
                    same(
                        &format!("cases of {object_type} {cases} {events} {what}"),
                        crate::project_ocel_cases_log(
                            &source,
                            &full,
                            &filter,
                            &object_type,
                            cases,
                            events,
                        ),
                        crate::project_ocel_cases_disk(path, &filter, &object_type, cases, events),
                    );
                }
                assert_eq!(
                    crate::improvement::improvement_ocel_log(
                        &full,
                        &filter,
                        &object_type,
                        &improvement_decisions(&full)
                    ),
                    crate::improvement::improvement_ocel_disk(
                        path,
                        &filter,
                        &object_type,
                        &improvement_decisions(&full)
                    ),
                    "improvement following {object_type} {what}"
                );
                same(
                    &format!("journey from {object_type} {what}"),
                    crate::ocel_journey_log(&full, &filter, &object_type),
                    crate::ocel_journey_disk(path, &filter, &object_type),
                );
            }
            for noise in [0, 200, 1_001] {
                let config = pm_core::OcelDiscoveryConfig {
                    noise_threshold_milli: noise,
                    variable_arc_threshold_milli: 900,
                };
                same(
                    &format!("model {noise} {what}"),
                    crate::discover_ocel_model_log(&source, &full, &filter, config),
                    crate::discover_ocel_model_disk(path, &filter, config),
                );
            }
            for object_type in full.object_types.iter().map(|t| t.name.clone()) {
                let mut loaded = Vec::new();
                let expected = follow_ocel_log(&full, &filter, &object_type, &mut loaded);
                let mut streamed = Vec::new();
                let found = follow_ocel_disk_view(path, &filter, &object_type, &mut streamed);
                same(&format!("follow {object_type} {what}"), expected, found);
                assert_eq!(streamed, loaded, "follow {object_type} {what}");
            }
            same(
                &format!("unknown activity {what}"),
                crate::measure_ocel_time_between_log(&source, &full, &filter, "nope", "nope"),
                crate::measure_ocel_time_between_disk(path, &filter, "nope", "nope"),
            );
        }
    }

    #[test]
    fn the_overview_from_disk_matches_the_loaded_log_on_the_official_sample() {
        overview_same_from_disk(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../testdata/ocel/order-management.sqlite"),
        );
    }

    #[test]
    fn following_from_disk_matches_the_loaded_log_on_the_official_sample() {
        same_from_disk(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../testdata/ocel/order-management.sqlite"),
        );
    }

    #[test]
    fn following_from_disk_matches_on_a_prepared_sap_log() {
        use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe};
        let dir = std::env::temp_dir().join(format!("pm-follow-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let settings = pm_testgen::sap_p2p::Settings {
            orders: 150,
            ..Default::default()
        };
        pm_testgen::sap_p2p::write(&dir, &settings).unwrap();
        let pack = Pack::parse(BUILT_IN[0].1).unwrap();
        let files = pm_testgen::sap_p2p::TABLES
            .iter()
            .map(|t| ((*t).to_owned(), dir.join(format!("{t}.csv"))))
            .collect::<std::collections::BTreeMap<_, _>>();
        let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
        let answers = std::collections::BTreeMap::from([(
            "setting:offset".to_owned(),
            Answer {
                choice: "value".into(),
                value: Some("+01:00".into()),
                confirmed_by: "test".into(),
                at: String::new(),
                question: String::new(),
                carries: Vec::new(),
            },
        )]);
        let built = build_recipe(&pack, &hashes, &answers).unwrap();
        let outcome = pm_ingest::prepare_ocel::prepare(&built.recipe, &files).unwrap();
        let database = dir.join("log.sqlite");
        outcome.log.unwrap().write_sqlite(&database).unwrap();
        same_from_disk(&database);
        overview_same_from_disk(&database);
        // Change steps carry who did them; the improvement view sees them.
        let performers =
            crate::improvement::ocel_performers(&pm_ingest::read_ocel(&database).unwrap());
        assert!(!performers.0.is_empty());
        // The disk journey totals amounts of items that stopped short.
        let full = pm_ingest::read_ocel(&database).unwrap();
        let valued = full
            .object_types
            .iter()
            .filter_map(|t| {
                crate::ocel_journey_disk(&database, &OcelFilter::default(), &t.name).ok()
            })
            .flat_map(|journey| journey.handoffs)
            .filter(|handoff| handoff.not_reached_value.is_some())
            .count();
        assert!(valued > 0);
        // The overview reads it without attribute values and finds the same.
        let full = crate::analyze_ocel_log(
            "x",
            &pm_ingest::read_ocel(&database).unwrap(),
            &OcelFilter::default(),
        )
        .unwrap();
        let lean = crate::analyze_ocel_log(
            "x",
            &pm_ingest::read_ocel_sqlite_skeleton(&database).unwrap(),
            &OcelFilter::default(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&lean).unwrap(),
            serde_json::to_value(&full).unwrap()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
