//! Excel workbooks (.xlsx) as rows of text: the first sheet, with shared
//! strings resolved and whole numbers written without ".0". Reports and
//! record lists exported to Excel often start with title rows (the report's
//! name, when it was exported, its filters); [`header_row`] finds the row
//! that names the columns.
//!
//! An .xlsx file is a zip archive of XML parts. This reads it with a small zip
//! reader of its own (stored and deflated entries, the only kinds Excel
//! writes) so no further dependency is needed.

/// Why a workbook could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;
use chrono::{Datelike, Duration, NaiveDate};
use flate2::read::DeflateDecoder;
use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DateFormat {
    Date,
    Time,
    DateTime,
}

fn date_format(id: u32, code: &str) -> Option<DateFormat> {
    match id {
        14..=17 | 27..=31 | 34..=36 | 50..=58 => return Some(DateFormat::Date),
        18..=21 | 32..=33 | 45 | 47 => return Some(DateFormat::Time),
        22 => return Some(DateFormat::DateTime),
        46 => return None, // elapsed hours are a duration, not a clock time
        _ => {}
    }
    let mut tokens = String::new();
    let mut quoted = false;
    let mut bracket = String::new();
    let mut in_bracket = false;
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            quoted = !quoted;
            continue;
        }
        if quoted {
            continue;
        }
        if matches!(c, '\\' | '_' | '*') {
            chars.next();
            continue;
        }
        if c == '[' {
            in_bracket = true;
            bracket.clear();
            continue;
        }
        if c == ']' {
            if matches!(
                bracket.to_ascii_lowercase().as_str(),
                "h" | "hh" | "m" | "mm" | "s" | "ss"
            ) {
                return None;
            }
            in_bracket = false;
            continue;
        }
        if in_bracket {
            bracket.push(c);
        } else {
            tokens.push(c.to_ascii_lowercase());
        }
    }
    let time = tokens.contains(['h', 's']);
    let date = tokens.contains(['y', 'd']) || (!time && tokens.contains('m'));
    match (date, time) {
        (true, true) => Some(DateFormat::DateTime),
        (true, false) => Some(DateFormat::Date),
        (false, true) => Some(DateFormat::Time),
        _ => None,
    }
}

fn styles(xml: &[u8]) -> Result<Vec<Option<DateFormat>>> {
    let mut reader = Reader::from_reader(xml);
    let mut formats = BTreeMap::new();
    let mut result = Vec::new();
    let mut cells = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"cellXfs" => cells = true,
            Ok(Event::End(e)) if e.local_name().as_ref() == b"cellXfs" => cells = false,
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == b"numFmt" => {
                let mut id = 0;
                let mut code = String::new();
                for a in e.attributes().flatten() {
                    match a.key.local_name().as_ref() {
                        b"numFmtId" => id = String::from_utf8_lossy(&a.value).parse().unwrap_or(0),
                        b"formatCode" => {
                            code = a
                                .decoded_and_normalized_value(
                                    quick_xml::XmlVersion::Implicit1_0,
                                    reader.decoder(),
                                )
                                .map_err(|_| Error("invalid Excel number format".into()))?
                                .into_owned()
                        }
                        _ => {}
                    }
                }
                formats.insert(id, code);
            }
            Ok(Event::Start(e) | Event::Empty(e)) if cells && e.local_name().as_ref() == b"xf" => {
                let id = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.local_name().as_ref() == b"numFmtId")
                    .and_then(|a| String::from_utf8_lossy(&a.value).parse().ok())
                    .unwrap_or(0);
                result.push(date_format(
                    id,
                    formats.get(&id).map(String::as_str).unwrap_or(""),
                ));
            }
            Ok(Event::Eof) => break,
            Err(_) => return Err(Error("the workbook's styles are not readable".into())),
            _ => {}
        }
    }
    Ok(result)
}

fn epoch_1904(xml: &[u8]) -> Result<bool> {
    let mut reader = Reader::from_reader(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == b"workbookPr" => {
                return Ok(e.attributes().flatten().any(|a| {
                    a.key.local_name().as_ref() == b"date1904"
                        && matches!(a.value.as_ref(), b"1" | b"true")
                }));
            }
            Ok(Event::Eof) => return Ok(false),
            Err(_) => return Err(Error("the workbook's date system is not readable".into())),
            _ => {}
        }
    }
}

fn excel_date(raw: &str, format: DateFormat, epoch1904: bool) -> Result<String> {
    let value: f64 = raw
        .parse()
        .map_err(|_| Error("invalid numeric Excel date".into()))?;
    if !value.is_finite() || !(0.0..=2_958_465.999_999_99).contains(&value) {
        return Err(Error(
            "Excel date is outside the supported calendar range".into(),
        ));
    }
    let mut days = value.floor() as i64;
    if format != DateFormat::Time && !epoch1904 && days == 60 {
        return Err(Error("Excel serial date 60 represents the nonexistent 1900-02-29; correct that date in the source".into()));
    }
    let micros = (value.fract() * 86_400_000_000.0).round() as i64;
    if !epoch1904 && days >= 60 {
        days -= 1;
    }
    let base = if epoch1904 {
        NaiveDate::from_ymd_opt(1904, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(1899, 12, 31)
    }
    .unwrap()
    .and_hms_opt(0, 0, 0)
    .unwrap();
    let day = base
        .checked_add_signed(Duration::days(days))
        .ok_or_else(|| Error("Excel date is outside the supported calendar range".into()))?;
    if day.year() > 9999 {
        return Err(Error(
            "Excel date is outside the supported calendar range".into(),
        ));
    }
    // Date-only display uses the calendar day, never a time rounded across midnight.
    if format == DateFormat::Date {
        return Ok(day.format("%Y-%m-%d").to_string());
    }
    let dt = day
        .checked_add_signed(Duration::microseconds(micros))
        .ok_or_else(|| Error("Excel date is outside the supported calendar range".into()))?;
    if dt.year() > 9999 {
        return Err(Error(
            "Excel date is outside the supported calendar range".into(),
        ));
    }
    let time = if micros % 1_000_000 == 0 {
        dt.format("%H:%M:%S").to_string()
    } else {
        dt.format("%H:%M:%S%.f").to_string()
    };
    Ok(match format {
        DateFormat::Date => dt.format("%Y-%m-%d").to_string(),
        DateFormat::Time => time,
        DateFormat::DateTime => format!("{}T{time}", dt.format("%Y-%m-%d")),
    })
}

fn iso_date(raw: &str, format: Option<DateFormat>) -> Result<String> {
    let at = chrono::DateTime::parse_from_rfc3339(raw).ok();
    let date = at
        .as_ref()
        .map(|d| d.date_naive())
        .or_else(|| NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok());
    let Some(date) = date else {
        return Err(Error("invalid ISO Excel date cell".into()));
    };
    Ok(match (format, at) {
        (Some(DateFormat::Date), _) => date.format("%Y-%m-%d").to_string(),
        (Some(DateFormat::Time), Some(at)) => at.format("%H:%M:%S%.f").to_string(),
        _ => raw.to_owned(),
    })
}

/// The largest part read from a workbook, unpacked.
const PART_LIMIT: u64 = 512 * 1024 * 1024;

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| Error("the workbook is not a readable .xlsx file".into()))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| Error("the workbook is not a readable .xlsx file".into()))
}

/// The zip entries of a workbook: name → (method, compressed size, local
/// header offset).
fn entries(bytes: &[u8]) -> Result<BTreeMap<String, (u16, usize, usize)>> {
    let bad = || Error("the workbook is not a readable .xlsx file".into());
    // The end of central directory record is in the last 64 KiB + 22 bytes.
    let from = bytes.len().saturating_sub(65_557);
    let end = (from..bytes.len().saturating_sub(21))
        .rev()
        .find(|&at| bytes[at..at + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(bad)?;
    let count = u16_at(bytes, end + 10)? as usize;
    let mut at = u32_at(bytes, end + 16)? as usize;
    let mut found = BTreeMap::new();
    for _ in 0..count {
        if bytes.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err(bad());
        }
        let method = u16_at(bytes, at + 10)?;
        let size = u32_at(bytes, at + 20)? as usize;
        let name_len = u16_at(bytes, at + 28)? as usize;
        let extra_len = u16_at(bytes, at + 30)? as usize;
        let comment_len = u16_at(bytes, at + 32)? as usize;
        let offset = u32_at(bytes, at + 42)? as usize;
        let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(bad)?;
        found.insert(
            String::from_utf8_lossy(name).into_owned(),
            (method, size, offset),
        );
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(found)
}

fn part(
    bytes: &[u8],
    entries: &BTreeMap<String, (u16, usize, usize)>,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    let bad = || Error("the workbook is not a readable .xlsx file".into());
    let Some(&(method, size, offset)) = entries.get(name) else {
        return Ok(None);
    };
    if bytes.get(offset..offset + 4) != Some(&[0x50, 0x4b, 0x03, 0x04]) {
        return Err(bad());
    }
    let start =
        offset + 30 + u16_at(bytes, offset + 26)? as usize + u16_at(bytes, offset + 28)? as usize;
    let data = bytes.get(start..start + size).ok_or_else(bad)?;
    let mut out = Vec::new();
    match method {
        0 => out.extend_from_slice(data),
        8 => {
            DeflateDecoder::new(data)
                .take(PART_LIMIT)
                .read_to_end(&mut out)
                .map_err(|_| bad())?;
        }
        _ => {
            return Err(Error(
                "the workbook uses a compression this reader does not support".into(),
            ));
        }
    }
    Ok(Some(out))
}

fn text(reader: &mut Reader<&[u8]>, end: &[u8]) -> Result<String> {
    let mut out = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Text(t)) => out.push_str(
                &t.decode()
                    .map_err(|_| Error("unreadable text in the workbook".into()))?,
            ),
            Ok(Event::GeneralRef(r)) => {
                let name = r
                    .decode()
                    .map_err(|_| Error("unreadable text in the workbook".into()))?;
                // Named entities and character references (`&#xa;` is a
                // line break inside a cell).
                let numeric = name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .map(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| name.strip_prefix('#').map(|n| n.parse::<u32>().ok()))
                    .flatten()
                    .and_then(char::from_u32);
                match numeric {
                    Some(c) => out.push(c),
                    None => out.push_str(match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    }),
                }
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == end => return Ok(out),
            Ok(Event::Eof) => return Ok(out),
            Err(_) => return Err(Error("the workbook's XML is not readable".into())),
            _ => {}
        }
        buf.clear();
    }
}

fn shared_strings(xml: &[u8]) -> Result<Vec<String>> {
    let mut reader = Reader::from_reader(xml);
    let mut strings = Vec::new();
    let mut buf = Vec::new();
    let mut current: Option<String> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"si" => {
                current = Some(String::new())
            }
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"t" => {
                let t = text(&mut reader, b"t")?;
                if let Some(c) = current.as_mut() {
                    c.push_str(&t);
                }
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == b"si" => {
                strings.push(current.take().unwrap_or_default());
            }
            Ok(Event::Eof) => break,
            Err(_) => {
                return Err(Error(
                    "the workbook's shared strings are not readable".into(),
                ));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(strings)
}

/// Column letters to a zero-based index: A → 0, AA → 26.
fn column(reference: &str) -> usize {
    reference
        .bytes()
        .take_while(u8::is_ascii_uppercase)
        .fold(0usize, |n, c| n * 26 + (c - b'A' + 1) as usize)
        .saturating_sub(1)
}

/// A number as Excel stores it, without a trailing ".0" on whole numbers.
fn number(value: &str) -> String {
    match value.strip_suffix(".0") {
        Some(whole)
            if !whole.is_empty() && whole.bytes().all(|c| c.is_ascii_digit() || c == b'-') =>
        {
            whole.to_owned()
        }
        _ => value.to_owned(),
    }
}

/// The files in a zip archive (stored or deflated), by name, for exports
/// delivered zipped. Folders and macOS resource entries are skipped.
pub fn zip_files(path: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let bytes = std::fs::read(path).map_err(|e| Error(format!("cannot read the archive: {e}")))?;
    let entries =
        entries(&bytes).map_err(|_| Error("the file is not a readable zip archive".into()))?;
    let mut files = Vec::new();
    for name in entries.keys() {
        if name.ends_with('/') || name.starts_with("__MACOSX/") || name.contains("/._") {
            continue;
        }
        if let Some(data) = part(&bytes, &entries, name)? {
            files.push((name.clone(), data));
        }
    }
    Ok(files)
}

/// Whether a file name (lower case) is a spreadsheet workbook: Excel
/// (.xlsx, .xlsm, .xls, .xlsb) or LibreOffice (.ods).
pub fn is_workbook(lower: &str) -> bool {
    [".xlsx", ".xlsm", ".xls", ".xlsb", ".ods"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

/// What to tell the person about a password-protected workbook.
pub const PROTECTED: &str = "the workbook is password-protected; ask for a copy without the password (in Excel: File, Info, Protect Workbook, remove the password, then save)";

/// Workbooks in the other formats (old Excel .xls, binary .xlsb and
/// LibreOffice .ods), read with calamine: each sheet's rows of text, with
/// dates as ISO dates and whole numbers without ".0".
fn other_sheets(path: &Path) -> Result<Vec<(String, Vec<Vec<String>>)>> {
    use calamine::{Data, Reader as _, open_workbook_auto};
    let mut book = open_workbook_auto(path).map_err(|e| {
        let text = e.to_string().to_ascii_lowercase();
        if text.contains("password") || text.contains("encrypt") {
            Error(PROTECTED.into())
        } else {
            Error(format!("cannot read the workbook: {e}"))
        }
    })?;
    let mut sheets = Vec::new();
    for name in book.sheet_names() {
        let range = book
            .worksheet_range(&name)
            .map_err(|e| Error(format!("cannot read sheet {name}: {e}")))?;
        let rows = range
            .rows()
            .map(|row| {
                row.iter()
                    .map(|cell| match cell {
                        Data::Empty | Data::Error(_) => String::new(),
                        Data::String(text) => text.clone(),
                        Data::Int(n) => n.to_string(),
                        Data::Float(f) => number(&f.to_string()),
                        Data::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).to_owned(),
                        Data::DateTime(at) => match at.as_datetime() {
                            Some(at) if at.time() == chrono::NaiveTime::MIN => {
                                at.date().format("%Y-%m-%d").to_string()
                            }
                            Some(at) => at.format("%Y-%m-%dT%H:%M:%S").to_string(),
                            None => at.as_f64().to_string(),
                        },
                        Data::DateTimeIso(text) | Data::DurationIso(text) => text.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        sheets.push((name, rows));
    }
    Ok(sheets)
}

/// The rows of a report as a table: from its header row (below any title
/// rows), with two-row headers combined. When the header repeats a name
/// ("No.", "Date" under "Order" and again under "Delivery"), the group
/// heading above it is put in front ("Order No.", "Delivery No.").
pub fn table(rows: &[Vec<String>]) -> Vec<Vec<String>> {
    let repeats = |row: &[String]| {
        row.iter()
            .enumerate()
            .any(|(i, h)| !h.trim().is_empty() && row[..i].iter().any(|o| o.trim() == h.trim()))
    };
    let mut at = header_row(rows);
    // Merged group headings ("Order", "Order", "Delivery", "Delivery") can
    // look like the header; the names are in the row below them.
    if rows.get(at).is_some_and(|r| repeats(r))
        && rows.get(at + 1).is_some_and(|next| {
            let named = next.iter().filter(|c| !c.trim().is_empty()).count();
            let numbers = next
                .iter()
                .filter(|c| c.trim().parse::<f64>().is_ok())
                .count();
            let width = rows[at].iter().filter(|c| !c.trim().is_empty()).count();
            numbers * 4 <= named && named >= width
        })
    {
        at += 1;
    }
    let mut table = rows[at..].to_vec();
    let Some(header) = table.first_mut() else {
        return table;
    };
    let repeated = header
        .iter()
        .enumerate()
        .any(|(i, h)| !h.trim().is_empty() && header[..i].iter().any(|o| o.trim() == h.trim()));
    if at == 0 || !repeated {
        return table;
    }
    let above = &rows[at - 1];
    let titled = above
        .iter()
        .map(|c| c.trim())
        .find(|c| !c.is_empty())
        .is_none_or(|c| c.ends_with(':'));
    if titled {
        return table;
    }
    // A group heading covers the columns to its right until the next one.
    let mut group = String::new();
    for (i, name) in header.iter_mut().enumerate() {
        if let Some(heading) = above.get(i).map(|c| c.trim()).filter(|c| !c.is_empty()) {
            group = heading.to_owned();
        }
        if !group.is_empty() && !name.trim().is_empty() {
            *name = format!("{group} {}", name.trim());
        }
    }
    table
}

/// The first sheet of a workbook as rows of text, padded to the same width.
pub fn read(path: &Path) -> Result<Vec<Vec<String>>> {
    read_sheets(path)?
        .into_iter()
        .next()
        .map(|(_, rows)| rows)
        .ok_or_else(|| Error("the workbook has no sheet".into()))
}

/// The sheets of a workbook in their tab order: each sheet's name and its
/// rows of text. An admin export often holds one table per sheet.
pub fn read_sheets(path: &Path) -> Result<Vec<(String, Vec<Vec<String>>)>> {
    let bytes = std::fs::read(path).map_err(|e| Error(format!("cannot read the workbook: {e}")))?;
    // An Office file in the old compound format: an .xls workbook, or a
    // password-protected one of any kind (Office encrypts into that format).
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        if bytes.windows(32).any(|w| {
            w == "EncryptedPackage"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
                .as_slice()
        }) {
            return Err(Error(PROTECTED.into()));
        }
        return other_sheets(path);
    }
    if !bytes.starts_with(b"PK") {
        return Err(Error(
            "the file is not a spreadsheet Glasstarn can read".into(),
        ));
    }
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if lower.ends_with(".xlsb") || lower.ends_with(".ods") {
        return other_sheets(path);
    }
    let entries = entries(&bytes)?;
    let formats = styles(&part(&bytes, &entries, "xl/styles.xml")?.unwrap_or_default())?;
    let workbook = part(&bytes, &entries, "xl/workbook.xml")?.unwrap_or_default();
    let epoch1904 = epoch_1904(&workbook)?;
    let shared = match part(&bytes, &entries, "xl/sharedStrings.xml")? {
        Some(xml) => shared_strings(xml.as_slice())?,
        None => Vec::new(),
    };
    let rels = part(&bytes, &entries, "xl/_rels/workbook.xml.rels")?.unwrap_or_default();
    let mut sheets = Vec::new();
    for (name, target) in sheet_parts(&workbook, &rels) {
        if entries.contains_key(&target) {
            let xml = part(&bytes, &entries, &target)?.unwrap_or_default();
            sheets.push((name, sheet_rows(&xml, &shared, &formats, epoch1904)?));
        }
    }
    // A workbook without the usual names: its sheet parts in file order.
    if sheets.is_empty() {
        let mut parts = entries
            .keys()
            .filter(|k| k.starts_with("xl/worksheets/sheet") && k.ends_with(".xml"))
            .cloned()
            .collect::<Vec<_>>();
        parts.sort_by_key(|k| (k.len(), k.clone()));
        for (n, target) in parts.into_iter().enumerate() {
            let xml = part(&bytes, &entries, &target)?.unwrap_or_default();
            sheets.push((
                format!("Sheet{}", n + 1),
                sheet_rows(&xml, &shared, &formats, epoch1904)?,
            ));
        }
    }
    Ok(sheets)
}

/// Each sheet's name and its part, from the workbook and its relationships.
fn sheet_parts(workbook: &[u8], rels: &[u8]) -> Vec<(String, String)> {
    let attributes = |xml: &[u8], tag: &[u8]| {
        let mut reader = Reader::from_reader(xml);
        let mut buf = Vec::new();
        let mut found = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == tag => {
                    found.push(
                        e.attributes()
                            .flatten()
                            .map(|a| {
                                let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                                let value = String::from_utf8_lossy(&a.value)
                                    .replace("&lt;", "<")
                                    .replace("&gt;", ">")
                                    .replace("&quot;", "\"")
                                    .replace("&apos;", "'")
                                    .replace("&amp;", "&");
                                (key, value)
                            })
                            .collect::<BTreeMap<_, _>>(),
                    );
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        found
    };
    let targets = attributes(rels, b"Relationship")
        .into_iter()
        .filter_map(|r| Some((r.get("Id")?.clone(), r.get("Target")?.clone())))
        .collect::<BTreeMap<_, _>>();
    attributes(workbook, b"sheet")
        .into_iter()
        .filter_map(|s| {
            let id = s.iter().find(|(k, _)| k.ends_with(":id") || *k == "id")?.1;
            let target = targets.get(id)?;
            let target = match target.strip_prefix('/') {
                Some(absolute) => absolute.to_owned(),
                None => format!("xl/{target}"),
            };
            Some((s.get("name")?.clone(), target))
        })
        .collect()
}

/// The rows of one sheet part, padded to the same width, with each merged
/// range filled with its first cell's value (merged headings and labels).
fn sheet_rows(
    xml: &[u8],
    shared: &[String],
    formats: &[Option<DateFormat>],
    epoch1904: bool,
) -> Result<Vec<Vec<String>>> {
    let mut rows = cell_rows(xml, shared, formats, epoch1904)?;
    for (top, left, bottom, right) in merged_ranges(xml) {
        let value = rows
            .get(top)
            .and_then(|r| r.get(left))
            .cloned()
            .unwrap_or_default();
        for row in rows.iter_mut().take(bottom + 1).skip(top) {
            if row.len() <= right {
                row.resize(right + 1, String::new());
            }
            for cell in &mut row[left..=right] {
                if cell.is_empty() {
                    cell.clone_from(&value);
                }
            }
        }
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
        row.resize(width, String::new());
    }
    Ok(rows)
}

/// Merged ranges as (top row, left column, bottom row, right column).
fn merged_ranges(xml: &[u8]) -> Vec<(usize, usize, usize, usize)> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut ranges = Vec::new();
    let place = |reference: &str| -> Option<(usize, usize)> {
        let digits = reference.find(|c: char| c.is_ascii_digit())?;
        let row = reference[digits..].parse::<usize>().ok()?.checked_sub(1)?;
        Some((row, column(&reference[..digits])))
    };
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == b"mergeCell" => {
                for a in e.attributes().flatten() {
                    if a.key.local_name().as_ref() == b"ref" {
                        let text = String::from_utf8_lossy(&a.value).into_owned();
                        if let Some((from, to)) = text.split_once(':')
                            && let (Some((t, l)), Some((b, r))) = (place(from), place(to))
                        {
                            ranges.push((t, l, b, r));
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    ranges
}

/// The cells of one sheet part, row by row, padded to the same width.
fn cell_rows(
    xml: &[u8],
    shared: &[String],
    formats: &[Option<DateFormat>],
    epoch1904: bool,
) -> Result<Vec<Vec<String>>> {
    let mut reader = Reader::from_reader(xml);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut buf = Vec::new();
    let (mut row, mut kind, mut at) = (Vec::<String>::new(), String::new(), 0usize);
    let mut next_column = 0usize;
    let mut cell_style = 0usize;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"row" => {
                row = Vec::new();
                next_column = 0;
            }
            Ok(Event::Empty(e)) if e.local_name().as_ref() == b"row" => rows.push(Vec::new()),
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"c" => {
                kind.clear();
                cell_style = 0;
                at = next_column;
                for a in e.attributes().flatten() {
                    match a.key.local_name().as_ref() {
                        b"t" => kind = String::from_utf8_lossy(&a.value).into_owned(),
                        b"r" => at = column(&String::from_utf8_lossy(&a.value)),
                        b"s" => {
                            cell_style = String::from_utf8_lossy(&a.value)
                                .parse()
                                .map_err(|_| Error("invalid Excel cell style".into()))?
                        }
                        _ => {}
                    }
                }
                next_column = at + 1;
            }
            Ok(Event::Start(e)) if matches!(e.local_name().as_ref(), b"v" | b"t") => {
                let tag = e.local_name().as_ref().to_vec();
                let raw = text(&mut reader, &tag)?;
                let value = match kind.as_str() {
                    "s" => raw
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| shared.get(i).cloned())
                        .unwrap_or_default(),
                    "inlineStr" | "str" => raw,
                    "b" => (if raw == "1" { "TRUE" } else { "FALSE" }).to_owned(),
                    "d" => iso_date(&raw, formats.get(cell_style).copied().flatten())?,
                    "" | "n" => match formats.get(cell_style).copied().flatten() {
                        Some(format) => excel_date(&raw, format, epoch1904)?,
                        None => number(&raw),
                    },
                    _ => raw,
                };
                if row.len() <= at {
                    row.resize(at + 1, String::new());
                }
                row[at].push_str(&value);
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == b"row" => {
                rows.push(std::mem::take(&mut row))
            }
            Ok(Event::Eof) => break,
            Err(_) => return Err(Error("the workbook's sheet is not readable".into())),
            _ => {}
        }
        buf.clear();
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
        row.resize(width, String::new());
    }
    Ok(rows)
}

/// The row that names the columns: the first with names in most columns
/// (title rows such as "Record Name:" and "Exported On:" fill only a few).
pub fn header_row(rows: &[Vec<String>]) -> usize {
    let width = rows
        .iter()
        .map(|r| r.iter().filter(|c| !c.trim().is_empty()).count())
        .max()
        .unwrap_or(0);
    rows.iter()
        .position(|r| {
            let mut cells = r.iter().map(|c| c.trim()).filter(|c| !c.is_empty());
            // "Report:", "Exported On:" start title rows, not headers.
            let titled = cells.clone().next().is_some_and(|c| c.ends_with(':'));
            // Column names are words; a row that is partly numbers is a title
            // line (Business Central's "package, table, table number") or data.
            let numbers = cells.clone().filter(|c| c.parse::<f64>().is_ok()).count();
            let named = cells.by_ref().count();
            !titled && numbers * 4 <= named && named * 4 >= width * 3 && named >= 2.min(width)
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_serial_boundaries_do_not_invent_a_leap_day_or_timezone() {
        assert_eq!(
            excel_date("1", DateFormat::Date, false).unwrap(),
            "1900-01-01"
        );
        assert_eq!(
            excel_date("59", DateFormat::Date, false).unwrap(),
            "1900-02-28"
        );
        assert!(excel_date("60", DateFormat::Date, false).is_err());
        assert_eq!(
            excel_date("61", DateFormat::Date, false).unwrap(),
            "1900-03-01"
        );
        assert_eq!(
            excel_date("0", DateFormat::Date, true).unwrap(),
            "1904-01-01"
        );
        assert_eq!(
            excel_date("0.5", DateFormat::Time, false).unwrap(),
            "12:00:00"
        );
        assert_eq!(
            excel_date("39448.99999999999", DateFormat::Date, false).unwrap(),
            "2008-01-01"
        );
        assert_eq!(
            excel_date("0.000001", DateFormat::Time, false).unwrap(),
            "00:00:00.086400"
        );
        assert!(excel_date("2958465", DateFormat::Date, true).is_err());
        assert!(excel_date("NaN", DateFormat::Date, false).is_err());
        assert!(excel_date("-1", DateFormat::Date, false).is_err());
        assert_eq!(date_format(164, r#"0.00 "days""#), None);
        assert_eq!(date_format(164, r#"0.00 \m"#), None);
        assert_eq!(date_format(164, "[Red]0.00"), None);
        assert_eq!(date_format(164, "[h]:mm:ss"), None);
        assert_eq!(
            date_format(164, "[$-409]m/d/yyyy h:mm"),
            Some(DateFormat::DateTime)
        );
    }

    #[test]
    fn whole_numbers_lose_their_decimal_point() {
        assert_eq!(number("1.0"), "1");
        assert_eq!(number("-12.0"), "-12");
        assert_eq!(number("1.5"), "1.5");
        assert_eq!(number("1.0E3"), "1.0E3");
        assert_eq!(column("A1"), 0);
        assert_eq!(column("AB12"), 27);
    }

    #[test]
    fn character_references_are_kept() {
        let xml = "<si><t>3/5/2026 9:41 AM&#xa;&#8211;&amp;</t></si>";
        assert_eq!(
            shared_strings(xml.as_bytes()).unwrap(),
            ["3/5/2026 9:41 AM\n–&"]
        );
    }

    #[test]
    fn two_row_headers_and_merged_cells_become_one_header() {
        let row = |cells: &[&str]| cells.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>();
        // Group headings over repeated names, as a merged "Order" and
        // "Delivery" spanning two columns each leaves them once filled in.
        let rows = vec![
            row(&["Report:", "Deliveries", "", "", ""]),
            row(&["", "Order", "Order", "Delivery", "Delivery"]),
            row(&["Customer", "No.", "Date", "No.", "Date"]),
            row(&["C1", "1", "2026-01-05", "D1", "2026-01-07"]),
        ];
        let combined = table(&rows);
        assert_eq!(
            combined[0],
            [
                "Customer",
                "Order No.",
                "Order Date",
                "Delivery No.",
                "Delivery Date"
            ]
        );
        assert_eq!(combined.len(), 2);
        // A plain header is left alone.
        let rows = vec![row(&["Id", "Name"]), row(&["1", "A"])];
        assert_eq!(table(&rows)[0], ["Id", "Name"]);
        // Merged ranges read from the sheet are filled with their value.
        let xml = br#"<worksheet><sheetData><row r="1"><c r="B1" t="inlineStr"><is><t>Order</t></is></c></row></sheetData><mergeCells><mergeCell ref="B1:C1"/></mergeCells></worksheet>"#;
        let rows = sheet_rows(xml, &[], &[], false).unwrap();
        assert_eq!(rows[0], ["", "Order", "Order"]);
    }

    #[test]
    fn a_password_protected_workbook_is_named_as_such() {
        let dir = std::env::temp_dir().join(format!("pm-protected-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("locked.xlsx");
        let mut bytes = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        bytes.extend(vec![0; 64]);
        bytes.extend("EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(&path, bytes).unwrap();
        let error = read_sheets(&path).unwrap_err().to_string();
        assert!(error.contains("password-protected"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_header_row_is_found_below_title_rows() {
        let row = |cells: &[&str]| cells.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>();
        let rows = vec![
            row(&["Record Name:", "Rental Requests", "", "", ""]),
            row(&["Exported On:", "Oct 15", "", "", ""]),
            row(&["Filter Selections", "", "", "", ""]),
            row(&["", "Id", "Start", "Status", "Created"]),
            row(&["1", "00001", "x", "Pending", "y"]),
        ];
        assert_eq!(header_row(&rows), 3);
        assert_eq!(header_row(&rows[3..]), 0);
        // A narrow sheet whose title row fills two of three columns.
        let rows = vec![
            row(&["Report:", "Claim steps", ""]),
            row(&["", "", ""]),
            row(&["Claim", "Step", "When"]),
            row(&["C-1", "Received", "3/5/2026 9:41 AM"]),
        ];
        assert_eq!(header_row(&rows), 2);
        // Business Central's configuration package: package, table and table
        // number above the field names, even on a narrow sheet.
        let rows = vec![
            row(&["GLASSTARN", "Purch. Inv. Header", "122"]),
            row(&["No.", "Order No.", "Posting Date"]),
            row(&["I1", "1000", "2026-02-05"]),
        ];
        assert_eq!(header_row(&rows), 1);
    }
}
