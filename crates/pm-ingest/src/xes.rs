//! Streaming XES and XES.gz ingestion.
//!
//! XES stores one trace per case and nested attributes rather than a flat
//! table. This adapter presents the standard fields as the same [`Row`] shape
//! used by CSV and Parquet, so validation and every downstream analysis remain
//! unchanged.
//!
//! Standard event attributes are mapped as follows:
//!
//! - `concept:name` → `activity`
//! - `time:timestamp` → `timestamp`
//! - `org:resource` → `resource`
//! - `lifecycle:transition` → `lifecycle`
//! - `cost:total` or `cost:amount` → `cost`
//!
//! Trace attributes other than `concept:name` are repeated onto their events
//! and therefore become ordinary case attributes. Unknown event attributes
//! are not yet represented by the canonical event model.

use crate::read::{ReadError, Row, RowStream, Schema, Table};
use flate2::read::MultiGzDecoder;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::rc::Rc;

const CASE_ID: &str = "case_id";
const ACTIVITY: &str = "activity";
const TIMESTAMP: &str = "timestamp";
const RESOURCE: &str = "resource";
const LIFECYCLE: &str = "lifecycle";
const COST: &str = "cost";

/// Whether a path looks like an XES file, compressed or plain.
pub fn is_xes(path: impl AsRef<Path>) -> bool {
    let name = path
        .as_ref()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.ends_with(".xes") || name.ends_with(".xes.gz")
}

/// Changes whenever the reader would produce different rows from the same
/// file, so kept copies of older rows are not reused. 2: step attributes
/// become columns and rows are numbered by step.
pub const READER_REVISION: u32 = 2;

/// Streams XES events as canonical rows.
///
/// The first pass discovers trace attributes and optional standard event
/// fields; the returned iterator performs the event-producing pass. Both are
/// streaming and bounded in memory.
pub fn stream_xes(path: impl AsRef<Path>) -> Result<RowStream, ReadError> {
    let path = path.as_ref();
    let discovery = discover(path)?;
    let mut headers = vec![
        CASE_ID.to_owned(),
        ACTIVITY.to_owned(),
        TIMESTAMP.to_owned(),
    ];
    if discovery.resource {
        headers.push(RESOURCE.to_owned());
    }
    if discovery.lifecycle {
        headers.push(LIFECYCLE.to_owned());
    }
    if discovery.cost {
        headers.push(COST.to_owned());
    }
    headers.extend(discovery.trace_columns.values().cloned());
    headers.extend(discovery.event_columns.values().cloned());

    let schema = Rc::new(Schema::new(headers));
    let rows = XesRows::open(
        path,
        Rc::clone(&schema),
        discovery.trace_columns,
        discovery.event_columns,
    )?;
    Ok((schema, Box::new(rows)))
}

/// Reads a complete XES file. Product analysis uses [`stream_xes`].
pub fn read_xes(path: impl AsRef<Path>) -> Result<Table, ReadError> {
    let (schema, rows) = stream_xes(path)?;
    Ok(Table {
        schema,
        rows: rows.collect::<Result<Vec<_>, _>>()?,
    })
}

#[derive(Default)]
struct Discovery {
    trace_columns: BTreeMap<String, String>,
    /// Other attributes recorded on events, such as an amount or a user, by
    /// key, with their column names.
    event_columns: BTreeMap<String, String>,
    resource: bool,
    lifecycle: bool,
    cost: bool,
}

fn discover(path: &Path) -> Result<Discovery, ReadError> {
    let display = path.display().to_string();
    let mut reader = Reader::from_reader(open_input(path)?);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut discovery = Discovery::default();
    let mut depth = 0_usize;
    let mut trace_depth = None;
    let mut event_depth = None;
    let mut saw_log = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(element)) => {
                depth += 1;
                match local(element.name()) {
                    b"log" => saw_log = true,
                    b"trace" => trace_depth = Some(depth),
                    b"event" if trace_depth.is_some() => event_depth = Some(depth),
                    _ if is_attribute(element.name()) => {
                        discover_attribute(
                            &reader,
                            &element,
                            direct_child_of(trace_depth, depth) && event_depth.is_none(),
                            direct_child_of(event_depth, depth),
                            &display,
                            &mut discovery,
                        )?;
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(element)) if is_attribute(element.name()) => {
                let element_depth = depth + 1;
                discover_attribute(
                    &reader,
                    &element,
                    direct_child_of(trace_depth, element_depth) && event_depth.is_none(),
                    direct_child_of(event_depth, element_depth),
                    &display,
                    &mut discovery,
                )?;
            }
            Ok(Event::End(element)) => {
                match local(element.name()) {
                    b"event" => event_depth = None,
                    b"trace" => trace_depth = None,
                    _ => {}
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(xml_error(&display, error)),
        }
        buf.clear();
    }

    if !saw_log {
        return Err(ReadError::Xes {
            path: display,
            detail: "no <log> element found".to_owned(),
        });
    }
    Ok(discovery)
}

fn discover_attribute(
    reader: &Reader<XesInput>,
    element: &BytesStart<'_>,
    in_trace: bool,
    in_event: bool,
    path: &str,
    discovery: &mut Discovery,
) -> Result<(), ReadError> {
    let Some((key, _)) = key_value(reader, element, path)? else {
        return Ok(());
    };
    if in_event {
        match key.as_str() {
            "org:resource" => discovery.resource = true,
            "lifecycle:transition" => discovery.lifecycle = true,
            "cost:total" | "cost:amount" => discovery.cost = true,
            "concept:name" | "time:timestamp" => {}
            _ => {
                if !discovery.event_columns.contains_key(&key) {
                    let column = event_column(
                        &key,
                        discovery
                            .trace_columns
                            .values()
                            .chain(discovery.event_columns.values()),
                    );
                    discovery.event_columns.insert(key, column);
                }
            }
        }
    } else if in_trace && key != "concept:name" {
        let column = case_column(&key, discovery.trace_columns.values());
        discovery.trace_columns.entry(key).or_insert(column);
    }
    Ok(())
}

/// A column name for an event attribute that collides with no other.
fn event_column<'a>(key: &str, existing: impl Iterator<Item = &'a String>) -> String {
    let reserved = [CASE_ID, ACTIVITY, TIMESTAMP, RESOURCE, LIFECYCLE, COST];
    let existing: BTreeSet<&str> = existing.map(String::as_str).collect();
    let mut column = key.to_owned();
    while reserved.contains(&column.as_str()) || existing.contains(column.as_str()) {
        column.insert_str(0, "event:");
    }
    column
}

fn case_column<'a>(key: &str, existing: impl Iterator<Item = &'a String>) -> String {
    let reserved = [CASE_ID, ACTIVITY, TIMESTAMP, RESOURCE, LIFECYCLE, COST];
    let mut column = if reserved.contains(&key) {
        format!("case:{key}")
    } else {
        key.to_owned()
    };
    let existing: BTreeSet<&str> = existing.map(String::as_str).collect();
    while existing.contains(column.as_str()) {
        column.insert_str(0, "case:");
    }
    column
}

struct XesRows {
    display: String,
    reader: Reader<XesInput>,
    buf: Vec<u8>,
    schema: Rc<Schema>,
    trace_columns: BTreeMap<String, String>,
    event_columns: BTreeMap<String, String>,
    trace_values: BTreeMap<String, String>,
    event_values: BTreeMap<String, String>,
    case_id: String,
    depth: usize,
    trace_depth: Option<usize>,
    event_depth: Option<usize>,
    event_line: usize,
    /// Events read so far: an event's row number. XML lines cannot serve,
    /// since many exports put the whole file on one line.
    events: usize,
    done: bool,
}

impl XesRows {
    fn open(
        path: &Path,
        schema: Rc<Schema>,
        trace_columns: BTreeMap<String, String>,
        event_columns: BTreeMap<String, String>,
    ) -> Result<Self, ReadError> {
        let mut reader = Reader::from_reader(open_input(path)?);
        reader.config_mut().trim_text(true);
        Ok(Self {
            display: path.display().to_string(),
            reader,
            buf: Vec::new(),
            schema,
            trace_columns,
            event_columns,
            trace_values: BTreeMap::new(),
            event_values: BTreeMap::new(),
            case_id: String::new(),
            depth: 0,
            trace_depth: None,
            event_depth: None,
            event_line: 1,
            events: 0,
            done: false,
        })
    }

    fn observe_attribute(
        &mut self,
        element: &BytesStart<'_>,
        element_depth: usize,
    ) -> Result<(), ReadError> {
        let Some((key, value)) = key_value(&self.reader, element, &self.display)? else {
            return Ok(());
        };
        if direct_child_of(self.event_depth, element_depth) {
            self.event_values.insert(key, value);
        } else if self.event_depth.is_none() && direct_child_of(self.trace_depth, element_depth) {
            if key == "concept:name" {
                self.case_id = value;
            } else {
                self.trace_values.insert(key, value);
            }
        }
        Ok(())
    }

    fn finish_event(&mut self) -> Result<Row, ReadError> {
        if self.case_id.is_empty() {
            return Err(ReadError::Xes {
                path: self.display.clone(),
                detail: format!(
                    "trace containing the event near line {} has no concept:name case identifier",
                    self.event_line
                ),
            });
        }
        let mut values = vec![String::new(); self.schema.len()];
        set(&self.schema, &mut values, CASE_ID, &self.case_id);
        set_event(
            &self.schema,
            &mut values,
            ACTIVITY,
            &self.event_values,
            &["concept:name"],
        );
        set_event(
            &self.schema,
            &mut values,
            TIMESTAMP,
            &self.event_values,
            &["time:timestamp"],
        );
        set_event(
            &self.schema,
            &mut values,
            RESOURCE,
            &self.event_values,
            &["org:resource"],
        );
        set_event(
            &self.schema,
            &mut values,
            LIFECYCLE,
            &self.event_values,
            &["lifecycle:transition"],
        );
        set_event(
            &self.schema,
            &mut values,
            COST,
            &self.event_values,
            &["cost:total", "cost:amount"],
        );
        for (key, column) in &self.event_columns {
            if let Some(value) = self.event_values.get(key) {
                set(&self.schema, &mut values, column, value);
            }
        }
        for (key, value) in &self.trace_values {
            if let Some(column) = self.trace_columns.get(key) {
                set(&self.schema, &mut values, column, value);
            }
        }
        Ok(Row::new(Rc::clone(&self.schema), values, self.events))
    }
}

impl Iterator for XesRows {
    type Item = Result<Row, ReadError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            let event = match self.reader.read_event_into(&mut self.buf) {
                Ok(event) => event.into_owned(),
                Err(error) => {
                    self.done = true;
                    return Some(Err(xml_error(&self.display, error)));
                }
            };
            self.buf.clear();
            match event {
                Event::Start(element) => {
                    self.depth += 1;
                    match local(element.name()) {
                        b"trace" => {
                            self.trace_depth = Some(self.depth);
                            self.trace_values.clear();
                            self.case_id.clear();
                        }
                        b"event" if self.trace_depth.is_some() => {
                            self.event_depth = Some(self.depth);
                            self.event_values.clear();
                            self.event_line = self.reader.get_ref().line();
                            self.events += 1;
                        }
                        _ if is_attribute(element.name()) => {
                            if let Err(error) = self.observe_attribute(&element, self.depth) {
                                self.done = true;
                                return Some(Err(error));
                            }
                        }
                        _ => {}
                    }
                }
                Event::Empty(element) if is_attribute(element.name()) => {
                    if let Err(error) = self.observe_attribute(&element, self.depth + 1) {
                        self.done = true;
                        return Some(Err(error));
                    }
                }
                Event::End(element) => {
                    let finished = match local(element.name()) {
                        b"event" if self.event_depth.is_some() => {
                            self.event_depth = None;
                            Some(self.finish_event())
                        }
                        b"trace" => {
                            self.trace_depth = None;
                            None
                        }
                        _ => None,
                    };
                    self.depth = self.depth.saturating_sub(1);
                    if let Some(row) = finished {
                        return Some(row);
                    }
                }
                Event::Eof => {
                    self.done = true;
                    return None;
                }
                _ => {}
            }
        }
    }
}

fn direct_child_of(parent_depth: Option<usize>, element_depth: usize) -> bool {
    parent_depth.is_some_and(|parent| element_depth == parent + 1)
}

fn set_event(
    schema: &Schema,
    values: &mut [String],
    column: &str,
    attributes: &BTreeMap<String, String>,
    keys: &[&str],
) {
    if let Some(value) = keys.iter().find_map(|key| attributes.get(*key)) {
        set(schema, values, column, value);
    }
}

fn set(schema: &Schema, values: &mut [String], column: &str, value: &str) {
    if let Some(index) = schema.position(column) {
        values[index] = value.to_owned();
    }
}

fn key_value(
    reader: &Reader<XesInput>,
    element: &BytesStart<'_>,
    path: &str,
) -> Result<Option<(String, String)>, ReadError> {
    let mut key = None;
    let mut value = None;
    for attribute in element.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| ReadError::Xes {
            path: path.to_owned(),
            detail: error.to_string(),
        })?;
        match local(attribute.key) {
            b"key" => {
                key = Some(
                    attribute
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .map_err(|error| xml_error(path, error))?
                        .into_owned(),
                )
            }
            b"value" => {
                value = Some(
                    attribute
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .map_err(|error| xml_error(path, error))?
                        .into_owned(),
                )
            }
            _ => {}
        }
    }
    Ok(key.zip(value))
}

fn is_attribute(name: QName<'_>) -> bool {
    matches!(
        local(name),
        b"string" | b"date" | b"int" | b"float" | b"boolean" | b"id"
    )
}

fn local(name: QName<'_>) -> &[u8] {
    name.local_name().into_inner()
}

fn xml_error(path: &str, error: quick_xml::Error) -> ReadError {
    ReadError::Xes {
        path: path.to_owned(),
        detail: error.to_string(),
    }
}

fn open_input(path: &Path) -> Result<XesInput, ReadError> {
    let display = path.display().to_string();
    let file = File::open(path).map_err(|source| ReadError::Io {
        path: display,
        source,
    })?;
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".gz"))
    {
        Ok(XesInput::Gzip(Box::new(LineReader::new(BufReader::new(
            MultiGzDecoder::new(file),
        )))))
    } else {
        Ok(XesInput::Plain(LineReader::new(BufReader::new(file))))
    }
}

/// Buffered input that retains the current XML line for evidence anchors.
struct LineReader<R> {
    inner: R,
    line: usize,
}

impl<R> LineReader<R> {
    fn new(inner: R) -> Self {
        Self { inner, line: 1 }
    }
}

impl<R: BufRead> Read for LineReader<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let available = self.fill_buf()?;
        let amount = available.len().min(output.len());
        output[..amount].copy_from_slice(&available[..amount]);
        self.consume(amount);
        Ok(amount)
    }
}

impl<R: BufRead> BufRead for LineReader<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        if let Ok(buffer) = self.inner.fill_buf() {
            self.line += buffer[..amount.min(buffer.len())]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count();
        }
        self.inner.consume(amount);
    }
}

enum XesInput {
    Plain(LineReader<BufReader<File>>),
    Gzip(Box<LineReader<BufReader<MultiGzDecoder<File>>>>),
}

impl XesInput {
    fn line(&self) -> usize {
        match self {
            Self::Plain(reader) => reader.line,
            Self::Gzip(reader) => reader.line,
        }
    }
}

impl Read for XesInput {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(reader) => reader.read(output),
            Self::Gzip(reader) => reader.read(output),
        }
    }
}

impl BufRead for XesInput {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        match self {
            Self::Plain(reader) => reader.fill_buf(),
            Self::Gzip(reader) => reader.fill_buf(),
        }
    }

    fn consume(&mut self, amount: usize) {
        match self {
            Self::Plain(reader) => reader.consume(amount),
            Self::Gzip(reader) => reader.consume(amount),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<log xes.version="1.0">
  <trace>
    <string key="concept:name" value="ORDER-1"/>
    <string key="region" value="EMEA"/>
    <event>
      <string key="concept:name" value="Receive"/>
      <date key="time:timestamp" value="2026-03-02T09:00:00.000+00:00"/>
      <string key="org:resource" value="alice"/>
      <float key="cost:total" value="12.50"/>
    </event>
    <event>
      <string key="concept:name" value="Ship"/>
      <date key="time:timestamp" value="2026-03-02T11:00:00.000+00:00"/>
      <string key="org:resource" value="bob"/>
    </event>
  </trace>
</log>"#;

    fn write_plain(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("pm_xes_{name}.xes"));
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// Large XES files are read through a kept copy of their rows; the copy
    /// must give exactly what the reader gives.
    #[test]
    fn a_kept_copy_matches_the_reader() {
        let path = write_plain("kept_copy", SAMPLE);
        let cache = std::env::temp_dir().join(format!("pm_xes_kept_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        let collect = |(schema, rows): RowStream| {
            let headers = schema.headers().to_vec();
            let rows = rows
                .map(|r| {
                    let r = r.unwrap();
                    let values = (0..headers.len())
                        .map(|i| r.at(i).to_owned())
                        .collect::<Vec<_>>();
                    (r.source_row, values)
                })
                .collect::<Vec<_>>();
            (headers, rows)
        };
        let direct = collect(stream_xes(&path).unwrap());
        for _ in 0..2 {
            let kept = collect(
                crate::row_cache::stream(&path, &cache, READER_REVISION, || stream_xes(&path))
                    .unwrap(),
            );
            assert_eq!(kept, direct);
        }
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);
    }

    #[test]
    fn keeps_other_event_attributes_as_columns() {
        let xes = r#"<log>
  <trace>
    <string key="concept:name" value="PO-1"/>
    <string key="region" value="EMEA"/>
    <event>
      <string key="concept:name" value="Create"/>
      <date key="time:timestamp" value="2026-03-02T09:00:00.000+00:00"/>
      <float key="Cumulative net worth (EUR)" value="420.5"/>
      <string key="region" value="North"/>
    </event>
    <event>
      <string key="concept:name" value="Pay"/>
      <date key="time:timestamp" value="2026-03-03T09:00:00.000+00:00"/>
    </event>
  </trace>
</log>"#;
        let table = read_xes(write_plain("event_attributes", xes)).unwrap();
        assert_eq!(table.rows[0].get("Cumulative net worth (EUR)"), "420.5");
        assert_eq!(table.rows[1].get("Cumulative net worth (EUR)"), "");
        // A name the trace already uses stays apart.
        assert_eq!(table.rows[0].get("region"), "EMEA");
        assert_eq!(table.rows[0].get("event:region"), "North");
    }

    #[test]
    fn recognizes_plain_and_compressed_paths() {
        assert!(is_xes("events.xes"));
        assert!(is_xes("EVENTS.XES.GZ"));
        assert!(!is_xes("events.xml"));
    }

    #[test]
    fn maps_standard_fields_and_trace_attributes() {
        let table = read_xes(write_plain("standard", SAMPLE)).unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].get(CASE_ID), "ORDER-1");
        assert_eq!(table.rows[0].get(ACTIVITY), "Receive");
        assert_eq!(
            table.rows[0].get(TIMESTAMP),
            "2026-03-02T09:00:00.000+00:00"
        );
        assert_eq!(table.rows[0].get(RESOURCE), "alice");
        assert_eq!(table.rows[0].get(COST), "12.50");
        assert_eq!(table.rows[1].get("region"), "EMEA");
        // Rows are numbered by event, whatever the XML layout.
        assert_eq!(table.rows[0].source_row, 1);
        assert_eq!(table.rows[1].source_row, 2);
    }

    #[test]
    fn streams_the_same_rows_as_eager_reading() {
        let path = write_plain("stream", SAMPLE);
        let eager = read_xes(&path).unwrap();
        let (_, stream) = stream_xes(&path).unwrap();
        assert_eq!(eager.rows, stream.collect::<Result<Vec<_>, _>>().unwrap());
    }

    #[test]
    fn nested_custom_attributes_cannot_overwrite_standard_event_fields() {
        let path = write_plain(
            "nested",
            r#"<log><trace>
  <string key="concept:name" value="CASE-1"/>
  <event>
    <string key="concept:name" value="Approve"/>
    <date key="time:timestamp" value="2026-03-02T09:00:00Z"/>
    <list key="metadata"><values>
      <string key="concept:name" value="Not the activity"/>
      <string key="org:resource" value="Not a resource"/>
    </values></list>
  </event>
</trace></log>"#,
        );
        let table = read_xes(path).unwrap();
        assert_eq!(table.rows[0].get(ACTIVITY), "Approve");
        assert!(!table.headers().contains(&RESOURCE.to_owned()));
    }

    #[test]
    fn reads_gzip_compressed_xes() {
        let path = std::env::temp_dir().join("pm_xes_compressed.xes.gz");
        let file = File::create(&path).unwrap();
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(SAMPLE.as_bytes()).unwrap();
        encoder.finish().unwrap();
        let table = read_xes(path).unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[1].get(ACTIVITY), "Ship");
    }

    #[test]
    fn generic_column_sampling_supports_xes() {
        let path = write_plain("sample_column", SAMPLE);
        assert_eq!(
            crate::read::sample_column(path, ACTIVITY, 1).unwrap(),
            vec!["Receive"]
        );
    }

    #[test]
    fn rejects_a_trace_without_a_case_identifier() {
        let path = write_plain(
            "missing_case",
            "<log><trace><event><string key=\"concept:name\" value=\"A\"/></event></trace></log>",
        );
        let error = read_xes(path).unwrap_err().to_string();
        assert!(error.contains("no concept:name case identifier"));
    }
}
