//! Compact object-centric event-log types aligned with the OCEL 2.0 model.
//!
//! Events remain atomic and are stored once. Their object participation,
//! attributes, object changes, and object-to-object relations live in sparse
//! sidecars so the common empty fields do not enlarge every event or object.
//! Nothing here chooses a case notion or flattens events into traces.

use crate::{Dictionary, Micros};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! compact_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub u32);
    };
}

compact_id!(OcelEventId);
compact_id!(ObjectId);
compact_id!(EventTypeId);
compact_id!(ObjectTypeId);
compact_id!(AttributeNameId);
compact_id!(QualifierId);
compact_id!(AttributeValueId);

/// OCEL 2.0 primitive attribute types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributeType {
    String,
    Time,
    Integer,
    Float,
    Boolean,
}

/// A declared attribute on an event or object type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeDefinition {
    pub name: AttributeNameId,
    pub value_type: AttributeType,
}

/// An OCEL event-type declaration (an activity plus its attribute schema).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventType {
    pub name: String,
    pub attributes: Vec<AttributeDefinition>,
}

/// An OCEL object-type declaration and its attribute schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectType {
    pub name: String,
    pub attributes: Vec<AttributeDefinition>,
}

/// One atomic activity execution. Object participation is stored separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcelEvent {
    pub event_type: EventTypeId,
    pub at: Micros,
}

/// One uniquely identified process entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Object {
    pub object_type: ObjectTypeId,
}

/// A typed value represented compactly. Strings and time spellings use the
/// shared value dictionary; numeric and Boolean values remain exact scalars.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AttributeValue {
    String(AttributeValueId),
    Time(Micros),
    Integer(i64),
    Float(f64),
    Boolean(bool),
}

/// A static attribute recorded on an event.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EventAttribute {
    pub event: OcelEventId,
    pub name: AttributeNameId,
    pub value: AttributeValue,
}

/// One object attribute assignment at a point in time. Several assignments
/// for the same object/name preserve the attribute's history.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ObjectAttribute {
    pub object: ObjectId,
    pub name: AttributeNameId,
    pub at: Micros,
    pub value: AttributeValue,
}

/// Qualified event-to-object participation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventObjectRelation {
    pub event: OcelEventId,
    pub object: ObjectId,
    pub qualifier: QualifierId,
}

/// Qualified directed object-to-object relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectObjectRelation {
    pub source: ObjectId,
    pub target: ObjectId,
    pub qualifier: QualifierId,
}

/// A complete object-centric log. Identifier labels are kept in deterministic
/// first-seen dictionaries; entity arrays are indexed directly by their IDs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OcelLog {
    pub event_ids: Dictionary,
    pub object_ids: Dictionary,
    pub attribute_names: Dictionary,
    pub qualifiers: Dictionary,
    pub attribute_values: Dictionary,
    pub event_types: Vec<EventType>,
    pub object_types: Vec<ObjectType>,
    pub events: Vec<OcelEvent>,
    pub objects: Vec<Object>,
    pub event_attributes: Vec<EventAttribute>,
    pub object_attributes: Vec<ObjectAttribute>,
    pub event_object_relations: Vec<EventObjectRelation>,
    pub object_object_relations: Vec<ObjectObjectRelation>,
    /// Links in the source that pointed at objects the file never declared.
    /// They are left out of the log and reported, never silently dropped.
    pub undeclared_links: UndeclaredLinks,
}

/// Count and a few examples of links left out because their object is missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndeclaredLinks {
    pub count: usize,
    /// Up to [`UndeclaredLinks::EXAMPLES`], as "source --qualifier--> missing".
    pub examples: Vec<String>,
}

impl UndeclaredLinks {
    pub const EXAMPLES: usize = 5;

    fn record(&mut self, source: &str, qualifier: &str, missing: &str) {
        self.count += 1;
        if self.examples.len() < Self::EXAMPLES {
            self.examples
                .push(format!("{source} --{qualifier}--> {missing}"));
        }
    }
}

impl OcelLog {
    pub fn event_label(&self, id: OcelEventId) -> Option<&str> {
        self.event_ids.label(id.0)
    }

    pub fn object_label(&self, id: ObjectId) -> Option<&str> {
        self.object_ids.label(id.0)
    }

    pub fn event_type(&self, id: EventTypeId) -> Option<&EventType> {
        self.event_types.get(id.0 as usize)
    }

    pub fn object_type(&self, id: ObjectTypeId) -> Option<&ObjectType> {
        self.object_types.get(id.0 as usize)
    }

    pub fn qualifier_label(&self, id: QualifierId) -> Option<&str> {
        self.qualifiers.label(id.0)
    }

    /// Latest value assigned no later than `at`. Equal-time assignments use
    /// input order, matching deterministic last-assignment semantics.
    pub fn object_attribute_at(
        &self,
        object: ObjectId,
        name: AttributeNameId,
        at: Micros,
    ) -> Option<AttributeValue> {
        self.object_attributes
            .iter()
            .filter(|attribute| {
                attribute.object == object && attribute.name == name && attribute.at <= at
            })
            .enumerate()
            .max_by_key(|(index, attribute)| (attribute.at, *index))
            .map(|(_, attribute)| attribute.value)
    }
}

/// Fallible builder that preserves first-seen identity and rejects dangling or
/// mismatched references before producing an analytical log.
#[derive(Debug, Default)]
pub struct OcelBuilder {
    log: OcelLog,
    event_type_ids: BTreeMap<String, EventTypeId>,
    object_type_ids: BTreeMap<String, ObjectTypeId>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OcelBuildError {
    #[error("event type '{0}' is already declared")]
    DuplicateEventType(String),
    #[error("object type '{0}' is already declared")]
    DuplicateObjectType(String),
    #[error("event '{0}' is already declared")]
    DuplicateEvent(String),
    #[error("object '{0}' is already declared")]
    DuplicateObject(String),
    #[error("event type '{0}' is not declared")]
    UnknownEventType(String),
    #[error("object type '{0}' is not declared")]
    UnknownObjectType(String),
    #[error("event '{0}' is not declared")]
    UnknownEvent(String),
    #[error("object '{0}' is not declared")]
    UnknownObject(String),
}

impl OcelBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn declare_event_type(
        &mut self,
        name: &str,
        attributes: Vec<AttributeDefinition>,
    ) -> Result<EventTypeId, OcelBuildError> {
        if self.event_type_ids.contains_key(name) {
            return Err(OcelBuildError::DuplicateEventType(name.to_owned()));
        }
        let id = EventTypeId(self.log.event_types.len() as u32);
        self.log.event_types.push(EventType {
            name: name.to_owned(),
            attributes,
        });
        self.event_type_ids.insert(name.to_owned(), id);
        Ok(id)
    }

    pub fn declare_object_type(
        &mut self,
        name: &str,
        attributes: Vec<AttributeDefinition>,
    ) -> Result<ObjectTypeId, OcelBuildError> {
        if self.object_type_ids.contains_key(name) {
            return Err(OcelBuildError::DuplicateObjectType(name.to_owned()));
        }
        let id = ObjectTypeId(self.log.object_types.len() as u32);
        self.log.object_types.push(ObjectType {
            name: name.to_owned(),
            attributes,
        });
        self.object_type_ids.insert(name.to_owned(), id);
        Ok(id)
    }

    pub fn add_event(
        &mut self,
        id: &str,
        event_type: &str,
        at: Micros,
    ) -> Result<OcelEventId, OcelBuildError> {
        if self.log.event_ids.get(id).is_some() {
            return Err(OcelBuildError::DuplicateEvent(id.to_owned()));
        }
        let event_type = self
            .event_type_ids
            .get(event_type)
            .copied()
            .ok_or_else(|| OcelBuildError::UnknownEventType(event_type.to_owned()))?;
        let event = OcelEventId(self.log.event_ids.intern(id));
        debug_assert_eq!(event.0 as usize, self.log.events.len());
        self.log.events.push(OcelEvent { event_type, at });
        Ok(event)
    }

    pub fn add_object(&mut self, id: &str, object_type: &str) -> Result<ObjectId, OcelBuildError> {
        if self.log.object_ids.get(id).is_some() {
            return Err(OcelBuildError::DuplicateObject(id.to_owned()));
        }
        let object_type = self
            .object_type_ids
            .get(object_type)
            .copied()
            .ok_or_else(|| OcelBuildError::UnknownObjectType(object_type.to_owned()))?;
        let object = ObjectId(self.log.object_ids.intern(id));
        debug_assert_eq!(object.0 as usize, self.log.objects.len());
        self.log.objects.push(Object { object_type });
        Ok(object)
    }

    pub fn relate_event_object(
        &mut self,
        event: &str,
        object: &str,
        qualifier: &str,
    ) -> Result<(), OcelBuildError> {
        let event = self.event_id(event)?;
        let object = self.object_id(object)?;
        let qualifier = QualifierId(self.log.qualifiers.intern(qualifier));
        self.log.event_object_relations.push(EventObjectRelation {
            event,
            object,
            qualifier,
        });
        Ok(())
    }

    pub fn relate_objects(
        &mut self,
        source: &str,
        target: &str,
        qualifier: &str,
    ) -> Result<(), OcelBuildError> {
        let source = self.object_id(source)?;
        let target = self.object_id(target)?;
        let qualifier = QualifierId(self.log.qualifiers.intern(qualifier));
        self.log.object_object_relations.push(ObjectObjectRelation {
            source,
            target,
            qualifier,
        });
        Ok(())
    }

    /// Like [`Self::relate_event_object`], but a link to an object the file
    /// never declares is left out and counted instead of failing the import.
    /// Published exchange files do contain such dangling links; refusing the
    /// whole file helps nobody, and the count reaches the quality report.
    /// An undeclared event is still an error.
    pub fn relate_event_object_or_record(
        &mut self,
        event: &str,
        object: &str,
        qualifier: &str,
    ) -> Result<(), OcelBuildError> {
        self.event_id(event)?;
        if self.log.object_ids.get(object).is_none() {
            self.log.undeclared_links.record(event, qualifier, object);
            return Ok(());
        }
        self.relate_event_object(event, object, qualifier)
    }

    /// Like [`Self::relate_objects`], recording rather than failing when
    /// either end is an undeclared object.
    pub fn relate_objects_or_record(
        &mut self,
        source: &str,
        target: &str,
        qualifier: &str,
    ) -> Result<(), OcelBuildError> {
        for missing in [source, target] {
            if self.log.object_ids.get(missing).is_none() {
                self.log.undeclared_links.record(source, qualifier, missing);
                return Ok(());
            }
        }
        self.relate_objects(source, target, qualifier)
    }

    pub fn attribute_name(&mut self, name: &str) -> AttributeNameId {
        AttributeNameId(self.log.attribute_names.intern(name))
    }

    pub fn string_value(&mut self, value: &str) -> AttributeValue {
        AttributeValue::String(AttributeValueId(self.log.attribute_values.intern(value)))
    }

    pub fn add_event_attribute(
        &mut self,
        event: &str,
        name: AttributeNameId,
        value: AttributeValue,
    ) -> Result<(), OcelBuildError> {
        let event = self.event_id(event)?;
        self.log
            .event_attributes
            .push(EventAttribute { event, name, value });
        Ok(())
    }

    pub fn add_object_attribute(
        &mut self,
        object: &str,
        name: AttributeNameId,
        at: Micros,
        value: AttributeValue,
    ) -> Result<(), OcelBuildError> {
        let object = self.object_id(object)?;
        self.log.object_attributes.push(ObjectAttribute {
            object,
            name,
            at,
            value,
        });
        Ok(())
    }

    pub fn build(self) -> OcelLog {
        self.log
    }

    fn event_id(&self, label: &str) -> Result<OcelEventId, OcelBuildError> {
        self.log
            .event_ids
            .get(label)
            .map(OcelEventId)
            .ok_or_else(|| OcelBuildError::UnknownEvent(label.to_owned()))
    }

    fn object_id(&self, label: &str) -> Result<ObjectId, OcelBuildError> {
        self.log
            .object_ids
            .get(label)
            .map(ObjectId)
            .ok_or_else(|| OcelBuildError::UnknownObject(label.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> OcelLog {
        let mut builder = OcelBuilder::new();
        let status = builder.attribute_name("status");
        builder.declare_event_type("place order", vec![]).unwrap();
        builder.declare_event_type("pack", vec![]).unwrap();
        builder
            .declare_object_type(
                "order",
                vec![AttributeDefinition {
                    name: status,
                    value_type: AttributeType::String,
                }],
            )
            .unwrap();
        builder.declare_object_type("item", vec![]).unwrap();
        builder.add_object("o1", "order").unwrap();
        builder.add_object("i1", "item").unwrap();
        builder.add_object("i2", "item").unwrap();
        builder.add_event("e1", "place order", 10).unwrap();
        builder.add_event("e2", "pack", 20).unwrap();
        builder.relate_event_object("e1", "o1", "order").unwrap();
        builder.relate_event_object("e1", "i1", "item").unwrap();
        builder.relate_event_object("e1", "i2", "item").unwrap();
        builder
            .relate_event_object("e2", "i1", "packed item")
            .unwrap();
        builder.relate_objects("i1", "o1", "part of").unwrap();
        let open = builder.string_value("open");
        let packed = builder.string_value("packed");
        builder.add_object_attribute("o1", status, 0, open).unwrap();
        builder
            .add_object_attribute("o1", status, 20, packed)
            .unwrap();
        builder.build()
    }

    #[test]
    fn keeps_one_event_related_to_several_object_types_without_duplication() {
        let log = fixture();
        assert_eq!(log.events.len(), 2);
        assert_eq!(log.objects.len(), 3);
        assert_eq!(log.event_object_relations.len(), 4);
        assert_eq!(log.event_object_relations[0].event, OcelEventId(0));
        assert_eq!(log.event_object_relations[1].event, OcelEventId(0));
        assert_eq!(log.event_object_relations[2].event, OcelEventId(0));
    }

    #[test]
    fn keeps_qualified_e2o_and_o2o_relationships_distinct() {
        let log = fixture();
        assert_eq!(
            log.qualifier_label(log.event_object_relations[3].qualifier),
            Some("packed item")
        );
        assert_eq!(
            log.qualifier_label(log.object_object_relations[0].qualifier),
            Some("part of")
        );
    }

    #[test]
    fn resolves_time_varying_object_attributes_as_of_an_instant() {
        let log = fixture();
        let status = AttributeNameId(log.attribute_names.get("status").unwrap());
        let open =
            AttributeValue::String(AttributeValueId(log.attribute_values.get("open").unwrap()));
        let packed = AttributeValue::String(AttributeValueId(
            log.attribute_values.get("packed").unwrap(),
        ));
        assert_eq!(log.object_attribute_at(ObjectId(0), status, 19), Some(open));
        assert_eq!(
            log.object_attribute_at(ObjectId(0), status, 20),
            Some(packed)
        );
    }

    #[test]
    fn rejects_duplicate_and_dangling_identifiers() {
        let mut builder = OcelBuilder::new();
        builder.declare_event_type("create", vec![]).unwrap();
        builder.declare_object_type("order", vec![]).unwrap();
        builder.add_event("e1", "create", 0).unwrap();
        builder.add_object("o1", "order").unwrap();
        assert_eq!(
            builder.add_event("e1", "create", 1),
            Err(OcelBuildError::DuplicateEvent("e1".to_owned()))
        );
        assert_eq!(
            builder.relate_event_object("missing", "o1", "order"),
            Err(OcelBuildError::UnknownEvent("missing".to_owned()))
        );
        assert_eq!(
            builder.relate_objects("o1", "missing", "part of"),
            Err(OcelBuildError::UnknownObject("missing".to_owned()))
        );
    }

    #[test]
    fn atomic_entities_remain_compact() {
        assert_eq!(std::mem::size_of::<OcelEvent>(), 16);
        assert_eq!(std::mem::size_of::<Object>(), 4);
        assert_eq!(std::mem::size_of::<EventObjectRelation>(), 12);
    }
}
