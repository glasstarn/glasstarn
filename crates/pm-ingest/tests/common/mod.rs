//! Shared by the pack tests: every build also runs on disk, and must give
//! the same report, log, lineage and OCEL database as the build in memory.
use pm_ingest::prepare_ocel::{Outcome, Recipe, prepare, prepare_on_disk};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Every row of every table of a SQLite file, tables by name, rows in order.
fn dump(path: &Path) -> Vec<(String, Vec<Vec<String>>)> {
    let connection = rusqlite::Connection::open(path).unwrap();
    let mut tables = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    tables.sort();
    tables
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
                .unwrap();
            let width = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..width)
                        .map(|i| {
                            row.get::<_, rusqlite::types::Value>(i)
                                .map(|v| format!("{v:?}"))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

/// Builds in memory and on disk, requires both to agree, and returns the
/// build in memory.
pub fn prepare_both(recipe: &Recipe, files: &BTreeMap<String, PathBuf>) -> Outcome {
    let memory = prepare(recipe, files).unwrap();
    let scratch = std::env::temp_dir();
    let disk = prepare_on_disk(recipe, files, &scratch).unwrap();
    assert_eq!(
        serde_json::to_value(&disk.report).unwrap(),
        serde_json::to_value(&memory.report).unwrap()
    );
    assert_eq!(
        serde_json::to_string(&disk.relationship_lineage).unwrap(),
        serde_json::to_string(&memory.relationship_lineage).unwrap()
    );
    match (&memory.log, &disk.log) {
        (Some(memory_log), Some(disk_log)) => {
            assert_eq!(
                serde_json::to_string(disk_log).unwrap(),
                serde_json::to_string(memory_log).unwrap()
            );
            static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let a = scratch.join(format!("pm-both-{}-{n}-memory.sqlite", std::process::id()));
            let b = scratch.join(format!("pm-both-{}-{n}-disk.sqlite", std::process::id()));
            let _ = std::fs::remove_file(&a);
            let _ = std::fs::remove_file(&b);
            memory_log.write_sqlite(&a).unwrap();
            disk_log.write_sqlite(&b).unwrap();
            assert_eq!(dump(&b), dump(&a));
            std::fs::remove_file(a).unwrap();
            std::fs::remove_file(b).unwrap();
        }
        (None, None) => {}
        _ => panic!("one build published a log and the other did not"),
    }
    memory
}
