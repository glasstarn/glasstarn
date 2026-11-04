//! Format-neutral OCEL 2.0 file entry point.

use crate::{
    OcelJsonError, OcelSqliteError, OcelXmlError, read_ocel_json, read_ocel_sqlite, read_ocel_xml,
};
use pm_core::OcelLog;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum OcelReadError {
    #[error(transparent)]
    Json(#[from] OcelJsonError),
    #[error(transparent)]
    Xml(#[from] OcelXmlError),
    #[error(transparent)]
    Sqlite(#[from] OcelSqliteError),
    #[error("unsupported OCEL file '{0}'; use .jsonocel, .xmlocel, .xml, .sqlite, or .sqlite3")]
    Unsupported(PathBuf),
}

/// Reads an OCEL 2.0 time in microseconds since the Unix epoch.
///
/// RFC 3339 with an offset is the standard form. Published exchange files also
/// write ISO 8601 times without an offset, with a space or `T` separator; the
/// SQLite format stores those as UTC, so every reader takes them as UTC too.
pub(crate) fn ocel_time(value: &str) -> Option<i64> {
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(time.timestamp_micros());
    }
    ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"]
        .iter()
        .find_map(|format| chrono::NaiveDateTime::parse_from_str(value, format).ok())
        .map(|time| time.and_utc().timestamp_micros())
}

/// Whether a path has a supported OCEL 2.0 exchange-format extension.
pub fn is_ocel(path: impl AsRef<Path>) -> bool {
    let name = path
        .as_ref()
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.ends_with(".jsonocel")
        || name.ends_with(".xmlocel")
        || name.ends_with(".xml")
        || name.ends_with(".sqlite")
        || name.ends_with(".sqlite3")
}

/// Reads a supported OCEL 2.0 JSON, XML, or SQLite file.
pub fn read_ocel(path: impl AsRef<Path>) -> Result<OcelLog, OcelReadError> {
    let path = path.as_ref();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.ends_with(".jsonocel") {
        read_ocel_json(path).map_err(Into::into)
    } else if name.ends_with(".xmlocel") || name.ends_with(".xml") {
        read_ocel_xml(path).map_err(Into::into)
    } else if name.ends_with(".sqlite") || name.ends_with(".sqlite3") {
        read_ocel_sqlite(path).map_err(Into::into)
    } else {
        Err(OcelReadError::Unsupported(path.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_extensions_case_insensitively() {
        assert!(is_ocel("log.JSONOCEL"));
        assert!(is_ocel("log.XMLOCEL"));
        assert!(is_ocel("log.XML"));
        assert!(is_ocel("log.SQLITE"));
        assert!(is_ocel("log.SQLite3"));
        assert!(!is_ocel("log.csv"));
    }
}
