//! Deterministic object-centric data-quality reporting.
//!
//! Structural impossibilities are rejected during ingestion. This report
//! covers representable conditions that would distort or weaken analysis.

use pm_core::{AttributeNameId, ObjectId, OcelEventId, OcelLog, QualifierId};
use serde::{Deserialize, Serialize};

const EXAMPLE_LIMIT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcelQualityKind {
    UndeclaredObjectReference,
    DuplicateEventObjectRelation,
    DuplicateObjectObjectRelation,
    DuplicateEventAttribute,
    DuplicateObjectAttributeAssignment,
    RepeatedObjectAttributeValue,
    EventWithoutObjects,
    ObjectWithoutEvents,
    EmptyRelationshipQualifier,
    SelfObjectRelationship,
    UnusedEventType,
    UnusedObjectType,
}

impl OcelQualityKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::UndeclaredObjectReference => "Links to objects missing from the file",
            Self::DuplicateEventObjectRelation => "Duplicate event-to-object relationships",
            Self::DuplicateObjectObjectRelation => "Duplicate object-to-object relationships",
            Self::DuplicateEventAttribute => "Duplicate event attribute values",
            Self::DuplicateObjectAttributeAssignment => "Conflicting object attribute assignments",
            Self::RepeatedObjectAttributeValue => "Repeated object attribute values",
            Self::EventWithoutObjects => "Events without objects",
            Self::ObjectWithoutEvents => "Objects without events",
            Self::EmptyRelationshipQualifier => "Relationships with empty qualifiers",
            Self::SelfObjectRelationship => "Objects related to themselves",
            Self::UnusedEventType => "Event types without events",
            Self::UnusedObjectType => "Object types without objects",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcelQualitySeverity {
    Information,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelQualityFinding {
    pub kind: OcelQualityKind,
    pub title: String,
    pub severity: OcelQualitySeverity,
    pub detail: String,
    pub affected: usize,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelQualitySummary {
    pub findings: usize,
    pub errors: usize,
    pub warnings: usize,
    pub information: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelQualityReport {
    pub ready_for_analysis: bool,
    pub events: usize,
    pub objects: usize,
    pub event_object_relations: usize,
    pub object_object_relations: usize,
    pub summary: OcelQualitySummary,
    pub findings: Vec<OcelQualityFinding>,
}

/// Assesses an already structurally valid OCEL without changing it.
pub fn assess_ocel_quality(log: &OcelLog) -> OcelQualityReport {
    let mut findings = Vec::new();
    undeclared_links(log, &mut findings);
    duplicate_e2o(log, &mut findings);
    duplicate_o2o(log, &mut findings);
    duplicate_event_attributes(log, &mut findings);
    duplicate_object_attributes(log, &mut findings);
    disconnected_events(log, &mut findings);
    disconnected_objects(log, &mut findings);
    empty_qualifiers(log, &mut findings);
    self_relationships(log, &mut findings);
    unused_types(log, &mut findings);

    report(
        findings,
        [
            log.events.len(),
            log.objects.len(),
            log.event_object_relations.len(),
            log.object_object_relations.len(),
        ],
    )
}

/// The report from its findings and the log's counts: events, objects,
/// event-to-object and object-to-object rows.
pub(crate) fn report(
    mut findings: Vec<OcelQualityFinding>,
    counts: [usize; 4],
) -> OcelQualityReport {
    findings.sort_by_key(|finding| finding.kind);
    let mut summary = OcelQualitySummary {
        findings: findings.len(),
        ..Default::default()
    };
    for finding in &findings {
        match finding.severity {
            OcelQualitySeverity::Information => summary.information += 1,
            OcelQualitySeverity::Warning => summary.warnings += 1,
            OcelQualitySeverity::Error => summary.errors += 1,
        }
    }
    OcelQualityReport {
        ready_for_analysis: summary.errors == 0,
        events: counts[0],
        objects: counts[1],
        event_object_relations: counts[2],
        object_object_relations: counts[3],
        summary,
        findings,
    }
}

/// Returns duplicate keys in canonical order with their full occurrence count.
/// Sorting a compact key vector is substantially smaller than one tree node per
/// source row while preserving exact counts and deterministic evidence.
fn duplicate_runs<T: Copy + Ord>(mut keys: Vec<T>) -> Vec<(T, usize)> {
    keys.sort_unstable();
    let mut duplicates = Vec::new();
    let mut start = 0;
    while start < keys.len() {
        let mut end = start + 1;
        while end < keys.len() && keys[end] == keys[start] {
            end += 1;
        }
        if end - start > 1 {
            duplicates.push((keys[start], end - start));
        }
        start = end;
    }
    duplicates
}

fn duplicate_e2o(log: &OcelLog, findings: &mut Vec<OcelQualityFinding>) {
    let duplicates = duplicate_runs(
        log.event_object_relations
            .iter()
            .map(|relation| (relation.event, relation.object, relation.qualifier))
            .collect(),
    );
    if duplicates.is_empty() {
        return;
    }
    let affected = duplicates.iter().map(|(_, count)| count - 1).sum();
    push(
        findings,
        OcelQualityKind::DuplicateEventObjectRelation,
        affected,
        duplicates
            .into_iter()
            .map(|((event, object, qualifier), _)| {
                format!(
                    "{} --{}--> {}",
                    event_label(log, event),
                    qualifier_label(log, qualifier),
                    object_label(log, object)
                )
            }),
    );
}

fn duplicate_o2o(log: &OcelLog, findings: &mut Vec<OcelQualityFinding>) {
    let duplicates = duplicate_runs(
        log.object_object_relations
            .iter()
            .map(|relation| (relation.source, relation.target, relation.qualifier))
            .collect(),
    );
    if duplicates.is_empty() {
        return;
    }
    let affected = duplicates.iter().map(|(_, count)| count - 1).sum();
    push(
        findings,
        OcelQualityKind::DuplicateObjectObjectRelation,
        affected,
        duplicates
            .into_iter()
            .map(|((source, target, qualifier), _)| {
                format!(
                    "{} --{}--> {}",
                    object_label(log, source),
                    qualifier_label(log, qualifier),
                    object_label(log, target)
                )
            }),
    );
}

fn duplicate_event_attributes(log: &OcelLog, findings: &mut Vec<OcelQualityFinding>) {
    let duplicates = duplicate_runs(
        log.event_attributes
            .iter()
            .map(|attribute| (attribute.event, attribute.name))
            .collect(),
    );
    if duplicates.is_empty() {
        return;
    }
    let affected = duplicates.iter().map(|(_, count)| count - 1).sum();
    push(
        findings,
        OcelQualityKind::DuplicateEventAttribute,
        affected,
        duplicates.into_iter().map(|((event, name), _)| {
            format!(
                "{}: {}",
                event_label(log, event),
                attribute_label(log, name)
            )
        }),
    );
}

/// Several values for one object attribute at one instant. Different values
/// make the state ambiguous and block analysis; the same value written twice
/// changes nothing and is only reported.
fn duplicate_object_attributes(log: &OcelLog, findings: &mut Vec<OcelQualityFinding>) {
    let attributes = &log.object_attributes;
    let mut order = (0..attributes.len() as u32).collect::<Vec<_>>();
    let key = |index: u32| {
        let attribute = &attributes[index as usize];
        (attribute.object, attribute.name, attribute.at)
    };
    order.sort_unstable_by_key(|index| (key(*index), *index));
    let (mut conflicts, mut repeats) = (Vec::new(), Vec::new());
    for run in order.chunk_by(|left, right| key(*left) == key(*right)) {
        if run.len() < 2 {
            continue;
        }
        let first = &attributes[run[0] as usize].value;
        let same = run
            .iter()
            .all(|index| attributes[*index as usize].value == *first);
        let entry = (key(run[0]), run.len() - 1);
        if same {
            repeats.push(entry);
        } else {
            conflicts.push(entry);
        }
    }
    let example = |((object, name, at), _): (_, usize)| {
        format!(
            "{}: {} at {}",
            object_label(log, object),
            attribute_label(log, name),
            at
        )
    };
    if !conflicts.is_empty() {
        push(
            findings,
            OcelQualityKind::DuplicateObjectAttributeAssignment,
            conflicts.iter().map(|(_, extra)| extra).sum(),
            conflicts.into_iter().map(example),
        );
    }
    if !repeats.is_empty() {
        push(
            findings,
            OcelQualityKind::RepeatedObjectAttributeValue,
            repeats.iter().map(|(_, extra)| extra).sum(),
            repeats.into_iter().map(example),
        );
    }
}

fn disconnected_events(log: &OcelLog, findings: &mut Vec<OcelQualityFinding>) {
    let mut connected = vec![false; log.events.len()];
    for relation in &log.event_object_relations {
        connected[relation.event.0 as usize] = true;
    }
    let disconnected = (0..log.events.len())
        .map(|index| OcelEventId(index as u32))
        .filter(|event| !connected[event.0 as usize])
        .collect::<Vec<_>>();
    if disconnected.is_empty() {
        return;
    }
    push(
        findings,
        OcelQualityKind::EventWithoutObjects,
        disconnected.len(),
        disconnected
            .into_iter()
            .map(|event| event_label(log, event)),
    );
}

fn disconnected_objects(log: &OcelLog, findings: &mut Vec<OcelQualityFinding>) {
    let mut connected = vec![false; log.objects.len()];
    for relation in &log.event_object_relations {
        connected[relation.object.0 as usize] = true;
    }
    let disconnected = (0..log.objects.len())
        .map(|index| ObjectId(index as u32))
        .filter(|object| !connected[object.0 as usize])
        .collect::<Vec<_>>();
    if disconnected.is_empty() {
        return;
    }
    push(
        findings,
        OcelQualityKind::ObjectWithoutEvents,
        disconnected.len(),
        disconnected
            .into_iter()
            .map(|object| object_label(log, object)),
    );
}

fn empty_qualifiers(log: &OcelLog, findings: &mut Vec<OcelQualityFinding>) {
    let mut affected = 0;
    let mut examples = Vec::with_capacity(EXAMPLE_LIMIT);
    for relation in &log.event_object_relations {
        if qualifier_label(log, relation.qualifier).trim().is_empty() {
            affected += 1;
            if examples.len() < EXAMPLE_LIMIT {
                examples.push(format!(
                    "{} → {}",
                    event_label(log, relation.event),
                    object_label(log, relation.object)
                ));
            }
        }
    }
    for relation in &log.object_object_relations {
        if qualifier_label(log, relation.qualifier).trim().is_empty() {
            affected += 1;
            if examples.len() < EXAMPLE_LIMIT {
                examples.push(format!(
                    "{} → {}",
                    object_label(log, relation.source),
                    object_label(log, relation.target)
                ));
            }
        }
    }
    if affected == 0 {
        return;
    }
    push(
        findings,
        OcelQualityKind::EmptyRelationshipQualifier,
        affected,
        examples.into_iter(),
    );
}

fn self_relationships(log: &OcelLog, findings: &mut Vec<OcelQualityFinding>) {
    let mut affected = 0;
    let mut examples = Vec::with_capacity(EXAMPLE_LIMIT);
    for relation in &log.object_object_relations {
        if relation.source == relation.target {
            affected += 1;
            if examples.len() < EXAMPLE_LIMIT {
                examples.push(format!(
                    "{} --{}--> itself",
                    object_label(log, relation.source),
                    qualifier_label(log, relation.qualifier)
                ));
            }
        }
    }
    if affected == 0 {
        return;
    }
    push(
        findings,
        OcelQualityKind::SelfObjectRelationship,
        affected,
        examples.into_iter(),
    );
}

fn unused_types(log: &OcelLog, findings: &mut Vec<OcelQualityFinding>) {
    let mut used_events = vec![false; log.event_types.len()];
    for event in &log.events {
        used_events[event.event_type.0 as usize] = true;
    }
    let event_types = log
        .event_types
        .iter()
        .enumerate()
        .filter(|(index, _)| !used_events[*index])
        .map(|(_, event_type)| event_type.name.clone())
        .collect::<Vec<_>>();
    if !event_types.is_empty() {
        push(
            findings,
            OcelQualityKind::UnusedEventType,
            event_types.len(),
            event_types.into_iter(),
        );
    }
    let mut used_objects = vec![false; log.object_types.len()];
    for object in &log.objects {
        used_objects[object.object_type.0 as usize] = true;
    }
    let object_types = log
        .object_types
        .iter()
        .enumerate()
        .filter(|(index, _)| !used_objects[*index])
        .map(|(_, object_type)| object_type.name.clone())
        .collect::<Vec<_>>();
    if !object_types.is_empty() {
        push(
            findings,
            OcelQualityKind::UnusedObjectType,
            object_types.len(),
            object_types.into_iter(),
        );
    }
}

/// Links the file made to objects it never declares. The reader left them
/// out; saying how many keeps that visible rather than silent.
fn undeclared_links(log: &OcelLog, findings: &mut Vec<OcelQualityFinding>) {
    let missing = &log.undeclared_links;
    if missing.count == 0 {
        return;
    }
    push(
        findings,
        OcelQualityKind::UndeclaredObjectReference,
        missing.count,
        missing.examples.iter().cloned(),
    );
}

/// Each finding's severity and explanation, the same whether the log was
/// checked in memory or on disk.
pub(crate) fn rule(kind: OcelQualityKind) -> (OcelQualitySeverity, &'static str) {
    match kind {
        OcelQualityKind::DuplicateEventObjectRelation => (
            OcelQualitySeverity::Error,
            "Repeated E2O rows would inflate object involvement and interaction counts.",
        ),
        OcelQualityKind::DuplicateObjectObjectRelation => (
            OcelQualitySeverity::Error,
            "Repeated O2O rows would inflate relationship counts.",
        ),
        OcelQualityKind::DuplicateEventAttribute => (
            OcelQualitySeverity::Error,
            "An event attribute must have one value; choosing among several would be arbitrary.",
        ),
        OcelQualityKind::EventWithoutObjects => (
            OcelQualitySeverity::Warning,
            "These events cannot contribute to an object lifecycle or interaction graph.",
        ),
        OcelQualityKind::ObjectWithoutEvents => (
            OcelQualitySeverity::Warning,
            "These objects have no observed lifecycle events in this log.",
        ),
        OcelQualityKind::UndeclaredObjectReference => (
            OcelQualitySeverity::Warning,
            "These links point to objects the file never declares, so they were left out. \
             Results that depend on them are understated; ask for a corrected export.",
        ),
        OcelQualityKind::DuplicateObjectAttributeAssignment => (
            OcelQualitySeverity::Error,
            "Several different values at the same instant make the object's state ambiguous.",
        ),
        OcelQualityKind::RepeatedObjectAttributeValue => (
            OcelQualitySeverity::Warning,
            "The same value is recorded more than once at the same instant; the repeats change nothing.",
        ),
        OcelQualityKind::EmptyRelationshipQualifier => (
            OcelQualitySeverity::Warning,
            "The relationship exists, but its role is not described.",
        ),
        OcelQualityKind::SelfObjectRelationship => (
            OcelQualitySeverity::Warning,
            "A self-relationship may be intentional, but often indicates a faulty join.",
        ),
        OcelQualityKind::UnusedEventType => (
            OcelQualitySeverity::Information,
            "The schema declares event types that have no events.",
        ),
        OcelQualityKind::UnusedObjectType => (
            OcelQualitySeverity::Information,
            "The schema declares object types that have no objects.",
        ),
    }
}

pub(crate) fn push(
    findings: &mut Vec<OcelQualityFinding>,
    kind: OcelQualityKind,
    affected: usize,
    examples: impl Iterator<Item = String>,
) {
    let (severity, detail) = rule(kind);
    findings.push(OcelQualityFinding {
        kind,
        title: kind.title().to_owned(),
        severity,
        detail: detail.to_owned(),
        affected,
        examples: examples.take(EXAMPLE_LIMIT).collect(),
    });
}

fn event_label(log: &OcelLog, event: OcelEventId) -> String {
    log.event_label(event)
        .unwrap_or("<unknown event>")
        .to_owned()
}

fn object_label(log: &OcelLog, object: ObjectId) -> String {
    log.object_label(object)
        .unwrap_or("<unknown object>")
        .to_owned()
}

fn qualifier_label(log: &OcelLog, qualifier: QualifierId) -> &str {
    log.qualifier_label(qualifier)
        .unwrap_or("<unknown qualifier>")
}

fn attribute_label(log: &OcelLog, attribute: AttributeNameId) -> &str {
    log.attribute_names
        .label(attribute.0)
        .unwrap_or("<unknown attribute>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_ocel_json;

    const FIXTURE: &[u8] = include_bytes!("../../../testdata/ocel/order-management.jsonocel");

    #[test]
    fn canonical_fixture_is_ready_for_analysis() {
        let report = assess_ocel_quality(&parse_ocel_json(FIXTURE).unwrap());
        assert!(report.ready_for_analysis);
        assert!(report.findings.is_empty());
        assert_eq!(report.events, 3);
        assert_eq!(report.objects, 4);
    }

    #[test]
    fn duplicate_relations_and_attributes_block_analysis() {
        let mut log = parse_ocel_json(FIXTURE).unwrap();
        log.event_object_relations
            .push(log.event_object_relations[0]);
        log.object_object_relations
            .push(log.object_object_relations[0]);
        log.event_attributes.push(log.event_attributes[0]);
        let mut conflicting = log.object_attributes[0];
        conflicting.value = pm_core::AttributeValue::Integer(-1);
        log.object_attributes.push(conflicting);
        let report = assess_ocel_quality(&log);
        assert!(!report.ready_for_analysis);
        assert_eq!(report.summary.errors, 4);
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.examples.len() <= 5)
        );
    }

    #[test]
    fn the_same_attribute_value_repeated_at_one_instant_is_only_a_warning() {
        let mut log = parse_ocel_json(FIXTURE).unwrap();
        log.object_attributes.push(log.object_attributes[0]);
        let report = assess_ocel_quality(&log);
        assert!(report.ready_for_analysis);
        assert_eq!((report.summary.errors, report.summary.warnings), (0, 1));
        assert_eq!(
            report.findings[0].kind,
            OcelQualityKind::RepeatedObjectAttributeValue
        );
    }

    #[test]
    fn duplicate_counts_include_every_row_after_the_first() {
        let mut log = parse_ocel_json(FIXTURE).unwrap();
        let duplicate = log.event_object_relations[0];
        log.event_object_relations.push(duplicate);
        log.event_object_relations.push(duplicate);

        let report = assess_ocel_quality(&log);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.kind == OcelQualityKind::DuplicateEventObjectRelation)
            .unwrap();
        assert_eq!(finding.affected, 2);
        assert_eq!(finding.examples.len(), 1);
    }

    #[test]
    fn disconnected_entities_and_weak_relationships_are_visible() {
        let mut log = parse_ocel_json(FIXTURE).unwrap();
        log.event_object_relations
            .retain(|relation| relation.event != OcelEventId(2));
        log.object_object_relations
            .push(pm_core::ObjectObjectRelation {
                source: ObjectId(0),
                target: ObjectId(0),
                qualifier: QualifierId(log.qualifiers.intern("")),
            });
        let report = assess_ocel_quality(&log);
        assert!(report.ready_for_analysis);
        assert!(report.findings.iter().any(|finding| {
            finding.kind == OcelQualityKind::EventWithoutObjects && finding.affected == 1
        }));
        assert!(
            report
                .findings
                .iter()
                .any(|finding| { finding.kind == OcelQualityKind::EmptyRelationshipQualifier })
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| { finding.kind == OcelQualityKind::SelfObjectRelationship })
        );
    }

    #[test]
    fn report_is_deterministic() {
        let log = parse_ocel_json(FIXTURE).unwrap();
        assert_eq!(assess_ocel_quality(&log), assess_ocel_quality(&log));
    }
}
