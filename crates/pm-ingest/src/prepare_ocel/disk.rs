//! Building a connected log on disk, for inputs larger than memory.
//!
//! The build is the same code as in memory; only where its items, steps and
//! links are kept differs: here in a scratch database, found by identity
//! through indexes, so memory stays small however large the inputs. The log,
//! report and lineage are the same as a build in memory.
use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

/// The scratch database's file, removed when the build is dropped.
struct ScratchFile(PathBuf);

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn failed(error: rusqlite::Error) -> Error {
    Error(format!("the scratch database for building failed: {error}"))
}

struct DiskStore {
    connection: Connection,
    file: ScratchFile,
    names: Names,
    objects: u32,
    events: u32,
    object_links: usize,
    event_links: usize,
    lineage: LineageFile,
}

impl DiskStore {
    fn new(scratch: &Path) -> Result<Self> {
        static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = scratch.join(format!(
            "pm-build-{}-{}.sqlite",
            std::process::id(),
            RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let file = ScratchFile(path.clone());
        let connection = Connection::open(&path).map_err(failed)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF; PRAGMA temp_store = FILE; PRAGMA soft_heap_limit = 134217728;
                 PRAGMA cache_size = -16384;
                 CREATE TABLE obj (pos INTEGER PRIMARY KEY, ident TEXT NOT NULL, kind INTEGER NOT NULL,
                                   rule INTEGER NOT NULL, row INTEGER NOT NULL, vals TEXT NOT NULL);
                 CREATE UNIQUE INDEX obj_ident ON obj (ident);
                 CREATE TABLE evt (pos INTEGER PRIMARY KEY, ident TEXT NOT NULL, kind INTEGER NOT NULL,
                                   time TEXT NOT NULL, rule INTEGER NOT NULL, row INTEGER NOT NULL, vals TEXT NOT NULL);
                 CREATE UNIQUE INDEX evt_ident ON evt (ident);
                 CREATE TABLE olink (seq INTEGER PRIMARY KEY, source INTEGER NOT NULL, target INTEGER NOT NULL, qualifier INTEGER NOT NULL);
                 CREATE UNIQUE INDEX olink_key ON olink (source, target, qualifier);
                 CREATE TABLE elink (seq INTEGER PRIMARY KEY, source INTEGER NOT NULL, target INTEGER NOT NULL, qualifier INTEGER NOT NULL);
                 CREATE UNIQUE INDEX elink_key ON elink (source, target, qualifier);
                 CREATE TABLE lookup (k TEXT PRIMARY KEY, vals TEXT NOT NULL);
                 BEGIN;",
            )
            .map_err(failed)?;
        Ok(Self {
            connection,
            file,
            names: Names::default(),
            objects: 0,
            events: 0,
            object_links: 0,
            event_links: 0,
            lineage: LineageFile::new()?,
        })
    }

    fn json(values: &[&str]) -> String {
        serde_json::to_string(values).unwrap_or_default()
    }
}

impl Store for DiskStore {
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
        let added = self
            .connection
            .prepare_cached("INSERT OR IGNORE INTO obj VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
            .map_err(failed)?
            .execute(params![
                self.objects,
                id,
                kind,
                rule,
                row as i64,
                Self::json(values)
            ])
            .map_err(failed)?;
        if added == 0 {
            return Ok(None);
        }
        self.objects += 1;
        Ok(Some(self.objects - 1))
    }
    fn find_object(&self, id: &str) -> Result<Option<u32>> {
        self.connection
            .prepare_cached("SELECT pos FROM obj WHERE ident = ?1")
            .map_err(failed)?
            .query_row([id], |row| row.get(0))
            .optional()
            .map_err(failed)
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
        let added = self
            .connection
            .prepare_cached("INSERT OR IGNORE INTO evt VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")
            .map_err(failed)?
            .execute(params![
                self.events,
                id,
                kind,
                time.to_rfc3339(),
                rule,
                row as i64,
                Self::json(values)
            ])
            .map_err(failed)?;
        if added == 0 {
            return Ok(None);
        }
        self.events += 1;
        Ok(Some(self.events - 1))
    }
    fn find_event(&self, id: &str) -> Result<Option<u32>> {
        self.connection
            .prepare_cached("SELECT pos FROM evt WHERE ident = ?1")
            .map_err(failed)?
            .query_row([id], |row| row.get(0))
            .optional()
            .map_err(failed)
    }
    fn add_link(&mut self, record: LinkRecord) -> Result<bool> {
        let sql = if record.from_event {
            "INSERT OR IGNORE INTO elink (source, target, qualifier) VALUES (?1, ?2, ?3)"
        } else {
            "INSERT OR IGNORE INTO olink (source, target, qualifier) VALUES (?1, ?2, ?3)"
        };
        let added = self
            .connection
            .prepare_cached(sql)
            .map_err(failed)?
            .execute(params![record.source, record.target, record.qualifier])
            .map_err(failed)?;
        if added == 0 {
            return Ok(false);
        }
        if record.from_event {
            self.event_links += 1;
        } else {
            self.object_links += 1;
        }
        self.lineage.push(record);
        Ok(true)
    }
    fn lookup_clear(&mut self) -> Result<()> {
        self.connection
            .execute_batch("DELETE FROM lookup;")
            .map_err(failed)
    }
    fn lookup_insert(&mut self, key: Vec<String>, values: Vec<String>) -> Result<bool> {
        let key = serde_json::to_string(&key).unwrap_or_default();
        let existed = self
            .connection
            .prepare_cached("SELECT 1 FROM lookup WHERE k = ?1")
            .map_err(failed)?
            .exists([&key])
            .map_err(failed)?;
        self.connection
            .prepare_cached("INSERT OR REPLACE INTO lookup VALUES (?1, ?2)")
            .map_err(failed)?
            .execute([&key, &serde_json::to_string(&values).unwrap_or_default()])
            .map_err(failed)?;
        Ok(!existed)
    }
    fn lookup_get(&mut self, key: &[String]) -> Result<Option<Vec<String>>> {
        let key = serde_json::to_string(key).unwrap_or_default();
        let found: Option<String> = self
            .connection
            .prepare_cached("SELECT vals FROM lookup WHERE k = ?1")
            .map_err(failed)?
            .query_row([&key], |row| row.get(0))
            .optional()
            .map_err(failed)?;
        Ok(found.and_then(|text| serde_json::from_str(&text).ok()))
    }
    fn carried(
        &mut self,
        event_rule: u16,
        through: u16,
        relationship: u32,
        f: &mut Carry<'_, Self>,
    ) -> Result<()> {
        self.connection
            .execute_batch(
                "DROP TABLE IF EXISTS temp.targets; DROP TABLE IF EXISTS temp.added;
                 CREATE TEMP TABLE targets (seq INTEGER PRIMARY KEY, source INTEGER, target INTEGER, input INTEGER, row INTEGER);
                 CREATE TEMP TABLE added (seq INTEGER PRIMARY KEY, position INTEGER, target INTEGER, input INTEGER, row INTEGER);",
            )
            .map_err(failed)?;
        {
            let mut insert = self
                .connection
                .prepare_cached(
                    "INSERT INTO targets (source, target, input, row) VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(failed)?;
            for record in self
                .lineage
                .records()
                .filter(|l| !l.from_event && l.rule == relationship)
            {
                insert
                    .execute(params![
                        record.source,
                        record.target,
                        record.input,
                        record.row as i64
                    ])
                    .map_err(failed)?;
            }
        }
        self.connection
            .execute(
                "CREATE INDEX temp.targets_source ON targets (source, seq)",
                [],
            )
            .map_err(failed)?;
        self.connection
            .execute(
                "INSERT INTO added (position, target, input, row)
                 SELECT e.pos, t.target, t.input, t.row FROM evt AS e
                 JOIN elink AS l ON l.source = e.pos
                 JOIN obj AS o ON o.pos = l.target
                 JOIN targets AS t ON t.source = l.target
                 WHERE e.rule = ?1 AND o.rule = ?2
                 ORDER BY e.pos, l.seq, t.seq",
                params![event_rule, through],
            )
            .map_err(failed)?;
        let total: i64 = self
            .connection
            .query_row("SELECT count(*) FROM added", [], |row| row.get(0))
            .map_err(failed)?;
        for seq in 1..=total {
            let (position, target, input, row): (u32, u32, u32, i64) = self
                .connection
                .prepare_cached("SELECT position, target, input, row FROM added WHERE seq = ?1")
                .map_err(failed)?
                .query_row([seq], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .map_err(failed)?;
            f(self, position, target, input, row as u64)?;
        }
        self.connection
            .execute_batch("DROP TABLE temp.targets; DROP TABLE temp.added;")
            .map_err(failed)
    }
    fn unlinked_events(&self) -> Result<Vec<(u16, u64)>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT rule, row FROM evt AS e WHERE NOT EXISTS (SELECT 1 FROM elink AS l WHERE l.source = e.pos) ORDER BY pos",
            )
            .map_err(failed)?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)? as u16, row.get::<_, i64>(1)? as u64))
            })
            .map_err(failed)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(failed)
    }
    fn totals(&self) -> Result<[usize; 4]> {
        Ok([
            self.objects as usize,
            self.events as usize,
            self.object_links,
            self.event_links,
        ])
    }
}

/// A connected log built on disk.
pub(super) struct DiskGraph {
    connection: Connection,
    _file: ScratchFile,
    names: Names,
    object_fields: Vec<(Vec<String>, u32)>,
    event_fields: Vec<(Vec<String>, u32)>,
    object_types: Vec<Value>,
    event_types: Vec<Value>,
    counts: [usize; 2],
    lineage: std::cell::RefCell<LineageFile>,
}

impl Graphs for DiskGraph {
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
        self.counts[usize::from(events)]
    }
    fn each(&self, events: bool, f: &mut dyn FnMut(EntityRow) -> Result<()>) -> Result<()> {
        let (entities, links, time) = if events {
            ("evt", "elink", "time")
        } else {
            ("obj", "olink", "NULL")
        };
        let mut rows = self
            .connection
            .prepare(&format!(
                "SELECT pos, ident, kind, {time}, rule, row, vals FROM {entities} ORDER BY pos"
            ))
            .map_err(failed)?;
        let mut rows = rows.query([]).map_err(failed)?;
        let mut linked = self
            .connection
            .prepare(&format!(
                "SELECT l.source, o.ident, l.qualifier FROM {links} AS l \
                 JOIN obj AS o ON o.pos = l.target ORDER BY l.source, l.seq"
            ))
            .map_err(failed)?;
        let mut linked = linked.query([]).map_err(failed)?;
        let mut pending: Option<(u32, String, u32)> = None;
        let mut next_link = |pending: &mut Option<(u32, String, u32)>| -> Result<()> {
            *pending = match linked.next().map_err(failed)? {
                Some(row) => Some((
                    row.get(0).map_err(failed)?,
                    row.get(1).map_err(failed)?,
                    row.get(2).map_err(failed)?,
                )),
                None => None,
            };
            Ok(())
        };
        next_link(&mut pending)?;
        while let Some(row) = rows.next().map_err(failed)? {
            let position: u32 = row.get(0).map_err(failed)?;
            let values: String = row.get(6).map_err(failed)?;
            let mut links = Vec::new();
            while let Some((source, target, qualifier)) = pending.take() {
                if source != position {
                    pending = Some((source, target, qualifier));
                    break;
                }
                links.push((target, self.names.get(qualifier).to_owned()));
                next_link(&mut pending)?;
            }
            f(EntityRow {
                id: row.get(1).map_err(failed)?,
                kind: self.names.get(row.get(2).map_err(failed)?).to_owned(),
                time: row.get(3).map_err(failed)?,
                rule: row.get(4).map_err(failed)?,
                row: row.get::<_, i64>(5).map_err(failed)? as u64,
                values: serde_json::from_str(&values).unwrap_or_default(),
                links,
            })?;
        }
        Ok(())
    }
    fn identity_at(&self, events: bool, position: u32) -> Result<String> {
        let sql = if events {
            "SELECT ident FROM evt WHERE pos = ?1"
        } else {
            "SELECT ident FROM obj WHERE pos = ?1"
        };
        self.connection
            .prepare_cached(sql)
            .map_err(failed)?
            .query_row([position], |row| row.get(0))
            .map_err(failed)
    }
    fn lineage_file(&self) -> std::cell::RefMut<'_, LineageFile> {
        self.lineage.borrow_mut()
    }
}

/// [`prepare`] with the build kept in a scratch database under `scratch`:
/// the same log, report and lineage, with memory that does not grow with the
/// inputs.
pub fn prepare_on_disk(
    recipe: &Recipe,
    inputs: &BTreeMap<String, PathBuf>,
    scratch: &Path,
) -> Result<Outcome> {
    let (excluded, set_aside) = set_aside_rows(recipe, inputs, || DiskStore::new(scratch))?;
    let mut store = DiskStore::new(scratch)?;
    let mut built = build(recipe, inputs, &mut store, &excluded)?;
    built.report.set_aside_rows(&set_aside);
    let DiskStore {
        connection,
        file,
        names,
        objects,
        events,
        mut lineage,
        ..
    } = store;
    connection
        .execute_batch(
            "COMMIT;
             CREATE INDEX elink_order ON elink (source, seq);
             CREATE INDEX olink_order ON olink (source, seq);",
        )
        .map_err(failed)?;
    lineage.finish();
    let complete = built.report.issue_count == 0;
    let graph = Rc::new(DiskGraph {
        connection,
        _file: file,
        names,
        object_fields: built.object_fields,
        event_fields: built.event_fields,
        object_types: built.object_types,
        event_types: built.event_types,
        counts: [objects as usize, events as usize],
        lineage: std::cell::RefCell::new(lineage),
    });
    Ok(Outcome {
        report: built.report,
        set_aside,
        log: complete.then(|| Log(Built::Disk(Rc::clone(&graph)))),
        relationship_lineage: Lineage(Built::Disk(graph)),
    })
}
