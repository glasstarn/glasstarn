//! Writing made-up extracts: one CSV per table, header row first.

use std::io;
use std::path::Path;

/// One table being written.
pub(crate) struct Table {
    header: &'static str,
    rows: Vec<String>,
}

impl Table {
    pub(crate) fn new(header: &'static str) -> Self {
        Self {
            header,
            rows: Vec::new(),
        }
    }

    /// Adds a row; the values must match the header.
    pub(crate) fn push(&mut self, values: &[&str]) {
        assert_eq!(
            values.len(),
            self.header.split(',').count(),
            "{}",
            self.header
        );
        self.rows.push(
            values
                .iter()
                .map(|v| {
                    if v.contains([',', '"', '\n']) {
                        format!("\"{}\"", v.replace('"', "\"\""))
                    } else {
                        (*v).to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join(","),
        );
    }
}

impl Table {
    /// Changes rows already written, such as a later cancellation.
    pub(crate) fn update(&mut self, change: impl FnMut(&mut String)) {
        self.rows.iter_mut().for_each(change);
    }
}

/// Saves each table as `<name>.csv` in `dir`.
pub(crate) fn save(dir: &Path, tables: &[(&str, &Table)]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, table) in tables {
        let mut text = String::from(table.header);
        text.push('\n');
        for row in &table.rows {
            text.push_str(row);
            text.push('\n');
        }
        std::fs::write(dir.join(format!("{name}.csv")), text)?;
    }
    Ok(())
}
