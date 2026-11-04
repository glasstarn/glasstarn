//! A saved connected log read in pieces from disk.
//!
//! A large log does not fit in memory, so the engine copies its structure
//! into a scratch database with numbered rows: events and items numbered
//! exactly as the reader numbers them, and every link with the types of both
//! ends. Questions then read it in the order each calculation needs (item by
//! item, event by event, link by link), and SQLite sorts on disk. Memory
//! stays small however large the log; only small summaries are kept.
//!
//! The numbering and every check follow the reader, so calculations fed from
//! here give exactly what they give on the loaded log.
use super::{
    Plain, object_assignments, parse_time, quote_identifier, read_type_tables,
    reject_orphan_type_rows, sql_text, table_names, validate_entity_types,
};
use crate::OcelSqliteError;
use crate::ocel_quality::{OcelQualityKind, OcelQualityReport, push, report};
use pm_core::{
    AttributeValue, EventTypeId, Micros, ObjectId, ObjectTypeId, OcelBuildError, OcelEventId,
    OcelFilter, OcelFilterError, QualifierId, TimelineStep, UndeclaredLinks,
};
use rusqlite::{Connection, OpenFlags, params};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The scratch database's file, removed when the log is dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub type LinkedIntervalRow = (ObjectId, Option<Micros>, Option<Micros>);

/// A saved log on disk, numbered as the reader numbers it.
pub struct DiskLog {
    connection: Connection,
    _scratch: Option<Scratch>,
    pub event_types: Vec<String>,
    pub object_types: Vec<String>,
    pub qualifiers: Vec<String>,
    pub attribute_names: Vec<String>,
    pub events: usize,
    pub objects: usize,
    pub event_object_relations: usize,
    pub object_object_relations: usize,
    pub undeclared_links: UndeclaredLinks,
    event_object_qualifiers: BTreeSet<u32>,
    object_object_qualifiers: BTreeSet<u32>,
    /// The saved log, and each item type's table there.
    source: PathBuf,
    object_tables: Vec<ObjectTable>,
    quality: Option<OcelQualityReport>,
}

/// An item type's table in the saved log, for reading attribute values.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ObjectTable {
    pub table: String,
    pub changed_field: bool,
    pub attributes: Vec<(String, pm_core::AttributeType)>,
}

/// The part of a log a filter selects, as conditions on the scratch tables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskView {
    object_ids: Option<BTreeSet<u32>>,
    object_types: Option<BTreeSet<u32>>,
    event_object_qualifiers: Option<BTreeSet<u32>>,
    object_object_qualifiers: Option<BTreeSet<u32>>,
}

/// The ids as an SQL list; an empty set matches nothing.
fn list(ids: &BTreeSet<u32>) -> String {
    if ids.is_empty() {
        return "NULL".into();
    }
    ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

impl DiskView {
    pub fn is_whole(&self) -> bool {
        *self == Self::default()
    }

    /// The same view narrowed to items of one type.
    pub fn only_type(&self, object_type: ObjectTypeId) -> Self {
        let mut narrowed = self.clone();
        let keep = self
            .object_types
            .as_ref()
            .is_none_or(|types| types.contains(&object_type.0));
        narrowed.object_types = Some(if keep {
            BTreeSet::from([object_type.0])
        } else {
            BTreeSet::new()
        });
        narrowed
    }

    /// Conditions on `eo` rows (alias `l`).
    fn links(&self) -> String {
        self.links_as("l")
    }

    /// Conditions on `eo` rows with the given alias.
    fn links_as(&self, alias: &str) -> String {
        let mut conditions = vec!["1".to_owned()];
        if let Some(ids) = &self.object_ids {
            conditions.push(format!("{alias}.object IN ({})", list(ids)));
        }
        if let Some(types) = &self.object_types {
            conditions.push(format!("{alias}.otype IN ({})", list(types)));
        }
        if let Some(qualifiers) = &self.event_object_qualifiers {
            conditions.push(format!("{alias}.qualifier IN ({})", list(qualifiers)));
        }
        conditions.join(" AND ")
    }

    /// Conditions on `ob` rows (alias `o`).
    fn objects(&self) -> String {
        let types = match &self.object_types {
            Some(types) => format!("o.type IN ({})", list(types)),
            None => "1".into(),
        };
        match &self.object_ids {
            Some(ids) => format!("{types} AND o.id IN ({})", list(ids)),
            None => types,
        }
    }

    /// Conditions on `oo` rows (alias `r`).
    fn relationships(&self) -> String {
        let mut conditions = vec!["1".to_owned()];
        if let Some(ids) = &self.object_ids {
            conditions.push(format!(
                "r.source IN ({0}) AND r.target IN ({0})",
                list(ids)
            ));
        }
        if let Some(types) = &self.object_types {
            conditions.push(format!(
                "r.stype IN ({0}) AND r.ttype IN ({0})",
                list(types)
            ));
        }
        if let Some(qualifiers) = &self.object_object_qualifiers {
            conditions.push(format!("r.qualifier IN ({})", list(qualifiers)));
        }
        conditions.join(" AND ")
    }
}

/// Prints how long each step took when `PM_DISK_TIMING` is set.
fn lap(started: &std::time::Instant, step: &str) {
    if std::env::var_os("PM_DISK_TIMING").is_some() {
        eprintln!("{:>8.2}s {step}", started.elapsed().as_secs_f64());
    }
}

fn count(connection: &Connection, sql: &str) -> Result<usize, OcelSqliteError> {
    Ok(connection.query_row(sql, [], |row| row.get::<_, i64>(0))? as usize)
}

impl DiskLog {
    /// Copies the structure of the log at `path` into a scratch database in
    /// `scratch`, removed when the log is dropped. Fails as reading the log
    /// fails.
    pub fn open(path: &Path, scratch: &Path) -> Result<Self, OcelSqliteError> {
        static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let file = scratch.join(format!(
            "pm-disk-{}-{}.sqlite",
            std::process::id(),
            RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let guard = Scratch(file.clone());
        let mut log = Self::build(path, &file)?;
        log._scratch = Some(guard);
        Ok(log)
    }

    /// Like [`Self::open`], but keeps the copy in `cache` for the next
    /// question about the same unchanged log; at most a few copies are kept.
    pub fn open_cached(path: &Path, cache: &Path) -> Result<Self, OcelSqliteError> {
        let metadata = std::fs::metadata(path).map_err(|error| OcelSqliteError::Open {
            path: path.to_owned(),
            source: rusqlite::Error::InvalidPath(PathBuf::from(error.to_string())),
        })?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        // FNV-1a over the path, size and change time.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in format!(
            "{}|{}|{modified}|{FORMAT}",
            canonical.display(),
            metadata.len()
        )
        .bytes()
        {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        std::fs::create_dir_all(cache).map_err(|error| {
            OcelSqliteError::Data(format!(
                "cannot use the scratch folder {}: {error}",
                cache.display()
            ))
        })?;
        let file = cache.join(format!("pm-disk-{hash:016x}.sqlite"));
        if file.is_file() {
            if let Ok(log) = Self::reopen(&file) {
                let _ = filetime_touch(&file);
                return Ok(log);
            }
            let _ = std::fs::remove_file(&file);
        }
        static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let building = cache.join(format!(
            "pm-disk-{hash:016x}.{}-{}.building",
            std::process::id(),
            RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let guard = Scratch(building.clone());
        drop(Self::build(path, &building)?);
        std::fs::rename(&building, &file).map_err(|error| {
            OcelSqliteError::Data(format!("cannot keep the scratch copy: {error}"))
        })?;
        drop(guard);
        evict(cache, &file);
        Self::reopen(&file)
    }

    /// Opens a kept copy.
    fn reopen(file: &Path) -> Result<Self, OcelSqliteError> {
        let connection = Connection::open_with_flags(
            file,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.execute_batch("PRAGMA cache_size = -16384; PRAGMA temp_store = FILE;")?;
        let text: String = connection.query_row("SELECT value FROM meta", [], |row| row.get(0))?;
        let meta: Meta = serde_json::from_str(&text)
            .map_err(|_| OcelSqliteError::Data("the scratch copy is not readable".into()))?;
        if meta.format != FORMAT {
            return Err(OcelSqliteError::Data(
                "the scratch copy is out of date".into(),
            ));
        }
        Ok(Self {
            connection,
            _scratch: None,
            event_types: meta.event_types,
            object_types: meta.object_types,
            qualifiers: meta.qualifiers,
            attribute_names: meta.attribute_names,
            events: meta.counts[0],
            objects: meta.counts[1],
            event_object_relations: meta.counts[2],
            object_object_relations: meta.counts[3],
            undeclared_links: UndeclaredLinks {
                count: meta.undeclared.0,
                examples: meta.undeclared.1,
            },
            event_object_qualifiers: meta.event_object_qualifiers,
            object_object_qualifiers: meta.object_object_qualifiers,
            source: meta.source,
            object_tables: meta.object_tables,
            quality: Some(meta.quality),
        })
    }

    fn build(path: &Path, file: &Path) -> Result<Self, OcelSqliteError> {
        let started = std::time::Instant::now();
        let source = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|source| OcelSqliteError::Open {
            path: path.to_owned(),
            source,
        })?;
        source.pragma_update(None, "query_only", true)?;
        let tables = table_names(&source)?;
        for required in ["event", "object", "event_object", "object_object"] {
            if !tables.contains(required) {
                return Err(OcelSqliteError::Schema(format!(
                    "required table '{required}' is missing"
                )));
            }
        }
        let event_tables = read_type_tables(&source, &tables, true)?;
        let object_tables = read_type_tables(&source, &tables, false)?;
        validate_entity_types(&source, "event", &event_tables)?;
        validate_entity_types(&source, "object", &object_tables)?;
        // Attribute names are numbered as declared: event types, then items.
        let mut attribute_names = Vec::<String>::new();
        for declared in event_tables.iter().chain(&object_tables) {
            for (name, _) in &declared.attributes {
                if !attribute_names.contains(name) {
                    attribute_names.push(name.clone());
                }
            }
        }

        let _ = std::fs::remove_file(file);
        let connection = Connection::open_with_flags(
            file,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let uri = format!(
            "file:{}?mode=ro",
            path.to_string_lossy()
                .replace('%', "%25")
                .replace('?', "%3f")
                .replace('#', "%23")
        );
        connection.execute("ATTACH DATABASE ?1 AS src", [uri])?;
        connection.execute_batch(
            "PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF; PRAGMA temp_store = FILE; PRAGMA soft_heap_limit = 134217728;
             PRAGMA cache_size = -16384;
             CREATE TABLE ev (id INTEGER PRIMARY KEY, label TEXT NOT NULL, type INTEGER NOT NULL, at INTEGER NOT NULL, res TEXT);
             CREATE TABLE ob (id INTEGER PRIMARY KEY, label TEXT NOT NULL, type INTEGER NOT NULL, short TEXT);
             CREATE TABLE eo (rid INTEGER PRIMARY KEY, event INTEGER NOT NULL, object INTEGER NOT NULL,
                              qualifier INTEGER NOT NULL, etype INTEGER NOT NULL, otype INTEGER NOT NULL, at INTEGER NOT NULL);
             CREATE TABLE oo (rid INTEGER PRIMARY KEY, source INTEGER NOT NULL, target INTEGER NOT NULL,
                              qualifier INTEGER NOT NULL, stype INTEGER NOT NULL, ttype INTEGER NOT NULL);
             CREATE TABLE oattr (seq INTEGER PRIMARY KEY, label TEXT NOT NULL, name INTEGER NOT NULL,
                                 at INTEGER NOT NULL, value TEXT NOT NULL);
             BEGIN;",
        )?;

        // Items, in file order, as the reader adds them first.
        let object_type_ids = object_tables
            .iter()
            .enumerate()
            .map(|(id, t)| (t.name.clone(), id as i64))
            .collect::<HashMap<_, _>>();
        {
            let mut insert = connection.prepare("INSERT INTO ob VALUES (?1, ?2, ?3, ?4)")?;
            let mut statement =
                source.prepare("SELECT ocel_id, ocel_type FROM object ORDER BY rowid")?;
            let mut rows = statement.query([])?;
            let mut id = 0i64;
            while let Some(row) = rows.next()? {
                let label = sql_text(row.get_ref(0)?, "object.ocel_id")?;
                let type_name = sql_text(row.get_ref(1)?, "object.ocel_type")?;
                let kind = object_type_ids
                    .get(&type_name)
                    .ok_or(OcelBuildError::UnknownObjectType(type_name.clone()))?;
                insert.execute(params![id, label, kind, super::short_label(&label)])?;
                id += 1;
            }
        }
        lap(&started, "objects");
        unique(&connection, "ob", |label| {
            OcelBuildError::DuplicateObject(label)
        })?;
        // Events in the event table's own order, as the reader numbers them:
        // inserted under their row in that table, then renumbered from 0.
        {
            let mut insert = connection.prepare("INSERT INTO ev VALUES (?1, ?2, ?3, ?4, ?5)")?;
            for (kind, declared) in event_tables.iter().enumerate() {
                reject_orphan_type_rows(&source, declared, "event")?;
                // Who did the step, when the type records it.
                let performer = PERFORMER_ATTRIBUTES
                    .iter()
                    .find(|name| declared.attributes.iter().any(|(a, _)| a == *name))
                    .map_or("NULL".to_owned(), |name| {
                        format!("typed.{}", quote_identifier(name))
                    });
                let sql = format!(
                    "SELECT entity.rowid, entity.ocel_id, typed.ocel_time, {performer} FROM event AS entity \
                     LEFT JOIN {} AS typed ON typed.ocel_id = entity.ocel_id \
                     WHERE entity.ocel_type = ?1 ORDER BY entity.rowid, typed.rowid",
                    quote_identifier(&declared.table)
                );
                let mut statement = source.prepare(&sql)?;
                let mut rows = statement.query([&declared.name])?;
                while let Some(row) = rows.next()? {
                    let position = row.get::<_, i64>(0)?;
                    let label = sql_text(row.get_ref(1)?, "event.ocel_id")?;
                    let time = sql_text(row.get_ref(2)?, &format!("event '{label}' timestamp"))?;
                    let at = parse_time(&time, &format!("event '{label}'"))?;
                    let performer = performer_text(row.get_ref(3)?);
                    insert.execute(params![position, label, kind as i64, at, performer])?;
                }
            }
        }
        connection.execute_batch(
            "UPDATE ev SET id = -id - 1;
             CREATE TEMP TABLE ev_order AS
                 SELECT id AS old, row_number() OVER (ORDER BY id DESC) - 1 AS new FROM ev;
             UPDATE ev SET id = (SELECT new FROM ev_order WHERE old = ev.id);
             DROP TABLE ev_order;",
        )?;
        lap(&started, "events");
        unique(&connection, "ev", |label| {
            OcelBuildError::DuplicateEvent(label)
        })?;
        lap(&started, "unique events");
        // Item attribute rows the data checks need: items with several rows.
        {
            let mut insert = connection
                .prepare("INSERT INTO oattr (label, name, at, value) VALUES (?1, ?2, ?3, ?4)")?;
            object_assignments(&source, &object_tables, false, |label, name, at, value| {
                let name = attribute_names
                    .iter()
                    .position(|n| n == name)
                    .expect("declared attribute") as i64;
                insert.execute(params![label, name, at, canonical(&value)])?;
                Ok(())
            })?;
            let unknown: Option<String> = connection
                .query_row(
                    "SELECT a.label FROM oattr AS a LEFT JOIN ob AS o ON o.label = a.label \
                     WHERE o.id IS NULL ORDER BY a.seq LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .ok();
            if let Some(label) = unknown {
                return Err(OcelBuildError::UnknownObject(label).into());
            }
        }

        lap(&started, "attributes");
        // Links in one pass each, in file order: an unknown event fails the
        // read; a link to an undeclared item is left out and counted, as the
        // reader does; qualifiers are numbered as first seen.
        let mut undeclared_links = UndeclaredLinks::default();
        let mut event_object_qualifiers = BTreeSet::new();
        let mut object_object_qualifiers = BTreeSet::new();
        let mut qualifiers = Vec::<String>::new();
        let mut qualifier_ids = HashMap::<String, i64>::new();
        let mut qualifier_id = |name: String| -> i64 {
            if let Some(id) = qualifier_ids.get(&name) {
                return *id;
            }
            let id = qualifiers.len() as i64;
            qualifier_ids.insert(name.clone(), id);
            qualifiers.push(name);
            id
        };
        {
            let mut insert =
                connection.prepare("INSERT INTO eo VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?;
            let mut statement = connection.prepare(
                "SELECT s.ocel_event_id, s.ocel_object_id, s.ocel_qualifier, e.id, e.type, e.at, o.id, o.type \
                 FROM src.event_object AS s \
                 LEFT JOIN ev AS e ON e.label = s.ocel_event_id \
                 LEFT JOIN ob AS o ON o.label = s.ocel_object_id ORDER BY s.rowid",
            )?;
            let mut rows = statement.query([])?;
            let mut rid = 0i64;
            while let Some(row) = rows.next()? {
                let Some(event) = row.get::<_, Option<i64>>(3)? else {
                    return Err(OcelBuildError::UnknownEvent(sql_text(
                        row.get_ref(0)?,
                        "event_object.ocel_event_id",
                    )?)
                    .into());
                };
                let qualifier = sql_text(row.get_ref(2)?, "event_object.ocel_qualifier")?;
                let Some(object) = row.get::<_, Option<i64>>(6)? else {
                    record(
                        &mut undeclared_links,
                        &sql_text(row.get_ref(0)?, "event_object.ocel_event_id")?,
                        &qualifier,
                        &sql_text(row.get_ref(1)?, "event_object.ocel_object_id")?,
                    );
                    continue;
                };
                let qualifier = qualifier_id(qualifier);
                event_object_qualifiers.insert(qualifier as u32);
                insert.execute(params![
                    rid,
                    event,
                    object,
                    qualifier,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(5)?
                ])?;
                rid += 1;
            }
        }
        lap(&started, "event links");
        {
            let mut insert =
                connection.prepare("INSERT INTO oo VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
            let mut statement = connection.prepare(
                "SELECT s.ocel_source_id, s.ocel_target_id, s.ocel_qualifier, so.id, so.type, t.id, t.type \
                 FROM src.object_object AS s \
                 LEFT JOIN ob AS so ON so.label = s.ocel_source_id \
                 LEFT JOIN ob AS t ON t.label = s.ocel_target_id ORDER BY s.rowid",
            )?;
            let mut rows = statement.query([])?;
            let mut rid = 0i64;
            while let Some(row) = rows.next()? {
                let source_label = sql_text(row.get_ref(0)?, "object_object.ocel_source_id")?;
                let target_label = sql_text(row.get_ref(1)?, "object_object.ocel_target_id")?;
                let qualifier = sql_text(row.get_ref(2)?, "object_object.ocel_qualifier")?;
                let (Some(source), Some(target)) =
                    (row.get::<_, Option<i64>>(3)?, row.get::<_, Option<i64>>(5)?)
                else {
                    let missing = if row.get::<_, Option<i64>>(3)?.is_none() {
                        &source_label
                    } else {
                        &target_label
                    };
                    record(&mut undeclared_links, &source_label, &qualifier, missing);
                    continue;
                };
                let qualifier = qualifier_id(qualifier);
                object_object_qualifiers.insert(qualifier as u32);
                insert.execute(params![
                    rid,
                    source,
                    target,
                    qualifier,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(6)?
                ])?;
                rid += 1;
            }
        }
        lap(&started, "item links");
        connection.execute_batch(
            "CREATE INDEX eo_life ON eo (object, at, event);
             CREATE INDEX eo_event ON eo (event, object);
             COMMIT;
             DETACH DATABASE src;",
        )?;
        lap(&started, "indexes");
        let events = count(&connection, "SELECT count(*) FROM ev")?;
        let objects = count(&connection, "SELECT count(*) FROM ob")?;
        let event_object_relations = count(&connection, "SELECT count(*) FROM eo")?;
        let object_object_relations = count(&connection, "SELECT count(*) FROM oo")?;
        drop(source);
        let mut log = Self {
            connection,
            _scratch: None,
            event_types: event_tables.iter().map(|t| t.name.clone()).collect(),
            object_types: object_tables.iter().map(|t| t.name.clone()).collect(),
            qualifiers,
            attribute_names,
            events,
            objects,
            event_object_relations,
            object_object_relations,
            undeclared_links,
            event_object_qualifiers,
            object_object_qualifiers,
            source: path.canonicalize().unwrap_or_else(|_| path.to_owned()),
            object_tables: object_tables
                .iter()
                .map(|t| ObjectTable {
                    table: t.table.clone(),
                    changed_field: t.has_changed_field,
                    attributes: t.attributes.clone(),
                })
                .collect(),
            quality: None,
        };
        let quality = log.check_quality()?;
        lap(&started, "quality");
        log.quality = Some(quality.clone());
        let meta = Meta {
            format: FORMAT,
            event_types: log.event_types.clone(),
            object_types: log.object_types.clone(),
            qualifiers: log.qualifiers.clone(),
            attribute_names: log.attribute_names.clone(),
            counts: [
                log.events,
                log.objects,
                log.event_object_relations,
                log.object_object_relations,
            ],
            undeclared: (
                log.undeclared_links.count,
                log.undeclared_links.examples.clone(),
            ),
            event_object_qualifiers: log.event_object_qualifiers.clone(),
            object_object_qualifiers: log.object_object_qualifiers.clone(),
            source: log.source.clone(),
            object_tables: log.object_tables.clone(),
            quality,
        };
        log.connection
            .execute_batch("CREATE TABLE meta (value TEXT NOT NULL);")?;
        log.connection.execute(
            "INSERT INTO meta VALUES (?1)",
            [serde_json::to_string(&meta).unwrap_or_default()],
        )?;
        lap(&started, "kept");
        Ok(log)
    }

    /// Qualifiers used on event-to-item rows, by number.
    pub fn event_object_qualifiers(&self) -> &BTreeSet<u32> {
        &self.event_object_qualifiers
    }

    /// Qualifiers used on item-to-item rows, by number.
    pub fn object_object_qualifiers(&self) -> &BTreeSet<u32> {
        &self.object_object_qualifiers
    }

    /// The part a filter selects; fails as [`pm_core::OcelView::apply`] does.
    pub fn view(&self, filter: &OcelFilter) -> Result<DiskView, OcelFilterError> {
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
            let mut view = self.view(&base)?;
            view.object_ids = Some(self.cohort_ids(&view, cohort)?);
            return Ok(view);
        }
        let object_types = if filter.object_types.is_empty() {
            None
        } else {
            let mut selected = BTreeSet::new();
            for label in &filter.object_types {
                let id = self
                    .object_types
                    .iter()
                    .position(|name| name == label)
                    .ok_or_else(|| OcelFilterError::UnknownObjectType(label.clone()))?;
                selected.insert(id as u32);
            }
            Some(selected)
        };
        let qualifiers = |labels: &[String],
                          available: &BTreeSet<u32>,
                          error: fn(String) -> OcelFilterError|
         -> Result<Option<BTreeSet<u32>>, OcelFilterError> {
            if labels.is_empty() {
                return Ok(None);
            }
            let mut selected = BTreeSet::new();
            for label in labels {
                let id = self
                    .qualifiers
                    .iter()
                    .position(|name| name == label)
                    .map(|id| id as u32)
                    .filter(|id| available.contains(id))
                    .ok_or_else(|| error(label.clone()))?;
                selected.insert(id);
            }
            Ok(Some(selected))
        };
        Ok(DiskView {
            object_ids: None,
            object_types,
            event_object_qualifiers: qualifiers(
                &filter.event_object_qualifiers,
                &self.event_object_qualifiers,
                OcelFilterError::UnknownEventObjectQualifier,
            )?,
            object_object_qualifiers: qualifiers(
                &filter.object_object_qualifiers,
                &self.object_object_qualifiers,
                OcelFilterError::UnknownObjectObjectQualifier,
            )?,
        })
    }

    fn cohort_ids(
        &self,
        view: &DiskView,
        cohort: &pm_core::OcelCohort,
    ) -> Result<BTreeSet<u32>, OcelFilterError> {
        let type_id = |name: &str| {
            self.object_types
                .iter()
                .position(|kind| kind == name)
                .map(|id| id as u32)
                .ok_or_else(|| OcelFilterError::UnknownObjectType(name.to_owned()))
        };
        let root_type = type_id(&cohort.root_type)?;
        let read = |_| OcelFilterError::CohortRead;
        let mut seen = BTreeSet::new();
        let mut roots = BTreeSet::new();
        let mut statement = self
            .connection
            .prepare("SELECT id,label,type FROM ob")
            .map_err(read)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                ))
            })
            .map_err(read)?;
        for row in rows {
            let (id, label, kind) = row.map_err(read)?;
            if cohort.roots.contains(&label) && kind == root_type {
                seen.insert(label);
                if view
                    .object_types
                    .as_ref()
                    .is_none_or(|types| types.contains(&kind))
                    && view.object_ids.as_ref().is_none_or(|ids| ids.contains(&id))
                {
                    roots.insert(id);
                }
            }
        }
        if let Some(missing) = cohort.roots.difference(&seen).next() {
            return Err(OcelFilterError::UnknownCohortObject(missing.clone()));
        }
        let mut keep = roots.clone();
        if cohort.paths.len() > 32 {
            return Err(OcelFilterError::InvalidCohortPath(
                "at most 32 paths".into(),
            ));
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
                let target = type_id(kind)?;
                let ids = list(&frontier);
                let sql = format!(
                    "SELECT DISTINCT b.object FROM eo a JOIN eo b ON a.event=b.event WHERE a.object IN ({ids}) AND b.otype={target} AND {} AND {} UNION SELECT r.target FROM oo r WHERE r.source IN ({ids}) AND r.ttype={target} AND {} UNION SELECT r.source FROM oo r WHERE r.target IN ({ids}) AND r.stype={target} AND {}",
                    view.links_as("a"),
                    view.links_as("b"),
                    view.relationships(),
                    view.relationships()
                );
                let mut statement = self.connection.prepare(&sql).map_err(read)?;
                frontier = statement
                    .query_map([], |row| row.get::<_, u32>(0))
                    .map_err(read)?
                    .collect::<Result<BTreeSet<_>, _>>()
                    .map_err(read)?;
                keep.extend(&frontier);
            }
        }
        Ok(keep)
    }

    /// Events, items, event-to-item rows and item-to-item rows in a view.
    pub fn view_counts(&self, view: &DiskView) -> Result<[usize; 4], OcelSqliteError> {
        if view.is_whole() {
            let linked = count(
                &self.connection,
                "SELECT count(*) FROM (SELECT DISTINCT event FROM eo)",
            )?;
            return Ok([
                linked,
                self.objects,
                self.event_object_relations,
                self.object_object_relations,
            ]);
        }
        Ok([
            count(
                &self.connection,
                &format!(
                    "SELECT count(*) FROM (SELECT DISTINCT event FROM eo AS l WHERE {})",
                    view.links()
                ),
            )?,
            count(
                &self.connection,
                &format!("SELECT count(*) FROM ob AS o WHERE {}", view.objects()),
            )?,
            count(
                &self.connection,
                &format!("SELECT count(*) FROM eo AS l WHERE {}", view.links()),
            )?,
            count(
                &self.connection,
                &format!(
                    "SELECT count(*) FROM oo AS r WHERE {}",
                    view.relationships()
                ),
            )?,
        ])
    }

    /// Calls `f` with every item of the view, in item order, and its
    /// timeline; an item without events gets an empty timeline.
    /// Who did each event, numbered: the performer names, and each event's
    /// performer in event order (`None` where none was recorded).
    pub fn event_performers(&self) -> Result<(Vec<String>, Vec<Option<u32>>), OcelSqliteError> {
        let mut names = Vec::new();
        let mut ids = HashMap::<String, u32>::new();
        let mut performers = Vec::with_capacity(self.events);
        let mut statement = self.connection.prepare("SELECT res FROM ev ORDER BY id")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let name: Option<String> = row.get(0)?;
            performers.push(name.map(|name| {
                *ids.entry(name.clone()).or_insert_with(|| {
                    names.push(name);
                    (names.len() - 1) as u32
                })
            }));
        }
        Ok((names, performers))
    }

    pub fn for_each_timeline(
        &self,
        view: &DiskView,
        f: impl FnMut(ObjectId, ObjectTypeId, &[TimelineStep]) -> Result<(), OcelSqliteError>,
    ) -> Result<(), OcelSqliteError> {
        self.for_each_timeline_of(view, None, f)
    }

    /// Like [`Self::for_each_timeline`], with only the steps of the given
    /// activities in each timeline.
    pub fn for_each_timeline_of(
        &self,
        view: &DiskView,
        activities: Option<&BTreeSet<u32>>,
        mut f: impl FnMut(ObjectId, ObjectTypeId, &[TimelineStep]) -> Result<(), OcelSqliteError>,
    ) -> Result<(), OcelSqliteError> {
        let only = activities.map_or(String::new(), |kinds| {
            format!(" AND l.etype IN ({})", list(kinds))
        });
        let mut objects = self.connection.prepare(&format!(
            "SELECT o.id, o.type FROM ob AS o WHERE {} ORDER BY o.id",
            view.objects()
        ))?;
        let mut objects = objects.query([])?;
        let mut steps = self.connection.prepare(&format!(
            "SELECT l.object, l.at, l.event, l.etype FROM eo AS l WHERE {}{only} ORDER BY l.object, l.at, l.event",
            view.links()
        ))?;
        let mut steps = steps.query([])?;
        let mut pending: Option<(u32, TimelineStep)> = None;
        let mut next_step =
            |pending: &mut Option<(u32, TimelineStep)>| -> Result<(), OcelSqliteError> {
                *pending = match steps.next()? {
                    Some(row) => Some((
                        row.get(0)?,
                        (
                            row.get::<_, Micros>(1)?,
                            OcelEventId(row.get(2)?),
                            EventTypeId(row.get(3)?),
                        ),
                    )),
                    None => None,
                };
                Ok(())
            };
        next_step(&mut pending)?;
        let mut timeline = Vec::new();
        while let Some(row) = objects.next()? {
            let object: u32 = row.get(0)?;
            let object_type = ObjectTypeId(row.get(1)?);
            while pending.is_some_and(|(o, _)| o < object) {
                next_step(&mut pending)?;
            }
            timeline.clear();
            while let Some((o, step)) = pending {
                if o != object {
                    break;
                }
                // Several qualified rows for one event are one occurrence.
                if timeline
                    .last()
                    .is_none_or(|last: &TimelineStep| last.1 != step.1)
                {
                    timeline.push(step);
                }
                next_step(&mut pending)?;
            }
            f(ObjectId(object), object_type, &timeline)?;
        }
        Ok(())
    }

    /// Calls `f` with every event of the view, in event order, with its
    /// type and the types of its distinct items, in item order.
    pub fn for_each_event(
        &self,
        view: &DiskView,
        mut f: impl FnMut(OcelEventId, EventTypeId, &[ObjectTypeId]) -> Result<(), OcelSqliteError>,
    ) -> Result<(), OcelSqliteError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT l.event, l.etype, l.object, l.otype FROM eo AS l WHERE {} ORDER BY l.event, l.object",
            view.links()
        ))?;
        let mut rows = statement.query([])?;
        let mut current: Option<(u32, u32, u32)> = None;
        let mut types = Vec::new();
        while let Some(row) = rows.next()? {
            let (event, event_type, object, object_type): (u32, u32, u32, u32) =
                (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
            match current {
                Some((e, _, o)) if e == event => {
                    if o != object {
                        types.push(ObjectTypeId(object_type));
                    }
                }
                _ => {
                    if let Some((e, t, _)) = current {
                        f(OcelEventId(e), EventTypeId(t), &types)?;
                    }
                    types.clear();
                    types.push(ObjectTypeId(object_type));
                }
            }
            current = Some((event, event_type, object));
        }
        if let Some((e, t, _)) = current {
            f(OcelEventId(e), EventTypeId(t), &types)?;
        }
        Ok(())
    }

    /// Calls `f` with every event-to-item row of the view, in file order:
    /// event, its type, the item's type and the qualifier.
    pub fn for_each_participation(
        &self,
        view: &DiskView,
        mut f: impl FnMut(OcelEventId, EventTypeId, ObjectTypeId, QualifierId),
    ) -> Result<(), OcelSqliteError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT l.event, l.etype, l.otype, l.qualifier FROM eo AS l WHERE {} ORDER BY l.rid",
            view.links()
        ))?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            f(
                OcelEventId(row.get(0)?),
                EventTypeId(row.get(1)?),
                ObjectTypeId(row.get(2)?),
                QualifierId(row.get(3)?),
            );
        }
        Ok(())
    }

    /// Calls `f` with every item-to-item row of the view, in file order: its
    /// index among all rows, both items' types and the qualifier.
    pub fn for_each_relationship(
        &self,
        view: &DiskView,
        mut f: impl FnMut(usize, ObjectTypeId, ObjectTypeId, QualifierId),
    ) -> Result<(), OcelSqliteError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT r.rid, r.stype, r.ttype, r.qualifier FROM oo AS r WHERE {} ORDER BY r.rid",
            view.relationships()
        ))?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            f(
                row.get::<_, i64>(0)? as usize,
                ObjectTypeId(row.get(1)?),
                ObjectTypeId(row.get(2)?),
                QualifierId(row.get(3)?),
            );
        }
        Ok(())
    }

    pub fn event_label(&self, event: OcelEventId) -> Result<String, OcelSqliteError> {
        Ok(self
            .connection
            .query_row("SELECT label FROM ev WHERE id = ?1", [event.0], |row| {
                row.get(0)
            })?)
    }

    pub fn event_at(&self, event: OcelEventId) -> Result<Micros, OcelSqliteError> {
        Ok(self
            .connection
            .query_row("SELECT at FROM ev WHERE id = ?1", [event.0], |row| {
                row.get(0)
            })?)
    }

    pub fn object_label(&self, object: ObjectId) -> Result<String, OcelSqliteError> {
        Ok(self
            .connection
            .query_row("SELECT label FROM ob WHERE id = ?1", [object.0], |row| {
                row.get(0)
            })?)
    }

    /// Endpoints correlated through one explicit recorded relationship or
    /// shared event. Each root is returned once, even with split deliveries.
    pub fn linked_interval(
        &self,
        view: &DiskView,
        types: [ObjectTypeId; 3],
        activities: [EventTypeId; 2],
        last: [bool; 2],
    ) -> Result<Vec<LinkedIntervalRow>, OcelSqliteError> {
        let source_aggregate = if last[0] { "MAX" } else { "MIN" };
        let target_aggregate = if last[1] { "MAX" } else { "MIN" };
        let sql=format!("WITH active AS (SELECT DISTINCT l.event,l.object,l.otype,l.etype,l.at FROM eo l WHERE {links}),
          roots AS (SELECT o.id FROM ob o WHERE {objects} AND o.type=?1),
          neighbours AS (
            SELECT id AS root,id AS object,?1 AS kind FROM roots
            UNION SELECT r.source,r.target,r.ttype FROM oo r JOIN roots ON roots.id=r.source WHERE {relationships} AND r.ttype IN (?2,?3)
            UNION SELECT r.target,r.source,r.stype FROM oo r JOIN roots ON roots.id=r.target WHERE {relationships} AND r.stype IN (?2,?3)
            UNION SELECT a.object,b.object,b.otype FROM active a JOIN roots ON roots.id=a.object JOIN active b ON a.event=b.event WHERE b.otype IN (?2,?3)),
          starts AS (SELECT n.root,{source_aggregate}(a.at) AS at FROM neighbours n JOIN active a ON a.object=n.object WHERE n.kind=?2 AND a.etype=?4 GROUP BY n.root),
          ends AS (SELECT n.root,{target_aggregate}(a.at) AS at FROM neighbours n JOIN active a ON a.object=n.object WHERE n.kind=?3 AND a.etype=?5 GROUP BY n.root)
          SELECT roots.id,starts.at,ends.at FROM roots LEFT JOIN starts ON starts.root=roots.id LEFT JOIN ends ON ends.root=roots.id ORDER BY roots.id",
          links=view.links(),objects=view.objects(),relationships=view.relationships());
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(
            params![
                types[0].0,
                types[1].0,
                types[2].0,
                activities[0].0,
                activities[1].0
            ],
            |row| Ok((ObjectId(row.get(0)?), row.get(1)?, row.get(2)?)),
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// The items an item-to-item row joins, by the row's index.
    pub fn relationship(&self, index: usize) -> Result<(ObjectId, ObjectId), OcelSqliteError> {
        Ok(self.connection.query_row(
            "SELECT source, target FROM oo WHERE rid = ?1",
            [index as i64],
            |row| Ok((ObjectId(row.get(0)?), ObjectId(row.get(1)?))),
        )?)
    }

    /// The data checks of [`crate::assess_ocel_quality`], made when the copy
    /// was made.
    pub fn quality(&self) -> Result<OcelQualityReport, OcelSqliteError> {
        match &self.quality {
            Some(quality) => Ok(quality.clone()),
            None => self.check_quality(),
        }
    }

    fn check_quality(&self) -> Result<OcelQualityReport, OcelSqliteError> {
        let connection = &self.connection;
        let mut findings = Vec::new();
        let qualifier = |id: i64| self.qualifiers[id as usize].clone();
        let examples = |sql: &str,
                        label: &dyn Fn(&rusqlite::Row<'_>) -> Result<String, OcelSqliteError>|
         -> Result<Vec<String>, OcelSqliteError> {
            let mut statement = connection.prepare(sql)?;
            let mut rows = statement.query([])?;
            let mut found = Vec::new();
            while let Some(row) = rows.next()? {
                found.push(label(row)?);
            }
            Ok(found)
        };
        let missing = &self.undeclared_links;
        if missing.count > 0 {
            push(
                &mut findings,
                OcelQualityKind::UndeclaredObjectReference,
                missing.count,
                missing.examples.iter().cloned(),
            );
        }
        for (table, kind, columns, from, to) in [
            (
                "eo",
                OcelQualityKind::DuplicateEventObjectRelation,
                "event, object, qualifier",
                "ev",
                "ob",
            ),
            (
                "oo",
                OcelQualityKind::DuplicateObjectObjectRelation,
                "source, target, qualifier",
                "ob",
                "ob",
            ),
        ] {
            let affected = count(
                connection,
                &format!(
                    "SELECT coalesce(sum(n - 1), 0) FROM (SELECT count(*) AS n FROM {table} GROUP BY {columns} HAVING n > 1)"
                ),
            )?;
            if affected > 0 {
                let first = columns.split(", ").next().unwrap_or_default();
                let second = columns.split(", ").nth(1).unwrap_or_default();
                let found = examples(
                    &format!(
                        "SELECT a.label, d.qualifier, b.label FROM (SELECT {columns} FROM {table} GROUP BY {columns} \
                         HAVING count(*) > 1 ORDER BY {columns} LIMIT 5) AS d \
                         JOIN {from} AS a ON a.id = d.{first} JOIN {to} AS b ON b.id = d.{second} \
                         ORDER BY d.{first}, d.{second}, d.qualifier"
                    ),
                    &|row| {
                        Ok(format!(
                            "{} --{}--> {}",
                            row.get::<_, String>(0)?,
                            qualifier(row.get(1)?),
                            row.get::<_, String>(2)?
                        ))
                    },
                )?;
                push(&mut findings, kind, affected, found.into_iter());
            }
        }
        self.object_attribute_checks(&mut findings)?;
        let disconnected_events = count(
            connection,
            "SELECT count(*) FROM ev AS e WHERE NOT EXISTS (SELECT 1 FROM eo WHERE eo.event = e.id)",
        )?;
        if disconnected_events > 0 {
            let found = examples(
                "SELECT label FROM ev AS e WHERE NOT EXISTS (SELECT 1 FROM eo WHERE eo.event = e.id) ORDER BY id LIMIT 5",
                &|row| Ok(row.get(0)?),
            )?;
            push(
                &mut findings,
                OcelQualityKind::EventWithoutObjects,
                disconnected_events,
                found.into_iter(),
            );
        }
        let disconnected_objects = count(
            connection,
            "SELECT count(*) FROM ob AS o WHERE NOT EXISTS (SELECT 1 FROM eo WHERE eo.object = o.id)",
        )?;
        if disconnected_objects > 0 {
            let found = examples(
                "SELECT label FROM ob AS o WHERE NOT EXISTS (SELECT 1 FROM eo WHERE eo.object = o.id) ORDER BY id LIMIT 5",
                &|row| Ok(row.get(0)?),
            )?;
            push(
                &mut findings,
                OcelQualityKind::ObjectWithoutEvents,
                disconnected_objects,
                found.into_iter(),
            );
        }
        let empty = self
            .qualifiers
            .iter()
            .enumerate()
            .filter(|(_, name)| name.trim().is_empty())
            .map(|(id, _)| id as u32)
            .collect::<BTreeSet<_>>();
        if !empty.is_empty() {
            let ids = list(&empty);
            let affected = count(
                connection,
                &format!("SELECT count(*) FROM eo WHERE qualifier IN ({ids})"),
            )? + count(
                connection,
                &format!("SELECT count(*) FROM oo WHERE qualifier IN ({ids})"),
            )?;
            if affected > 0 {
                let mut found = examples(
                    &format!(
                        "SELECT e.label, o.label FROM (SELECT rid, event, object FROM eo WHERE qualifier IN ({ids}) ORDER BY rid LIMIT 5) AS l \
                         JOIN ev AS e ON e.id = l.event JOIN ob AS o ON o.id = l.object ORDER BY l.rid"
                    ),
                    &|row| {
                        Ok(format!(
                            "{} → {}",
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?
                        ))
                    },
                )?;
                found.extend(examples(
                    &format!(
                        "SELECT s.label, t.label FROM (SELECT rid, source, target FROM oo WHERE qualifier IN ({ids}) ORDER BY rid LIMIT 5) AS r \
                         JOIN ob AS s ON s.id = r.source JOIN ob AS t ON t.id = r.target ORDER BY r.rid"
                    ),
                    &|row| Ok(format!("{} → {}", row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )?);
                push(
                    &mut findings,
                    OcelQualityKind::EmptyRelationshipQualifier,
                    affected,
                    found.into_iter(),
                );
            }
        }
        let selves = count(connection, "SELECT count(*) FROM oo WHERE source = target")?;
        if selves > 0 {
            let found = examples(
                "SELECT o.label, r.qualifier FROM (SELECT rid, source, qualifier FROM oo WHERE source = target ORDER BY rid LIMIT 5) AS r \
                 JOIN ob AS o ON o.id = r.source ORDER BY r.rid",
                &|row| {
                    Ok(format!(
                        "{} --{}--> itself",
                        row.get::<_, String>(0)?,
                        qualifier(row.get(1)?)
                    ))
                },
            )?;
            push(
                &mut findings,
                OcelQualityKind::SelfObjectRelationship,
                selves,
                found.into_iter(),
            );
        }
        let used = |sql: &str| -> Result<BTreeSet<usize>, OcelSqliteError> {
            let mut statement = connection.prepare(sql)?;
            let found = statement
                .query_map([], |row| row.get::<_, i64>(0).map(|v| v as usize))?
                .collect::<Result<_, _>>()?;
            Ok(found)
        };
        let used_events = used("SELECT DISTINCT type FROM ev")?;
        let unused = self
            .event_types
            .iter()
            .enumerate()
            .filter(|(index, _)| !used_events.contains(index))
            .map(|(_, name)| name.clone())
            .collect::<Vec<_>>();
        if !unused.is_empty() {
            push(
                &mut findings,
                OcelQualityKind::UnusedEventType,
                unused.len(),
                unused.into_iter(),
            );
        }
        let used_objects = used("SELECT DISTINCT type FROM ob")?;
        let unused = self
            .object_types
            .iter()
            .enumerate()
            .filter(|(index, _)| !used_objects.contains(index))
            .map(|(_, name)| name.clone())
            .collect::<Vec<_>>();
        if !unused.is_empty() {
            push(
                &mut findings,
                OcelQualityKind::UnusedObjectType,
                unused.len(),
                unused.into_iter(),
            );
        }
        Ok(report(
            findings,
            [
                self.events,
                self.objects,
                self.event_object_relations,
                self.object_object_relations,
            ],
        ))
    }

    /// Several values for one item attribute at one instant: different values
    /// conflict, the same value repeated is only reported.
    fn object_attribute_checks(
        &self,
        findings: &mut Vec<crate::OcelQualityFinding>,
    ) -> Result<(), OcelSqliteError> {
        let mut statement = self.connection.prepare(
            "SELECT o.label, a.name, a.at, count(*), count(DISTINCT a.value) \
             FROM oattr AS a JOIN ob AS o ON o.label = a.label \
             GROUP BY o.id, a.name, a.at HAVING count(*) > 1 ORDER BY o.id, a.name, a.at",
        )?;
        let mut rows = statement.query([])?;
        let (mut conflicts, mut repeats) = ((0usize, Vec::new()), (0usize, Vec::new()));
        while let Some(row) = rows.next()? {
            let label: String = row.get(0)?;
            let name: i64 = row.get(1)?;
            let at: i64 = row.get(2)?;
            let rows: i64 = row.get(3)?;
            let distinct: i64 = row.get(4)?;
            let target = if distinct > 1 {
                &mut conflicts
            } else {
                &mut repeats
            };
            target.0 += rows as usize - 1;
            if target.1.len() < 5 {
                target.1.push(format!(
                    "{label}: {} at {at}",
                    self.attribute_names[name as usize]
                ));
            }
        }
        if conflicts.0 > 0 {
            push(
                findings,
                OcelQualityKind::DuplicateObjectAttributeAssignment,
                conflicts.0,
                conflicts.1.into_iter(),
            );
        }
        if repeats.0 > 0 {
            push(
                findings,
                OcelQualityKind::RepeatedObjectAttributeValue,
                repeats.0,
                repeats.1.into_iter(),
            );
        }
        Ok(())
    }
}

/// The scratch copy's layout; a copy of another layout is rebuilt.
// 4: events keep who did them.
const FORMAT: u32 = 4;

/// What a kept copy remembers besides its tables.
#[derive(serde::Serialize, serde::Deserialize)]
struct Meta {
    format: u32,
    event_types: Vec<String>,
    object_types: Vec<String>,
    qualifiers: Vec<String>,
    attribute_names: Vec<String>,
    counts: [usize; 4],
    undeclared: (usize, Vec<String>),
    event_object_qualifiers: BTreeSet<u32>,
    object_object_qualifiers: BTreeSet<u32>,
    source: PathBuf,
    object_tables: Vec<ObjectTable>,
    quality: OcelQualityReport,
}

/// Marks a kept copy as just used, so it is kept longest.
/// Event attributes that name who did a step, in order of preference.
pub const PERFORMER_ATTRIBUTES: [&str; 2] = ["resource", "org:resource"];

/// A performer as text, whatever type the column stores; blank is none.
fn performer_text(value: rusqlite::types::ValueRef<'_>) -> Option<String> {
    use rusqlite::types::ValueRef;
    let text = match value {
        ValueRef::Null => return None,
        ValueRef::Integer(n) => n.to_string(),
        ValueRef::Real(x) => x.to_string(),
        ValueRef::Text(t) | ValueRef::Blob(t) => String::from_utf8_lossy(t).into_owned(),
    };
    (!text.trim().is_empty()).then_some(text)
}

fn filetime_touch(file: &Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new()
        .append(true)
        .open(file)?
        .set_modified(std::time::SystemTime::now())
}

/// Keeps the three most recently used copies in `cache`, and `keep`.
fn evict(cache: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    let mut copies = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("pm-disk-") && name.ends_with(".sqlite") && e.path() != keep
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect::<Vec<_>>();
    copies.sort();
    let extra = copies.len().saturating_sub(2);
    for (_, path) in copies.into_iter().take(extra) {
        let _ = std::fs::remove_file(path);
    }
}

impl DiskLog {
    /// Each item type's table in the saved log, in type order.
    pub fn object_tables(&self) -> &[ObjectTable] {
        &self.object_tables
    }

    /// Calls `f` with every event of the view that involves at least two
    /// items, in event order: its type, time and distinct items in item
    /// order, each with the time of its most recent strictly earlier step.
    pub fn for_each_shared_event(
        &self,
        view: &DiskView,
        mut f: impl FnMut(
            OcelEventId,
            EventTypeId,
            Micros,
            &[pm_core::OcelParticipant],
        ) -> Result<(), OcelSqliteError>,
    ) -> Result<(), OcelSqliteError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT l.event, l.etype, l.at, l.object, l.otype FROM eo AS l WHERE {} ORDER BY l.event, l.object",
            view.links()
        ))?;
        let mut prior = self.connection.prepare(&format!(
            "SELECT max(p.at) FROM eo AS p WHERE p.object = ?1 AND p.at < ?2 AND {}",
            view.links_as("p")
        ))?;
        let mut rows = statement.query([])?;
        let mut current: Option<(u32, u32, i64)> = None;
        let mut objects: Vec<(u32, u32)> = Vec::new();
        let mut flush = |current: Option<(u32, u32, i64)>,
                         objects: &[(u32, u32)]|
         -> Result<(), OcelSqliteError> {
            let Some((event, kind, at)) = current else {
                return Ok(());
            };
            if objects.len() < 2 {
                return Ok(());
            }
            let mut joined = Vec::with_capacity(objects.len());
            for (object, object_type) in objects {
                let prior_at: Option<i64> =
                    prior.query_row(params![object, at], |row| row.get(0))?;
                joined.push(pm_core::OcelParticipant {
                    object: ObjectId(*object),
                    object_type: ObjectTypeId(*object_type),
                    prior_at,
                });
            }
            f(OcelEventId(event), EventTypeId(kind), at, &joined)
        };
        while let Some(row) = rows.next()? {
            let event: u32 = row.get(0)?;
            let object: u32 = row.get(3)?;
            if current.is_some_and(|(e, _, _)| e == event) {
                if objects.last().is_some_and(|(o, _)| *o == object) {
                    continue;
                }
            } else {
                flush(current, &objects)?;
                objects.clear();
                current = Some((event, row.get(1)?, row.get(2)?));
            }
            objects.push((object, row.get(4)?));
        }
        flush(current, &objects)
    }

    /// Calls `f` with every event that involves items of the view of one
    /// type, in event order, with those distinct items in item order.
    pub fn for_each_event_items(
        &self,
        view: &DiskView,
        object_type: ObjectTypeId,
        mut f: impl FnMut(OcelEventId, &[ObjectId]),
    ) -> Result<(), OcelSqliteError> {
        let view = view.only_type(object_type);
        let mut statement = self.connection.prepare(&format!(
            "SELECT l.event, l.object FROM eo AS l WHERE {} ORDER BY l.event, l.object",
            view.links()
        ))?;
        let mut rows = statement.query([])?;
        let mut current: Option<u32> = None;
        let mut items: Vec<ObjectId> = Vec::new();
        while let Some(row) = rows.next()? {
            let event: u32 = row.get(0)?;
            let object = ObjectId(row.get(1)?);
            if current != Some(event) {
                if let Some(previous) = current {
                    f(OcelEventId(previous), &items);
                }
                items.clear();
                current = Some(event);
            }
            if items.last() != Some(&object) {
                items.push(object);
            }
        }
        if let Some(previous) = current {
            f(OcelEventId(previous), &items);
        }
        Ok(())
    }

    /// An event's items in the view other than those of one type, by type.
    pub fn other_items(
        &self,
        view: &DiskView,
        event: OcelEventId,
        except: ObjectTypeId,
    ) -> Result<std::collections::BTreeMap<ObjectTypeId, BTreeSet<ObjectId>>, OcelSqliteError> {
        let mut statement = self.connection.prepare_cached(&format!(
            "SELECT l.otype, l.object FROM eo AS l WHERE l.event = ?1 AND l.otype != ?2 AND {}",
            view.links()
        ))?;
        let mut rows = statement.query(params![event.0, except.0])?;
        let mut found = std::collections::BTreeMap::<ObjectTypeId, BTreeSet<ObjectId>>::new();
        while let Some(row) = rows.next()? {
            found
                .entry(ObjectTypeId(row.get(0)?))
                .or_default()
                .insert(ObjectId(row.get(1)?));
        }
        Ok(found)
    }

    /// The type of an event.
    pub fn event_kind(&self, event: OcelEventId) -> Result<EventTypeId, OcelSqliteError> {
        Ok(self
            .connection
            .prepare_cached("SELECT type FROM ev WHERE id = ?1")?
            .query_row([event.0], |row| row.get(0).map(EventTypeId))?)
    }

    /// An item's events at one instant in the view, in event order, at most
    /// `limit`.
    pub fn events_at(
        &self,
        view: &DiskView,
        object: ObjectId,
        at: Micros,
        limit: usize,
    ) -> Result<Vec<OcelEventId>, OcelSqliteError> {
        let mut statement = self.connection.prepare_cached(&format!(
            "SELECT DISTINCT l.event FROM eo AS l WHERE l.object = ?1 AND l.at = ?2 AND {} ORDER BY l.event LIMIT ?3",
            view.links()
        ))?;
        let found = statement
            .query_map(params![object.0, at, limit as i64], |row| {
                row.get(0).map(OcelEventId)
            })?
            .collect::<Result<_, _>>()?;
        Ok(found)
    }

    /// Follows one item type: each item of the view becomes a case of every
    /// step that involves it, in time order, with the item's details as of
    /// its last step, as following the loaded log writes it. `header`
    /// receives the detail names; `write` each step.
    pub fn follow(
        &self,
        view: &DiskView,
        object_type: ObjectTypeId,
        fixed: &[&str],
        header: impl FnOnce(&[String]) -> Result<(), OcelSqliteError>,
        mut write: impl FnMut(&super::FollowedStep<'_>) -> Result<(), OcelSqliteError>,
    ) -> Result<super::FollowedLog, OcelSqliteError> {
        let only = view.only_type(object_type);
        let table = self
            .object_tables
            .get(object_type.0 as usize)
            .cloned()
            .ok_or_else(|| OcelSqliteError::Data("unknown object type".into()))?;
        // Item details, minus fixed and lineage names, each name once.
        let mut seen = BTreeSet::new();
        let details = table
            .attributes
            .iter()
            .enumerate()
            .filter(|(_, (name, _))| {
                !fixed.contains(&name.as_str())
                    && !name.starts_with("_source_")
                    && seen.insert(name.clone())
            })
            .map(|(index, (name, kind))| (index, name.clone(), *kind))
            .collect::<Vec<_>>();
        header(
            &details
                .iter()
                .map(|(_, name, _)| name.clone())
                .collect::<Vec<_>>(),
        )?;
        // Short labels when every item has one and none repeats.
        let (items, shorts, distinct): (i64, i64, i64) = self.connection.query_row(
            &format!(
                "SELECT count(*), count(o.short), count(DISTINCT o.short) FROM ob AS o WHERE {}",
                only.objects()
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let short = items == shorts && shorts == distinct;
        // Shared steps and the steps no item of the type takes part in.
        let (mut included, mut shared, mut copies) = (0u64, 0u64, 0u64);
        self.for_each_event_items(view, object_type, |_, cases| {
            included += 1;
            if cases.len() > 1 {
                shared += 1;
                copies += cases.len() as u64 - 1;
            }
        })?;
        let view_events = self.view_counts(view)?[0] as u64;
        // The type's recorded details, item by item, from the saved log.
        let uri = format!(
            "file:{}?mode=ro",
            self.source
                .to_string_lossy()
                .replace('%', "%25")
                .replace('?', "%3f")
                .replace('#', "%23")
        );
        self.connection
            .execute("ATTACH DATABASE ?1 AS src", [uri])?;
        struct Detach<'a>(&'a Connection);
        impl Drop for Detach<'_> {
            fn drop(&mut self) {
                let _ = self.0.execute_batch("DETACH DATABASE src;");
            }
        }
        let _detach = Detach(&self.connection);
        let selected = table
            .attributes
            .iter()
            .map(|(name, _)| format!(", a.{}", quote_identifier(name)))
            .collect::<String>();
        let changed = if table.changed_field {
            "a.ocel_changed_field"
        } else {
            "NULL"
        };
        let mut attribute_statement = self.connection.prepare(&format!(
            "SELECT o.id, a.ocel_time, {changed}{selected} FROM src.{} AS a \
             JOIN ob AS o ON o.label = a.ocel_id WHERE {} ORDER BY o.id, a.rowid",
            quote_identifier(&table.table),
            only.objects()
        ))?;
        let mut attribute_rows = attribute_statement.query([])?;
        let mut pending: Option<(u32, i64, Vec<Option<String>>)> = None;
        let mut next_attribute = |item: u32| -> Result<Vec<Assignment>, OcelSqliteError> {
            let mut found = Vec::new();
            loop {
                if pending.is_none() {
                    let Some(row) = attribute_rows.next()? else {
                        break;
                    };
                    let seq: u32 = row.get(0)?;
                    let at = match super::optional_sql_text(row.get_ref(1)?, "object timestamp")? {
                        Some(time) => parse_time(&time, "object")?,
                        None => 0,
                    };
                    let changed =
                        super::optional_sql_text(row.get_ref(2)?, "object changed-field marker")?;
                    let mut values = Vec::with_capacity(details.len());
                    for (index, name, value_type) in &details {
                        if changed.as_deref().is_some_and(|c| c != name) {
                            values.push(None);
                            continue;
                        }
                        values.push(super::value_text(
                            row.get_ref(index + 3)?,
                            *value_type,
                            "object attribute",
                        )?);
                    }
                    pending = Some((seq, at, values));
                }
                let (seq, _, _) = pending.as_ref().expect("just read");
                if *seq < item {
                    pending = None;
                    continue;
                }
                if *seq > item {
                    break;
                }
                let (_, at, values) = pending.take().expect("just read");
                found.push((at, values));
            }
            Ok(found)
        };
        let mut labels = self
            .connection
            .prepare_cached("SELECT label, short FROM ob WHERE id = ?1")?;
        let (mut cases, mut events) = (0u64, 0u64);
        let mut failed = None;
        self.for_each_timeline(&only, |object, _, timeline| {
            cases += 1;
            let (full, short_label): (String, Option<String>) =
                labels.query_row([object.0], |row| Ok((row.get(0)?, row.get(1)?)))?;
            let label = if short {
                short_label.unwrap_or(full)
            } else {
                full
            };
            let assignments = match next_attribute(object.0) {
                Ok(found) => found,
                Err(error) => {
                    failed.get_or_insert(error);
                    return Ok(());
                }
            };
            let last = timeline.last().map_or(0, |step| step.0);
            let values = (0..details.len())
                .map(|d| {
                    assignments
                        .iter()
                        .filter(|(at, values)| *at <= last && values[d].is_some())
                        .max_by_key(|(at, _)| *at)
                        .and_then(|(_, values)| values[d].clone())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>();
            for (position, (at, _, kind)) in timeline.iter().enumerate() {
                write(&super::FollowedStep {
                    case: &label,
                    activity: &self.event_types[kind.0 as usize],
                    at: *at,
                    position: position as u64 + 1,
                    details: &values,
                })?;
                events += 1;
            }
            Ok(())
        })?;
        if let Some(error) = failed {
            return Err(error);
        }
        Ok(super::FollowedLog {
            attributes: details.into_iter().map(|(_, name, _)| name).collect(),
            cases,
            events,
            excluded_events: view_events.saturating_sub(included),
            shared_events: shared,
            shared_event_copies: copies,
        })
    }

    /// The item types of the view's items.
    pub fn view_object_types(
        &self,
        view: &DiskView,
    ) -> Result<BTreeSet<ObjectTypeId>, OcelSqliteError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT DISTINCT o.type FROM ob AS o WHERE {}",
            view.objects()
        ))?;
        let found = statement
            .query_map([], |row| row.get(0).map(ObjectTypeId))?
            .collect::<Result<_, _>>()?;
        Ok(found)
    }

    /// An item by its identifier.
    pub fn find_object(
        &self,
        label: &str,
    ) -> Result<Option<(ObjectId, ObjectTypeId)>, OcelSqliteError> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT id, type FROM ob WHERE label = ?1")?;
        let mut rows = statement.query([label])?;
        Ok(match rows.next()? {
            Some(row) => Some((ObjectId(row.get(0)?), ObjectTypeId(row.get(1)?))),
            None => None,
        })
    }

    /// Whether an item is in the view.
    pub fn in_view(&self, view: &DiskView, object_type: ObjectTypeId) -> bool {
        view.object_types
            .as_ref()
            .is_none_or(|types| types.contains(&object_type.0))
    }

    /// An item's events in the view, by time then event, each with its type,
    /// time and the qualifiers linking it, in qualifier order.
    pub fn object_events(
        &self,
        view: &DiskView,
        object: ObjectId,
    ) -> Result<Vec<ObjectStep>, OcelSqliteError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT l.event, l.etype, l.at, l.qualifier FROM eo AS l WHERE l.object = ?1 AND {} \
             ORDER BY l.at, l.event, l.qualifier",
            view.links()
        ))?;
        let mut rows = statement.query([object.0])?;
        let mut found: Vec<ObjectStep> = Vec::new();
        while let Some(row) = rows.next()? {
            let event = OcelEventId(row.get(0)?);
            let qualifier = QualifierId(row.get(3)?);
            match found.last_mut() {
                Some(last) if last.0 == event => {
                    if last.3.last() != Some(&qualifier) {
                        last.3.push(qualifier);
                    }
                }
                _ => found.push((
                    event,
                    EventTypeId(row.get(1)?),
                    row.get(2)?,
                    vec![qualifier],
                )),
            }
        }
        Ok(found)
    }

    /// Calls `f` with every item's identifier in item order until it
    /// returns `true`; returns that identifier.
    pub fn find_object_label(
        &self,
        mut f: impl FnMut(&str) -> bool,
    ) -> Result<Option<String>, OcelSqliteError> {
        let mut statement = self
            .connection
            .prepare("SELECT label FROM ob ORDER BY id")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let label: String = row.get(0)?;
            if f(&label) {
                return Ok(Some(label));
            }
        }
        Ok(None)
    }

    /// A list of numbers kept on disk, such as elapsed times, by kind.
    pub fn new_list(&self) -> Result<DiskList<'_>, OcelSqliteError> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let name = format!(
            "list_{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        self.connection.execute_batch(&format!(
            "CREATE TEMP TABLE {name} (kind INTEGER NOT NULL, value INTEGER NOT NULL);"
        ))?;
        Ok(DiskList {
            log: self,
            name,
            sorted: false,
        })
    }

    /// The sum of the latest numeric value of `field` over the items in
    /// `list` under `kind`, in item order, as the loaded log reads it: the
    /// latest assignment by time, then file order, and a number read from
    /// integers, finite reals or text. Returns the numbers.
    pub fn latest_numbers(
        &self,
        object_type: ObjectTypeId,
        field: &str,
        list: &DiskList<'_>,
        kind: i64,
    ) -> Result<Vec<f64>, OcelSqliteError> {
        let Some(table) = self.object_tables.get(object_type.0 as usize) else {
            return Ok(Vec::new());
        };
        let Some((_, value_type)) = table.attributes.iter().find(|(name, _)| name == field) else {
            return Ok(Vec::new());
        };
        let uri = format!(
            "file:{}?mode=ro",
            self.source
                .to_string_lossy()
                .replace('%', "%25")
                .replace('?', "%3f")
                .replace('#', "%23")
        );
        let source = Connection::open_with_flags(
            &uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let changed = if table.changed_field {
            "a.ocel_changed_field"
        } else {
            "NULL"
        };
        let mut items = self.connection.prepare(&format!(
            "SELECT o.label FROM {} AS w JOIN ob AS o ON o.id = w.value WHERE w.kind = ?1 ORDER BY o.id",
            list.name
        ))?;
        let mut items = items.query([kind])?;
        let mut rows = source.prepare(&format!(
            "SELECT a.ocel_time, {changed}, a.{} FROM {} AS a WHERE a.ocel_id = ?1 ORDER BY a.rowid",
            quote_identifier(field),
            quote_identifier(&table.table)
        ))?;
        let mut numbers = Vec::new();
        while let Some(item) = items.next()? {
            let label: String = item.get(0)?;
            let mut latest: Option<(i64, usize, Option<f64>)> = None;
            let mut values = rows.query([&label])?;
            let mut index = 0usize;
            while let Some(row) = values.next()? {
                index += 1;
                if let Some(changed) =
                    super::optional_sql_text(row.get_ref(1)?, "object changed-field marker")?
                    && changed != field
                {
                    continue;
                }
                let at = match super::optional_sql_text(row.get_ref(0)?, "object timestamp")? {
                    Some(time) => parse_time(&time, &format!("object '{label}'"))?,
                    None => 0,
                };
                let Some(value) = super::convert_plain(
                    row.get_ref(2)?,
                    *value_type,
                    &format!("object '{label}' attribute '{field}'"),
                )?
                else {
                    continue;
                };
                let number = match value {
                    Plain::Value(AttributeValue::Integer(number)) => Some(number as f64),
                    Plain::Value(AttributeValue::Float(number)) => {
                        Some(number).filter(|n| n.is_finite())
                    }
                    Plain::Text(text) => pm_core::parse_number(text),
                    _ => None,
                };
                if latest.is_none_or(|(t, i, _)| (at, index) >= (t, i)) {
                    latest = Some((at, index, number));
                }
            }
            if let Some((_, _, Some(number))) = latest {
                numbers.push(number);
            }
        }
        Ok(numbers)
    }
}

/// An item's attribute row: its time and a value per detail.
type Assignment = (i64, Vec<Option<String>>);

/// One of an item's events: the event, its type, its time and the
/// qualifiers linking it, in qualifier order.
pub type ObjectStep = (OcelEventId, EventTypeId, Micros, Vec<QualifierId>);

/// Numbers kept in a scratch table for one question, removed when dropped.
pub struct DiskList<'a> {
    log: &'a DiskLog,
    name: String,
    sorted: bool,
}

impl DiskList<'_> {
    /// How many distinct values the list holds.
    pub fn distinct(&self) -> Result<u64, OcelSqliteError> {
        Ok(self.log.connection.query_row(
            &format!("SELECT count(DISTINCT value) FROM {}", self.name),
            [],
            |row| row.get::<_, i64>(0),
        )? as u64)
    }

    pub fn push(&self, kind: i64, value: i64) -> Result<(), OcelSqliteError> {
        self.log
            .connection
            .prepare_cached(&format!("INSERT INTO {} VALUES (?1, ?2)", self.name))?
            .execute(params![kind, value])?;
        Ok(())
    }

    /// The nearest-rank percentile `numerator / denominator` of the values
    /// under `kind`; zero when there are none.
    pub fn rank(
        &mut self,
        kind: i64,
        numerator: usize,
        denominator: usize,
    ) -> Result<i64, OcelSqliteError> {
        if !self.sorted {
            self.log.connection.execute_batch(&format!(
                "CREATE INDEX {0}_order ON {0} (kind, value);",
                self.name
            ))?;
            self.sorted = true;
        }
        let count: i64 = self.log.connection.query_row(
            &format!("SELECT count(*) FROM {} WHERE kind = ?1", self.name),
            [kind],
            |row| row.get(0),
        )?;
        if count == 0 {
            return Ok(0);
        }
        let position = pm_core::nearest_rank(count as usize, numerator, denominator);
        Ok(self.log.connection.query_row(
            &format!(
                "SELECT value FROM {} WHERE kind = ?1 ORDER BY value LIMIT 1 OFFSET ?2",
                self.name
            ),
            params![kind, position as i64],
            |row| row.get(0),
        )?)
    }
}

impl Drop for DiskList<'_> {
    fn drop(&mut self) {
        let _ = self
            .log
            .connection
            .execute_batch(&format!("DROP TABLE IF EXISTS {};", self.name));
    }
}

/// Fails on the first label used twice in a table of events or items.
fn unique(
    connection: &Connection,
    table: &str,
    error: impl Fn(String) -> OcelBuildError,
) -> Result<(), OcelSqliteError> {
    let repeated: Option<String> = connection
        .query_row(
            &format!(
                "SELECT label FROM {table} GROUP BY label HAVING count(*) > 1 ORDER BY max(id) LIMIT 1"
            ),
            [],
            |row| row.get(0),
        )
        .ok();
    if let Some(label) = repeated {
        return Err(error(label).into());
    }
    connection.execute_batch(&format!(
        "CREATE UNIQUE INDEX {table}_label ON {table} (label);"
    ))?;
    Ok(())
}

fn record(links: &mut UndeclaredLinks, source: &str, qualifier: &str, missing: &str) {
    links.count += 1;
    if links.examples.len() < UndeclaredLinks::EXAMPLES {
        links
            .examples
            .push(format!("{source} --{qualifier}--> {missing}"));
    }
}

/// A value as text that is equal exactly when the values are equal.
fn canonical(value: &Plain<'_>) -> String {
    match value {
        Plain::Text(text) => format!("s:{text}"),
        Plain::Value(AttributeValue::String(id)) => format!("i:{}", id.0),
        Plain::Value(AttributeValue::Time(at)) => format!("t:{at}"),
        Plain::Value(AttributeValue::Integer(value)) => format!("n:{value}"),
        Plain::Value(AttributeValue::Float(value)) => {
            // -0.0 equals 0.0.
            format!("f:{}", if *value == 0.0 { 0.0 } else { *value }.to_bits())
        }
        Plain::Value(AttributeValue::Boolean(value)) => format!("b:{value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("pm-disk-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A log with every kind of data problem the checks look for.
    fn messy(path: &Path) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE event_map_type (ocel_type TEXT, ocel_type_map TEXT);
                 INSERT INTO event_map_type VALUES ('place','Place'),('pack','Pack'),('unused','Unused');
                 CREATE TABLE object_map_type (ocel_type TEXT, ocel_type_map TEXT);
                 INSERT INTO object_map_type VALUES ('order','Order'),('item','Item'),('ghost','Ghost');
                 CREATE TABLE event (ocel_id TEXT, ocel_type TEXT);
                 INSERT INTO event VALUES ('e3','pack'),('e1','place'),('e2','pack'),('e4','place'),('lonely','place');
                 CREATE TABLE object (ocel_id TEXT, ocel_type TEXT);
                 INSERT INTO object VALUES ('o2','order'),('o1','order'),('i1','item'),('i2','item'),('idle','item');
                 CREATE TABLE event_Place (ocel_id TEXT, ocel_time TIMESTAMP);
                 INSERT INTO event_Place VALUES ('e1','2026-01-01 00:00:00'),('e4','2026-01-01 00:00:00'),('lonely','2026-01-05 00:00:00');
                 CREATE TABLE event_Pack (ocel_id TEXT, ocel_time TIMESTAMP);
                 INSERT INTO event_Pack VALUES ('e3','2026-01-03 00:00:00'),('e2','2026-01-02 00:00:00');
                 CREATE TABLE event_Unused (ocel_id TEXT, ocel_time TIMESTAMP);
                 CREATE TABLE object_Order (ocel_id TEXT, ocel_time TIMESTAMP, ocel_changed_field TEXT, status TEXT, amount REAL);
                 INSERT INTO object_Order VALUES ('o1',NULL,NULL,'open',1.0),('o1','2026-01-02 00:00:00','status','paid',NULL),
                   ('o1','2026-01-02 00:00:00','status','late',NULL),('o2',NULL,NULL,'open',2.0),('o2',NULL,NULL,'open',2.0);
                 CREATE TABLE object_Item (ocel_id TEXT, ocel_time TIMESTAMP, ocel_changed_field TEXT);
                 INSERT INTO object_Item VALUES ('i1',NULL,NULL);
                 CREATE TABLE object_Ghost (ocel_id TEXT, ocel_time TIMESTAMP, ocel_changed_field TEXT);
                 CREATE TABLE event_object (ocel_event_id TEXT, ocel_object_id TEXT, ocel_qualifier TEXT);
                 INSERT INTO event_object VALUES ('e1','o1','order'),('e1','i1','item'),('e1','i1','item'),('e1','i2',''),
                   ('e2','i1','packed'),('e3','missing','x'),('e3','o1','order'),('e4','o2','order'),('e4','i2','item');
                 CREATE TABLE object_object (ocel_source_id TEXT, ocel_target_id TEXT, ocel_qualifier TEXT);
                 INSERT INTO object_object VALUES ('i1','o1','part of'),('i1','o1','part of'),('o1','o1','self'),
                   ('i2','nowhere','part of'),('i2','o2',' ');",
            )
            .unwrap();
    }

    #[test]
    fn checks_on_disk_find_exactly_what_the_loaded_log_shows() {
        let dir = Dir::new("checks");
        let path = dir.path().join("messy.sqlite");
        messy(&path);
        let loaded = crate::read_ocel_sqlite_skeleton(&path).unwrap();
        let expected = crate::assess_ocel_quality(&loaded);
        let disk = DiskLog::open(&path, dir.path()).unwrap();
        assert_eq!(disk.quality().unwrap(), expected);
        assert!(!expected.ready_for_analysis);
        assert!(expected.findings.len() >= 10, "{:#?}", expected.findings);
        assert_eq!(
            disk.qualifiers,
            loaded
                .qualifiers
                .iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        );
        assert_eq!(disk.undeclared_links, loaded.undeclared_links);
        // The scratch copy is removed with the log.
        drop(disk);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_kept_copy_is_reused_until_the_log_changes() {
        let dir = Dir::new("kept");
        let path = dir.path().join("messy.sqlite");
        messy(&path);
        let cache = dir.path().join("cache");
        let first = DiskLog::open_cached(&path, &cache).unwrap();
        let expected = first.quality().unwrap();
        drop(first);
        let copies = || std::fs::read_dir(&cache).unwrap().count();
        assert_eq!(copies(), 1);
        let again = DiskLog::open_cached(&path, &cache).unwrap();
        assert_eq!(again.quality().unwrap(), expected);
        assert_eq!(again.undeclared_links.count, 2);
        drop(again);
        assert_eq!(copies(), 1);
        // A changed log gets a new copy; old copies beyond three are removed.
        for n in 0..4 {
            let other = dir.path().join(format!("other-{n}.sqlite"));
            messy(&other);
            DiskLog::open_cached(&other, &cache).unwrap();
        }
        assert_eq!(copies(), 3);
    }

    #[test]
    fn timelines_on_disk_match_the_loaded_log() {
        let dir = Dir::new("timelines");
        let path = dir.path().join("messy.sqlite");
        messy(&path);
        let loaded = crate::read_ocel_sqlite_skeleton(&path).unwrap();
        let view = pm_core::OcelView::apply(&loaded, &OcelFilter::default()).unwrap();
        let mut expected = Vec::new();
        pm_core::for_each_timeline(&loaded, &view, |object, kind, timeline| {
            expected.push((object, kind, timeline.to_vec()));
        });
        let disk = DiskLog::open(&path, dir.path()).unwrap();
        let mut found = Vec::new();
        disk.for_each_timeline(&DiskView::default(), |object, kind, timeline| {
            found.push((object, kind, timeline.to_vec()));
            Ok(())
        })
        .unwrap();
        assert_eq!(found, expected);
    }
}
