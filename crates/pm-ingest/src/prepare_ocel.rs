//! Reviewed relational CSV -> OCEL construction, independent of source vendors.
//! Keys are exact strings. Source rows stream; entity indexes and the built
//! graph reside in memory without an artificial entity-count ceiling.
//!
//! The graph is held as compact typed records: names that repeat on every row
//! (types, activities, qualifiers, rules, input hashes) are stored once and
//! referred to by number, and links point at objects by position. The OCEL
//! JSON and the lineage are written straight from these records, so no
//! general-purpose JSON copy of the whole log is ever built.
use crate::prepare::{ActivitySource, InputReference};
use chrono::{DateTime, FixedOffset};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, RandomState};
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, thiserror::Error)]
#[error("connected preparation: {0}")]
pub struct Error(String);
type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub version: u32,
    pub format: String,
    pub inputs: Vec<InputReference>,
    pub objects: Vec<ObjectRule>,
    pub events: Vec<EventRule>,
    #[serde(default)]
    pub relationships: Vec<RelationshipRule>,
    #[serde(default)]
    pub event_links: Vec<EventLinkRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried_links: Vec<CarriedLinkRule>,
    /// Records that do not fit (a repeated key, a reference that finds
    /// nothing, a missing time) are set aside instead of blocking: each such
    /// row is left out, along with anything that depended on it, and listed
    /// in the report. Only with the person's approval.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub set_aside: bool,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Attribute {
    pub column: String,
    pub name: String,
}
/// Keeps only rows whose column value is (or is not) one of the listed
/// exact strings, e.g. SAP purchase-order history rows of one kind.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub column: String,
    #[serde(default, rename = "in", skip_serializing_if = "Vec::is_empty")]
    pub any_of: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_in: Vec<String>,
}
impl Condition {
    pub(crate) fn holds(&self, value: &str) -> bool {
        if self.any_of.is_empty() {
            !self.not_in.iter().any(|v| v == value)
        } else {
            self.any_of.iter().any(|v| v == value)
        }
    }
}
/// One part of a reference key: a whole column, or a fixed character range
/// of one, such as the first ten characters of SAP's `AWKEY`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum KeyPart {
    Column(String),
    Slice(KeySlice),
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeySlice {
    pub column: String,
    pub start: usize,
    pub length: usize,
}
impl KeyPart {
    /// This part's value from its column's text.
    pub(crate) fn take(&self, value: &str) -> String {
        match self {
            Self::Column(_) => value.to_owned(),
            Self::Slice(slice) => value.chars().skip(slice.start).take(slice.length).collect(),
        }
    }
    pub fn column(&self) -> &str {
        match self {
            Self::Column(column) => column,
            Self::Slice(slice) => &slice.column,
        }
    }
}
/// Where an event's time comes from: one RFC3339 column, or a date column
/// with an optional time-of-day column and an approved UTC offset, as ERP
/// exports usually store them (`20260314` and `093000`).
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Timestamp {
    Column(String),
    Parts(TimestampParts),
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimestampParts {
    pub date: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    pub offset: String,
}
impl Timestamp {
    fn columns(&self) -> Vec<String> {
        match self {
            Self::Column(column) => vec![column.clone()],
            Self::Parts(parts) => std::iter::once(parts.date.clone())
                .chain(parts.time.clone())
                .collect(),
        }
    }
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectRule {
    pub id: String,
    pub input: String,
    pub namespace: String,
    pub object_type: String,
    pub key: Vec<String>,
    #[serde(default)]
    pub attributes: Vec<Attribute>,
    #[serde(default, rename = "where", skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<Condition>,
    /// Rows naming an object an earlier rule (or row) already made are
    /// skipped rather than an issue: a second source for the same objects,
    /// such as order lines known only from their posted shipments once the
    /// order itself is gone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub existing_ok: bool,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectRef {
    pub object: String,
    pub columns: Vec<KeyPart>,
    pub qualifier: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub allow_unmatched: bool,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventRule {
    pub id: String,
    pub input: String,
    pub namespace: String,
    pub key: Vec<String>,
    pub activity: ActivitySource,
    pub timestamp: Timestamp,
    #[serde(default)]
    pub optional_timestamp: bool,
    pub objects: Vec<ObjectRef>,
    #[serde(default)]
    pub attributes: Vec<Attribute>,
    #[serde(default, rename = "where", skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<Condition>,
    /// Columns read from the one matching row of another input, such as a
    /// change's time from the change header beside each changed field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lookup: Option<Lookup>,
    /// Additional many-to-one joins, applied after `lookup` in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lookups: Vec<Lookup>,
}
/// A many-to-one lookup: each row's `matching` columns must equal the `key`
/// of exactly one row of `input`, whose `columns` the rule may then use. A
/// looked-up column replaces the row's own column of the same name, such as
/// an approval's draft number replaced by the number of the document the
/// draft became.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Lookup {
    pub input: String,
    pub key: Vec<String>,
    pub matching: Vec<String>,
    pub columns: Vec<String>,
    /// Rows with no match are skipped and counted instead of blocking.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    /// Only these source rows participate in the many-to-one lookup.
    #[serde(default, rename = "where", skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<Condition>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipRule {
    pub id: String,
    pub input: String,
    pub source: ObjectRef,
    pub target: ObjectRef,
    #[serde(default, rename = "where", skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<Condition>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventLinkRule {
    pub id: String,
    pub input: String,
    pub event: String,
    pub event_key: Vec<String>,
    pub target: ObjectRef,
    /// Bridge rows whose event this recipe did not emit are skipped and
    /// counted instead of blocking, e.g. accounting documents not yet paid.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional_event: bool,
    #[serde(default, rename = "where", skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<Condition>,
}
/// Carries an event's link one step further along links already built: an
/// event linked to an object of rule `through` is also linked to every object
/// that the relationship rule `relationship` links from that object. For
/// example, a payment linked to an invoice also reaches the order lines that
/// the invoice's lines charge. Only reconciled links are followed.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CarriedLinkRule {
    pub id: String,
    pub event: String,
    pub through: String,
    pub relationship: String,
    pub qualifier: String,
}
#[derive(Debug, Default, Serialize)]
pub struct Counts {
    pub rows: u64,
    pub emitted: u64,
    pub skipped: u64,
    pub unmatched: u64,
    pub omitted_blank_links: u64,
    /// Rows left out by the rule's `where` conditions.
    #[serde(skip_serializing_if = "is_zero")]
    pub filtered: u64,
    /// Rows set aside because they did not fit (see [`Recipe::set_aside`]).
    #[serde(skip_serializing_if = "is_zero")]
    pub set_aside: u64,
}
fn is_zero(value: &u64) -> bool {
    *value == 0
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub rule: String,
    pub row: u64,
    pub code: String,
}
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub rules: BTreeMap<String, Counts>,
    pub objects: usize,
    pub events: usize,
    pub object_relationships: usize,
    pub event_relationships: usize,
    pub issue_count: u64,
    pub issues: Vec<Issue>,
    pub issues_truncated: bool,
    /// Rows set aside with the person's approval, by rule and reason.
    #[serde(skip_serializing_if = "is_zero")]
    pub set_aside_count: u64,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub set_aside_by_reason: BTreeMap<String, u64>,
    /// The first rows set aside, as examples; the outcome holds them all.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub set_aside: Vec<Issue>,
    /// Row-level issues with the rule scope they can be set aside by,
    /// collected only when setting aside.
    #[serde(skip)]
    rows: Option<Vec<(String, Issue)>>,
}
impl Report {
    /// Records the rows set aside before this build.
    fn set_aside_rows(&mut self, rows: &[Issue]) {
        self.set_aside_count = rows.len() as u64;
        for issue in rows {
            *self
                .set_aside_by_reason
                .entry(format!("{}: {}", issue.rule, issue.code))
                .or_default() += 1;
        }
        self.set_aside = rows.iter().take(50).cloned().collect();
    }
    fn issue(&mut self, rule: &str, row: u64, code: &str) {
        self.issue_in(rule, rule, row, code);
    }
    /// An issue whose row belongs to `scope` (a rule, or a rule's lookup).
    fn issue_in(&mut self, scope: &str, rule: &str, row: u64, code: &str) {
        if let Some(rows) = self.rows.as_mut().filter(|_| row > 0) {
            rows.push((
                scope.to_owned(),
                Issue {
                    rule: rule.into(),
                    row,
                    code: code.into(),
                },
            ));
        }
        self.issue_count += 1;
        if self.issues.len() < 50 {
            self.issues.push(Issue {
                rule: rule.into(),
                row,
                code: code.into(),
            });
        } else {
            self.issues_truncated = true;
        }
    }
}
pub struct Outcome {
    pub report: Report,
    /// Every row set aside, in the order found.
    pub set_aside: Vec<Issue>,
    /// The OCEL JSON log. Absent when any unresolved reconciliation issue
    /// blocks publication.
    pub log: Option<Log>,
    /// One record per emitted link, serialized as a JSON array.
    pub relationship_lineage: Lineage,
}

/// Names that repeat on many rows, stored once and referred to by number.
#[derive(Default)]
struct Names {
    index: HashMap<Rc<str>, u32>,
    names: Vec<Rc<str>>,
}
impl Names {
    fn id(&mut self, name: &str) -> u32 {
        if let Some(id) = self.index.get(name) {
            return *id;
        }
        let shared: Rc<str> = Rc::from(name);
        let id = self.names.len() as u32;
        self.names.push(Rc::clone(&shared));
        self.index.insert(shared, id);
        id
    }
    fn get(&self, id: u32) -> &str {
        &self.names[id as usize]
    }
}

/// One object or event.
struct Entity {
    id: Box<str>,
    /// Object type or activity name.
    kind: u32,
    /// Events only.
    time: Option<DateTime<FixedOffset>>,
    /// Position of the object or event rule that emitted it.
    rule: u16,
    row: u64,
    /// Attribute values, in the rule's attribute order, as names: most values
    /// (plants, materials, statuses, users) repeat on many rows.
    values: Box<[u32]>,
    /// (object position, qualifier name).
    links: Vec<(u32, u32)>,
}

/// Where one emitted link came from.
#[derive(Clone, Copy)]
struct LinkRecord {
    from_event: bool,
    source: u32,
    target: u32,
    qualifier: u32,
    /// Input hash name and rule id name.
    input: u32,
    rule: u32,
    row: u64,
}

/// Link records kept in a scratch file, not in memory: there is one per
/// link, and they are needed only to write the lineage at the end (and to
/// look up a relationship's links for carried links).
struct LineageFile {
    path: PathBuf,
    writer: Option<std::io::BufWriter<std::fs::File>>,
    len: usize,
}
const RECORD: usize = 33;
impl LineageFile {
    fn new() -> Result<Self> {
        static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "pm-lineage-{}-{}.bin",
            std::process::id(),
            RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let file = std::fs::File::create(&path)
            .map_err(|_| Error("cannot create a scratch file for link lineage".into()))?;
        Ok(Self {
            path,
            writer: Some(std::io::BufWriter::with_capacity(1 << 20, file)),
            len: 0,
        })
    }
    fn push(&mut self, record: LinkRecord) {
        use std::io::Write;
        let mut bytes = [0u8; RECORD];
        bytes[0] = u8::from(record.from_event);
        for (at, value) in [
            record.source,
            record.target,
            record.qualifier,
            record.input,
            record.rule,
        ]
        .into_iter()
        .enumerate()
        {
            bytes[1 + at * 4..5 + at * 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[21..29].copy_from_slice(&record.row.to_le_bytes());
        // Writing to the scratch file can only fail on a full disk; the
        // record count then disagrees with the file and reading stops early.
        if self
            .writer
            .as_mut()
            .is_some_and(|writer| writer.write_all(&bytes).is_ok())
        {
            self.len += 1;
        }
    }
    /// Every record written so far, in order.
    fn records(&mut self) -> impl Iterator<Item = LinkRecord> {
        use std::io::{Read, Write};
        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.flush();
        }
        let len = self.len;
        let mut reader = std::fs::File::open(&self.path)
            .ok()
            .map(|file| std::io::BufReader::with_capacity(1 << 20, file));
        (0..len).map_while(move |_| {
            let mut bytes = [0u8; RECORD];
            reader.as_mut()?.read_exact(&mut bytes).ok()?;
            let word =
                |at: usize| u32::from_le_bytes(bytes[1 + at * 4..5 + at * 4].try_into().unwrap());
            Some(LinkRecord {
                from_event: bytes[0] == 1,
                source: word(0),
                target: word(1),
                qualifier: word(2),
                input: word(3),
                rule: word(4),
                row: u64::from_le_bytes(bytes[21..29].try_into().unwrap()),
            })
        })
    }
    fn finish(&mut self) {
        use std::io::Write;
        if let Some(mut writer) = self.writer.take() {
            let _ = writer.flush();
        }
    }
}
impl Drop for LineageFile {
    fn drop(&mut self) {
        self.writer.take();
        let _ = std::fs::remove_file(&self.path);
    }
}

struct Graph {
    names: Names,
    objects: Vec<Entity>,
    events: Vec<Entity>,
    /// Per object rule, then per event rule: attribute names and input hash name.
    object_fields: Vec<(Vec<String>, u32)>,
    event_fields: Vec<(Vec<String>, u32)>,
    object_types: Vec<Value>,
    event_types: Vec<Value>,
    lineage: std::cell::RefCell<LineageFile>,
}

mod disk;
pub use disk::prepare_on_disk;

/// A built graph, in memory or on disk.
#[derive(Clone)]
enum Built {
    Memory(Rc<Graph>),
    Disk(Rc<disk::DiskGraph>),
}

impl Built {
    fn graph(&self) -> &dyn Graphs {
        match self {
            Self::Memory(graph) => &**graph,
            Self::Disk(graph) => &**graph,
        }
    }
}

/// The connected log, serialized as the official four-array OCEL JSON shape.
pub struct Log(Built);
/// The relationship lineage, serialized as a JSON array of link records.
pub struct Lineage(Built);

/// One item or step as written out: identity, type, time, the rule and row
/// that emitted it, attribute values in the rule's order, and its links as
/// (target identity, qualifier).
struct EntityRow {
    id: String,
    kind: String,
    time: Option<String>,
    rule: u16,
    row: u64,
    values: Vec<String>,
    links: Vec<(String, String)>,
}

/// What writing a built graph needs, from memory or from disk alike.
trait Graphs {
    fn names_of(&self, id: u32) -> String;
    fn fields(&self, events: bool) -> &[(Vec<String>, u32)];
    fn types(&self, events: bool) -> &[Value];
    fn count(&self, events: bool) -> usize;
    /// Every item (or step) in position order.
    fn each(&self, events: bool, f: &mut dyn FnMut(EntityRow) -> Result<()>) -> Result<()>;
    /// The identity of the item (or step) at a position.
    fn identity_at(&self, events: bool, position: u32) -> Result<String>;
    fn lineage_file(&self) -> std::cell::RefMut<'_, LineageFile>;
}

impl Graphs for Graph {
    fn names_of(&self, id: u32) -> String {
        self.names.get(id).to_owned()
    }
    fn fields(&self, events: bool) -> &[(Vec<String>, u32)] {
        if events {
            &self.event_fields
        } else {
            &self.object_fields
        }
    }
    fn types(&self, events: bool) -> &[Value] {
        if events {
            &self.event_types
        } else {
            &self.object_types
        }
    }
    fn count(&self, events: bool) -> usize {
        if events {
            self.events.len()
        } else {
            self.objects.len()
        }
    }
    fn each(&self, events: bool, f: &mut dyn FnMut(EntityRow) -> Result<()>) -> Result<()> {
        let entities = if events { &self.events } else { &self.objects };
        for entity in entities {
            f(EntityRow {
                id: entity.id.to_string(),
                kind: self.names.get(entity.kind).to_owned(),
                time: entity.time.map(|time| time.to_rfc3339()),
                rule: entity.rule,
                row: entity.row,
                values: entity
                    .values
                    .iter()
                    .map(|value| self.names.get(*value).to_owned())
                    .collect(),
                links: entity
                    .links
                    .iter()
                    .map(|(target, qualifier)| {
                        (
                            self.objects[*target as usize].id.to_string(),
                            self.names.get(*qualifier).to_owned(),
                        )
                    })
                    .collect(),
            })?;
        }
        Ok(())
    }
    fn identity_at(&self, events: bool, position: u32) -> Result<String> {
        let entities = if events { &self.events } else { &self.objects };
        Ok(entities[position as usize].id.to_string())
    }
    fn lineage_file(&self) -> std::cell::RefMut<'_, LineageFile> {
        self.lineage.borrow_mut()
    }
}

impl Serialize for Log {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let graph = self.0.graph();
        // Keys in sorted order, matching serde_json's own map ordering.
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("eventTypes", graph.types(true))?;
        map.serialize_entry("events", &Entities(graph, true))?;
        map.serialize_entry("objectTypes", graph.types(false))?;
        map.serialize_entry("objects", &Entities(graph, false))?;
        map.end()
    }
}

struct Entities<'a>(&'a dyn Graphs, bool);
impl Serialize for Entities<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::Error as _;
        let (graph, event) = (self.0, self.1);
        let mut seq = serializer.serialize_seq(Some(graph.count(event)))?;
        let mut failed = None;
        graph
            .each(event, &mut |entity| {
                if failed.is_none()
                    && let Err(error) = seq.serialize_element(&EntityView(graph, &entity, event))
                {
                    failed = Some(error);
                }
                Ok(())
            })
            .map_err(|error| S::Error::custom(error.to_string()))?;
        if let Some(error) = failed {
            return Err(error);
        }
        seq.end()
    }
}

struct EntityView<'a>(&'a dyn Graphs, &'a EntityRow, bool);
impl Serialize for EntityView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let (graph, entity, event) = (self.0, self.1, self.2);
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("attributes", &AttributesView(graph, entity, event))?;
        map.serialize_entry("id", &entity.id)?;
        map.serialize_entry("relationships", &LinksView(entity))?;
        if let Some(time) = &entity.time {
            map.serialize_entry("time", time)?;
        }
        map.serialize_entry("type", &entity.kind)?;
        map.end()
    }
}

struct AttributesView<'a>(&'a dyn Graphs, &'a EntityRow, bool);
impl Serialize for AttributesView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let (graph, entity, event) = (self.0, self.1, self.2);
        let (names, input) = &graph.fields(event)[entity.rule as usize];
        // Object attributes are static values at OCEL's initial epoch.
        let time = (!event).then_some("1970-01-01T00:00:00Z");
        let mut seq = serializer.serialize_seq(Some(names.len() + 2))?;
        for (name, value) in names.iter().zip(entity.values.iter()) {
            seq.serialize_element(&AttributeView(name, time, AttributeValue::Text(value)))?;
        }
        let input = graph.names_of(*input);
        seq.serialize_element(&AttributeView(
            "_source_input",
            time,
            AttributeValue::Text(&input),
        ))?;
        seq.serialize_element(&AttributeView(
            "_source_row",
            time,
            AttributeValue::Row(entity.row),
        ))?;
        seq.end()
    }
}

enum AttributeValue<'a> {
    Text(&'a str),
    Row(u64),
}
struct AttributeView<'a>(&'a str, Option<&'a str>, AttributeValue<'a>);
impl Serialize for AttributeView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("name", self.0)?;
        if let Some(time) = self.1 {
            map.serialize_entry("time", time)?;
        }
        match &self.2 {
            AttributeValue::Text(value) => map.serialize_entry("value", value)?,
            AttributeValue::Row(row) => map.serialize_entry("value", row)?,
        }
        map.end()
    }
}

struct LinksView<'a>(&'a EntityRow);
impl Serialize for LinksView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.links.len()))?;
        for (target, qualifier) in &self.0.links {
            seq.serialize_element(&LinkView(target, qualifier))?;
        }
        seq.end()
    }
}
struct LinkView<'a>(&'a str, &'a str);
impl Serialize for LinkView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("objectId", self.0)?;
        map.serialize_entry("qualifier", self.1)?;
        map.end()
    }
}

impl Lineage {
    pub fn len(&self) -> usize {
        self.0.graph().lineage_file().len
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
impl Serialize for Lineage {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::Error as _;
        let graph = self.0.graph();
        let mut lineage = graph.lineage_file();
        let mut seq = serializer.serialize_seq(Some(lineage.len))?;
        for record in lineage.records() {
            let source = graph
                .identity_at(record.from_event, record.source)
                .map_err(|error| S::Error::custom(error.to_string()))?;
            let target = graph
                .identity_at(false, record.target)
                .map_err(|error| S::Error::custom(error.to_string()))?;
            seq.serialize_element(&LineageView(graph, &record, &source, &target))?;
        }
        seq.end()
    }
}
struct LineageView<'a>(&'a dyn Graphs, &'a LinkRecord, &'a str, &'a str);
impl Serialize for LineageView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let (graph, record) = (self.0, self.1);
        let mut map = serializer.serialize_map(Some(7))?;
        map.serialize_entry("input_sha256", &graph.names_of(record.input))?;
        map.serialize_entry("qualifier", &graph.names_of(record.qualifier))?;
        map.serialize_entry("row", &record.row)?;
        map.serialize_entry("rule", &graph.names_of(record.rule))?;
        map.serialize_entry("source", self.2)?;
        map.serialize_entry(
            "source_kind",
            if record.from_event { "event" } else { "object" },
        )?;
        map.serialize_entry("target", self.3)?;
        map.end()
    }
}
fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error(message.into()))
    }
}
fn label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
fn key_valid(k: &[String]) -> bool {
    !k.is_empty()
        && k.len() <= 8
        && k.iter().all(|s| label(s))
        && k.iter().collect::<BTreeSet<_>>().len() == k.len()
}
fn attributes_valid(attrs: &[Attribute]) -> bool {
    attrs.len() <= 32
        && attrs
            .iter()
            .all(|a| label(&a.column) && label(&a.name) && !a.name.starts_with("_source_"))
        && attrs.iter().map(|a| &a.name).collect::<BTreeSet<_>>().len() == attrs.len()
}
fn parts_valid(parts: &[KeyPart]) -> bool {
    !parts.is_empty()
        && parts.len() <= 8
        && parts.iter().all(|p| {
            label(p.column()) && !matches!(p, KeyPart::Slice(s) if s.length == 0 || s.length > 256)
        })
}
fn conditions_valid(conditions: &[Condition]) -> bool {
    conditions.len() <= 8
        && conditions.iter().all(|c| {
            label(&c.column)
                && (c.any_of.is_empty() != c.not_in.is_empty())
                && c.any_of.len() + c.not_in.len() <= 64
        })
}
fn timestamp_valid(timestamp: &Timestamp) -> bool {
    match timestamp {
        Timestamp::Column(column) => label(column),
        Timestamp::Parts(parts) => {
            label(&parts.date)
                && parts.time.as_deref().is_none_or(label)
                && parts.time.as_deref() != Some(parts.date.as_str())
                && offset(&parts.offset).is_some()
        }
    }
}
fn offset(text: &str) -> Option<FixedOffset> {
    if text == "Z" {
        return FixedOffset::east_opt(0);
    }
    DateTime::parse_from_rfc3339(&format!("2000-01-01T00:00:00{text}"))
        .ok()
        .map(|time| *time.offset())
}
/// Blank, or all zeros as ERP systems write for "no date".
pub(crate) fn blank_date(text: &str) -> bool {
    let text = text.trim();
    text.is_empty() || text.bytes().all(|b| b == b'0' || b == b'-' || b == b'.')
}
/// A date as ERP exports write it: `YYYYMMDD`, `YYYY-MM-DD`, `DD.MM.YYYY`,
/// or JD Edwards' Julian `CYYDDD` (`126014` is 14 January 2026).
pub(crate) fn erp_date(text: &str) -> Option<chrono::NaiveDate> {
    let text = text.trim();
    ["%Y%m%d", "%Y-%m-%d", "%d.%m.%Y"]
        .iter()
        .find_map(|format| chrono::NaiveDate::parse_from_str(text, format).ok())
        .or_else(|| {
            DateTime::parse_from_rfc3339(text)
                .ok()
                .map(|at| at.date_naive())
        })
        .or_else(|| erp_date_time(text).map(|at| at.date()))
        .or_else(|| julian_date(text))
}

/// JD Edwards' `CYYDDD`: century since 1900, year in the century, day of
/// the year. Five digits are a date in the 1900s.
fn julian_date(text: &str) -> Option<chrono::NaiveDate> {
    if !(5..=6).contains(&text.len()) || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: i32 = text.parse().ok()?;
    let year = 1900 + number / 1000;
    let day = u32::try_from(number % 1000).ok()?;
    chrono::NaiveDate::from_yo_opt(year, day)
}
/// A local date and time in one field: `2026-03-14 09:30:00`,
/// `2026-03-14T09:30:00` or `14.03.2026 09:30:00`, with or without fractions
/// of a second as database exports write them (`2026-03-14 00:00:00.000`).
fn erp_date_time(text: &str) -> Option<chrono::NaiveDateTime> {
    let text = text.trim();
    [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%d.%m.%Y %H:%M:%S",
        "%Y%m%d%H%M%S%.f",
    ]
    .iter()
    .find_map(|format| chrono::NaiveDateTime::parse_from_str(text, format).ok())
}
/// A time of day: `093000`, `09:30:00`, or a whole number that lost its
/// leading zeros, as some systems store times (`93000` is 09:30:00).
fn erp_time(text: &str) -> Option<chrono::NaiveTime> {
    let padded;
    let text = if !text.is_empty() && text.len() < 6 && text.bytes().all(|b| b.is_ascii_digit()) {
        padded = format!("{text:0>6}");
        padded.as_str()
    } else {
        text
    };
    ["%H%M%S", "%H:%M:%S"]
        .iter()
        .find_map(|format| chrono::NaiveTime::parse_from_str(text, format).ok())
}
enum Time {
    Blank,
    Invalid,
    At(DateTime<FixedOffset>),
}
/// Reads an event time. A date of all zeros, as ERP systems write for "no
/// date", counts as blank.
fn event_time(row: &Row<'_>, timestamp: &Timestamp) -> Result<Time> {
    let parts = match timestamp {
        Timestamp::Column(column) => {
            let text = row.get(column)?;
            return Ok(if text.trim().is_empty() {
                Time::Blank
            } else {
                DateTime::parse_from_rfc3339(text).map_or(Time::Invalid, Time::At)
            });
        }
        Timestamp::Parts(parts) => parts,
    };
    let date = row.get(&parts.date)?;
    if blank_date(date) {
        return Ok(Time::Blank);
    }
    // A date column may carry the time itself, as reporting exports often do.
    if parts.time.is_none() {
        if let Ok(at) = DateTime::parse_from_rfc3339(date) {
            return Ok(Time::At(at));
        }
        if let Some(at) = erp_date_time(date) {
            return Ok(offset(&parts.offset)
                .and_then(|zone| at.and_local_timezone(zone).single())
                .map_or(Time::Invalid, Time::At));
        }
    }
    let date = erp_date(date);
    let time = match &parts.time {
        None => Some(chrono::NaiveTime::MIN),
        Some(column) => {
            let text = row.get(column)?.trim();
            if text.is_empty() {
                Some(chrono::NaiveTime::MIN)
            } else {
                erp_time(text)
            }
        }
    };
    let (Some(date), Some(time), Some(zone)) = (date, time, offset(&parts.offset)) else {
        return Ok(Time::Invalid);
    };
    Ok(date
        .and_time(time)
        .and_local_timezone(zone)
        .single()
        .map_or(Time::Invalid, Time::At))
}
fn identity(namespace: &str, kind: &str, keys: &[String]) -> String {
    // JSON tuples avoid delimiter collisions and preserve leading zeros.
    json!([namespace, kind, keys]).to_string()
}
struct Row<'a> {
    headers: &'a csv::StringRecord,
    record: csv::StringRecord,
    /// Columns brought in by a lookup.
    extra: Vec<(String, String)>,
}
impl Row<'_> {
    /// A column's value; a column brought in by a lookup stands in for the
    /// row's own column of the same name.
    fn get(&self, name: &str) -> Result<&str> {
        if let Some((_, value)) = self.extra.iter().rev().find(|(column, _)| column == name) {
            return Ok(value);
        }
        let Some(i) = self.headers.iter().position(|h| h == name) else {
            return Err(Error(format!("missing selected column '{name}'")));
        };
        self.record
            .get(i)
            .ok_or_else(|| Error("invalid CSV width".into()))
    }
    /// A reference key; a slice takes characters, not bytes.
    fn parts(&self, parts: &[KeyPart]) -> Result<Option<Vec<String>>> {
        let values = parts
            .iter()
            .map(|part| Ok(part.take(self.get(part.column())?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(if values.iter().any(|v| v.trim().is_empty()) {
            None
        } else {
            Some(values)
        })
    }
    fn key(&self, cols: &[String]) -> Result<Option<Vec<String>>> {
        let values = cols
            .iter()
            .map(|c| self.get(c).map(str::to_owned))
            .collect::<Result<Vec<_>>>()?;
        Ok(if values.iter().any(|v| v.trim().is_empty()) {
            None
        } else {
            Some(values)
        })
    }
}
/// Streams the input's rows that meet `filter` and returns how many it left out.
fn rows(
    input: &str,
    inputs: &BTreeMap<String, PathBuf>,
    required: &[String],
    filter: &[Condition],
    mut f: impl FnMut(u64, Row<'_>) -> Result<()>,
) -> Result<u64> {
    let path = inputs
        .get(input)
        .ok_or_else(|| Error("input was not resolved".into()))?;
    let mut reader =
        csv::Reader::from_path(path).map_err(|_| Error("cannot read CSV input".into()))?;
    // As the pack check reads them: a byte-order mark (Excel's "CSV UTF-8")
    // is not part of the first column's name.
    let headers = reader
        .headers()
        .map_err(|_| Error("invalid CSV headers".into()))?
        .iter()
        .map(|h| h.trim_start_matches('\u{feff}'))
        .collect::<csv::StringRecord>();
    // A repeated column name is ambiguous only where it is used; exports
    // often repeat captions of columns nothing reads.
    for name in required.iter().chain(filter.iter().map(|c| &c.column)) {
        let found = headers.iter().filter(|h| *h == name).count();
        require(found > 0, &format!("missing selected column '{name}'"))?;
        require(
            found == 1,
            &format!("column '{name}' appears more than once"),
        )?;
    }
    let filter = filter
        .iter()
        .map(|c| (headers.iter().position(|h| h == c.column).unwrap(), c))
        .collect::<Vec<_>>();
    let mut filtered = 0;
    for (i, record) in reader.records().enumerate() {
        let record = record.map_err(|_| Error("invalid CSV record".into()))?;
        if !filter
            .iter()
            .all(|(at, c)| c.holds(record.get(*at).unwrap_or_default()))
        {
            filtered += 1;
            continue;
        }
        f(
            i as u64 + 1,
            Row {
                headers: &headers,
                record,
                extra: Vec::new(),
            },
        )?;
    }
    Ok(filtered)
}
fn defs(attrs: &[Attribute]) -> Vec<Value> {
    let mut result = attrs
        .iter()
        .map(|a| json!({"name":a.name,"type":"string"}))
        .collect::<Vec<_>>();
    result.extend([
        json!({"name":"_source_input","type":"string"}),
        json!({"name":"_source_row","type":"integer"}),
    ]);
    result.sort_by_key(|v| v["name"].as_str().unwrap().to_owned());
    result
}
fn register_type(
    types: &mut BTreeMap<String, Vec<Value>>,
    name: &str,
    fields: &[Attribute],
) -> Result<()> {
    let definition = defs(fields);
    if let Some(existing) = types.get(name) {
        require(
            *existing == definition,
            "conflicting attribute definitions for one type",
        )?;
    } else {
        types.insert(name.into(), definition);
    }
    Ok(())
}
/// Called with each carried link: the store, the step's position, the
/// target item, and the input and row the link came from.
type Carry<'a, S> = dyn FnMut(&mut S, u32, u32, u32, u64) -> Result<()> + 'a;

/// Where a build keeps its items, steps and links: in memory, or in a
/// scratch database on disk for inputs larger than memory. The build itself
/// is the same code either way, so both give the same log.
trait Store {
    /// A name that repeats on many rows: a type, activity, qualifier, rule
    /// or input.
    fn name(&mut self, name: &str) -> u32;
    /// Adds an item unless its identity is taken; its position otherwise.
    fn add_object(
        &mut self,
        id: String,
        kind: u32,
        rule: u16,
        row: u64,
        values: &[&str],
    ) -> Result<Option<u32>>;
    fn find_object(&self, id: &str) -> Result<Option<u32>>;
    /// Adds a step unless its identity is taken; its position otherwise.
    #[allow(clippy::too_many_arguments)]
    fn add_event(
        &mut self,
        id: String,
        kind: u32,
        time: DateTime<FixedOffset>,
        rule: u16,
        row: u64,
        values: &[&str],
    ) -> Result<Option<u32>>;
    fn find_event(&self, id: &str) -> Result<Option<u32>>;
    /// Adds a link from the step or item at `record.source` unless it has
    /// the same qualified link; records its lineage. False for a duplicate.
    fn add_link(&mut self, record: LinkRecord) -> Result<bool>;
    /// Starts an empty lookup table.
    fn lookup_clear(&mut self) -> Result<()>;
    /// Keeps a lookup row's values, the later row winning; false when the
    /// key was already there.
    fn lookup_insert(&mut self, key: Vec<String>, values: Vec<String>) -> Result<bool>;
    fn lookup_get(&mut self, key: &[String]) -> Result<Option<Vec<String>>>;
    /// The carried links of a rule, in order: for each step of `event_rule`
    /// in step order, each link to an item of `through` in link order, each
    /// target that `relationship` links from that item, in lineage order.
    fn carried(
        &mut self,
        event_rule: u16,
        through: u16,
        relationship: u32,
        f: &mut Carry<'_, Self>,
    ) -> Result<()>
    where
        Self: Sized;
    /// Steps with no link to any item.
    /// The rule position and source row of each event linked to no object.
    fn unlinked_events(&self) -> Result<Vec<(u16, u64)>>;
    /// Items, steps, item links and step links.
    fn totals(&self) -> Result<[usize; 4]>;
}

/// Positions by identity, holding only positions: an identity is found by
/// comparing with the entity's own.
#[derive(Default)]
struct Identities {
    table: hashbrown::HashTable<u32>,
}
impl Identities {
    fn find(&self, hasher: &RandomState, entities: &[Entity], id: &str) -> Option<u32> {
        self.table
            .find(hasher.hash_one(id), |&at| *entities[at as usize].id == *id)
            .copied()
    }
    /// Records the entity at `at` unless its identity is already taken.
    fn insert(&mut self, hasher: &RandomState, entities: &[Entity], at: u32) -> bool {
        let id = &entities[at as usize].id;
        let hash = hasher.hash_one(&**id);
        if self
            .table
            .find(hash, |&other| entities[other as usize].id == *id)
            .is_some()
        {
            return false;
        }
        self.table.insert_unique(hash, at, |&other| {
            hasher.hash_one(&*entities[other as usize].id)
        });
        true
    }
}

struct MemoryStore {
    names: Names,
    objects: Vec<Entity>,
    events: Vec<Entity>,
    object_ids: Identities,
    event_ids: Identities,
    hasher: RandomState,
    lineage: LineageFile,
    lookup: HashMap<Vec<String>, Vec<String>>,
}

impl MemoryStore {
    fn new() -> Result<Self> {
        Ok(Self {
            names: Names::default(),
            objects: Vec::new(),
            events: Vec::new(),
            object_ids: Identities::default(),
            event_ids: Identities::default(),
            hasher: RandomState::new(),
            lineage: LineageFile::new()?,
            lookup: HashMap::new(),
        })
    }
}

impl Store for MemoryStore {
    fn name(&mut self, name: &str) -> u32 {
        self.names.id(name)
    }
    fn add_object(
        &mut self,
        id: String,
        kind: u32,
        rule: u16,
        row: u64,
        values: &[&str],
    ) -> Result<Option<u32>> {
        let values = values.iter().map(|value| self.names.id(value)).collect();
        self.objects.push(Entity {
            id: id.into_boxed_str(),
            kind,
            time: None,
            rule,
            row,
            values,
            links: Vec::new(),
        });
        let at = self.objects.len() as u32 - 1;
        if self.object_ids.insert(&self.hasher, &self.objects, at) {
            Ok(Some(at))
        } else {
            self.objects.pop();
            Ok(None)
        }
    }
    fn find_object(&self, id: &str) -> Result<Option<u32>> {
        Ok(self.object_ids.find(&self.hasher, &self.objects, id))
    }
    fn add_event(
        &mut self,
        id: String,
        kind: u32,
        time: DateTime<FixedOffset>,
        rule: u16,
        row: u64,
        values: &[&str],
    ) -> Result<Option<u32>> {
        let values = values.iter().map(|value| self.names.id(value)).collect();
        self.events.push(Entity {
            id: id.into_boxed_str(),
            kind,
            time: Some(time),
            rule,
            row,
            values,
            links: Vec::new(),
        });
        let at = self.events.len() as u32 - 1;
        if self.event_ids.insert(&self.hasher, &self.events, at) {
            Ok(Some(at))
        } else {
            self.events.pop();
            Ok(None)
        }
    }
    fn find_event(&self, id: &str) -> Result<Option<u32>> {
        Ok(self.event_ids.find(&self.hasher, &self.events, id))
    }
    fn add_link(&mut self, record: LinkRecord) -> Result<bool> {
        let entity = if record.from_event {
            &mut self.events[record.source as usize]
        } else {
            &mut self.objects[record.source as usize]
        };
        let link = (record.target, record.qualifier);
        if entity.links.contains(&link) {
            return Ok(false);
        }
        entity.links.push(link);
        self.lineage.push(record);
        Ok(true)
    }
    fn lookup_clear(&mut self) -> Result<()> {
        self.lookup.clear();
        Ok(())
    }
    fn lookup_insert(&mut self, key: Vec<String>, values: Vec<String>) -> Result<bool> {
        Ok(self.lookup.insert(key, values).is_none())
    }
    fn lookup_get(&mut self, key: &[String]) -> Result<Option<Vec<String>>> {
        Ok(self.lookup.get(key).cloned())
    }
    fn carried(
        &mut self,
        event_rule: u16,
        through: u16,
        relationship: u32,
        f: &mut Carry<'_, Self>,
    ) -> Result<()> {
        let mut targets = HashMap::<u32, Vec<(u32, u32, u64)>>::new();
        for record in self
            .lineage
            .records()
            .filter(|l| !l.from_event && l.rule == relationship)
        {
            targets.entry(record.source).or_default().push((
                record.target,
                record.input,
                record.row,
            ));
        }
        let mut added = Vec::new();
        for (position, event) in self.events.iter().enumerate() {
            if event.rule != event_rule {
                continue;
            }
            for (object, _) in &event.links {
                if self.objects[*object as usize].rule != through {
                    continue;
                }
                for (target, input, row) in targets.get(object).into_iter().flatten() {
                    added.push((position as u32, *target, *input, *row));
                }
            }
        }
        for (position, target, input, row) in added {
            f(self, position, target, input, row)?;
        }
        Ok(())
    }
    fn unlinked_events(&self) -> Result<Vec<(u16, u64)>> {
        Ok(self
            .events
            .iter()
            .filter(|e| e.links.is_empty())
            .map(|e| (e.rule, e.row))
            .collect())
    }
    fn totals(&self) -> Result<[usize; 4]> {
        Ok([
            self.objects.len(),
            self.events.len(),
            self.objects.iter().map(|o| o.links.len()).sum(),
            self.events.iter().map(|e| e.links.len()).sum(),
        ])
    }
}

/// Resolves an item reference to the item's position.
#[allow(clippy::too_many_arguments)]
fn resolve<S: Store>(
    reference: &ObjectRef,
    row: &Row<'_>,
    object_keys: &BTreeMap<String, (String, String)>,
    store: &S,
    report: &mut Report,
    rule: &str,
    number: u64,
) -> Result<Option<usize>> {
    let Some(key) = row.parts(&reference.columns)? else {
        if reference.optional {
            report.rules.get_mut(rule).unwrap().omitted_blank_links += 1;
        } else {
            report.issue(rule, number, "blank_object_reference");
        }
        return Ok(None);
    };
    let (namespace, kind) = &object_keys[&reference.object];
    match store.find_object(&identity(namespace, kind, &key))? {
        Some(i) => Ok(Some(i as usize)),
        None => {
            report.rules.get_mut(rule).unwrap().unmatched += 1;
            if !reference.allow_unmatched {
                report.issue(rule, number, "unmatched_object_reference");
            }
            Ok(None)
        }
    }
}

/// Attribute values in the rule's order; a missing column is an error.
fn row_values<'r>(row: &'r Row<'_>, fields: &[Attribute]) -> Result<Vec<&'r str>> {
    fields.iter().map(|a| row.get(&a.column)).collect()
}

/// Builds the official four-array OCEL JSON shape. Unresolved rows block output.
/// The graph resides in memory; there is no fixed entity-count limit, so
/// practical capacity depends on available memory. [`prepare_on_disk`] keeps
/// it on disk instead.
pub fn prepare(recipe: &Recipe, inputs: &BTreeMap<String, PathBuf>) -> Result<Outcome> {
    let (excluded, set_aside) = set_aside_rows(recipe, inputs, MemoryStore::new)?;
    let mut store = MemoryStore::new()?;
    let mut built = build(recipe, inputs, &mut store, &excluded)?;
    built.report.set_aside_rows(&set_aside);
    let MemoryStore {
        names,
        objects,
        events,
        mut lineage,
        ..
    } = store;
    lineage.finish();
    let complete = built.report.issue_count == 0;
    let graph = Rc::new(Graph {
        names,
        objects,
        events,
        object_fields: built.object_fields,
        event_fields: built.event_fields,
        object_types: built.object_types,
        event_types: built.event_types,
        lineage: std::cell::RefCell::new(lineage),
    });
    Ok(Outcome {
        report: built.report,
        set_aside,
        log: complete.then(|| Log(Built::Memory(Rc::clone(&graph)))),
        relationship_lineage: Lineage(Built::Memory(graph)),
    })
}

/// Inputs at least this large are built on disk by [`prepare_auto`].
pub const DISK_BUILD_BYTES: u64 = 512 * 1024 * 1024;

/// The engine's choice: inputs of [`DISK_BUILD_BYTES`] or more in all are
/// built on disk, anything smaller in memory, which is faster. Both give the
/// same log, report and lineage.
pub fn prepare_auto(recipe: &Recipe, inputs: &BTreeMap<String, PathBuf>) -> Result<Outcome> {
    let size = inputs
        .values()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|m| m.len())
        .sum::<u64>();
    if size >= DISK_BUILD_BYTES || crate::force_disk() {
        prepare_on_disk(recipe, inputs, &std::env::temp_dir())
    } else {
        prepare(recipe, inputs)
    }
}

/// With [`Recipe::set_aside`], the rows to leave out and why: build, leave
/// out every row that raised an issue, and build again until none does.
/// Leaving a row out can strand another (an event whose only object was left
/// out), which the next round finds.
fn set_aside_rows<S: Store>(
    recipe: &Recipe,
    inputs: &BTreeMap<String, PathBuf>,
    mut store: impl FnMut() -> Result<S>,
) -> Result<(Excluded, Vec<Issue>)> {
    let mut excluded = Excluded::new();
    let mut found = Vec::new();
    if !recipe.set_aside {
        return Ok((excluded, found));
    }
    for _ in 0..16 {
        let mut scratch = store()?;
        let built = build(recipe, inputs, &mut scratch, &excluded)?;
        let mut added = false;
        for (scope, issue) in built.report.rows.unwrap_or_default() {
            if excluded.entry(scope).or_default().insert(issue.row) {
                found.push(issue);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    Ok((excluded, found))
}

/// What a build found besides its items and steps.
struct BuildResult {
    report: Report,
    object_fields: Vec<(Vec<String>, u32)>,
    event_fields: Vec<(Vec<String>, u32)>,
    object_types: Vec<Value>,
    event_types: Vec<Value>,
}

/// Source rows to leave out, by the rule (or rule lookup) that reads them.
type Excluded = BTreeMap<String, HashSet<u64>>;

fn build<S: Store>(
    recipe: &Recipe,
    inputs: &BTreeMap<String, PathBuf>,
    store: &mut S,
    excluded: &Excluded,
) -> Result<BuildResult> {
    require(
        recipe.version == 1 && recipe.format == "ocel",
        "expected version 1 format ocel",
    )?;
    require(
        !recipe.inputs.is_empty()
            && recipe.inputs.len() <= 64
            && !recipe.objects.is_empty()
            && recipe.objects.len() <= 64
            && !recipe.events.is_empty()
            && recipe.events.len() <= 64
            && recipe.relationships.len() <= 64
            && recipe.event_links.len() <= 64
            && recipe.carried_links.len() <= 64,
        "invalid recipe rule count",
    )?;
    let hashes = recipe
        .inputs
        .iter()
        .map(|i| (i.id.clone(), i.sha256.clone()))
        .collect::<BTreeMap<_, _>>();
    require(
        hashes.len() == recipe.inputs.len()
            && recipe.inputs.iter().all(|i| {
                label(&i.id)
                    && i.sha256.len() == 64
                    && i.sha256.bytes().all(|c| c.is_ascii_hexdigit())
                    && inputs.contains_key(&i.id)
            }),
        "invalid or duplicate input identity",
    )?;
    let object_rules = recipe
        .objects
        .iter()
        .map(|o| (o.id.clone(), o))
        .collect::<BTreeMap<_, _>>();
    let event_rules = recipe
        .events
        .iter()
        .map(|e| (e.id.clone(), e))
        .collect::<BTreeMap<_, _>>();
    require(
        object_rules.len() == recipe.objects.len() && event_rules.len() == recipe.events.len(),
        "duplicate entity rule id",
    )?;
    let mut all_ids = BTreeSet::new();
    for (id, input) in recipe
        .objects
        .iter()
        .map(|o| (&o.id, &o.input))
        .chain(recipe.events.iter().map(|e| (&e.id, &e.input)))
        .chain(recipe.relationships.iter().map(|r| (&r.id, &r.input)))
        .chain(recipe.event_links.iter().map(|r| (&r.id, &r.input)))
    {
        require(
            label(id) && all_ids.insert(id) && hashes.contains_key(input),
            "duplicate/invalid rule or unresolved input",
        )?;
    }
    let check_ref = |r: &ObjectRef| -> Result<()> {
        let target = object_rules
            .get(&r.object)
            .ok_or_else(|| Error("unknown object rule".into()))?;
        require(
            label(&r.qualifier) && parts_valid(&r.columns) && r.columns.len() == target.key.len(),
            "invalid relationship qualifier or key width",
        )
    };
    let mut namespaces = BTreeSet::new();
    for o in &recipe.objects {
        require(
            label(&o.namespace)
                && label(&o.object_type)
                && key_valid(&o.key)
                && attributes_valid(&o.attributes)
                && conditions_valid(&o.filter)
                // Only a second source of existing objects may share a type.
                && (namespaces.insert((&o.namespace, &o.object_type)) || o.existing_ok),
            "invalid object rule or ambiguous namespace/type",
        )?;
    }
    for e in &recipe.events {
        require(
            label(&e.namespace)
                && key_valid(&e.key)
                && timestamp_valid(&e.timestamp)
                && attributes_valid(&e.attributes)
                && conditions_valid(&e.filter),
            "invalid event rule",
        )?;
        match &e.activity {
            ActivitySource::Column(c) => require(label(&c.column), "invalid activity column")?,
            ActivitySource::Value(v) => require(label(&v.value), "invalid activity")?,
        }
        for r in &e.objects {
            check_ref(r)?;
        }
        require(e.objects.len() <= 64, "too many event references")?;
        require(
            e.lookup.iter().chain(&e.lookups).count() <= 8,
            "too many lookups (maximum eight per event)",
        )?;
        for lookup in e.lookup.iter().chain(&e.lookups) {
            require(
                hashes.contains_key(&lookup.input)
                    && key_valid(&lookup.key)
                    && key_valid(&lookup.matching)
                    && lookup.key.len() == lookup.matching.len()
                    && !lookup.columns.is_empty()
                    && lookup.columns.len() <= 32
                    && lookup.columns.iter().all(|c| label(c))
                    && conditions_valid(&lookup.filter),
                "invalid lookup",
            )?;
        }
    }
    for r in &recipe.relationships {
        require(conditions_valid(&r.filter), "invalid where condition")?;
        check_ref(&r.source)?;
        check_ref(&r.target)?;
    }
    for r in &recipe.carried_links {
        let relationship = recipe
            .relationships
            .iter()
            .find(|rel| rel.id == r.relationship);
        require(
            label(&r.id)
                && all_ids.insert(&r.id)
                && label(&r.qualifier)
                && event_rules.contains_key(&r.event)
                && relationship.is_some_and(|rel| rel.source.object == r.through),
            "invalid carried link: it needs a known event rule and a relationship from the 'through' object",
        )?;
    }
    for r in &recipe.event_links {
        let e = event_rules
            .get(&r.event)
            .ok_or_else(|| Error("unknown event rule".into()))?;
        require(
            key_valid(&r.event_key)
                && r.event_key.len() == e.key.len()
                && conditions_valid(&r.filter),
            "invalid event reference key",
        )?;
        check_ref(&r.target)?;
    }
    let mut report = Report {
        rows: recipe.set_aside.then(Vec::new),
        ..Report::default()
    };
    let left_out = |scope: &str, number: u64| {
        excluded
            .get(scope)
            .is_some_and(|rows| rows.contains(&number))
    };
    let mut object_types = BTreeMap::new();
    let mut event_types = BTreeMap::new();
    let object_keys = recipe
        .objects
        .iter()
        .map(|o| (o.id.clone(), (o.namespace.clone(), o.object_type.clone())))
        .collect::<BTreeMap<_, _>>();
    let event_keys = recipe
        .events
        .iter()
        .map(|e| (e.id.clone(), e.namespace.clone()))
        .collect::<BTreeMap<_, _>>();
    let object_fields = recipe
        .objects
        .iter()
        .map(|o| {
            let input = store.name(&hashes[&o.input]);
            (o.attributes.iter().map(|a| a.name.clone()).collect(), input)
        })
        .collect::<Vec<_>>();
    let event_fields = recipe
        .events
        .iter()
        .map(|e| {
            let input = store.name(&hashes[&e.input]);
            (e.attributes.iter().map(|a| a.name.clone()).collect(), input)
        })
        .collect::<Vec<_>>();
    for (rule_position, o) in recipe.objects.iter().enumerate() {
        // A second source of existing objects may carry fewer attributes;
        // the type keeps the first source's definition.
        if !(o.existing_ok && object_types.contains_key(&o.object_type)) {
            register_type(&mut object_types, &o.object_type, &o.attributes)?;
        }
        report.rules.insert(o.id.clone(), Counts::default());
        let kind = store.name(&o.object_type);
        let required = o
            .key
            .iter()
            .cloned()
            .chain(o.attributes.iter().map(|a| a.column.clone()))
            .collect::<Vec<_>>();
        let filtered = rows(&o.input, inputs, &required, &o.filter, |number, row| {
            report.rules.get_mut(&o.id).unwrap().rows += 1;
            if left_out(&o.id, number) {
                report.rules.get_mut(&o.id).unwrap().set_aside += 1;
                return Ok(());
            }
            let Some(key) = row.key(&o.key)? else {
                report.issue(&o.id, number, "blank_object_key");
                return Ok(());
            };
            let id = identity(&o.namespace, &o.object_type, &key);
            let values = row_values(&row, &o.attributes)?;
            if store
                .add_object(id, kind, rule_position as u16, number, &values)?
                .is_none()
            {
                if o.existing_ok {
                    report.rules.get_mut(&o.id).unwrap().skipped += 1;
                } else {
                    report.issue(&o.id, number, "duplicate_object_key");
                }
                return Ok(());
            }
            report.rules.get_mut(&o.id).unwrap().emitted += 1;
            Ok(())
        })?;
        report.rules.get_mut(&o.id).unwrap().filtered = filtered;
    }
    for (rule_position, e) in recipe.events.iter().enumerate() {
        report.rules.insert(e.id.clone(), Counts::default());
        let rule = store.name(&e.id);
        let input = store.name(&hashes[&e.input]);
        let mut required = e.key.clone();
        required.extend(e.timestamp.columns());
        if let ActivitySource::Column(c) = &e.activity {
            required.push(c.column.clone());
        }
        required.extend(e.attributes.iter().map(|a| a.column.clone()));
        required.extend(
            e.objects
                .iter()
                .flat_map(|r| r.columns.iter().map(|p| p.column().to_owned())),
        );
        // A lookup's columns come from its own input, keyed by its key.
        store.lookup_clear()?;
        let lookups = e.lookup.iter().chain(&e.lookups).collect::<Vec<_>>();
        // Work backwards to discover fields needed before any joins. A later
        // join can match a column produced by an earlier one.
        for lookup in lookups.iter().rev() {
            required.retain(|c| !lookup.columns.contains(c));
            required.extend(lookup.matching.iter().cloned());
        }
        for (index, lookup) in lookups.iter().enumerate() {
            let scope = format!("{}#lookup{index}", e.id);
            let wanted = lookup
                .key
                .iter()
                .chain(&lookup.columns)
                .cloned()
                .collect::<Vec<_>>();
            rows(
                &lookup.input,
                inputs,
                &wanted,
                &lookup.filter,
                |number, row| {
                    if left_out(&scope, number) {
                        return Ok(());
                    }
                    let Some(mut key) = row.key(&lookup.key)? else {
                        return Ok(());
                    };
                    if lookups.len() > 1 {
                        key.insert(0, index.to_string());
                    }
                    let values = lookup
                        .columns
                        .iter()
                        .map(|c| row.get(c).map(str::to_owned))
                        .collect::<Result<Vec<_>>>()?;
                    if !store.lookup_insert(key, values)? {
                        report.issue_in(&scope, &e.id, number, "duplicate_lookup_key");
                    }
                    Ok(())
                },
            )?;
        }
        let filtered = rows(&e.input, inputs, &required, &e.filter, |number, row| {
            report.rules.get_mut(&e.id).unwrap().rows += 1;
            if left_out(&e.id, number) {
                report.rules.get_mut(&e.id).unwrap().set_aside += 1;
                return Ok(());
            }
            let mut row = row;
            for (index, lookup) in lookups.iter().enumerate() {
                let matched = match row.key(&lookup.matching)? {
                    Some(mut key) => {
                        if lookups.len() > 1 {
                            key.insert(0, index.to_string());
                        }
                        store.lookup_get(&key)?
                    }
                    None => None,
                };
                let Some(values) = matched else {
                    if lookup.optional {
                        report.rules.get_mut(&e.id).unwrap().skipped += 1;
                    } else {
                        report.issue(&e.id, number, "unmatched_lookup");
                    }
                    return Ok(());
                };
                row.extra
                    .extend(lookup.columns.iter().cloned().zip(values.iter().cloned()));
            }
            let time = match event_time(&row, &e.timestamp)? {
                Time::At(time) => time,
                Time::Blank if e.optional_timestamp => {
                    report.rules.get_mut(&e.id).unwrap().skipped += 1;
                    return Ok(());
                }
                Time::Blank | Time::Invalid => {
                    report.issue(&e.id, number, "invalid_or_missing_timestamp");
                    return Ok(());
                }
            };
            let activity = match &e.activity {
                ActivitySource::Column(c) => c.name(row.get(&c.column)?),
                ActivitySource::Value(v) => v.value.as_str().into(),
            };
            let activity = activity.as_ref();
            if !label(activity) {
                report.issue(&e.id, number, "blank_or_invalid_activity");
                return Ok(());
            }
            register_type(&mut event_types, activity, &e.attributes)?;
            let Some(key) = row.key(&e.key)? else {
                report.issue(&e.id, number, "blank_event_key");
                return Ok(());
            };
            let id = identity(&e.namespace, &e.id, &key);
            let kind = store.name(activity);
            let values = row_values(&row, &e.attributes)?;
            let Some(position) =
                store.add_event(id, kind, time, rule_position as u16, number, &values)?
            else {
                report.issue(&e.id, number, "duplicate_event_key");
                return Ok(());
            };
            for r in &e.objects {
                if let Some(i) =
                    resolve(r, &row, &object_keys, &*store, &mut report, &e.id, number)?
                {
                    let record = LinkRecord {
                        from_event: true,
                        source: position,
                        target: i as u32,
                        qualifier: store.name(&r.qualifier),
                        input,
                        rule,
                        row: number,
                    };
                    if !store.add_link(record)? {
                        report.issue(&e.id, number, "duplicate_event_relationship");
                    }
                }
            }
            report.rules.get_mut(&e.id).unwrap().emitted += 1;
            Ok(())
        })?;
        report.rules.get_mut(&e.id).unwrap().filtered = filtered;
    }
    for r in &recipe.relationships {
        report.rules.insert(r.id.clone(), Counts::default());
        let rule = store.name(&r.id);
        let input = store.name(&hashes[&r.input]);
        let qualifier = store.name(&r.target.qualifier);
        let required = r
            .source
            .columns
            .iter()
            .chain(&r.target.columns)
            .map(|p| p.column().to_owned())
            .collect::<Vec<_>>();
        let filtered = rows(&r.input, inputs, &required, &r.filter, |number, row| {
            report.rules.get_mut(&r.id).unwrap().rows += 1;
            if left_out(&r.id, number) {
                report.rules.get_mut(&r.id).unwrap().set_aside += 1;
                return Ok(());
            }
            let source = resolve(
                &r.source,
                &row,
                &object_keys,
                &*store,
                &mut report,
                &r.id,
                number,
            )?;
            let target = resolve(
                &r.target,
                &row,
                &object_keys,
                &*store,
                &mut report,
                &r.id,
                number,
            )?;
            if let (Some(s), Some(t)) = (source, target) {
                let record = LinkRecord {
                    from_event: false,
                    source: s as u32,
                    target: t as u32,
                    qualifier,
                    input,
                    rule,
                    row: number,
                };
                if store.add_link(record)? {
                    report.rules.get_mut(&r.id).unwrap().emitted += 1;
                } else {
                    report.issue(&r.id, number, "duplicate_object_relationship");
                }
            }
            Ok(())
        })?;
        report.rules.get_mut(&r.id).unwrap().filtered = filtered;
    }
    for r in &recipe.event_links {
        report.rules.insert(r.id.clone(), Counts::default());
        let rule = store.name(&r.id);
        let input = store.name(&hashes[&r.input]);
        let qualifier = store.name(&r.target.qualifier);
        let required = r
            .event_key
            .iter()
            .cloned()
            .chain(r.target.columns.iter().map(|p| p.column().to_owned()))
            .collect::<Vec<_>>();
        let filtered = rows(&r.input, inputs, &required, &r.filter, |number, row| {
            report.rules.get_mut(&r.id).unwrap().rows += 1;
            if left_out(&r.id, number) {
                report.rules.get_mut(&r.id).unwrap().set_aside += 1;
                return Ok(());
            }
            let event = match row.key(&r.event_key)? {
                Some(k) => store.find_event(&identity(&event_keys[&r.event], &r.event, &k))?,
                None => None,
            };
            let Some(e) = event else {
                if r.optional_event {
                    report.rules.get_mut(&r.id).unwrap().skipped += 1;
                } else {
                    report.issue(&r.id, number, "unmatched_or_blank_event_reference");
                }
                return Ok(());
            };
            if let Some(o) = resolve(
                &r.target,
                &row,
                &object_keys,
                &*store,
                &mut report,
                &r.id,
                number,
            )? {
                let record = LinkRecord {
                    from_event: true,
                    source: e,
                    target: o as u32,
                    qualifier,
                    input,
                    rule,
                    row: number,
                };
                if store.add_link(record)? {
                    report.rules.get_mut(&r.id).unwrap().emitted += 1;
                } else {
                    report.issue(&r.id, number, "duplicate_event_relationship");
                }
            }
            Ok(())
        })?;
        report.rules.get_mut(&r.id).unwrap().filtered = filtered;
    }
    for r in &recipe.carried_links {
        report.rules.insert(r.id.clone(), Counts::default());
        let rule = store.name(&r.id);
        let qualifier = store.name(&r.qualifier);
        let relationship = store.name(&r.relationship);
        let event_rule = recipe.events.iter().position(|e| e.id == r.event).unwrap() as u16;
        let through = recipe
            .objects
            .iter()
            .position(|o| o.id == r.through)
            .unwrap() as u16;
        store.carried(
            event_rule,
            through,
            relationship,
            &mut |store, position, target, input, row| {
                let record = LinkRecord {
                    from_event: true,
                    source: position,
                    target,
                    qualifier,
                    input,
                    rule,
                    row,
                };
                let added = store.add_link(record)?;
                let counts = report.rules.get_mut(&r.id).unwrap();
                counts.rows += 1;
                // Two routes to the same object add one link.
                if added {
                    counts.emitted += 1;
                } else {
                    counts.skipped += 1;
                }
                Ok(())
            },
        )?;
    }
    for (rule, row) in store.unlinked_events()? {
        let id = &recipe.events[rule as usize].id;
        report.issue_in(id, "event_coverage", row, "event_has_no_object");
    }
    let [objects, events, object_links, event_links] = store.totals()?;
    report.objects = objects;
    report.events = events;
    report.object_relationships = object_links;
    report.event_relationships = event_links;
    if objects == 0 || events == 0 {
        report.issue("coverage", 0, "empty_objects_or_events");
    }
    let types = |map: BTreeMap<String, Vec<Value>>| {
        map.into_iter()
            .map(|(name, attributes)| json!({"name":name,"attributes":attributes}))
            .collect::<Vec<_>>()
    };
    Ok(BuildResult {
        report,
        object_fields,
        event_fields,
        object_types: types(object_types),
        event_types: types(event_types),
    })
}

/// SQL identifier quoting: double quotes, inner quotes doubled.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Table-name suffixes for type names: letters and digits only, unique.
fn type_tables(names: &[String]) -> Vec<String> {
    let mut used = BTreeSet::new();
    names
        .iter()
        .map(|name| {
            let base: String = name
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|part| !part.is_empty())
                .map(|part| {
                    let mut chars = part.chars();
                    chars
                        .next()
                        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                        .unwrap_or_default()
                })
                .collect();
            let base = if base.is_empty() {
                "Type".to_owned()
            } else {
                base
            };
            let mut candidate = base.clone();
            let mut n = 2;
            while !used.insert(candidate.to_ascii_lowercase()) {
                candidate = format!("{base}{n}");
                n += 1;
            }
            candidate
        })
        .collect()
}

impl Log {
    /// Writes the log as an OCEL 2.0 SQLite file, the relational exchange
    /// format: much smaller than the JSON form and read back row by row.
    pub fn write_sqlite(&self, path: &std::path::Path) -> Result<()> {
        let graph = self.0.graph();
        let fail =
            |error: rusqlite::Error| Error(format!("cannot write the OCEL database: {error}"));
        let mut connection = rusqlite::Connection::open(path).map_err(fail)?;
        connection
            .execute_batch("PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;")
            .map_err(fail)?;
        let transaction = connection.transaction().map_err(fail)?;
        let schema = |types: &[Value]| -> Vec<(String, Vec<(String, String)>)> {
            types
                .iter()
                .map(|declared| {
                    let columns = declared["attributes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|attribute| {
                            let sql = match attribute["type"].as_str() {
                                Some("integer") => "INTEGER",
                                _ => "TEXT",
                            };
                            (
                                attribute["name"].as_str().unwrap_or_default().to_owned(),
                                sql.to_owned(),
                            )
                        })
                        .collect();
                    (
                        declared["name"].as_str().unwrap_or_default().to_owned(),
                        columns,
                    )
                })
                .collect()
        };
        let event_types = schema(graph.types(true));
        let object_types = schema(graph.types(false));
        let event_tables = type_tables(
            &event_types
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>(),
        );
        let object_tables = type_tables(
            &object_types
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>(),
        );
        transaction
            .execute_batch(
                "CREATE TABLE event (ocel_id TEXT, ocel_type TEXT);\n\
                 CREATE TABLE object (ocel_id TEXT, ocel_type TEXT);\n\
                 CREATE TABLE event_object (ocel_event_id TEXT, ocel_object_id TEXT, ocel_qualifier TEXT);\n\
                 CREATE TABLE object_object (ocel_source_id TEXT, ocel_target_id TEXT, ocel_qualifier TEXT);\n\
                 CREATE TABLE event_map_type (ocel_type TEXT, ocel_type_map TEXT);\n\
                 CREATE TABLE object_map_type (ocel_type TEXT, ocel_type_map TEXT);",
            )
            .map_err(fail)?;
        for (prefix, types, tables, reserved) in [
            (
                "event",
                &event_types,
                &event_tables,
                "ocel_id TEXT, ocel_time TIMESTAMP",
            ),
            (
                "object",
                &object_types,
                &object_tables,
                "ocel_id TEXT, ocel_time TIMESTAMP, ocel_changed_field TEXT",
            ),
        ] {
            for ((name, columns), table) in types.iter().zip(tables) {
                transaction
                    .execute(
                        &format!("INSERT INTO {prefix}_map_type VALUES (?1, ?2)"),
                        [name, table],
                    )
                    .map_err(fail)?;
                let columns = columns
                    .iter()
                    .map(|(column, sql)| format!(", {} {sql}", quoted(column)))
                    .collect::<String>();
                transaction
                    .execute_batch(&format!(
                        "CREATE TABLE {} ({reserved}{columns});",
                        quoted(&format!("{prefix}_{table}"))
                    ))
                    .map_err(fail)?;
            }
        }
        // One prepared insert per typed table, columns in declared order.
        let typed_insert = |prefix: &str, table: &str, reserved: usize, columns: usize| {
            format!(
                "INSERT INTO {} VALUES ({})",
                quoted(&format!("{prefix}_{table}")),
                (1..=reserved + columns)
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let value_of = |entity: &EntityRow, event: bool, column: &str| -> rusqlite::types::Value {
            let (names, input) = &graph.fields(event)[entity.rule as usize];
            match column {
                "_source_input" => rusqlite::types::Value::Text(graph.names_of(*input)),
                "_source_row" => rusqlite::types::Value::Integer(entity.row as i64),
                _ => names
                    .iter()
                    .position(|name| name == column)
                    .and_then(|index| entity.values.get(index))
                    .map_or(rusqlite::types::Value::Null, |value| {
                        rusqlite::types::Value::Text(value.clone())
                    }),
            }
        };
        {
            let mut objects = transaction
                .prepare("INSERT INTO object VALUES (?1, ?2)")
                .map_err(fail)?;
            let mut links = transaction
                .prepare("INSERT INTO object_object VALUES (?1, ?2, ?3)")
                .map_err(fail)?;
            let mut typed = object_types
                .iter()
                .zip(&object_tables)
                .map(|((name, columns), table)| {
                    Ok((
                        name.as_str(),
                        (
                            columns,
                            transaction.prepare(&typed_insert(
                                "object",
                                table,
                                3,
                                columns.len(),
                            ))?,
                        ),
                    ))
                })
                .collect::<std::result::Result<BTreeMap<_, _>, rusqlite::Error>>()
                .map_err(fail)?;
            graph.each(false, &mut |object| {
                let kind = object.kind.as_str();
                objects.execute([&object.id, kind]).map_err(fail)?;
                if let Some((columns, insert)) = typed.get_mut(kind) {
                    let mut row = vec![
                        rusqlite::types::Value::Text(object.id.to_string()),
                        rusqlite::types::Value::Text("1970-01-01T00:00:00Z".to_owned()),
                        rusqlite::types::Value::Null,
                    ];
                    row.extend(
                        columns
                            .iter()
                            .map(|(column, _)| value_of(&object, false, column)),
                    );
                    insert
                        .execute(rusqlite::params_from_iter(row))
                        .map_err(fail)?;
                }
                for (target, qualifier) in &object.links {
                    links
                        .execute([&object.id, target, qualifier])
                        .map_err(fail)?;
                }
                Ok(())
            })?;
        }
        {
            let mut events = transaction
                .prepare("INSERT INTO event VALUES (?1, ?2)")
                .map_err(fail)?;
            let mut links = transaction
                .prepare("INSERT INTO event_object VALUES (?1, ?2, ?3)")
                .map_err(fail)?;
            let mut typed = event_types
                .iter()
                .zip(&event_tables)
                .map(|((name, columns), table)| {
                    Ok((
                        name.as_str(),
                        (
                            columns,
                            transaction.prepare(&typed_insert("event", table, 2, columns.len()))?,
                        ),
                    ))
                })
                .collect::<std::result::Result<BTreeMap<_, _>, rusqlite::Error>>()
                .map_err(fail)?;
            graph.each(true, &mut |event| {
                let kind = event.kind.as_str();
                events.execute([&event.id, kind]).map_err(fail)?;
                if let Some((columns, insert)) = typed.get_mut(kind) {
                    let mut row = vec![
                        rusqlite::types::Value::Text(event.id.to_string()),
                        rusqlite::types::Value::Text(event.time.clone().unwrap_or_default()),
                    ];
                    row.extend(
                        columns
                            .iter()
                            .map(|(column, _)| value_of(&event, true, column)),
                    );
                    insert
                        .execute(rusqlite::params_from_iter(row))
                        .map_err(fail)?;
                }
                for (target, qualifier) in &event.links {
                    links
                        .execute([&event.id, target, qualifier])
                        .map_err(fail)?;
                }
                Ok(())
            })?;
        }
        // Identity indexes are built once at the end, which is far faster than
        // keeping them ordered during the inserts; readers join on them.
        transaction
            .execute_batch(
                "CREATE UNIQUE INDEX event_id ON event (ocel_id);\n\
                 CREATE UNIQUE INDEX object_id ON object (ocel_id);",
            )
            .map_err(fail)?;
        transaction.commit().map_err(fail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds in memory and on disk; both must fail alike or give the same
    /// report, log and lineage. Returns the build in memory.
    fn checked(recipe: &Recipe, inputs: &BTreeMap<String, PathBuf>) -> Result<Outcome> {
        let memory = prepare(recipe, inputs);
        let disk = prepare_on_disk(recipe, inputs, &std::env::temp_dir());
        match (&memory, &disk) {
            (Ok(memory), Ok(disk)) => {
                assert_eq!(
                    serde_json::to_value(&disk.report).unwrap(),
                    serde_json::to_value(&memory.report).unwrap()
                );
                assert_eq!(
                    serde_json::to_string(&disk.relationship_lineage).unwrap(),
                    serde_json::to_string(&memory.relationship_lineage).unwrap()
                );
                assert_eq!(
                    serde_json::to_string(&disk.log).unwrap(),
                    serde_json::to_string(&memory.log).unwrap()
                );
            }
            (Err(memory), Err(disk)) => assert_eq!(disk.to_string(), memory.to_string()),
            _ => panic!("one build failed and the other did not"),
        }
        memory
    }

    /// Every fact of a log as sorted text, independent of storage order.
    fn canonical(log: &pm_core::OcelLog) -> BTreeSet<String> {
        let event = |id: pm_core::OcelEventId| log.event_label(id).unwrap_or_default().to_owned();
        let object = |id: pm_core::ObjectId| log.object_label(id).unwrap_or_default().to_owned();
        let value = |value: &pm_core::AttributeValue| match value {
            pm_core::AttributeValue::String(id) => log
                .attribute_values
                .label(id.0)
                .unwrap_or_default()
                .to_owned(),
            other => format!("{other:?}"),
        };
        let mut facts = BTreeSet::new();
        for (index, e) in log.events.iter().enumerate() {
            facts.insert(format!(
                "event {} {} {}",
                event(pm_core::OcelEventId(index as u32)),
                log.event_type(e.event_type).unwrap().name,
                e.at
            ));
        }
        for (index, o) in log.objects.iter().enumerate() {
            facts.insert(format!(
                "object {} {}",
                object(pm_core::ObjectId(index as u32)),
                log.object_type(o.object_type).unwrap().name
            ));
        }
        for r in &log.event_object_relations {
            facts.insert(format!(
                "e2o {} {} {}",
                event(r.event),
                object(r.object),
                log.qualifier_label(r.qualifier).unwrap_or_default()
            ));
        }
        for r in &log.object_object_relations {
            facts.insert(format!(
                "o2o {} {} {}",
                object(r.source),
                object(r.target),
                log.qualifier_label(r.qualifier).unwrap_or_default()
            ));
        }
        for a in &log.event_attributes {
            facts.insert(format!(
                "ea {} {} {}",
                event(a.event),
                log.attribute_names.label(a.name.0).unwrap_or_default(),
                value(&a.value)
            ));
        }
        for a in &log.object_attributes {
            facts.insert(format!(
                "oa {} {} {} {}",
                object(a.object),
                log.attribute_names.label(a.name.0).unwrap_or_default(),
                a.at,
                value(&a.value)
            ));
        }
        facts
    }

    /// The compact graph must write exactly the bytes the earlier JSON-value
    /// builder wrote: keys sorted, object attributes at the initial epoch,
    /// and the source row as a number.
    #[test]
    fn writes_the_same_ocel_and_lineage_bytes_as_json_values() {
        let dir = std::env::temp_dir().join(format!("pm-prepare-ocel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("orders.csv"), "order,region\n0007,EU\n").unwrap();
        std::fs::write(
            dir.join("events.csv"),
            "event,order,at\nE1,0007,2026-01-02T09:00:00+01:00\n",
        )
        .unwrap();
        let hash = "a".repeat(64);
        let recipe: Recipe = serde_json::from_value(json!({
            "version":1,"format":"ocel",
            "inputs":[{"id":"orders","sha256":hash},{"id":"events","sha256":"b".repeat(64)}],
            "objects":[{"id":"orders","input":"orders","namespace":"erp","object_type":"order","key":["order"],
                "attributes":[{"column":"region","name":"region"}]}],
            "events":[{"id":"created","input":"events","namespace":"erp","key":["event"],
                "activity":{"value":"Order created"},"timestamp":"at",
                "objects":[{"object":"orders","columns":["order"],"qualifier":"created order"}]}]
        }))
        .unwrap();
        let inputs = BTreeMap::from([
            ("orders".to_owned(), dir.join("orders.csv")),
            ("events".to_owned(), dir.join("events.csv")),
        ]);
        let outcome = checked(&recipe, &inputs).unwrap();
        let log = serde_json::to_string(outcome.log.as_ref().unwrap()).unwrap();
        let object = r#"["erp","order",["0007"]]"#;
        let event = r#"["erp","created",["E1"]]"#;
        let expected = json!({
            "objectTypes":[{"name":"order","attributes":[
                {"name":"_source_input","type":"string"},{"name":"_source_row","type":"integer"},{"name":"region","type":"string"}]}],
            "eventTypes":[{"name":"Order created","attributes":[
                {"name":"_source_input","type":"string"},{"name":"_source_row","type":"integer"}]}],
            "objects":[{"id":object,"type":"order","relationships":[],"attributes":[
                {"name":"region","value":"EU","time":"1970-01-01T00:00:00Z"},
                {"name":"_source_input","value":hash,"time":"1970-01-01T00:00:00Z"},
                {"name":"_source_row","value":1,"time":"1970-01-01T00:00:00Z"}]}],
            "events":[{"id":event,"type":"Order created","time":"2026-01-02T09:00:00+01:00",
                "relationships":[{"objectId":object,"qualifier":"created order"}],
                "attributes":[{"name":"_source_input","value":"b".repeat(64)},{"name":"_source_row","value":1}]}]
        });
        assert_eq!(log, expected.to_string());
        // The SQLite form holds exactly the same log as the JSON form.
        let database = dir.join("log.sqlite");
        outcome
            .log
            .as_ref()
            .unwrap()
            .write_sqlite(&database)
            .unwrap();
        let from_json = crate::parse_ocel_json(log.as_bytes()).unwrap();
        let from_sqlite = crate::read_ocel_sqlite(&database).unwrap();
        assert_eq!(canonical(&from_sqlite), canonical(&from_json));
        let lineage = serde_json::to_string(&outcome.relationship_lineage).unwrap();
        let expected = json!([{"source":event,"source_kind":"event","target":object,"qualifier":"created order",
            "input_sha256":"b".repeat(64),"row":1,"rule":"created"}]);
        assert_eq!(lineage, expected.to_string());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// SAP-shaped tables: invoice number plus fiscal year as one key (the
    /// same number in two years is two invoices), split date and time
    /// fields, history rows of one kind picked by a `where` condition, an
    /// accounting key cut from `AWKEY`, and an unpaid invoice whose bridge
    /// row has no payment event.
    #[test]
    fn reads_sap_style_keys_dates_and_row_kinds() {
        let dir = std::env::temp_dir().join(format!("pm-prepare-sap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("rbkp.csv"),
            "BELNR,GJAHR,CPUDT,CPUTM\n5100000001,2025,20251230,170500\n5100000001,2026,20260105,083000\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("ekbe.csv"),
            "EBELN,BELNR,GJAHR,VGABE,BUDAT\n\
             4500000001,5000000001,2025,1,20251220\n\
             4500000001,5100000001,2025,2,20251230\n\
             4500000001,5100000001,2026,2,00000000\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("bkpf.csv"),
            "BUKRS,BELNR,GJAHR,AWTYP,AWKEY\n\
             1000,1900000001,2025,RMRP,51000000012025\n\
             1000,1900000002,2026,RMRP,51000000012026\n\
             1000,0100000001,2026,BKPF,01000000012026\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("bsak.csv"),
            "BUKRS,BELNR,GJAHR,AUGDT\n1000,1900000001,2025,15.01.2026\n",
        )
        .unwrap();
        let inputs = ["rbkp", "ekbe", "bkpf", "bsak"];
        let recipe: Recipe = serde_json::from_value(json!({
            "version":1,"format":"ocel",
            "inputs": inputs.iter().enumerate().map(|(n, id)| json!({"id":id,"sha256":n.to_string().repeat(64)})).collect::<Vec<_>>(),
            "objects":[{"id":"invoice","input":"rbkp","namespace":"sap","object_type":"invoice","key":["BELNR","GJAHR"]}],
            "events":[
                {"id":"recorded","input":"rbkp","namespace":"sap","key":["BELNR","GJAHR"],
                 "activity":{"value":"Record invoice"},
                 "timestamp":{"date":"CPUDT","time":"CPUTM","offset":"+01:00"},
                 "objects":[{"object":"invoice","columns":["BELNR","GJAHR"],"qualifier":"invoice"}]},
                {"id":"receipt","input":"ekbe","namespace":"sap","key":["BELNR","GJAHR"],
                 "where":[{"column":"VGABE","in":["2"]}],
                 "activity":{"value":"Post invoice receipt"},"optional_timestamp":true,
                 "timestamp":{"date":"BUDAT","offset":"+01:00"},
                 "objects":[{"object":"invoice","columns":["BELNR","GJAHR"],"qualifier":"invoice"}]},
                {"id":"paid","input":"bsak","namespace":"sap","key":["BUKRS","BELNR","GJAHR"],
                 "activity":{"value":"Clear invoice"},
                 "timestamp":{"date":"AUGDT","offset":"Z"},
                 "objects":[]}
            ],
            "event_links":[{"id":"paid_invoice","input":"bkpf","event":"paid","event_key":["BUKRS","BELNR","GJAHR"],
                "where":[{"column":"AWTYP","in":["RMRP"]}],"optional_event":true,
                "target":{"object":"invoice","qualifier":"paid invoice","columns":[
                    {"column":"AWKEY","start":0,"length":10},{"column":"AWKEY","start":10,"length":4}]}}]
        }))
        .unwrap();
        let inputs = inputs
            .iter()
            .map(|id| ((*id).to_owned(), dir.join(format!("{id}.csv"))))
            .collect::<BTreeMap<_, _>>();
        let outcome = checked(&recipe, &inputs).unwrap();
        let report = &outcome.report;
        assert_eq!(report.issue_count, 0, "{:?}", report.issues);
        // Same invoice number, two fiscal years: two invoices.
        assert_eq!(report.objects, 2);
        assert_eq!(report.rules["receipt"].filtered, 1);
        assert_eq!(report.rules["receipt"].skipped, 1);
        assert_eq!(report.rules["paid_invoice"].filtered, 1);
        assert_eq!(report.rules["paid_invoice"].skipped, 1);
        assert_eq!(report.rules["paid_invoice"].emitted, 1);
        let log: Value = serde_json::to_value(outcome.log.as_ref().unwrap()).unwrap();
        let times = log["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["type"].as_str().unwrap(), e["time"].as_str().unwrap()))
            .collect::<BTreeSet<_>>();
        assert!(times.contains(&("Record invoice", "2025-12-30T17:05:00+01:00")));
        assert!(times.contains(&("Post invoice receipt", "2025-12-30T00:00:00+01:00")));
        assert!(times.contains(&("Clear invoice", "2026-01-15T00:00:00+00:00")));
        let paid = log["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "Clear invoice")
            .unwrap();
        assert_eq!(
            paid["relationships"][0]["objectId"],
            r#"["sap","invoice",["5100000001","2025"]]"#
        );
        // New recipe fields are written back only when used.
        let saved = serde_json::to_value(&recipe).unwrap();
        assert!(saved["objects"][0].get("where").is_none());
        assert_eq!(saved["events"][0]["timestamp"]["date"], "CPUDT");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_bad_offsets_and_reports_bad_dates() {
        let dir = std::env::temp_dir().join(format!("pm-prepare-dates-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("t.csv"),
            "ID,D\n1,20261399\n2,2026-03-01\n3,2026-03-01 09:30:00\n",
        )
        .unwrap();
        let recipe = |offset: &str| -> Recipe {
            serde_json::from_value(json!({
                "version":1,"format":"ocel","inputs":[{"id":"t","sha256":"a".repeat(64)}],
                "objects":[{"id":"o","input":"t","namespace":"x","object_type":"thing","key":["ID"]}],
                "events":[{"id":"e","input":"t","namespace":"x","key":["ID"],"activity":{"value":"Seen"},
                    "timestamp":{"date":"D","offset":offset},
                    "objects":[{"object":"o","columns":["ID"],"qualifier":"thing"}]}]
            }))
            .unwrap()
        };
        let inputs = BTreeMap::from([("t".to_owned(), dir.join("t.csv"))]);
        assert!(checked(&recipe("local"), &inputs).is_err());
        let outcome = checked(&recipe("-05:00"), &inputs).unwrap();
        assert_eq!(outcome.report.issue_count, 1);
        assert_eq!(
            outcome.report.issues[0].code,
            "invalid_or_missing_timestamp"
        );
        assert_eq!(outcome.report.issues[0].row, 1);
        let log = serde_json::to_value(outcome.log.as_ref()).unwrap();
        assert!(log.is_null(), "an issue blocks publication");
        assert!(erp_date_time("2026-03-01 09:30:00").is_some());
        assert_eq!(
            erp_date_time("20261015123456.1234567"),
            chrono::NaiveDate::from_ymd_opt(2026, 10, 15)
                .unwrap()
                .and_hms_nano_opt(12, 34, 56, 123_456_700)
        );
        assert!(erp_date_time("20261315123456.1234567").is_none());
        assert_eq!(
            erp_date("2008-01-01T12:34:56Z"),
            chrono::NaiveDate::from_ymd_opt(2008, 1, 1)
        );
        let iso_row = Row {
            headers: &csv::StringRecord::from(vec!["D"]),
            record: csv::StringRecord::from(vec!["2008-01-01T12:34:56Z"]),
            extra: Vec::new(),
        };
        let source_clock = Timestamp::Parts(TimestampParts {
            date: "D".into(),
            time: None,
            offset: "+02:00".into(),
        });
        let Time::At(at) = event_time(&iso_row, &source_clock).unwrap() else {
            panic!("the explicit source offset must be preserved")
        };
        assert_eq!(at.to_rfc3339(), "2008-01-01T12:34:56+00:00");
        // Database exports keep fractions of a second; whole-number times
        // lose their leading zeros.
        assert_eq!(
            erp_date("2026-03-01 00:00:00.000"),
            chrono::NaiveDate::from_ymd_opt(2026, 3, 1)
        );
        assert_eq!(erp_time("93005"), chrono::NaiveTime::from_hms_opt(9, 30, 5));
        // JD Edwards' Julian dates.
        assert_eq!(
            erp_date("126014"),
            chrono::NaiveDate::from_ymd_opt(2026, 1, 14)
        );
        assert_eq!(
            erp_date("99365"),
            chrono::NaiveDate::from_ymd_opt(1999, 12, 31)
        );
        assert_eq!(erp_date("126400"), None);
        assert_eq!(erp_time("5"), chrono::NaiveTime::from_hms_opt(0, 0, 5));
        assert_eq!(
            erp_time("143025"),
            chrono::NaiveTime::from_hms_opt(14, 30, 25)
        );
        assert_eq!(
            erp_time("9:30:05"),
            chrono::NaiveTime::from_hms_opt(9, 30, 5)
        );
        assert_eq!(
            erp_time("2500"),
            None.or(chrono::NaiveTime::from_hms_opt(0, 25, 0))
        );
        assert_eq!(
            erp_date("2026-03-01T09:30:00"),
            chrono::NaiveDate::from_ymd_opt(2026, 3, 1)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Each changed field takes its time and user from the one change header
    /// with the same object and change number; a field with no header blocks.
    #[test]
    fn a_lookup_brings_the_change_time_beside_each_changed_field() {
        let dir = std::env::temp_dir().join(format!("pm-prepare-lookup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("hdr.csv"),
            "OBJECTID,CHANGENR,UDATE,UTIME,USERNAME\n4500000001,0001,20260105,101500,LHEAD\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("pos.csv"),
            "OBJECTID,CHANGENR,FNAME,VALUE_NEW\n4500000001,0001,FRGKE,R\n4500000001,0002,FRGKE,R\n",
        )
        .unwrap();
        let recipe = |optional: bool| -> Recipe {
            serde_json::from_value(json!({
                "version":1,"format":"ocel",
                "inputs":[{"id":"hdr","sha256":"a".repeat(64)},{"id":"pos","sha256":"b".repeat(64)}],
                "objects":[{"id":"order","input":"hdr","namespace":"sap","object_type":"order","key":["OBJECTID"]}],
                "events":[{"id":"released","input":"pos","namespace":"sap","key":["OBJECTID","CHANGENR","FNAME"],
                    "activity":{"value":"Release order"},
                    "lookup":{"input":"hdr","key":["OBJECTID","CHANGENR"],"matching":["OBJECTID","CHANGENR"],
                        "columns":["UDATE","UTIME","USERNAME"],"optional":optional},
                    "timestamp":{"date":"UDATE","time":"UTIME","offset":"+01:00"},
                    "attributes":[{"column":"USERNAME","name":"user"}],
                    "objects":[{"object":"order","columns":["OBJECTID"],"qualifier":"order"}]}]
            }))
            .unwrap()
        };
        let inputs = BTreeMap::from([
            ("hdr".to_owned(), dir.join("hdr.csv")),
            ("pos".to_owned(), dir.join("pos.csv")),
        ]);
        let blocked = checked(&recipe(false), &inputs).unwrap();
        assert_eq!(blocked.report.issues[0].code, "unmatched_lookup");
        assert_eq!(blocked.report.issues[0].row, 2);
        let outcome = checked(&recipe(true), &inputs).unwrap();
        assert_eq!(outcome.report.issue_count, 0);
        assert_eq!(outcome.report.rules["released"].skipped, 1);
        let log = serde_json::to_value(outcome.log.as_ref().unwrap()).unwrap();
        assert_eq!(log["events"][0]["time"], "2026-01-05T10:15:00+01:00");
        assert_eq!(log["events"][0]["attributes"][0]["value"], "LHEAD");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
