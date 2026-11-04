//! OCEL 2.0 SQLite ingestion.
//!
//! The database is opened read-only and decoded directly into the native
//! object-centric representation. Type-specific tables provide the attribute
//! schemas; no SQL is accepted from the file and no case projection is made.

use pm_core::{
    AttributeDefinition, AttributeType, AttributeValue, OcelBuildError, OcelBuilder, OcelLog,
};
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

mod disk;
pub use disk::{DiskList, DiskLog, DiskView, ObjectTable, PERFORMER_ATTRIBUTES};

#[derive(Debug, thiserror::Error)]
pub enum OcelSqliteError {
    #[error("could not open OCEL SQLite database '{path}': {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("could not read OCEL SQLite database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("OCEL SQLite schema is not valid: {0}")]
    Schema(String),
    #[error("OCEL SQLite data is not valid: {0}")]
    Data(String),
    #[error("OCEL structure is not valid: {0}")]
    Structure(#[from] OcelBuildError),
}

#[derive(Debug)]
struct TypeTable {
    name: String,
    table: String,
    attributes: Vec<(String, AttributeType)>,
    /// Object tables only. Published files sometimes omit the column; without
    /// it every row is an assignment, none is a single-field change.
    has_changed_field: bool,
}

/// Reads an official OCEL 2.0 relational exchange file without flattening it.
pub fn read_ocel_sqlite(path: impl AsRef<Path>) -> Result<OcelLog, OcelSqliteError> {
    let path = path.as_ref();
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|source| OcelSqliteError::Open {
        path: path.to_owned(),
        source,
    })?;
    connection.pragma_update(None, "query_only", true)?;
    parse_connection(&connection)
}

/// Reads a log for the overview only: events, items and links, without the
/// recorded attribute values the overview never uses. Attribute values are
/// read only for items with several rows, the only place two values can meet
/// at one instant, so the data checks find exactly what a full read finds.
/// For a large log this leaves out most of what a full read holds.
pub fn read_ocel_sqlite_skeleton(path: impl AsRef<Path>) -> Result<OcelLog, OcelSqliteError> {
    let path = path.as_ref();
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|source| OcelSqliteError::Open {
        path: path.to_owned(),
        source,
    })?;
    connection.pragma_update(None, "query_only", true)?;
    read(&connection, false)
}

fn parse_connection(connection: &Connection) -> Result<OcelLog, OcelSqliteError> {
    read(connection, true)
}

fn read(connection: &Connection, attributes: bool) -> Result<OcelLog, OcelSqliteError> {
    let tables = table_names(connection)?;
    for required in ["event", "object", "event_object", "object_object"] {
        if !tables.contains(required) {
            return Err(OcelSqliteError::Schema(format!(
                "required table '{required}' is missing"
            )));
        }
    }

    let event_types = read_type_tables(connection, &tables, true)?;
    let object_types = read_type_tables(connection, &tables, false)?;
    validate_entity_types(connection, "event", &event_types)?;
    validate_entity_types(connection, "object", &object_types)?;

    let mut builder = OcelBuilder::new();
    declare_types(&mut builder, &event_types, true)?;
    declare_types(&mut builder, &object_types, false)?;
    add_objects(connection, &mut builder)?;
    add_events(connection, &event_types, &mut builder, attributes)?;
    object_assignments(
        connection,
        &object_types,
        attributes,
        |id, name, at, value| {
            let name = builder.attribute_name(name);
            let value = match value {
                Plain::Text(text) => builder.string_value(text),
                Plain::Value(value) => value,
            };
            builder.add_object_attribute(id, name, at, value)?;
            Ok(())
        },
    )?;
    read_event_object_relations(connection, &mut builder)?;
    read_object_object_relations(connection, &mut builder)?;
    Ok(builder.build())
}

fn table_names(connection: &Connection) -> Result<BTreeSet<String>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name COLLATE BINARY",
    )?;
    statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()
}

fn read_type_tables(
    connection: &Connection,
    tables: &BTreeSet<String>,
    events: bool,
) -> Result<Vec<TypeTable>, OcelSqliteError> {
    let prefix = if events { "event" } else { "object" };
    let map_table = format!("{prefix}_map_type");
    let corr_table = format!("{prefix}_corr_type");
    let (mapping_table, mapped_column) =
        match (tables.contains(&map_table), tables.contains(&corr_table)) {
            (true, false) => (map_table, "ocel_type_map"),
            (false, true) => (corr_table, "ocel_type_corr"),
            (false, false) => {
                return Err(OcelSqliteError::Schema(format!(
                    "required {prefix} type correspondence table is missing"
                )));
            }
            (true, true) => {
                return Err(OcelSqliteError::Schema(format!(
                    "both legacy and current {prefix} type correspondence tables are present"
                )));
            }
        };
    require_columns(connection, &mapping_table, &["ocel_type", mapped_column])?;
    let sql = format!(
        "SELECT ocel_type, {mapped_column} FROM {} ORDER BY rowid",
        quote_identifier(&mapping_table)
    );
    let rows = query_values(connection, &sql)?;
    let mut seen_names = BTreeSet::new();
    let mut seen_tables = BTreeSet::new();
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let name = text(&row[0], &format!("{mapping_table}.ocel_type"))?;
        let mapped = text(&row[1], &format!("{mapping_table}.{mapped_column}"))?;
        if !seen_names.insert(name.clone()) {
            return Err(OcelSqliteError::Schema(format!(
                "{prefix} type '{name}' is mapped more than once"
            )));
        }
        let table = format!("{prefix}_{mapped}");
        if !seen_tables.insert(table.clone()) {
            return Err(OcelSqliteError::Schema(format!(
                "several {prefix} types map to table '{table}'"
            )));
        }
        if !tables.contains(&table) {
            return Err(OcelSqliteError::Schema(format!(
                "type '{name}' maps to missing table '{table}'"
            )));
        }
        let reserved: &[&str] = if events {
            &["ocel_id", "ocel_time"]
        } else {
            &["ocel_id", "ocel_time", "ocel_changed_field"]
        };
        let columns = table_columns(connection, &table)?;
        let has_changed_field = columns
            .iter()
            .any(|(column, _)| column == "ocel_changed_field");
        for required in reserved
            .iter()
            .filter(|required| **required != "ocel_changed_field")
        {
            if !columns.iter().any(|(column, _)| column == required) {
                return Err(OcelSqliteError::Schema(format!(
                    "table '{table}' is missing required column '{required}'"
                )));
            }
        }
        let mut attributes = Vec::new();
        for (column, declared_type) in columns {
            if reserved.contains(&column.as_str()) {
                continue;
            }
            if column.starts_with("ocel_") {
                return Err(OcelSqliteError::Schema(format!(
                    "table '{table}' has unknown reserved column '{column}'"
                )));
            }
            let value_type = sqlite_attribute_type(&declared_type).ok_or_else(|| {
                OcelSqliteError::Schema(format!(
                    "table '{table}' attribute '{column}' has unsupported SQL type '{declared_type}'"
                ))
            })?;
            attributes.push((column, value_type));
        }
        result.push(TypeTable {
            name,
            table,
            attributes,
            has_changed_field,
        });
    }
    Ok(result)
}

fn declare_types(
    builder: &mut OcelBuilder,
    types: &[TypeTable],
    events: bool,
) -> Result<(), OcelBuildError> {
    for declared in types {
        let definitions = declared
            .attributes
            .iter()
            .map(|(name, value_type)| AttributeDefinition {
                name: builder.attribute_name(name),
                value_type: *value_type,
            })
            .collect();
        if events {
            builder.declare_event_type(&declared.name, definitions)?;
        } else {
            builder.declare_object_type(&declared.name, definitions)?;
        }
    }
    Ok(())
}

fn validate_entity_types(
    connection: &Connection,
    table: &str,
    types: &[TypeTable],
) -> Result<(), OcelSqliteError> {
    require_columns(connection, table, &["ocel_id", "ocel_type"])?;
    let declared = types
        .iter()
        .map(|value| value.name.as_str())
        .collect::<BTreeSet<_>>();
    let sql = format!(
        "SELECT ocel_id, ocel_type FROM {} ORDER BY rowid",
        quote_identifier(table)
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id = sql_text(row.get_ref(0)?, &format!("{table}.ocel_id"))?;
        let type_name = sql_text(row.get_ref(1)?, &format!("{table}.ocel_type"))?;
        if !declared.contains(type_name.as_str()) {
            return Err(OcelSqliteError::Data(format!(
                "{table} '{id}' uses undeclared type '{type_name}'"
            )));
        }
    }
    Ok(())
}

fn add_objects(connection: &Connection, builder: &mut OcelBuilder) -> Result<(), OcelSqliteError> {
    let mut statement =
        connection.prepare("SELECT ocel_id, ocel_type FROM object ORDER BY rowid")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id = sql_text(row.get_ref(0)?, "object.ocel_id")?;
        let type_name = sql_text(row.get_ref(1)?, "object.ocel_type")?;
        builder.add_object(&id, &type_name)?;
    }
    Ok(())
}

fn add_events(
    connection: &Connection,
    types: &[TypeTable],
    builder: &mut OcelBuilder,
    with_attributes: bool,
) -> Result<(), OcelSqliteError> {
    // Events are added in the event table's own order, not type by type:
    // events at the same time then keep the order the producer wrote them
    // in (for a prepared log, the process order of its rules) rather than
    // the alphabetical order of their type tables.
    let mut events = Vec::new();
    for (type_index, declared) in types.iter().enumerate() {
        reject_orphan_type_rows(connection, declared, "event")?;
        let wanted: &[(String, AttributeType)] = if with_attributes {
            &declared.attributes
        } else {
            &[]
        };
        let attributes = wanted
            .iter()
            .map(|(name, _)| format!("typed.{}", quote_identifier(name)))
            .collect::<Vec<_>>();
        let selected = if attributes.is_empty() {
            String::new()
        } else {
            format!(", {}", attributes.join(", "))
        };
        let sql = format!(
            "SELECT entity.rowid, entity.ocel_id, typed.ocel_time{selected} \
             FROM event AS entity LEFT JOIN {} AS typed ON typed.ocel_id = entity.ocel_id \
             WHERE entity.ocel_type = ?1 ORDER BY entity.rowid, typed.rowid",
            quote_identifier(&declared.table)
        );
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query([&declared.name])?;
        while let Some(row) = rows.next()? {
            let position = row.get::<_, i64>(0)?;
            let id = sql_text(row.get_ref(1)?, "event.ocel_id")?;
            let time = sql_text(row.get_ref(2)?, &format!("event '{id}' timestamp"))?;
            let at = parse_time(&time, &format!("event '{id}'"))?;
            let values = (0..wanted.len())
                .map(|index| row.get::<_, rusqlite::types::Value>(index + 3))
                .collect::<Result<Vec<_>, _>>()?;
            events.push((position, type_index, id, at, values));
        }
    }
    events.sort_by_key(|event| event.0);
    for (_, type_index, id, at, values) in events {
        let declared = &types[type_index];
        builder.add_event(&id, &declared.name, at)?;
        let wanted: &[(String, AttributeType)] = if with_attributes {
            &declared.attributes
        } else {
            &[]
        };
        for ((name, value_type), value) in wanted.iter().zip(&values) {
            if let Some(value) = convert_ref_value(
                builder,
                rusqlite::types::ValueRef::from(value),
                *value_type,
                &format!("event '{id}' attribute '{name}'"),
            )? {
                let name = builder.attribute_name(name);
                builder.add_event_attribute(&id, name, value)?;
            }
        }
    }
    Ok(())
}

/// Reads object attribute assignments in file order, table by table, and
/// passes each to `assign`: the object, the attribute, when, and the value.
fn object_assignments(
    connection: &Connection,
    types: &[TypeTable],
    all: bool,
    mut assign: impl FnMut(&str, &str, i64, Plain<'_>) -> Result<(), OcelSqliteError>,
) -> Result<(), OcelSqliteError> {
    for declared in types {
        reject_orphan_type_rows(connection, declared, "object")?;
        // Without all values, only items with several rows are read: a single
        // row cannot assign one attribute twice.
        let several: Option<BTreeSet<String>> = if all {
            None
        } else {
            let sql = format!(
                "SELECT ocel_id FROM {} GROUP BY ocel_id HAVING count(*) > 1",
                quote_identifier(&declared.table)
            );
            let mut statement = connection.prepare(&sql)?;
            let ids = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<_, _>>()?;
            Some(ids)
        };
        let attributes = declared
            .attributes
            .iter()
            .map(|(name, _)| quote_identifier(name))
            .collect::<Vec<_>>();
        let selected = if attributes.is_empty() {
            String::new()
        } else {
            format!(", {}", attributes.join(", "))
        };
        let changed_column = if declared.has_changed_field {
            "ocel_changed_field"
        } else {
            "NULL"
        };
        let sql = format!(
            "SELECT ocel_id, ocel_time, {changed_column}{selected} FROM {} ORDER BY rowid",
            quote_identifier(&declared.table)
        );
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id = sql_text(row.get_ref(0)?, &format!("{}.ocel_id", declared.table))?;
            if several.as_ref().is_some_and(|ids| !ids.contains(&id)) {
                continue;
            }
            // A blank time marks an initial value, which OCEL 2.0 places at
            // the Unix epoch, the same instant JSON and XML exports write.
            let at = match optional_sql_text(row.get_ref(1)?, &format!("object '{id}' timestamp"))?
            {
                Some(time) => parse_time(&time, &format!("object '{id}'"))?,
                None => 0,
            };
            let changed = optional_sql_text(
                row.get_ref(2)?,
                &format!("object '{id}' changed-field marker"),
            )?;
            for (index, (name, value_type)) in declared.attributes.iter().enumerate() {
                if let Some(changed_name) = changed.as_deref().filter(|changed| *changed != name) {
                    if !matches!(row.get_ref(index + 3)?, rusqlite::types::ValueRef::Null) {
                        return Err(OcelSqliteError::Data(format!(
                            "object '{id}' change row for '{changed_name}' also assigns '{name}'"
                        )));
                    }
                    continue;
                }
                if let Some(value) = convert_plain(
                    row.get_ref(index + 3)?,
                    *value_type,
                    &format!("object '{id}' attribute '{name}'"),
                )? {
                    assign(&id, name, at, value)?;
                }
                // A change row naming the attribute with no value records that
                // it was cleared; there is no value to assign.
            }
            if let Some(changed) = changed
                && !declared.attributes.iter().any(|(name, _)| name == &changed)
            {
                return Err(OcelSqliteError::Data(format!(
                    "object '{id}' changes undeclared attribute '{changed}'"
                )));
            }
        }
    }
    Ok(())
}

fn reject_orphan_type_rows(
    connection: &Connection,
    declared: &TypeTable,
    entity_table: &str,
) -> Result<(), OcelSqliteError> {
    let sql = format!(
        "SELECT typed.ocel_id FROM {} AS typed \
         LEFT JOIN {} AS entity ON entity.ocel_id = typed.ocel_id AND entity.ocel_type = ?1 \
         WHERE entity.ocel_id IS NULL ORDER BY typed.rowid LIMIT 1",
        quote_identifier(&declared.table),
        quote_identifier(entity_table)
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query([&declared.name])?;
    if let Some(row) = rows.next()? {
        let id = sql_text(row.get_ref(0)?, &format!("{}.ocel_id", declared.table))?;
        return Err(OcelSqliteError::Data(format!(
            "row for '{id}' in table '{}' has no matching {entity_table} of type '{}'",
            declared.table, declared.name
        )));
    }
    Ok(())
}

fn read_event_object_relations(
    connection: &Connection,
    builder: &mut OcelBuilder,
) -> Result<(), OcelSqliteError> {
    require_columns(
        connection,
        "event_object",
        &["ocel_event_id", "ocel_object_id", "ocel_qualifier"],
    )?;
    let mut statement = connection.prepare(
        "SELECT ocel_event_id, ocel_object_id, ocel_qualifier FROM event_object ORDER BY rowid",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        builder.relate_event_object_or_record(
            &sql_text(row.get_ref(0)?, "event_object.ocel_event_id")?,
            &sql_text(row.get_ref(1)?, "event_object.ocel_object_id")?,
            &sql_text(row.get_ref(2)?, "event_object.ocel_qualifier")?,
        )?;
    }
    Ok(())
}

fn read_object_object_relations(
    connection: &Connection,
    builder: &mut OcelBuilder,
) -> Result<(), OcelSqliteError> {
    require_columns(
        connection,
        "object_object",
        &["ocel_source_id", "ocel_target_id", "ocel_qualifier"],
    )?;
    let mut statement = connection.prepare(
        "SELECT ocel_source_id, ocel_target_id, ocel_qualifier FROM object_object ORDER BY rowid",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        builder.relate_objects_or_record(
            &sql_text(row.get_ref(0)?, "object_object.ocel_source_id")?,
            &sql_text(row.get_ref(1)?, "object_object.ocel_target_id")?,
            &sql_text(row.get_ref(2)?, "object_object.ocel_qualifier")?,
        )?;
    }
    Ok(())
}

fn require_columns(
    connection: &Connection,
    table: &str,
    required: &[&str],
) -> Result<(), OcelSqliteError> {
    let columns = table_columns(connection, table)?;
    for required in required {
        if !columns.iter().any(|(name, _)| name == required) {
            return Err(OcelSqliteError::Schema(format!(
                "table '{table}' is missing required column '{required}'"
            )));
        }
    }
    Ok(())
}

fn table_columns(
    connection: &Connection,
    table: &str,
) -> Result<Vec<(String, String)>, OcelSqliteError> {
    let sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let rows = query_values(connection, &sql)?;
    if rows.is_empty() {
        return Err(OcelSqliteError::Schema(format!(
            "table '{table}' does not exist or has no columns"
        )));
    }
    rows.into_iter()
        .map(|row| {
            Ok((
                text(&row[1], &format!("{table} column name"))?,
                text(&row[2], &format!("{table} column type"))?,
            ))
        })
        .collect()
}

fn query_values(connection: &Connection, sql: &str) -> Result<Vec<Vec<Value>>, rusqlite::Error> {
    let mut statement = connection.prepare(sql)?;
    let width = statement.column_count();
    statement
        .query_map([], |row| {
            (0..width)
                .map(|column| row.get(column))
                .collect::<Result<Vec<Value>, _>>()
        })?
        .collect()
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn sqlite_attribute_type(value: &str) -> Option<AttributeType> {
    match value.trim().to_ascii_uppercase().as_str() {
        "TEXT" => Some(AttributeType::String),
        "TIMESTAMP" => Some(AttributeType::Time),
        "INTEGER" => Some(AttributeType::Integer),
        "REAL" => Some(AttributeType::Float),
        "BOOLEAN" => Some(AttributeType::Boolean),
        _ => None,
    }
}

/// A converted value: text as read, or any other value.
enum Plain<'a> {
    Text(&'a str),
    Value(AttributeValue),
}

fn convert_ref_value(
    builder: &mut OcelBuilder,
    value: ValueRef<'_>,
    value_type: AttributeType,
    context: &str,
) -> Result<Option<AttributeValue>, OcelSqliteError> {
    Ok(
        convert_plain(value, value_type, context)?.map(|plain| match plain {
            Plain::Text(text) => builder.string_value(text),
            Plain::Value(value) => value,
        }),
    )
}

fn convert_plain<'a>(
    value: ValueRef<'a>,
    value_type: AttributeType,
    context: &str,
) -> Result<Option<Plain<'a>>, OcelSqliteError> {
    // SQLite columns are loosely typed, and some exporters write a missing
    // number as the text 'null' or '' and numbers as text. Accept those
    // exactly; any other text in a typed column is still an error.
    if let (
        AttributeType::Integer
        | AttributeType::Float
        | AttributeType::Boolean
        | AttributeType::Time,
        ValueRef::Text(raw),
    ) = (value_type, value)
    {
        let text = utf8(raw, context)?.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("null") {
            return Ok(None);
        }
        let parsed = match value_type {
            AttributeType::Integer => text.parse().ok().map(AttributeValue::Integer),
            AttributeType::Float => text
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(AttributeValue::Float),
            _ => None,
        };
        if let Some(parsed) = parsed {
            return Ok(Some(Plain::Value(parsed)));
        }
    }
    let converted = match (value_type, value) {
        (_, ValueRef::Null) => return Ok(None),
        (AttributeType::String, ValueRef::Text(value)) => {
            return Ok(Some(Plain::Text(utf8(value, context)?)));
        }
        (AttributeType::Time, ValueRef::Text(value)) => {
            AttributeValue::Time(parse_time(utf8(value, context)?, context)?)
        }
        (AttributeType::Integer, ValueRef::Integer(value)) => AttributeValue::Integer(value),
        (AttributeType::Float, ValueRef::Real(value)) => AttributeValue::Float(value),
        (AttributeType::Float, ValueRef::Integer(value)) => AttributeValue::Float(value as f64),
        (AttributeType::Boolean, ValueRef::Integer(0)) => AttributeValue::Boolean(false),
        (AttributeType::Boolean, ValueRef::Integer(1)) => AttributeValue::Boolean(true),
        _ => {
            return Err(OcelSqliteError::Data(format!(
                "{context} expects {}, but SQLite stored {}",
                attribute_type_name(value_type),
                sqlite_ref_value_name(value)
            )));
        }
    };
    Ok(Some(Plain::Value(converted)))
}

fn text(value: &Value, context: &str) -> Result<String, OcelSqliteError> {
    match value {
        Value::Text(value) => Ok(value.clone()),
        _ => Err(OcelSqliteError::Data(format!(
            "{context} must be text, but SQLite stored {}",
            sqlite_value_name(value)
        ))),
    }
}

fn sql_text(value: ValueRef<'_>, context: &str) -> Result<String, OcelSqliteError> {
    match value {
        ValueRef::Text(value) => Ok(utf8(value, context)?.to_owned()),
        _ => Err(OcelSqliteError::Data(format!(
            "{context} must be text, but SQLite stored {}",
            sqlite_ref_value_name(value)
        ))),
    }
}

fn optional_sql_text(
    value: ValueRef<'_>,
    context: &str,
) -> Result<Option<String>, OcelSqliteError> {
    match value {
        ValueRef::Null => Ok(None),
        ValueRef::Text(value) => Ok(Some(utf8(value, context)?.to_owned())),
        _ => Err(OcelSqliteError::Data(format!(
            "{context} must be text or NULL, but SQLite stored {}",
            sqlite_ref_value_name(value)
        ))),
    }
}

fn utf8<'a>(value: &'a [u8], context: &str) -> Result<&'a str, OcelSqliteError> {
    std::str::from_utf8(value)
        .map_err(|_| OcelSqliteError::Data(format!("{context} is not valid UTF-8 text")))
}

fn parse_time(value: &str, context: &str) -> Result<i64, OcelSqliteError> {
    crate::ocel_read::ocel_time(value).ok_or_else(|| {
        OcelSqliteError::Data(format!(
            "{context} has invalid ISO 8601 timestamp '{value}'"
        ))
    })
}

fn attribute_type_name(value_type: AttributeType) -> &'static str {
    match value_type {
        AttributeType::String => "TEXT",
        AttributeType::Time => "TIMESTAMP",
        AttributeType::Integer => "INTEGER",
        AttributeType::Float => "REAL",
        AttributeType::Boolean => "BOOLEAN encoded as 0 or 1",
    }
}

fn sqlite_value_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NULL",
        Value::Integer(_) => "INTEGER",
        Value::Real(_) => "REAL",
        Value::Text(_) => "TEXT",
        Value::Blob(_) => "BLOB",
    }
}

fn sqlite_ref_value_name(value: ValueRef<'_>) -> &'static str {
    match value {
        ValueRef::Null => "NULL",
        ValueRef::Integer(_) => "INTEGER",
        ValueRef::Real(_) => "REAL",
        ValueRef::Text(_) => "TEXT",
        ValueRef::Blob(_) => "BLOB",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_ocel_json;

    const JSON: &[u8] = include_bytes!("../../../testdata/ocel/order-management.jsonocel");

    #[test]
    fn canonical_sqlite_is_equivalent_to_json_without_flattening() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.sqlite");
        assert_eq!(
            read_ocel_sqlite(path).unwrap(),
            parse_ocel_json(JSON).unwrap()
        );
    }

    #[test]
    fn rejects_a_database_without_the_ocel_schema() {
        let connection = Connection::open_in_memory().unwrap();
        let error = parse_connection(&connection).unwrap_err().to_string();
        assert!(error.contains("required table 'event' is missing"));
    }

    #[test]
    fn supports_both_official_correspondence_table_spellings() {
        for correlation in [false, true] {
            let connection = canonical_database(correlation);
            let log = parse_connection(&connection).unwrap();
            assert_eq!(log.events.len(), 3);
            assert_eq!(log.objects.len(), 4);
            assert_eq!(log.event_object_relations.len(), 7);
            assert_eq!(log.object_object_relations.len(), 3);
        }
    }

    #[test]
    fn rejects_non_boolean_integer_values() {
        let connection = canonical_database(false);
        connection
            .execute(
                "UPDATE event_DeliverPackage SET successful = 2 WHERE ocel_id = 'e-deliver'",
                [],
            )
            .unwrap();
        assert!(
            parse_connection(&connection)
                .unwrap_err()
                .to_string()
                .contains("BOOLEAN encoded as 0 or 1")
        );
    }

    #[test]
    fn blank_initial_object_times_mean_the_epoch() {
        let connection = canonical_database(false);
        connection
            .execute(
                "UPDATE object_Order SET ocel_time = NULL WHERE ocel_changed_field IS NULL",
                [],
            )
            .unwrap();
        assert_eq!(
            parse_connection(&connection).unwrap(),
            parse_connection(&canonical_database(false)).unwrap()
        );
    }

    #[test]
    fn a_missing_changed_field_column_means_plain_assignments() {
        let connection = canonical_database(false);
        connection
            .execute_batch(
                "ALTER TABLE object_Package DROP COLUMN ocel_changed_field;\n\
                 ALTER TABLE object_Item DROP COLUMN ocel_changed_field;",
            )
            .unwrap();
        assert_eq!(
            parse_connection(&connection).unwrap(),
            parse_connection(&canonical_database(false)).unwrap()
        );
    }

    #[test]
    fn null_or_numeric_text_in_number_columns_is_read_exactly() {
        let connection = canonical_database(false);
        connection
            .execute_batch(
                "UPDATE object_Item SET price = 'null' WHERE ocel_id = 'item-1';\n\
                 UPDATE object_Item SET price = '20.0' WHERE ocel_id = 'item-2';",
            )
            .unwrap();
        let log = parse_connection(&connection).unwrap();
        let canonical = parse_connection(&canonical_database(false)).unwrap();
        // item-1's price is now missing; item-2's text '20.0' reads as 20.0.
        assert_eq!(
            log.object_attributes.len(),
            canonical.object_attributes.len() - 1
        );
        connection
            .execute(
                "UPDATE object_Item SET price = 'cheap' WHERE ocel_id = 'item-2'",
                [],
            )
            .unwrap();
        assert!(parse_connection(&connection).is_err());
        // A change row naming a field with no value clears it: nothing assigned.
        let cleared = canonical_database(false);
        cleared
            .execute(
                "UPDATE object_Order SET status = NULL WHERE ocel_changed_field = 'status'",
                [],
            )
            .unwrap();
        assert_eq!(
            parse_connection(&cleared).unwrap().object_attributes.len(),
            canonical.object_attributes.len() - 1
        );
    }

    #[test]
    fn an_object_without_attribute_rows_simply_has_no_attributes() {
        // JSON and XML objects may carry no attributes; SQLite agrees.
        let connection = canonical_database(false);
        connection
            .execute("DELETE FROM object_Package", [])
            .unwrap();
        let log = parse_connection(&connection).unwrap();
        assert_eq!(log.objects.len(), 4);
        assert_eq!(
            log.object_attributes.len(),
            parse_connection(&canonical_database(false))
                .unwrap()
                .object_attributes
                .len()
        );
    }

    #[test]
    fn links_to_undeclared_objects_are_left_out_and_counted() {
        let connection = canonical_database(false);
        connection
            .execute_batch(
                "INSERT INTO event_object VALUES ('e-place','ghost','item');\n\
                 INSERT INTO object_object VALUES ('item-1','ghost','part of');",
            )
            .unwrap();
        let log = parse_connection(&connection).unwrap();
        assert_eq!(log.event_object_relations.len(), 7);
        assert_eq!(log.object_object_relations.len(), 3);
        assert_eq!(log.undeclared_links.count, 2);
        assert_eq!(
            log.undeclared_links.examples,
            ["e-place --item--> ghost", "item-1 --part of--> ghost"]
        );
        let report = crate::assess_ocel_quality(&log);
        assert!(report.ready_for_analysis);
        assert_eq!(
            report.findings[0].kind,
            crate::OcelQualityKind::UndeclaredObjectReference
        );
        assert_eq!(report.findings[0].affected, 2);
    }

    fn canonical_database(correlation: bool) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        let event_mapping = if correlation {
            "CREATE TABLE event_corr_type (ocel_type TEXT, ocel_type_corr TEXT);\n\
             INSERT INTO event_corr_type VALUES ('place order','PlaceOrder'),('create package','CreatePackage'),('deliver package','DeliverPackage');"
        } else {
            "CREATE TABLE event_map_type (ocel_type TEXT, ocel_type_map TEXT);\n\
             INSERT INTO event_map_type VALUES ('place order','PlaceOrder'),('create package','CreatePackage'),('deliver package','DeliverPackage');"
        };
        let object_mapping = if correlation {
            "CREATE TABLE object_corr_type (ocel_type TEXT, ocel_type_corr TEXT);\n\
             INSERT INTO object_corr_type VALUES ('order','Order'),('item','Item'),('package','Package');"
        } else {
            "CREATE TABLE object_map_type (ocel_type TEXT, ocel_type_map TEXT);\n\
             INSERT INTO object_map_type VALUES ('order','Order'),('item','Item'),('package','Package');"
        };
        connection
            .execute_batch(&format!(
                "{event_mapping}\n{object_mapping}\n\
                 CREATE TABLE event (ocel_id TEXT, ocel_type TEXT);\n\
                 INSERT INTO event VALUES ('e-place','place order'),('e-pack','create package'),('e-deliver','deliver package');\n\
                 CREATE TABLE object (ocel_id TEXT, ocel_type TEXT);\n\
                 INSERT INTO object VALUES ('order-1','order'),('item-1','item'),('item-2','item'),('package-1','package');\n\
                 CREATE TABLE event_PlaceOrder (ocel_id TEXT, ocel_time TIMESTAMP, 'item count' INTEGER);\n\
                 INSERT INTO event_PlaceOrder VALUES ('e-place','1970-01-01 00:00:00',2);\n\
                 CREATE TABLE event_CreatePackage (ocel_id TEXT, ocel_time TIMESTAMP);\n\
                 INSERT INTO event_CreatePackage VALUES ('e-pack','1970-01-02T00:00:00Z');\n\
                 CREATE TABLE event_DeliverPackage (ocel_id TEXT, ocel_time TIMESTAMP, successful BOOLEAN);\n\
                 INSERT INTO event_DeliverPackage VALUES ('e-deliver','1970-01-03 00:00:00',1);\n\
                 CREATE TABLE object_Order (ocel_id TEXT, ocel_time TIMESTAMP, ocel_changed_field TEXT, status TEXT);\n\
                 INSERT INTO object_Order VALUES ('order-1','1970-01-01 00:00:00',NULL,'open'),('order-1','1970-01-02 00:00:00','status','shipped');\n\
                 CREATE TABLE object_Item (ocel_id TEXT, ocel_time TIMESTAMP, ocel_changed_field TEXT, price REAL);\n\
                 INSERT INTO object_Item VALUES ('item-1','1970-01-01 00:00:00',NULL,10.5),('item-2','1970-01-01 00:00:00',NULL,20.0);\n\
                 CREATE TABLE object_Package (ocel_id TEXT, ocel_time TIMESTAMP, ocel_changed_field TEXT);\n\
                 INSERT INTO object_Package VALUES ('package-1','1970-01-01 00:00:00',NULL);\n\
                 CREATE TABLE event_object (ocel_event_id TEXT, ocel_object_id TEXT, ocel_qualifier TEXT);\n\
                 INSERT INTO event_object VALUES\n\
                   ('e-place','order-1','order'),('e-place','item-1','item'),('e-place','item-2','item'),\n\
                   ('e-pack','package-1','result'),('e-pack','item-1','packed item'),('e-pack','item-2','packed item'),\n\
                   ('e-deliver','package-1','package');\n\
                 CREATE TABLE object_object (ocel_source_id TEXT, ocel_target_id TEXT, ocel_qualifier TEXT);\n\
                 INSERT INTO object_object VALUES ('item-1','order-1','part of'),('item-1','package-1','packed in'),('item-2','order-1','part of');"
            ))
            .unwrap();
        connection
    }
}

/// One written step when following an item type from disk.
pub struct FollowedStep<'a> {
    pub case: &'a str,
    pub activity: &'a str,
    pub at: i64,
    /// Position of the step within its item, from 1.
    pub position: u64,
    /// The item's details as of its last step, in [`FollowedLog::attributes`] order.
    pub details: &'a [String],
}

/// What following an item type from disk produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowedLog {
    pub attributes: Vec<String>,
    /// Items of the followed type.
    pub cases: u64,
    pub events: u64,
    pub excluded_events: u64,
    pub shared_events: u64,
    pub shared_event_copies: u64,
}

/// The readable label of a prepared identity tuple `["ns","type",["k1","k2"]]`:
/// its key values joined by a space; `None` for any other identifier.
pub fn short_label(id: &str) -> Option<String> {
    serde_json::from_str::<(String, String, Vec<String>)>(id)
        .ok()
        .map(|(_, _, keys)| keys.join(" "))
}

fn rfc3339(micros: i64) -> String {
    chrono::DateTime::from_timestamp_micros(micros)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
        .unwrap_or_default()
}

/// An attribute value as following writes it, or `None` when it assigns
/// nothing. Mirrors the reader's conversion and the follow text format.
fn value_text(
    value: ValueRef<'_>,
    value_type: AttributeType,
    context: &str,
) -> Result<Option<String>, OcelSqliteError> {
    if let (
        AttributeType::Integer
        | AttributeType::Float
        | AttributeType::Boolean
        | AttributeType::Time,
        ValueRef::Text(raw),
    ) = (value_type, value)
    {
        let text = utf8(raw, context)?.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("null") {
            return Ok(None);
        }
        let parsed = match value_type {
            AttributeType::Integer => text.parse::<i64>().ok().map(|v| v.to_string()),
            AttributeType::Float => text
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(|v| v.to_string()),
            _ => None,
        };
        if parsed.is_some() {
            return Ok(parsed);
        }
    }
    Ok(Some(match (value_type, value) {
        (_, ValueRef::Null) => return Ok(None),
        (AttributeType::String, ValueRef::Text(value)) => utf8(value, context)?.to_owned(),
        (AttributeType::Time, ValueRef::Text(value)) => {
            rfc3339(parse_time(utf8(value, context)?, context)?)
        }
        (AttributeType::Integer, ValueRef::Integer(value)) => value.to_string(),
        (AttributeType::Float, ValueRef::Real(value)) => value.to_string(),
        (AttributeType::Float, ValueRef::Integer(value)) => (value as f64).to_string(),
        (AttributeType::Boolean, ValueRef::Integer(0)) => "false".into(),
        (AttributeType::Boolean, ValueRef::Integer(1)) => "true".into(),
        _ => {
            return Err(OcelSqliteError::Data(format!(
                "{context} expects {}, but SQLite stored {}",
                attribute_type_name(value_type),
                sqlite_ref_value_name(value)
            )));
        }
    }))
}
