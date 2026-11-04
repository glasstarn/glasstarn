//! OCEL 2.0 JSON ingestion.
//!
//! The exchange format is decoded into the native object-centric core model;
//! no object type is selected as a case and no event is duplicated.

use pm_core::{
    AttributeDefinition, AttributeType, AttributeValue, OcelBuildError, OcelBuilder, OcelLog,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum OcelJsonError {
    #[error("could not read OCEL JSON '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("OCEL JSON is not valid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("OCEL structure is not valid: {0}")]
    Structure(#[from] OcelBuildError),
    #[error("{context} declares unsupported attribute type '{value_type}'")]
    UnsupportedAttributeType { context: String, value_type: String },
    #[error("{context} declares attribute '{attribute}' more than once")]
    DuplicateAttribute { context: String, attribute: String },
    #[error("{context} contains undeclared attribute '{attribute}'")]
    UndeclaredAttribute { context: String, attribute: String },
    #[error("{context} attribute '{attribute}' expects {expected}, but received {actual}")]
    AttributeTypeMismatch {
        context: String,
        attribute: String,
        expected: &'static str,
        actual: &'static str,
    },
    #[error("{context} has invalid timestamp '{value}'")]
    InvalidTimestamp { context: String, value: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JsonLog {
    event_types: Vec<JsonType>,
    object_types: Vec<JsonType>,
    events: Vec<JsonEvent>,
    objects: Vec<JsonObject>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonType {
    name: String,
    attributes: Vec<JsonAttributeDefinition>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonAttributeDefinition {
    name: String,
    #[serde(rename = "type")]
    value_type: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonEvent {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    time: String,
    #[serde(default)]
    attributes: Vec<JsonEventAttribute>,
    /// Optional: published logs omit the list for unlinked entities.
    #[serde(default)]
    relationships: Vec<JsonRelationship>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonObject {
    id: String,
    #[serde(rename = "type")]
    object_type: String,
    #[serde(default)]
    attributes: Vec<JsonObjectAttribute>,
    #[serde(default)]
    relationships: Vec<JsonRelationship>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonEventAttribute {
    name: String,
    value: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonObjectAttribute {
    name: String,
    time: String,
    value: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JsonRelationship {
    object_id: String,
    qualifier: String,
}

/// Reads an OCEL 2.0 JSON file into the native object-centric representation.
pub fn read_ocel_json(path: impl AsRef<Path>) -> Result<OcelLog, OcelJsonError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| OcelJsonError::Io {
        path: path.to_owned(),
        source,
    })?;
    parse_ocel_json(&bytes)
}

/// Parses OCEL 2.0 JSON bytes without inventing a case notion.
pub fn parse_ocel_json(bytes: &[u8]) -> Result<OcelLog, OcelJsonError> {
    let source: JsonLog = serde_json::from_slice(bytes)?;
    let mut builder = OcelBuilder::new();
    let event_schemas = declare_types(&mut builder, &source.event_types, true)?;
    let object_schemas = declare_types(&mut builder, &source.object_types, false)?;

    for object in &source.objects {
        builder.add_object(&object.id, &object.object_type)?;
    }
    for event in &source.events {
        let at = parse_time(&event.time, &format!("event '{}'", event.id))?;
        builder.add_event(&event.id, &event.event_type, at)?;
    }

    for event in &source.events {
        let context = format!("event '{}' of type '{}'", event.id, event.event_type);
        let schema = event_schemas
            .get(&event.event_type)
            .ok_or_else(|| OcelBuildError::UnknownEventType(event.event_type.clone()))?;
        for attribute in event.attributes.iter().filter(|a| !a.value.is_null()) {
            let value_type = declared_type(schema, &context, &attribute.name)?;
            let value = convert_value(
                &mut builder,
                value_type,
                &attribute.value,
                &context,
                &attribute.name,
            )?;
            let name = builder.attribute_name(&attribute.name);
            builder.add_event_attribute(&event.id, name, value)?;
        }
        for relationship in &event.relationships {
            builder.relate_event_object_or_record(
                &event.id,
                &relationship.object_id,
                &relationship.qualifier,
            )?;
        }
    }

    for object in &source.objects {
        let context = format!("object '{}' of type '{}'", object.id, object.object_type);
        let schema = object_schemas
            .get(&object.object_type)
            .ok_or_else(|| OcelBuildError::UnknownObjectType(object.object_type.clone()))?;
        // A null value records nothing; published exports write one where
        // an attribute had no value yet.
        for attribute in object.attributes.iter().filter(|a| !a.value.is_null()) {
            let value_type = declared_type(schema, &context, &attribute.name)?;
            let value = convert_value(
                &mut builder,
                value_type,
                &attribute.value,
                &context,
                &attribute.name,
            )?;
            let at = parse_time(
                &attribute.time,
                &format!("{context} attribute '{}'", attribute.name),
            )?;
            let name = builder.attribute_name(&attribute.name);
            builder.add_object_attribute(&object.id, name, at, value)?;
        }
        for relationship in &object.relationships {
            builder.relate_objects_or_record(
                &object.id,
                &relationship.object_id,
                &relationship.qualifier,
            )?;
        }
    }
    Ok(builder.build())
}

fn declare_types(
    builder: &mut OcelBuilder,
    types: &[JsonType],
    events: bool,
) -> Result<BTreeMap<String, BTreeMap<String, AttributeType>>, OcelJsonError> {
    let mut schemas = BTreeMap::new();
    for declared in types {
        let context = format!(
            "{} type '{}'",
            if events { "event" } else { "object" },
            declared.name
        );
        let mut names = BTreeSet::new();
        let mut schema = BTreeMap::new();
        let mut definitions = Vec::new();
        for attribute in &declared.attributes {
            if !names.insert(attribute.name.clone()) {
                return Err(OcelJsonError::DuplicateAttribute {
                    context,
                    attribute: attribute.name.clone(),
                });
            }
            let value_type = parse_attribute_type(&attribute.value_type).ok_or_else(|| {
                OcelJsonError::UnsupportedAttributeType {
                    context: context.clone(),
                    value_type: attribute.value_type.clone(),
                }
            })?;
            let name = builder.attribute_name(&attribute.name);
            definitions.push(AttributeDefinition { name, value_type });
            schema.insert(attribute.name.clone(), value_type);
        }
        if events {
            builder.declare_event_type(&declared.name, definitions)?;
        } else {
            builder.declare_object_type(&declared.name, definitions)?;
        }
        schemas.insert(declared.name.clone(), schema);
    }
    Ok(schemas)
}

fn parse_attribute_type(value: &str) -> Option<AttributeType> {
    match value {
        "string" => Some(AttributeType::String),
        "time" => Some(AttributeType::Time),
        "integer" => Some(AttributeType::Integer),
        "float" => Some(AttributeType::Float),
        "boolean" => Some(AttributeType::Boolean),
        _ => None,
    }
}

fn declared_type(
    schema: &BTreeMap<String, AttributeType>,
    context: &str,
    attribute: &str,
) -> Result<AttributeType, OcelJsonError> {
    schema
        .get(attribute)
        .copied()
        .ok_or_else(|| OcelJsonError::UndeclaredAttribute {
            context: context.to_owned(),
            attribute: attribute.to_owned(),
        })
}

fn convert_value(
    builder: &mut OcelBuilder,
    value_type: AttributeType,
    value: &Value,
    context: &str,
    attribute: &str,
) -> Result<AttributeValue, OcelJsonError> {
    let converted = match value_type {
        // Published logs write some declared-string values as bare numbers
        // or booleans; their JSON text is the exact string they stand for.
        AttributeType::String => match value {
            Value::String(text) => Some(builder.string_value(text)),
            Value::Number(_) | Value::Bool(_) => Some(builder.string_value(&value.to_string())),
            _ => None,
        },
        AttributeType::Time => value
            .as_str()
            .and_then(crate::ocel_read::ocel_time)
            .map(AttributeValue::Time),
        AttributeType::Integer => value.as_i64().map(AttributeValue::Integer),
        AttributeType::Float => value.as_f64().map(AttributeValue::Float),
        AttributeType::Boolean => value.as_bool().map(AttributeValue::Boolean),
    };
    converted.ok_or_else(|| OcelJsonError::AttributeTypeMismatch {
        context: context.to_owned(),
        attribute: attribute.to_owned(),
        expected: attribute_type_name(value_type),
        actual: json_type_name(value),
    })
}

fn parse_time(value: &str, context: &str) -> Result<i64, OcelJsonError> {
    crate::ocel_read::ocel_time(value).ok_or_else(|| OcelJsonError::InvalidTimestamp {
        context: context.to_owned(),
        value: value.to_owned(),
    })
}

fn attribute_type_name(value_type: AttributeType) -> &'static str {
    match value_type {
        AttributeType::String => "string",
        AttributeType::Time => "ISO 8601 time string",
        AttributeType::Integer => "integer",
        AttributeType::Float => "number",
        AttributeType::Boolean => "boolean",
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::{AttributeNameId, AttributeValueId, ObjectId};

    const FIXTURE: &[u8] = include_bytes!("../../../testdata/ocel/order-management.jsonocel");

    #[test]
    fn imports_the_canonical_object_centric_fixture_without_flattening() {
        let log = parse_ocel_json(FIXTURE).unwrap();
        assert_eq!(log.event_types.len(), 3);
        assert_eq!(log.object_types.len(), 3);
        assert_eq!(log.events.len(), 3);
        assert_eq!(log.objects.len(), 4);
        assert_eq!(log.event_object_relations.len(), 7);
        assert_eq!(log.object_object_relations.len(), 3);
        assert_eq!(log.event_label(pm_core::OcelEventId(0)), Some("e-place"));
        assert_eq!(log.object_label(ObjectId(0)), Some("order-1"));
    }

    #[test]
    fn reads_the_canonical_fixture_through_the_file_entry_point() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.jsonocel");
        assert_eq!(
            read_ocel_json(path).unwrap(),
            parse_ocel_json(FIXTURE).unwrap()
        );
    }

    #[test]
    fn imports_typed_values_and_dynamic_object_history() {
        let log = parse_ocel_json(FIXTURE).unwrap();
        assert!(matches!(
            log.event_attributes[0].value,
            AttributeValue::Integer(2)
        ));
        let status = AttributeNameId(log.attribute_names.get("status").unwrap());
        let open =
            AttributeValue::String(AttributeValueId(log.attribute_values.get("open").unwrap()));
        let shipped = AttributeValue::String(AttributeValueId(
            log.attribute_values.get("shipped").unwrap(),
        ));
        assert_eq!(log.object_attribute_at(ObjectId(0), status, 0), Some(open));
        assert_eq!(
            log.object_attribute_at(ObjectId(0), status, 86_400_000_000),
            Some(shipped)
        );
    }

    #[test]
    fn rejects_values_that_disagree_with_the_declared_schema() {
        let invalid = FIXTURE
            .windows(b"\"value\": 2".len())
            .position(|window| window == b"\"value\": 2")
            .unwrap();
        let mut changed = FIXTURE.to_vec();
        changed.splice(
            invalid..invalid + b"\"value\": 2".len(),
            b"\"value\": true".iter().copied(),
        );
        assert!(matches!(
            parse_ocel_json(&changed),
            Err(OcelJsonError::AttributeTypeMismatch { .. })
        ));
    }

    #[test]
    fn links_to_undeclared_objects_are_left_out_and_counted() {
        let original = parse_ocel_json(FIXTURE).unwrap();
        let changed = String::from_utf8(FIXTURE.to_vec())
            .unwrap()
            .replace("item-2\", \"qualifier", "missing\", \"qualifier");
        let log = parse_ocel_json(changed.as_bytes()).unwrap();
        let removed = original.event_object_relations.len()
            + original.object_object_relations.len()
            - log.event_object_relations.len()
            - log.object_object_relations.len();
        assert!(removed > 0);
        assert_eq!(log.undeclared_links.count, removed);
        assert!(log.undeclared_links.examples[0].ends_with("--> missing"));
        // An unknown type is still a structural error.
        let event = String::from_utf8(FIXTURE.to_vec()).unwrap().replacen(
            "\"type\": \"order\"",
            "\"type\": \"no-such-type\"",
            1,
        );
        assert!(parse_ocel_json(event.as_bytes()).is_err());
    }

    #[test]
    fn times_without_an_offset_are_utc_and_null_values_record_nothing() {
        let log = parse_ocel_json(
            br#"{"objectTypes":[{"name":"order","attributes":[{"name":"status","type":"string"},{"name":"due","type":"time"}]}],
            "eventTypes":[{"name":"place","attributes":[]}],
            "objects":[{"id":"o1","type":"order","attributes":[
                {"name":"status","time":"1970-01-01T00:00:00","value":null},
                {"name":"due","time":"1970-01-01T00:00:00Z","value":"2026-01-05 09:30:21.5"}]}],
            "events":[{"id":"e1","type":"place","time":"2026-01-01 10:00:00","relationships":[{"objectId":"o1","qualifier":"placed"}]}]}"#,
        )
        .unwrap();
        assert_eq!(log.events[0].at, 1_767_261_600_000_000);
        let numeric = parse_ocel_json(
            br#"{"objectTypes":[{"name":"product","attributes":[{"name":"price","type":"string"}]}],"eventTypes":[],
            "objects":[{"id":"p1","type":"product","attributes":[{"name":"price","time":"1970-01-01T00:00:00Z","value":12.5}]}],"events":[]}"#,
        )
        .unwrap();
        assert_eq!(
            numeric.attribute_values.label(0),
            Some("12.5"),
            "a declared string written as a number keeps its exact text"
        );
        assert_eq!(log.object_attributes.len(), 1);
        assert_eq!(
            log.object_attributes[0].value,
            AttributeValue::Time(1_767_605_421_500_000)
        );
    }

    #[test]
    fn omitted_relationship_and_attribute_lists_mean_none() {
        let log = parse_ocel_json(
            br#"{"objectTypes":[{"name":"order","attributes":[]}],"eventTypes":[{"name":"place","attributes":[]}],
            "objects":[{"id":"o1","type":"order"}],
            "events":[{"id":"e1","type":"place","time":"2026-01-01T00:00:00Z","relationships":[{"objectId":"o1","qualifier":"placed"}]}]}"#,
        )
        .unwrap();
        assert_eq!(log.objects.len(), 1);
        assert_eq!(log.event_object_relations.len(), 1);
        assert_eq!(log.undeclared_links.count, 0);
    }
}
