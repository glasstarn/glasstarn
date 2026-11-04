//! Strict, two-pass OCEL 2.0 XML ingestion.
//!
//! The parser streams the file twice: declarations and identities are built
//! first, then values and relationships are attached. This permits forward
//! object references without retaining a second copy of the event log.

use pm_core::{
    AttributeDefinition, AttributeType, AttributeValue, OcelBuildError, OcelBuilder, OcelLog,
};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Cursor};
use std::path::{Path, PathBuf};

type Schemas = BTreeMap<String, BTreeMap<String, AttributeType>>;

#[derive(Debug, thiserror::Error)]
pub enum OcelXmlError {
    #[error("could not read OCEL XML '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("OCEL XML is not valid: {0}")]
    Xml(String),
    #[error("OCEL structure is not valid: {0}")]
    Structure(#[from] OcelBuildError),
    #[error("{context} declares unsupported attribute type '{value_type}'")]
    UnsupportedAttributeType { context: String, value_type: String },
    #[error("{context} declares attribute '{attribute}' more than once")]
    DuplicateAttribute { context: String, attribute: String },
    #[error("{context} contains undeclared attribute '{attribute}'")]
    UndeclaredAttribute { context: String, attribute: String },
    #[error("{context} attribute '{attribute}' expects {expected}, but received '{value}'")]
    AttributeTypeMismatch {
        context: String,
        attribute: String,
        expected: &'static str,
        value: String,
    },
    #[error("{context} has invalid timestamp '{value}'")]
    InvalidTimestamp { context: String, value: String },
}

/// Reads an OCEL 2.0 XML file without loading the source document into memory.
pub fn read_ocel_xml(path: impl AsRef<Path>) -> Result<OcelLog, OcelXmlError> {
    let path = path.as_ref();
    let first = open(path)?;
    let (mut builder, event_schemas, object_schemas) = first_pass(first)?;
    second_pass(open(path)?, &mut builder, &event_schemas, &object_schemas)?;
    Ok(builder.build())
}

/// Parses OCEL 2.0 XML bytes. File ingestion uses [`read_ocel_xml`] so large
/// inputs remain streaming; this entry point exists for tests and embedding.
pub fn parse_ocel_xml(bytes: &[u8]) -> Result<OcelLog, OcelXmlError> {
    let (mut builder, event_schemas, object_schemas) =
        first_pass(BufReader::new(Cursor::new(bytes)))?;
    second_pass(
        BufReader::new(Cursor::new(bytes)),
        &mut builder,
        &event_schemas,
        &object_schemas,
    )?;
    Ok(builder.build())
}

fn open(path: &Path) -> Result<BufReader<std::fs::File>, OcelXmlError> {
    std::fs::File::open(path)
        .map(BufReader::new)
        .map_err(|source| OcelXmlError::Io {
            path: path.to_owned(),
            source,
        })
}

struct Declaration {
    name: String,
    events: bool,
    definitions: Vec<AttributeDefinition>,
    schema: BTreeMap<String, AttributeType>,
    names: BTreeSet<String>,
}

fn first_pass<R: BufRead>(input: R) -> Result<(OcelBuilder, Schemas, Schemas), OcelXmlError> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut stack = Vec::<String>::new();
    let mut builder = OcelBuilder::new();
    let mut declaration: Option<Declaration> = None;
    let mut event_schemas = BTreeMap::new();
    let mut object_schemas = BTreeMap::new();
    let mut sections = Vec::new();
    let mut saw_root = false;

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) => {
                let tag = tag(element.name());
                validate_child(&stack, &tag)?;
                if stack.as_slice() == ["log"] {
                    sections.push(tag.clone());
                }
                observe_first(
                    &reader,
                    &element,
                    &stack,
                    &tag,
                    &mut builder,
                    &mut declaration,
                )?;
                if tag == "log" {
                    if saw_root {
                        return Err(xml("more than one <log> root"));
                    }
                    saw_root = true;
                }
                stack.push(tag);
            }
            Ok(Event::Empty(element)) => {
                let tag = tag(element.name());
                validate_child(&stack, &tag)?;
                if stack.as_slice() == ["log"] {
                    sections.push(tag.clone());
                }
                observe_first(
                    &reader,
                    &element,
                    &stack,
                    &tag,
                    &mut builder,
                    &mut declaration,
                )?;
                finish_declaration(
                    &tag,
                    &mut builder,
                    &mut declaration,
                    &mut event_schemas,
                    &mut object_schemas,
                )?;
            }
            Ok(Event::End(element)) => {
                let tag = tag(element.name());
                if stack.pop().as_deref() != Some(tag.as_str()) {
                    return Err(xml(format!("unexpected closing </{tag}>")));
                }
                finish_declaration(
                    &tag,
                    &mut builder,
                    &mut declaration,
                    &mut event_schemas,
                    &mut object_schemas,
                )?;
            }
            Ok(Event::Text(text)) => {
                let value = decoded_text(&text)?;
                if !value.trim().is_empty()
                    && !matches!(stack.last().map(String::as_str), Some("attribute"))
                {
                    return Err(xml("text is only allowed inside <attribute>"));
                }
            }
            Ok(Event::CData(text)) => {
                if !matches!(stack.last().map(String::as_str), Some("attribute"))
                    && !text.iter().all(u8::is_ascii_whitespace)
                {
                    return Err(xml("CDATA is only allowed inside <attribute>"));
                }
            }
            Ok(Event::Decl(_) | Event::Comment(_)) => {}
            Ok(Event::Eof) => break,
            Ok(_) => return Err(xml("unsupported XML construct")),
            Err(error) => return Err(xml(error.to_string())),
        }
        buffer.clear();
    }
    if !saw_root || !stack.is_empty() {
        return Err(xml("the document must contain one complete <log> root"));
    }
    let expected = ["object-types", "event-types", "objects", "events"];
    if sections.iter().map(String::as_str).collect::<Vec<_>>() != expected {
        return Err(xml(
            "<log> must contain object-types, event-types, objects, and events in OCEL 2.0 order",
        ));
    }
    Ok((builder, event_schemas, object_schemas))
}

fn observe_first<R: BufRead>(
    reader: &Reader<R>,
    element: &BytesStart<'_>,
    stack: &[String],
    tag: &str,
    builder: &mut OcelBuilder,
    declaration: &mut Option<Declaration>,
) -> Result<(), OcelXmlError> {
    let parent = path(stack);
    match (parent.as_str(), tag) {
        ("log/object-types", "object-type") | ("log/event-types", "event-type") => {
            let attrs = attributes(reader, element, &["name"])?;
            *declaration = Some(Declaration {
                name: required(&attrs, "name", tag)?,
                events: tag == "event-type",
                definitions: Vec::new(),
                schema: BTreeMap::new(),
                names: BTreeSet::new(),
            });
        }
        ("log/object-types/object-type/attributes", "attribute")
        | ("log/event-types/event-type/attributes", "attribute") => {
            let attrs = attributes(reader, element, &["name", "type"])?;
            let name = required(&attrs, "name", tag)?;
            let value_type_name = required(&attrs, "type", tag)?;
            let current = declaration
                .as_mut()
                .ok_or_else(|| xml("attribute declaration has no enclosing type"))?;
            let context = format!(
                "{} type '{}'",
                if current.events { "event" } else { "object" },
                current.name
            );
            if !current.names.insert(name.clone()) {
                return Err(OcelXmlError::DuplicateAttribute {
                    context,
                    attribute: name,
                });
            }
            let value_type = parse_attribute_type(&value_type_name).ok_or({
                OcelXmlError::UnsupportedAttributeType {
                    context,
                    value_type: value_type_name,
                }
            })?;
            let compact_name = builder.attribute_name(&name);
            current.definitions.push(AttributeDefinition {
                name: compact_name,
                value_type,
            });
            current.schema.insert(name, value_type);
        }
        ("log/objects", "object") => {
            let attrs = attributes(reader, element, &["id", "type"])?;
            builder.add_object(
                &required(&attrs, "id", tag)?,
                &required(&attrs, "type", tag)?,
            )?;
        }
        ("log/events", "event") => {
            let attrs = attributes(reader, element, &["id", "type", "time"])?;
            let id = required(&attrs, "id", tag)?;
            let time = required(&attrs, "time", tag)?;
            builder.add_event(
                &id,
                &required(&attrs, "type", tag)?,
                parse_time(&time, &format!("event '{id}'"))?,
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn finish_declaration(
    tag: &str,
    builder: &mut OcelBuilder,
    declaration: &mut Option<Declaration>,
    event_schemas: &mut Schemas,
    object_schemas: &mut Schemas,
) -> Result<(), OcelXmlError> {
    if !matches!(tag, "event-type" | "object-type") {
        return Ok(());
    }
    let current = declaration
        .take()
        .ok_or_else(|| xml(format!("closing {tag} has no declaration")))?;
    if current.events {
        builder.declare_event_type(&current.name, current.definitions)?;
        event_schemas.insert(current.name, current.schema);
    } else {
        builder.declare_object_type(&current.name, current.definitions)?;
        object_schemas.insert(current.name, current.schema);
    }
    Ok(())
}

#[derive(Clone)]
enum Owner {
    Event { id: String, event_type: String },
    Object { id: String, object_type: String },
}

struct Capture {
    owner: Owner,
    name: String,
    time: Option<String>,
    value: String,
}

fn second_pass<R: BufRead>(
    input: R,
    builder: &mut OcelBuilder,
    event_schemas: &Schemas,
    object_schemas: &Schemas,
) -> Result<(), OcelXmlError> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut stack = Vec::<String>::new();
    let mut owner: Option<Owner> = None;
    let mut capture: Option<Capture> = None;

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) => {
                let tag = tag(element.name());
                if path(&stack) == "log/objects" && tag == "object" {
                    let attrs = attributes(&reader, &element, &["id", "type"])?;
                    owner = Some(Owner::Object {
                        id: required(&attrs, "id", &tag)?,
                        object_type: required(&attrs, "type", &tag)?,
                    });
                } else if path(&stack) == "log/events" && tag == "event" {
                    let attrs = attributes(&reader, &element, &["id", "type", "time"])?;
                    owner = Some(Owner::Event {
                        id: required(&attrs, "id", &tag)?,
                        event_type: required(&attrs, "type", &tag)?,
                    });
                } else if is_data_attribute(&stack, &tag) {
                    capture = Some(start_capture(&reader, &element, owner.clone())?);
                } else if is_relationship(&stack, &tag) {
                    add_relationship(&reader, &element, builder, owner.as_ref())?;
                }
                stack.push(tag);
            }
            Ok(Event::Empty(element)) => {
                let tag = tag(element.name());
                if is_data_attribute(&stack, &tag) {
                    let value = start_capture(&reader, &element, owner.clone())?;
                    finish_capture(value, builder, event_schemas, object_schemas)?;
                } else if is_relationship(&stack, &tag) {
                    add_relationship(&reader, &element, builder, owner.as_ref())?;
                }
            }
            Ok(Event::Text(text)) => {
                let value = decoded_text(&text)?;
                if let Some(capture) = &mut capture {
                    capture.value.push_str(&value);
                } else if !value.trim().is_empty() {
                    return Err(xml("text is only allowed inside <attribute>"));
                }
            }
            Ok(Event::CData(text)) => {
                if let Some(capture) = &mut capture {
                    capture.value.push_str(
                        &reader
                            .decoder()
                            .decode(text.as_ref())
                            .map_err(|error| xml(error.to_string()))?,
                    );
                } else {
                    return Err(xml("CDATA is only allowed inside <attribute>"));
                }
            }
            Ok(Event::End(element)) => {
                let tag = tag(element.name());
                if tag == "attribute"
                    && let Some(value) = capture.take()
                {
                    finish_capture(value, builder, event_schemas, object_schemas)?;
                }
                if matches!(tag.as_str(), "event" | "object")
                    && matches!(
                        stack.get(stack.len().saturating_sub(2)).map(String::as_str),
                        Some("events" | "objects")
                    )
                {
                    owner = None;
                }
                stack.pop();
            }
            Ok(Event::Decl(_) | Event::Comment(_)) => {}
            Ok(Event::Eof) => break,
            Ok(_) => return Err(xml("unsupported XML construct")),
            Err(error) => return Err(xml(error.to_string())),
        }
        buffer.clear();
    }
    Ok(())
}

fn is_data_attribute(stack: &[String], tag: &str) -> bool {
    tag == "attribute"
        && matches!(
            path(stack).as_str(),
            "log/objects/object/attributes" | "log/events/event/attributes"
        )
}

fn is_relationship(stack: &[String], tag: &str) -> bool {
    tag == "relationship"
        && matches!(
            path(stack).as_str(),
            "log/objects/object/objects" | "log/events/event/objects"
        )
}

fn start_capture<R: BufRead>(
    reader: &Reader<R>,
    element: &BytesStart<'_>,
    owner: Option<Owner>,
) -> Result<Capture, OcelXmlError> {
    let owner = owner.ok_or_else(|| xml("data attribute has no enclosing event or object"))?;
    let allowed = match owner {
        Owner::Event { .. } => &["name"][..],
        Owner::Object { .. } => &["name", "time"][..],
    };
    let attrs = attributes(reader, element, allowed)?;
    Ok(Capture {
        owner,
        name: required(&attrs, "name", "attribute")?,
        time: attrs.get("time").cloned(),
        value: String::new(),
    })
}

fn finish_capture(
    capture: Capture,
    builder: &mut OcelBuilder,
    event_schemas: &Schemas,
    object_schemas: &Schemas,
) -> Result<(), OcelXmlError> {
    let (schema, context) = match &capture.owner {
        Owner::Event { id, event_type } => (
            event_schemas
                .get(event_type)
                .ok_or_else(|| OcelBuildError::UnknownEventType(event_type.clone()))?,
            format!("event '{id}' of type '{event_type}'"),
        ),
        Owner::Object { id, object_type } => (
            object_schemas
                .get(object_type)
                .ok_or_else(|| OcelBuildError::UnknownObjectType(object_type.clone()))?,
            format!("object '{id}' of type '{object_type}'"),
        ),
    };
    let value_type =
        schema
            .get(&capture.name)
            .copied()
            .ok_or_else(|| OcelXmlError::UndeclaredAttribute {
                context: context.clone(),
                attribute: capture.name.clone(),
            })?;
    let value = convert_value(builder, value_type, &capture.value, &context, &capture.name)?;
    let name = builder.attribute_name(&capture.name);
    match capture.owner {
        Owner::Event { id, .. } => builder.add_event_attribute(&id, name, value)?,
        Owner::Object { id, .. } => {
            let time = capture
                .time
                .ok_or_else(|| xml(format!("object attribute '{}' requires time", capture.name)))?;
            let at = parse_time(&time, &format!("{context} attribute '{}'", capture.name))?;
            builder.add_object_attribute(&id, name, at, value)?;
        }
    }
    Ok(())
}

fn add_relationship<R: BufRead>(
    reader: &Reader<R>,
    element: &BytesStart<'_>,
    builder: &mut OcelBuilder,
    owner: Option<&Owner>,
) -> Result<(), OcelXmlError> {
    let attrs = attributes(reader, element, &["object-id", "qualifier"])?;
    let object = required(&attrs, "object-id", "relationship")?;
    let qualifier = required(&attrs, "qualifier", "relationship")?;
    match owner.ok_or_else(|| xml("relationship has no enclosing event or object"))? {
        Owner::Event { id, .. } => {
            builder.relate_event_object_or_record(id, &object, &qualifier)?
        }
        Owner::Object { id, .. } => builder.relate_objects_or_record(id, &object, &qualifier)?,
    }
    Ok(())
}

fn validate_child(stack: &[String], child: &str) -> Result<(), OcelXmlError> {
    let parent = path(stack);
    let valid = match parent.as_str() {
        "" => child == "log",
        "log" => matches!(child, "object-types" | "event-types" | "objects" | "events"),
        "log/object-types" => child == "object-type",
        "log/event-types" => child == "event-type",
        "log/object-types/object-type" | "log/event-types/event-type" => child == "attributes",
        "log/object-types/object-type/attributes" | "log/event-types/event-type/attributes" => {
            child == "attribute"
        }
        "log/objects" => child == "object",
        "log/events" => child == "event",
        "log/objects/object" | "log/events/event" => matches!(child, "attributes" | "objects"),
        "log/objects/object/attributes" | "log/events/event/attributes" => child == "attribute",
        "log/objects/object/objects" | "log/events/event/objects" => child == "relationship",
        _ if stack
            .last()
            .is_some_and(|tag| matches!(tag.as_str(), "attribute" | "relationship")) =>
        {
            false
        }
        _ => false,
    };
    valid
        .then_some(())
        .ok_or_else(|| xml(format!("<{child}> is not allowed inside <{parent}>")))
}

fn attributes<R: BufRead>(
    reader: &Reader<R>,
    element: &BytesStart<'_>,
    allowed: &[&str],
) -> Result<BTreeMap<String, String>, OcelXmlError> {
    let mut values = BTreeMap::new();
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| xml(error.to_string()))?;
        let name = tag(attribute.key);
        if !allowed.contains(&name.as_str()) {
            return Err(xml(format!(
                "attribute '{name}' is not allowed on <{}>",
                tag(element.name())
            )));
        }
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|error| xml(error.to_string()))?
            .into_owned();
        if values.insert(name.clone(), value).is_some() {
            return Err(xml(format!("attribute '{name}' is repeated")));
        }
    }
    Ok(values)
}

fn required(
    attrs: &BTreeMap<String, String>,
    name: &str,
    tag: &str,
) -> Result<String, OcelXmlError> {
    attrs
        .get(name)
        .cloned()
        .ok_or_else(|| xml(format!("<{tag}> requires attribute '{name}'")))
}

fn convert_value(
    builder: &mut OcelBuilder,
    value_type: AttributeType,
    text: &str,
    context: &str,
    attribute: &str,
) -> Result<AttributeValue, OcelXmlError> {
    let converted = match value_type {
        AttributeType::String => Some(builder.string_value(text)),
        AttributeType::Time => crate::ocel_read::ocel_time(text).map(AttributeValue::Time),
        AttributeType::Integer => text.parse().ok().map(AttributeValue::Integer),
        AttributeType::Float => text
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(AttributeValue::Float),
        AttributeType::Boolean => match text {
            "1" => Some(AttributeValue::Boolean(true)),
            "0" => Some(AttributeValue::Boolean(false)),
            _ => None,
        },
    };
    converted.ok_or_else(|| OcelXmlError::AttributeTypeMismatch {
        context: context.to_owned(),
        attribute: attribute.to_owned(),
        expected: attribute_type_name(value_type),
        value: text.to_owned(),
    })
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

fn attribute_type_name(value_type: AttributeType) -> &'static str {
    match value_type {
        AttributeType::String => "string",
        AttributeType::Time => "ISO 8601 time",
        AttributeType::Integer => "integer",
        AttributeType::Float => "finite number",
        AttributeType::Boolean => "0 or 1",
    }
}

fn parse_time(value: &str, context: &str) -> Result<i64, OcelXmlError> {
    crate::ocel_read::ocel_time(value).ok_or_else(|| OcelXmlError::InvalidTimestamp {
        context: context.to_owned(),
        value: value.to_owned(),
    })
}

fn decoded_text(text: &quick_xml::events::BytesText<'_>) -> Result<String, OcelXmlError> {
    let decoded = text.decode().map_err(|error| xml(error.to_string()))?;
    quick_xml::escape::unescape(&decoded)
        .map(|value| value.into_owned())
        .map_err(|error| xml(error.to_string()))
}

fn path(stack: &[String]) -> String {
    stack.join("/")
}

fn tag(name: QName<'_>) -> String {
    String::from_utf8_lossy(name.local_name().as_ref()).into_owned()
}

fn xml(detail: impl Into<String>) -> OcelXmlError {
    OcelXmlError::Xml(detail.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_ocel_json;

    const XML: &[u8] = include_bytes!("../../../testdata/ocel/order-management.xmlocel");
    const JSON: &[u8] = include_bytes!("../../../testdata/ocel/order-management.jsonocel");

    #[test]
    fn xml_is_canonically_equivalent_to_json() {
        let xml = parse_ocel_xml(XML).unwrap();
        let json = parse_ocel_json(JSON).unwrap();
        assert_eq!(xml.events, json.events);
        assert_eq!(xml.objects, json.objects);
        assert_eq!(
            xml.event_types
                .iter()
                .map(|item| &item.name)
                .collect::<Vec<_>>(),
            json.event_types
                .iter()
                .map(|item| &item.name)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            xml.object_types
                .iter()
                .map(|item| &item.name)
                .collect::<Vec<_>>(),
            json.object_types
                .iter()
                .map(|item| &item.name)
                .collect::<Vec<_>>()
        );
        assert_eq!(xml.event_attributes.len(), json.event_attributes.len());
        assert_eq!(xml.object_attributes.len(), json.object_attributes.len());
        assert_eq!(
            xml.event_object_relations.len(),
            json.event_object_relations.len()
        );
        assert_eq!(
            xml.object_object_relations.len(),
            json.object_object_relations.len()
        );
        assert_eq!(xml.event_ids.labels(), json.event_ids.labels());
        assert_eq!(xml.object_ids.labels(), json.object_ids.labels());
    }

    #[test]
    fn file_entry_point_is_streaming_and_equivalent() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.xmlocel");
        assert_eq!(read_ocel_xml(path).unwrap(), parse_ocel_xml(XML).unwrap());
    }

    #[test]
    fn rejects_non_standard_boolean_spellings() {
        let changed = String::from_utf8(XML.to_vec())
            .unwrap()
            .replace(">1</attribute>", ">true</attribute>");
        assert!(matches!(
            parse_ocel_xml(changed.as_bytes()),
            Err(OcelXmlError::AttributeTypeMismatch { .. })
        ));
    }

    #[test]
    fn rejects_unknown_elements_and_dangling_relationships() {
        let unknown = String::from_utf8(XML.to_vec())
            .unwrap()
            .replace("<events>", "<events><surprise/>");
        assert!(matches!(
            parse_ocel_xml(unknown.as_bytes()),
            Err(OcelXmlError::Xml(_))
        ));

        // A link to an undeclared object is left out and counted, not fatal.
        let dangling = String::from_utf8(XML.to_vec())
            .unwrap()
            .replace("object-id=\"item-2\"", "object-id=\"missing\"");
        let log = parse_ocel_xml(dangling.as_bytes()).unwrap();
        assert!(log.undeclared_links.count > 0);
        assert!(log.undeclared_links.examples[0].ends_with("--> missing"));
    }
}
