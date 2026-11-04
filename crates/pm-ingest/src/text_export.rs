//! Text exports as Windows tools write them, turned into plain UTF-8 CSV:
//! UTF-16 (with its byte-order mark), Windows-1252 (Excel's "CSV" and many
//! ERP downloads), and semicolon- or tab-separated values (European Excel,
//! SAP spreadsheet downloads). Values are not changed, only their encoding
//! and separator.

use std::path::Path;

/// Characters 0x80–0x9F of Windows-1252; the rest of the upper half is
/// Latin-1.
const CP1252: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
];

/// The text of an export, decoded, and what was done to read it.
fn decode(bytes: &[u8]) -> (String, Option<&'static str>) {
    let utf16 = |big: bool, body: &[u8]| {
        let units = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&p| {
                if big {
                    u16::from_be_bytes(p)
                } else {
                    u16::from_le_bytes(p)
                }
            })
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, body @ ..] => (utf16(false, body), Some("UTF-16")),
        [0xFE, 0xFF, body @ ..] => (utf16(true, body), Some("UTF-16")),
        [0xEF, 0xBB, 0xBF, body @ ..] => match std::str::from_utf8(body) {
            Ok(text) => (text.to_owned(), None),
            Err(_) => (windows_1252(body), Some("Windows-1252")),
        },
        // UTF-16 without its mark: every other byte of plain text is zero.
        _ if bytes.len() >= 4 && zeros(bytes, 1) * 3 > bytes.len() => {
            (utf16(false, bytes), Some("UTF-16"))
        }
        _ if bytes.len() >= 4 && zeros(bytes, 0) * 3 > bytes.len() => {
            (utf16(true, bytes), Some("UTF-16"))
        }
        _ => match std::str::from_utf8(bytes) {
            Ok(text) => (text.to_owned(), None),
            Err(_) => (windows_1252(bytes), Some("Windows-1252")),
        },
    }
}

/// Whether a file is text in some encoding rather than a document, archive
/// or image: no known binary signature, and no zero bytes outside UTF-16.
pub fn looks_like_text(bytes: &[u8]) -> bool {
    let binary = [
        &[0xD0, 0xCF, 0x11, 0xE0][..],
        b"PK\x03\x04",
        b"%PDF",
        &[0x1F, 0x8B],
        b"PAR1",
        &[0x89, b'P', b'N', b'G'],
        &[0xFF, 0xD8, 0xFF],
    ];
    if binary.iter().any(|magic| bytes.starts_with(magic)) {
        return false;
    }
    let utf16 = bytes.starts_with(&[0xFF, 0xFE])
        || bytes.starts_with(&[0xFE, 0xFF])
        || (bytes.len() >= 4
            && (zeros(bytes, 0) * 3 > bytes.len() || zeros(bytes, 1) * 3 > bytes.len()));
    utf16 || !bytes.iter().take(64 * 1024).any(|b| *b == 0)
}

/// Zero bytes at even (0) or odd (1) positions.
fn zeros(bytes: &[u8], at: usize) -> usize {
    bytes
        .iter()
        .skip(at)
        .step_by(2)
        .filter(|b| **b == 0)
        .count()
}

fn windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252[(b - 0x80) as usize],
            _ => b as char,
        })
        .collect()
}

/// The separator of the header line: whichever of comma, semicolon, tab and
/// pipe it has most of, a comma on a tie.
fn separator(text: &str) -> u8 {
    let header = text.lines().next().unwrap_or_default();
    let count = |c: char| header.matches(c).count();
    [
        (b',', count(',')),
        (b';', count(';')),
        (b'\t', count('\t')),
        (b'|', count('|')),
    ]
    .into_iter()
    .fold(
        (b',', 0),
        |best, (c, n)| if n > best.1 { (c, n) } else { best },
    )
    .0
}

/// SAP's list output ("unconverted" download, or a list printed to file):
/// table lines between pipes, with a title above and dashed rules between.
fn sap_list(text: &str) -> Option<Vec<Vec<String>>> {
    let lines = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>();
    let piped = lines
        .iter()
        .filter(|l| l.trim_start().starts_with('|') && l.matches('|').count() >= 3)
        .collect::<Vec<_>>();
    let rules = lines
        .iter()
        .filter(|l| l.trim().chars().all(|c| matches!(c, '-' | '=' | '|' | '+')))
        .count();
    if piped.len() < 2 || (piped.len() + rules) * 2 < lines.len() {
        return None;
    }
    let rows = piped
        .iter()
        .filter(|l| l.chars().any(|c| c.is_alphanumeric()))
        .map(|l| {
            let inner = l.trim().trim_start_matches('|').trim_end_matches('|');
            inner
                .split('|')
                .map(|c| c.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    (rows.len() >= 2).then_some(rows)
}

/// Fixed-width text: columns lined up with spaces under a header whose
/// names are at least two spaces apart. A column starts where the header
/// has a name after spaces, and the data lines agree.
fn fixed_width(text: &str) -> Option<Vec<Vec<String>>> {
    let lines = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>();
    let header = *lines.first()?;
    if lines.len() < 2 || header.contains([',', ';', '\t', '|']) || !header.contains("  ") {
        return None;
    }
    let chars = header.chars().collect::<Vec<_>>();
    let starts = (0..chars.len())
        .filter(|&i| {
            chars[i] != ' ' && (i == 0 || (i >= 2 && chars[i - 1] == ' ' && chars[i - 2] == ' '))
        })
        .collect::<Vec<_>>();
    if starts.len() < 2 {
        return None;
    }
    // Data lines must leave the column boundaries blank (or end there).
    let agree = lines[1..]
        .iter()
        .filter(|l| {
            let c = l.chars().collect::<Vec<_>>();
            starts[1..]
                .iter()
                .all(|&at| c.get(at - 1).is_none_or(|ch| *ch == ' '))
        })
        .count();
    if agree * 10 < (lines.len() - 1) * 9 {
        return None;
    }
    Some(
        lines
            .iter()
            .map(|l| {
                let c = l.chars().collect::<Vec<_>>();
                starts
                    .iter()
                    .enumerate()
                    .map(|(n, &from)| {
                        let to = starts.get(n + 1).copied().unwrap_or(c.len()).min(c.len());
                        c.get(from.min(c.len())..to)
                            .map(|s| s.iter().collect::<String>().trim().to_owned())
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .collect(),
    )
}

/// When `path` is not already UTF-8 and comma-separated: the same table as
/// UTF-8 CSV, with a note of what was converted. `None` when nothing needs
/// converting.
pub fn as_utf8_csv(path: &Path) -> Result<Option<(Vec<u8>, String)>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !looks_like_text(&bytes) {
        return Err(format!(
            "{} is {}, not a text table",
            path.display(),
            describe(path)["kind"].as_str().unwrap_or("not text")
        ));
    }
    let (text, encoding) = decode(&bytes);
    let laid_out = sap_list(&text)
        .map(|rows| (rows, "SAP list output"))
        .or_else(|| fixed_width(&text).map(|rows| (rows, "fixed-width columns")));
    if let Some((rows, layout)) = laid_out {
        let mut writer = csv::Writer::from_writer(Vec::new());
        for row in rows {
            writer
                .write_record(&row)
                .map_err(|e| format!("cannot convert {}: {e}", path.display()))?;
        }
        let out = writer
            .into_inner()
            .map_err(|e| format!("cannot convert {}: {e}", path.display()))?;
        let note = match encoding {
            Some(encoding) => format!("read as {encoding}, {layout}"),
            None => layout.to_owned(),
        };
        return Ok(Some((out, note)));
    }
    let separator = separator(&text);
    if encoding.is_none() && separator == b',' {
        return Ok(None);
    }
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(separator)
        .from_reader(text.as_bytes());
    let mut writer = csv::Writer::from_writer(Vec::new());
    for record in reader.records() {
        let record = record.map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        writer
            .write_record(&record)
            .map_err(|e| format!("cannot convert {}: {e}", path.display()))?;
    }
    let out = writer
        .into_inner()
        .map_err(|e| format!("cannot convert {}: {e}", path.display()))?;
    let mut done = Vec::new();
    if let Some(encoding) = encoding {
        done.push(format!("read as {encoding}"));
    }
    match separator {
        b';' => done.push("semicolon-separated".into()),
        b'\t' => done.push("tab-separated".into()),
        b'|' => done.push("pipe-separated".into()),
        _ => {}
    }
    Ok(Some((out, done.join(", "))))
}

/// A description of a file Glasstarn could not read, for the agent to
/// advise the person: what kind of file it seems to be, its size, and for
/// text its encoding, separator and first-line names (column names only,
/// never data rows), with a suggestion.
pub fn describe(path: &Path) -> serde_json::Value {
    use serde_json::json;
    let Ok(bytes) = std::fs::read(path) else {
        return json!({"kind": "missing or unreadable file"});
    };
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    let (kind, suggestion) = if starts(&[0xD0, 0xCF, 0x11, 0xE0]) {
        (
            "an old-style Office file (an .xls workbook, a Word or Outlook file, or a password-protected Office file)",
            "If it is a workbook with a password, ask for a copy without the password; otherwise ask for it saved as .xlsx or CSV.",
        )
    } else if starts(b"PK") {
        (
            "a zip-based file (a workbook, a Word or PowerPoint file, or a zip archive)",
            "If it is a Word or PowerPoint file, the table inside needs exporting to Excel or CSV first.",
        )
    } else if starts(b"%PDF") {
        (
            "a PDF document",
            "PDF reports do not hold the table itself; ask for the same report exported to Excel or CSV.",
        )
    } else if starts(&[0x1F, 0x8B]) {
        (
            "a gzip-compressed file",
            "Unzip it (double-click on a Mac) and add the file inside.",
        )
    } else if starts(b"PAR1") {
        (
            "a Parquet file",
            "Parquet event logs can be imported directly with workspace import; tables need converting to CSV.",
        )
    } else if starts(&[0x89, b'P', b'N', b'G']) || starts(&[0xFF, 0xD8, 0xFF]) {
        (
            "an image",
            "A screenshot does not hold the data; ask for the export itself.",
        )
    } else {
        ("text", "")
    };
    let mut out = json!({"kind": kind, "bytes": bytes.len()});
    if kind == "text" {
        let (text, encoding) = decode(&bytes);
        let first = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or_default();
        let separator = separator(&text);
        let names = first
            .split(separator as char)
            .take(30)
            .map(|n| n.trim().chars().take(40).collect::<String>())
            .collect::<Vec<_>>();
        let trimmed = text.trim_start();
        let suggestion = if trimmed.starts_with('{') || trimmed.starts_with('[') {
            "It looks like JSON; ask for a CSV or Excel export, or an OCEL JSON file if it is an object-centric event log."
        } else if trimmed.starts_with('<') {
            "It looks like XML or HTML; XES and OCEL XML event logs can be imported directly, other XML needs a CSV or Excel export."
        } else {
            "Check that the first line names the columns; if the file has a title or notes above them, ask for the table alone."
        };
        out["encoding"] = json!(encoding.unwrap_or("UTF-8"));
        out["separator"] = json!((separator as char).to_string());
        out["lines"] = json!(text.lines().count());
        out["first_line_names"] = json!(names);
        out["suggestion"] = json!(suggestion);
    } else {
        out["suggestion"] = json!(suggestion);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(bytes: &[u8]) -> Option<(String, String)> {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("pm-text-export-{}-{n}.csv", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let out = as_utf8_csv(&path)
            .unwrap()
            .map(|(out, note)| (String::from_utf8(out).unwrap(), note));
        std::fs::remove_file(&path).unwrap();
        out
    }

    #[test]
    fn windows_exports_become_utf8_csv() {
        // Already fine: nothing to do.
        assert!(convert("id,name\n1,Müller\n".as_bytes()).is_none());
        // Windows-1252, as Excel's plain "CSV" writes on Windows.
        let (text, note) = convert(b"id,name\n1,M\xfcller \x80 5\n").unwrap();
        assert_eq!(text, "id,name\n1,Müller € 5\n");
        assert_eq!(note, "read as Windows-1252");
        // UTF-16 with a byte-order mark, tab-separated, as SAP's spreadsheet
        // download writes.
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "BELNR\tNAME1\n0001\tSchön\n".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let (text, note) = convert(&utf16).unwrap();
        assert_eq!(text, "BELNR,NAME1\n0001,Schön\n");
        assert_eq!(note, "read as UTF-16, tab-separated");
        // UTF-16 without its mark, pipe-separated.
        let mut bare = Vec::new();
        for unit in "id|name\n1|Søren\n".encode_utf16() {
            bare.extend_from_slice(&unit.to_le_bytes());
        }
        let (text, note) = convert(&bare).unwrap();
        assert_eq!(text, "id,name\n1,Søren\n");
        assert_eq!(note, "read as UTF-16, pipe-separated");
        // SAP list output: a title, dashed rules and piped columns.
        let list = "18.10.2026   Dynamic List Display   1\n\
                    ----------------------------------\n\
                    |BELNR     |GJAHR|BUKRS|\n\
                    ----------------------------------\n\
                    |0000000001|2026 |1000 |\n\
                    |0000000002|2026 |1000 |\n\
                    ----------------------------------\n";
        let (text, note) = convert(list.as_bytes()).unwrap();
        assert_eq!(
            text,
            "BELNR,GJAHR,BUKRS\n0000000001,2026,1000\n0000000002,2026,1000\n"
        );
        assert_eq!(note, "SAP list output");
        // Fixed-width columns.
        let fixed = "Order       Step              Date\n\
                     4500000001  Create order      2026-01-05\n\
                     4500000001  Goods receipt     2026-01-09\n";
        let (text, note) = convert(fixed.as_bytes()).unwrap();
        assert_eq!(
            text,
            "Order,Step,Date\n4500000001,Create order,2026-01-05\n4500000001,Goods receipt,2026-01-09\n"
        );
        assert_eq!(note, "fixed-width columns");
        // Semicolons, as European Excel writes, with commas inside values.
        let (text, _) = convert("id;amount\n1;1,50\n".as_bytes()).unwrap();
        assert_eq!(text, "id,amount\n1,\"1,50\"\n");
    }
}
