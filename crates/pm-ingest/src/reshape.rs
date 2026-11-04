//! Exports as people make them, turned into the files a pack reads. A report
//! or record list exported by a business user often names columns by their
//! labels (`rentalRequest` rather than `recordId`), writes times as the user
//! saw them (`3/5/2026 9:41 AM`, in their own time zone) and starts with
//! title rows. The agent looks at the column names only, proposes which
//! column is which, the person confirms, and this rewrites the file: renamed
//! columns, times in the pack's form. Every row goes through the engine, none
//! through the conversation.

use chrono::{FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use std::path::Path;

/// The time zone an export's local times are in.
#[derive(Debug, Clone)]
pub enum Zone {
    /// One offset all year, such as `-05:00`.
    Fixed(FixedOffset),
    /// A named zone (`America/New_York`) with its daylight-saving changes,
    /// read from the computer's time zone database.
    Named(Named),
}

/// A named zone's offsets: from each transition (seconds since 1970, UTC),
/// the offset in seconds east of UTC.
#[derive(Debug, Clone)]
pub struct Named {
    pub name: String,
    first: i32,
    transitions: Vec<(i64, i32)>,
    /// The zone still changes its clocks after the last listed transition,
    /// so times beyond it cannot be placed.
    changes_after: bool,
}

impl Named {
    /// Reads a zone from the time zone database (`/usr/share/zoneinfo` on Mac
    /// and Linux).
    pub fn load(name: &str) -> Result<Self, String> {
        let unknown = || {
            format!(
                "unknown time zone {name}; give one such as America/New_York, or an offset such as -05:00"
            )
        };
        if name.is_empty()
            || name.starts_with('/')
            || name
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/_-+".contains(c))
        {
            return Err(unknown());
        }
        let root = std::env::var_os("TZDIR").unwrap_or_else(|| "/usr/share/zoneinfo".into());
        let bytes = std::fs::read(Path::new(&root).join(name)).map_err(|_| unknown())?;
        Self::parse(name, &bytes).ok_or_else(unknown)
    }

    /// Parses a TZif file (version 2 or later, 64-bit section).
    fn parse(name: &str, bytes: &[u8]) -> Option<Self> {
        let counts = |at: usize| -> Option<[usize; 6]> {
            if bytes.get(at..at + 4)? != b"TZif" {
                return None;
            }
            let mut out = [0usize; 6];
            for (i, n) in out.iter_mut().enumerate() {
                let b = bytes.get(at + 20 + 4 * i..at + 24 + 4 * i)?;
                *n = u32::from_be_bytes(b.try_into().ok()?) as usize;
            }
            Some(out)
        };
        let [isut, isstd, leap, time, types, chars] = counts(0)?;
        if bytes.get(4).copied()? < b'2' {
            return None;
        }
        let second = 44 + time * 5 + types * 6 + chars + leap * 8 + isstd + isut;
        let [isut, isstd, leap, time, types, chars] = counts(second)?;
        let mut at = second + 44;
        let mut times = Vec::with_capacity(time);
        for i in 0..time {
            let b = bytes.get(at + 8 * i..at + 8 * i + 8)?;
            times.push(i64::from_be_bytes(b.try_into().ok()?));
        }
        at += time * 8;
        let index = bytes.get(at..at + time)?.to_vec();
        at += time;
        let mut offsets = Vec::with_capacity(types);
        for i in 0..types {
            let b = bytes.get(at + 6 * i..at + 6 * i + 4)?;
            offsets.push(i32::from_be_bytes(b.try_into().ok()?));
        }
        at += types * 6 + chars + leap * 12 + isstd + isut;
        let footer = bytes
            .get(at..)
            .map(String::from_utf8_lossy)
            .unwrap_or_default();
        let transitions = times
            .into_iter()
            .zip(index)
            .map(|(t, i)| Some((t, *offsets.get(i as usize)?)))
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            name: name.to_owned(),
            first: *offsets.first()?,
            transitions,
            changes_after: footer.contains(','),
        })
    }

    /// The offset in force at a UTC instant, or `None` past the listed
    /// changes of a zone that keeps changing.
    fn offset_at(&self, utc: i64) -> Option<i32> {
        match self.transitions.iter().rposition(|(t, _)| *t <= utc) {
            None => Some(self.first),
            Some(i) if i + 1 == self.transitions.len() && self.changes_after => None,
            Some(i) => Some(self.transitions[i].1),
        }
    }

    /// A local time as UTC seconds. A time repeated when clocks go back is
    /// taken as the first; one skipped when they go forward has none.
    fn to_utc(&self, local: NaiveDateTime) -> Option<i64> {
        let wall = local.and_utc().timestamp();
        let mut candidates = self
            .transitions
            .iter()
            .map(|(_, o)| *o)
            .chain([self.first])
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        candidates.dedup();
        candidates
            .into_iter()
            .rev()
            .map(|o| wall - o as i64)
            .find(|utc| {
                self.offset_at(*utc)
                    .is_some_and(|o| wall - o as i64 == *utc)
            })
    }
}
use std::collections::BTreeMap;

/// How to rewrite one export.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Export column → the pack's column.
    pub rename: BTreeMap<String, String>,
    /// The time zone the export's times are in; `None` keeps times without
    /// a zone.
    pub zone: Option<Zone>,
    /// Dates written day first (`5/3/2026` is 5 March).
    pub day_first: bool,
}

/// A slash date with an optional time: `3/5/2026`, `3/5/2026 9:41 AM`,
/// `3/5/2026 14:41`, `3/5/2026 14:41:05`.
fn slash_time(value: &str, day_first: bool) -> Option<(NaiveDate, Option<NaiveTime>)> {
    let value = value.trim();
    let (date, time) = value
        .split_once(' ')
        .map_or((value, ""), |(d, t)| (d, t.trim()));
    let parts = date.split('/').collect::<Vec<_>>();
    let [a, b, year] = parts.as_slice() else {
        return None;
    };
    let (a, b, year) = (
        a.parse::<u32>().ok()?,
        b.parse::<u32>().ok()?,
        year.parse::<i32>().ok()?,
    );
    if !(1000..=9999).contains(&year) {
        return None;
    }
    let (month, day) = if day_first { (b, a) } else { (a, b) };
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    if time.is_empty() {
        return Some((date, None));
    }
    let upper = time.to_ascii_uppercase();
    let parsed = ["%I:%M %p", "%I:%M:%S %p", "%H:%M", "%H:%M:%S"]
        .iter()
        .find_map(|f| NaiveTime::parse_from_str(&upper, f).ok())?;
    Some((date, Some(parsed)))
}

/// A time as the pack reads it: RFC 3339 in UTC when the time zone is known,
/// otherwise as written in ISO form. Anything that is not a slash date, or a
/// cell whose first line is one followed by more (a name under the time),
/// keeps or takes its first line as found.
pub fn time(value: &str, plan: &Plan) -> Option<String> {
    let first = value.lines().next().unwrap_or_default();
    let (date, time) = slash_time(first, plan.day_first)?;
    let utc = |t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    Some(match (time, &plan.zone) {
        (None, _) => date.to_string(),
        (Some(t), Some(Zone::Fixed(offset))) => utc(offset
            .from_local_datetime(&NaiveDateTime::new(date, t))
            .single()?
            .with_timezone(&chrono::Utc)),
        (Some(t), Some(Zone::Named(zone))) => utc(chrono::DateTime::from_timestamp(
            zone.to_utc(NaiveDateTime::new(date, t))?,
            0,
        )?),
        (Some(t), None) => NaiveDateTime::new(date, t)
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string(),
    })
}

/// Rewrites rows (the first being the header): renames columns and writes
/// slash dates in the renamed columns in the pack's form. Returns the new
/// header and rows, and how many times were rewritten.
pub fn apply(rows: &[Vec<String>], plan: &Plan) -> (Vec<String>, Vec<Vec<String>>, u64) {
    let Some((header, body)) = rows.split_first() else {
        return (Vec::new(), Vec::new(), 0);
    };
    let header = header
        .iter()
        .enumerate()
        .map(|(i, h)| {
            // An unnamed column is `column_N`, and can be renamed by that.
            let h = match h.trim() {
                "" => format!("column_{}", i + 1),
                h => h.to_owned(),
            };
            plan.rename.get(&h).cloned().unwrap_or(h)
        })
        .collect::<Vec<_>>();
    let renamed = header
        .iter()
        .map(|h| plan.rename.values().any(|v| v == h))
        .collect::<Vec<_>>();
    let mut rewritten = 0;
    let body = body
        .iter()
        .filter(|row| row.iter().any(|c| !c.trim().is_empty()))
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(i, cell)| {
                    if renamed.get(i).copied().unwrap_or(false)
                        && let Some(t) = time(cell, plan)
                    {
                        rewritten += 1;
                        return t;
                    }
                    cell.clone()
                })
                .collect()
        })
        .collect();
    (header, body, rewritten)
}

/// For each pack column the export lacks, the export column that most
/// likely holds it, by name: equal ignoring case and separators, then one
/// name inside the other (`eventType` for `eventTypeId`).
pub fn suggest(missing: &[String], columns: &[String]) -> BTreeMap<String, String> {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase()
    };
    let mut taken = Vec::new();
    let mut out = BTreeMap::new();
    for field in missing {
        let f = norm(field);
        let pick = columns
            .iter()
            .filter(|c| !taken.contains(*c))
            .find(|c| norm(c) == f)
            .or_else(|| {
                columns
                    .iter()
                    .filter(|c| !taken.contains(*c) && norm(c).len() >= 3)
                    .find(|c| f.contains(&norm(c)) || norm(c).contains(&f))
            });
        if let Some(c) = pick {
            taken.push(c.clone());
            out.insert(field.clone(), c.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|c| (*c).to_owned()).collect()
    }

    #[test]
    fn times_as_a_person_saw_them_become_utc() {
        let eastern = Plan {
            zone: FixedOffset::west_opt(4 * 3600).map(Zone::Fixed),
            ..Plan::default()
        };
        assert_eq!(
            time("10/15/2026 1:47 PM", &eastern).unwrap(),
            "2026-10-15T17:47:00Z"
        );
        // A name under the time, as record lists write "Created".
        assert_eq!(
            time("10/15/2026 1:50 PM\nTropical Glacier", &eastern).unwrap(),
            "2026-10-15T17:50:00Z"
        );
        assert_eq!(time("3/5/2026", &eastern).unwrap(), "2026-03-05");
        let unknown = Plan::default();
        assert_eq!(
            time("3/5/2026 14:41", &unknown).unwrap(),
            "2026-03-05T14:41:00"
        );
        let european = Plan {
            day_first: true,
            ..Plan::default()
        };
        assert_eq!(time("5/3/2026", &european).unwrap(), "2026-03-05");
        assert!(time("13/13/2026", &unknown).is_none());
        assert!(time("Pending", &unknown).is_none());
    }

    #[test]
    fn a_named_zone_follows_daylight_saving() {
        let Ok(zone) = Named::load("America/New_York") else {
            return; // No time zone database on this computer.
        };
        let plan = Plan {
            zone: Some(Zone::Named(zone)),
            ..Plan::default()
        };
        // Winter is five hours behind UTC, summer four.
        assert_eq!(
            time("3/5/2026 9:41 AM", &plan).unwrap(),
            "2026-03-05T14:41:00Z"
        );
        assert_eq!(
            time("3/27/2026 8:48 AM", &plan).unwrap(),
            "2026-03-27T12:48:00Z"
        );
        // 2:30 AM did not happen on 8 March 2026; 1:30 AM on 1 November
        // happened twice and is taken as the first.
        assert!(time("3/8/2026 2:30 AM", &plan).is_none());
        assert_eq!(
            time("11/1/2026 1:30 AM", &plan).unwrap(),
            "2026-11-01T05:30:00Z"
        );
        assert!(Named::load("../etc/passwd").is_err());
        assert!(Named::load("Not/AZone").is_err());
    }

    #[test]
    fn columns_are_renamed_and_only_their_times_rewritten() {
        let plan = Plan {
            rename: BTreeMap::from([
                ("rentalRequest".to_owned(), "recordId".to_owned()),
                ("Timestamp".to_owned(), "timestamp".to_owned()),
                ("column_1".to_owned(), "id".to_owned()),
            ]),
            zone: FixedOffset::west_opt(4 * 3600).map(Zone::Fixed),
            day_first: false,
        };
        let rows = vec![
            row(&["", "rentalRequest", "Timestamp", "Comment"]),
            row(&[
                "1",
                "26",
                "10/15/2026 1:47 PM",
                "made on 10/15/2026 1:00 PM",
            ]),
            row(&["", "", "", ""]),
        ];
        let (header, body, rewritten) = apply(&rows, &plan);
        assert_eq!(header, ["id", "recordId", "timestamp", "Comment"]);
        assert_eq!(
            body,
            vec![row(&[
                "1",
                "26",
                "2026-10-15T17:47:00Z",
                "made on 10/15/2026 1:00 PM"
            ])]
        );
        assert_eq!(rewritten, 1);
    }

    #[test]
    fn suggestions_match_names_loosely_and_once() {
        let columns = row(&["Id", "rentalRequest", "eventType", "eventUser", "Timestamp"]);
        let missing = row(&["recordId", "eventTypeId", "timestamp", "user"]);
        let s = suggest(&missing, &columns);
        assert_eq!(s["eventTypeId"], "eventType");
        assert_eq!(s["timestamp"], "Timestamp");
        assert_eq!(s["user"], "eventUser");
        // No name says which column holds the record: the person says.
        assert!(!s.contains_key("recordId"));
    }
}
