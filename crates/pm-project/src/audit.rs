//! The project's append-only audit log.
//!
//! One JSON line per consequential action: who did it, when, and what it
//! touched. Each line carries the SHA-256 of the line before it, so an edited
//! or deleted entry breaks the chain and [`verify`] says where. The log stays
//! in the project folder; nothing is sent anywhere.
//!
//! The chain makes tampering evident, not impossible: anyone who can write the
//! folder can rewrite the whole file. Keep a copy elsewhere when that matters.

use chrono::Utc;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The log's location inside a project folder.
pub fn path(root: &Path) -> PathBuf {
    root.join("audit").join("log.jsonl")
}

fn digest(line: &str) -> String {
    format!("{:x}", Sha256::digest(line.as_bytes()))
}

/// Appends one entry. `details` must be a JSON object; its fields sit beside
/// the sequence number, time, action, operating-system user and chain hash.
///
/// Appends from several processes are serialized with a file lock, so the
/// chain stays linear when the app and the command line both write.
pub fn append(root: &Path, action: &str, details: Value) -> Result<(), String> {
    use fs2::FileExt;
    let Value::Object(details) = details else {
        return Err("audit details must be an object".into());
    };
    let path = path(root);
    let dir = path.parent().expect("audit file has a folder");
    if dir.exists()
        && !fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
    {
        return Err("audit directory is not a regular project directory".into());
    }
    fs::create_dir_all(dir).map_err(|_| "cannot create the audit log")?;
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&path)
        .map_err(|_| "cannot open the audit log")?;
    file.lock_exclusive()
        .map_err(|_| "cannot lock the audit log")?;
    let mut existing = String::new();
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_to_string(&mut existing))
        .map_err(|_| "cannot read the audit log")?;
    let previous = existing.lines().rfind(|line| !line.is_empty());
    let mut entry = json!({
        "seq": existing.lines().filter(|line| !line.is_empty()).count() + 1,
        "at": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "action": action,
        "os_user": std::env::var("USER").or_else(|_| std::env::var("USERNAME")).ok(),
        "previous_sha256": previous.map(digest),
    });
    for (key, value) in details {
        entry[key] = value;
    }
    let line = serde_json::to_string(&entry).map_err(|error| error.to_string())?;
    file.write_all(format!("{line}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| "cannot write the audit log")?;
    FileExt::unlock(&file).map_err(|_| "cannot unlock the audit log")?;
    Ok(())
}

/// Checks the chain. Returns the number of entries, or the first entry whose
/// link to the one before it is broken.
pub fn verify(root: &Path) -> Result<usize, String> {
    let text = match fs::read_to_string(path(root)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(_) => return Err("cannot read the audit log".into()),
    };
    let mut previous: Option<&str> = None;
    let mut count = 0;
    for (index, line) in text.lines().filter(|line| !line.is_empty()).enumerate() {
        let entry: Value = serde_json::from_str(line)
            .map_err(|_| format!("audit entry {} is not valid JSON", index + 1))?;
        let expected = previous.map(digest);
        if entry["previous_sha256"].as_str() != expected.as_deref()
            || entry["seq"].as_u64() != Some(index as u64 + 1)
        {
            return Err(format!(
                "audit entry {} does not follow the entry before it",
                index + 1
            ));
        }
        previous = Some(line);
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_chain_and_an_edit_breaks_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), "extract_requested", json!({"spec_sha256":"a"})).unwrap();
        append(dir.path(), "extract_approved", json!({"approved_by":"Pat"})).unwrap();
        append(dir.path(), "extract_completed", json!({"rows":3})).unwrap();
        assert_eq!(verify(dir.path()).unwrap(), 3);
        let text = fs::read_to_string(path(dir.path())).unwrap();
        let first: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(first["action"], "extract_requested");
        assert!(first["previous_sha256"].is_null());
        fs::write(path(dir.path()), text.replace("Pat", "Sam")).unwrap();
        assert!(verify(dir.path()).unwrap_err().contains("entry 3"));
    }
}
