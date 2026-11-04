//! Non-destructive object-centric filtering with stable source identities.

use crate::{ObjectTypeId, OcelEventId, OcelLog};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Label-based selection suitable for CLI, browser, project, and MCP inputs.
/// Empty lists mean "all" in that dimension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OcelFilter {
    pub object_types: Vec<String>,
    pub event_object_qualifiers: Vec<String>,
    pub object_object_qualifiers: Vec<String>,
    /// A rooted cohort and explicit correlation paths. Empty roots select
    /// nothing. Paths never expand through arbitrary shared reference data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cohort: Option<OcelCohort>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcelCohort {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Box<OcelCohort>>,
    pub root_type: String,
    pub roots: BTreeSet<String>,
    pub paths: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OcelFilterError {
    #[error("could not read the linked cohort")]
    CohortRead,
    #[error("invalid cohort path: {0}")]
    InvalidCohortPath(String),
    #[error("unknown cohort object '{0}'")]
    UnknownCohortObject(String),
    #[error("unknown object type '{0}'")]
    UnknownObjectType(String),
    #[error("unknown event-to-object qualifier '{0}'")]
    UnknownEventObjectQualifier(String),
    #[error("unknown object-to-object qualifier '{0}'")]
    UnknownObjectObjectQualifier(String),
}

fn cohort_objects(
    log: &OcelLog,
    view: &OcelView,
    cohort: &OcelCohort,
) -> Result<BTreeSet<crate::ObjectId>, OcelFilterError> {
    let root_type = log
        .object_types
        .iter()
        .position(|kind| kind.name == cohort.root_type)
        .ok_or_else(|| OcelFilterError::UnknownObjectType(cohort.root_type.clone()))?;
    let allowed = view.objects.iter().copied().collect::<BTreeSet<_>>();
    let roots = cohort
        .roots
        .iter()
        .map(|name| {
            let id = log
                .object_ids
                .get(name)
                .map(crate::ObjectId)
                .filter(|id| log.objects[id.0 as usize].object_type.0 as usize == root_type)
                .ok_or_else(|| OcelFilterError::UnknownCohortObject(name.clone()))?;
            Ok(id)
        })
        .collect::<Result<BTreeSet<_>, OcelFilterError>>()?;
    let roots = roots
        .intersection(&allowed)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut keep = roots.clone();
    if cohort.paths.len() > 32 {
        return Err(OcelFilterError::InvalidCohortPath(
            "at most 32 paths".into(),
        ));
    }
    let mut events = vec![Vec::new(); log.objects.len()];
    let mut participants = vec![Vec::new(); log.events.len()];
    let mut neighbours = vec![Vec::new(); log.objects.len()];
    for index in &view.event_object_relations {
        let link = log.event_object_relations[*index];
        events[link.object.0 as usize].push(link.event);
        participants[link.event.0 as usize].push(link.object);
    }
    for index in &view.object_object_relations {
        let link = log.object_object_relations[*index];
        neighbours[link.source.0 as usize].push(link.target);
        neighbours[link.target.0 as usize].push(link.source);
    }
    for path in &cohort.paths {
        if path.first() != Some(&cohort.root_type)
            || path.len() > 14
            || path.iter().collect::<BTreeSet<_>>().len() != path.len()
        {
            return Err(OcelFilterError::InvalidCohortPath(
                "start at the root type, use at most fourteen distinct types".into(),
            ));
        }
        let mut frontier = roots.clone();
        for kind in path.iter().skip(1) {
            let target = log
                .object_types
                .iter()
                .position(|candidate| &candidate.name == kind)
                .ok_or_else(|| OcelFilterError::UnknownObjectType(kind.clone()))?;
            let mut next = BTreeSet::new();
            for id in &frontier {
                for event in &events[id.0 as usize] {
                    next.extend(
                        participants[event.0 as usize]
                            .iter()
                            .copied()
                            .filter(|other| {
                                log.objects[other.0 as usize].object_type.0 as usize == target
                            }),
                    );
                }
                next.extend(neighbours[id.0 as usize].iter().copied().filter(|other| {
                    allowed.contains(other)
                        && log.objects[other.0 as usize].object_type.0 as usize == target
                }));
            }
            keep.extend(&next);
            frontier = next;
        }
    }
    Ok(keep)
}

/// A filtered index into an [`OcelLog`]. Entity IDs and relationship indexes
/// always refer to the original log; no event is duplicated or renumbered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelView {
    pub events: Vec<OcelEventId>,
    pub objects: Vec<crate::ObjectId>,
    pub event_attributes: Vec<usize>,
    pub object_attributes: Vec<usize>,
    pub event_object_relations: Vec<usize>,
    pub object_object_relations: Vec<usize>,
}

impl OcelView {
    pub fn apply(log: &OcelLog, filter: &OcelFilter) -> Result<Self, OcelFilterError> {
        if let Some(cohort) = &filter.cohort {
            let mut depth = 0;
            let mut parent = cohort.parent.as_deref();
            while let Some(cohort) = parent {
                depth += 1;
                if depth > 8 {
                    return Err(OcelFilterError::InvalidCohortPath(
                        "at most eight nested cohorts".into(),
                    ));
                }
                parent = cohort.parent.as_deref();
            }
            let mut base = filter.clone();
            base.cohort = cohort.parent.as_deref().cloned();
            let mut view = Self::apply(log, &base)?;
            let keep = cohort_objects(log, &view, cohort)?;
            view.objects.retain(|id| keep.contains(id));
            view.event_object_relations
                .retain(|index| keep.contains(&log.event_object_relations[*index].object));
            let events = view
                .event_object_relations
                .iter()
                .map(|index| log.event_object_relations[*index].event)
                .collect::<BTreeSet<_>>();
            view.events.retain(|id| events.contains(id));
            view.event_attributes
                .retain(|index| events.contains(&log.event_attributes[*index].event));
            view.object_attributes
                .retain(|index| keep.contains(&log.object_attributes[*index].object));
            view.object_object_relations.retain(|index| {
                let link = log.object_object_relations[*index];
                keep.contains(&link.source) && keep.contains(&link.target)
            });
            return Ok(view);
        }
        let selected_types = resolve_object_types(log, &filter.object_types)?;
        let selected_e2o = resolve_qualifiers(
            log,
            &filter.event_object_qualifiers,
            RelationshipKind::EventObject,
        )?;
        let selected_o2o = resolve_qualifiers(
            log,
            &filter.object_object_qualifiers,
            RelationshipKind::ObjectObject,
        )?;

        let objects = log
            .objects
            .iter()
            .enumerate()
            .filter(|(_, object)| {
                selected_types
                    .as_ref()
                    .is_none_or(|types| types.contains(&object.object_type))
            })
            .map(|(index, _)| crate::ObjectId(index as u32))
            .collect::<Vec<_>>();
        // OCEL entity IDs are dense array indexes. A bit mask keeps these
        // membership tests linear and compact; an ordered tree here made an
        // identity view unnecessarily O(relationships × log(objects)).
        let mut object_set = vec![false; log.objects.len()];
        for object in &objects {
            object_set[object.0 as usize] = true;
        }

        let event_object_relations = log
            .event_object_relations
            .iter()
            .enumerate()
            .filter(|(_, relation)| {
                object_set[relation.object.0 as usize]
                    && selected_e2o
                        .as_ref()
                        .is_none_or(|qualifiers| qualifiers.contains(&relation.qualifier.0))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let mut event_set = vec![false; log.events.len()];
        for index in &event_object_relations {
            event_set[log.event_object_relations[*index].event.0 as usize] = true;
        }
        let events = event_set
            .iter()
            .enumerate()
            .filter(|(_, selected)| **selected)
            .map(|(index, _)| OcelEventId(index as u32))
            .collect::<Vec<_>>();

        let event_attributes = log
            .event_attributes
            .iter()
            .enumerate()
            .filter(|(_, attribute)| event_set[attribute.event.0 as usize])
            .map(|(index, _)| index)
            .collect();
        let object_attributes = log
            .object_attributes
            .iter()
            .enumerate()
            .filter(|(_, attribute)| object_set[attribute.object.0 as usize])
            .map(|(index, _)| index)
            .collect();
        let object_object_relations = log
            .object_object_relations
            .iter()
            .enumerate()
            .filter(|(_, relation)| {
                object_set[relation.source.0 as usize]
                    && object_set[relation.target.0 as usize]
                    && selected_o2o
                        .as_ref()
                        .is_none_or(|qualifiers| qualifiers.contains(&relation.qualifier.0))
            })
            .map(|(index, _)| index)
            .collect();

        Ok(Self {
            events,
            objects,
            event_attributes,
            object_attributes,
            event_object_relations,
            object_object_relations,
        })
    }
}

fn resolve_object_types(
    log: &OcelLog,
    labels: &[String],
) -> Result<Option<BTreeSet<ObjectTypeId>>, OcelFilterError> {
    if labels.is_empty() {
        return Ok(None);
    }
    let mut selected = BTreeSet::new();
    for label in labels {
        let id = log
            .object_types
            .iter()
            .position(|object_type| object_type.name == *label)
            .map(|index| ObjectTypeId(index as u32))
            .ok_or_else(|| OcelFilterError::UnknownObjectType(label.clone()))?;
        selected.insert(id);
    }
    Ok(Some(selected))
}

#[derive(Clone, Copy)]
enum RelationshipKind {
    EventObject,
    ObjectObject,
}

fn resolve_qualifiers(
    log: &OcelLog,
    labels: &[String],
    kind: RelationshipKind,
) -> Result<Option<BTreeSet<u32>>, OcelFilterError> {
    if labels.is_empty() {
        return Ok(None);
    }
    let available = match kind {
        RelationshipKind::EventObject => log
            .event_object_relations
            .iter()
            .map(|relation| relation.qualifier.0)
            .collect::<BTreeSet<_>>(),
        RelationshipKind::ObjectObject => log
            .object_object_relations
            .iter()
            .map(|relation| relation.qualifier.0)
            .collect::<BTreeSet<_>>(),
    };
    let mut selected = BTreeSet::new();
    for label in labels {
        let id = log
            .qualifiers
            .get(label)
            .filter(|id| available.contains(id));
        let id = id.ok_or_else(|| match kind {
            RelationshipKind::EventObject => {
                OcelFilterError::UnknownEventObjectQualifier(label.clone())
            }
            RelationshipKind::ObjectObject => {
                OcelFilterError::UnknownObjectObjectQualifier(label.clone())
            }
        })?;
        selected.insert(id);
    }
    Ok(Some(selected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OcelBuilder;

    fn log() -> OcelLog {
        let mut builder = OcelBuilder::new();
        builder.declare_event_type("place", vec![]).unwrap();
        builder.declare_event_type("pack", vec![]).unwrap();
        builder.declare_object_type("order", vec![]).unwrap();
        builder.declare_object_type("item", vec![]).unwrap();
        builder.declare_object_type("package", vec![]).unwrap();
        builder.add_event("e1", "place", 1).unwrap();
        builder.add_event("e2", "pack", 2).unwrap();
        builder.add_object("o1", "order").unwrap();
        builder.add_object("i1", "item").unwrap();
        builder.add_object("p1", "package").unwrap();
        builder.relate_event_object("e1", "o1", "order").unwrap();
        builder.relate_event_object("e1", "i1", "item").unwrap();
        builder
            .relate_event_object("e2", "i1", "packed item")
            .unwrap();
        builder.relate_event_object("e2", "p1", "result").unwrap();
        builder.relate_objects("i1", "o1", "part of").unwrap();
        builder.relate_objects("i1", "p1", "packed in").unwrap();
        builder.build()
    }

    #[test]
    fn empty_filter_is_a_complete_identity_view() {
        let view = OcelView::apply(&log(), &OcelFilter::default()).unwrap();
        assert_eq!(view.events.len(), 2);
        assert_eq!(view.objects.len(), 3);
        assert_eq!(view.event_object_relations.len(), 4);
        assert_eq!(view.object_object_relations.len(), 2);
    }

    #[test]
    fn object_type_selection_keeps_shared_events_once_and_original_ids() {
        let view = OcelView::apply(
            &log(),
            &OcelFilter {
                object_types: vec!["item".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(view.objects, vec![crate::ObjectId(1)]);
        assert_eq!(view.events, vec![OcelEventId(0), OcelEventId(1)]);
        assert_eq!(view.event_object_relations, vec![1, 2]);
        assert!(view.object_object_relations.is_empty());
    }

    #[test]
    fn relationship_qualifiers_are_filtered_by_relationship_kind() {
        let view = OcelView::apply(
            &log(),
            &OcelFilter {
                event_object_qualifiers: vec!["packed item".into()],
                object_object_qualifiers: vec!["part of".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(view.events, vec![OcelEventId(1)]);
        assert_eq!(view.event_object_relations, vec![2]);
        assert_eq!(view.object_object_relations, vec![0]);
    }

    #[test]
    fn unknown_labels_are_errors_instead_of_silent_empty_results() {
        let error = OcelView::apply(
            &log(),
            &OcelFilter {
                object_types: vec!["invoice".into()],
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error, OcelFilterError::UnknownObjectType("invoice".into()));
    }

    #[test]
    fn filtering_is_deterministic() {
        let log = log();
        let filter = OcelFilter {
            object_types: vec!["item".into(), "order".into(), "item".into()],
            ..Default::default()
        };
        assert_eq!(
            OcelView::apply(&log, &filter),
            OcelView::apply(&log, &filter)
        );
    }
}
