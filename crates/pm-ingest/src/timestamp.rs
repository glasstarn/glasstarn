//! Timestamp format detection.
//!
//! # Why this is not a one-liner
//!
//! `05/01/2026` is either 5 January or 1 May. Both readings parse cleanly, and
//! guessing wrong shifts every date by months while producing output that
//! looks entirely plausible — every duration, every bottleneck, and every
//! throughput figure silently wrong.
//!
//! So detection never returns a single answer when the data does not justify
//! one. It returns what the evidence supports, states plainly when the sample
//! is ambiguous, and leaves the decision to the user. When the data settles
//! the question by itself — a `13` cannot be a month — no question is asked.
//!
//! # Coverage
//!
//! The catalogue covers what operational systems actually emit: ISO in its
//! several spellings, day-first and month-first with slashes, dashes and dots,
//! twelve-hour clocks with AM/PM, month-name forms such as `02-MAR-2026` used
//! by SAP and Oracle exports, compact `YYYYMMDD`, and Unix epoch values in
//! seconds, milliseconds and microseconds.
//!
//! No fixed list can be complete, so [`Format::custom`] accepts a
//! user-supplied pattern for anything unusual.
//!
//! Detection reads a *sample*; the confirmed format is then applied in bulk.

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use std::borrow::Cow;

/// What kind of value a format reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Shape {
    /// A date and a time.
    DateTime,
    /// A date only; the time becomes midnight.
    DateOnly,
    /// Whole seconds since the Unix epoch.
    EpochSeconds,
    /// Milliseconds since the Unix epoch.
    EpochMillis,
    /// Microseconds since the Unix epoch.
    EpochMicros,
}

/// Whether the day or the month is written first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Order {
    /// Year first, so no confusion is possible.
    YearFirst,
    /// Day before month.
    DayFirst,
    /// Month before day.
    MonthFirst,
    /// The month is a name, so no confusion is possible.
    Named,
    /// Not a written date at all.
    NotApplicable,
}

/// What the format says about time zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Zone {
    /// No zone information at all.
    Naive,
    /// Explicitly UTC, via a trailing `Z`.
    Utc,
    /// An explicit numeric offset such as `+05:30`.
    Offset,
    /// Epoch values, which are UTC instants by definition.
    EpochUtc,
}

/// A recognised timestamp layout.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Format {
    /// Stable identifier, recorded in saved projects. Never change these.
    pub id: Cow<'static, str>,
    /// The `chrono` pattern, unused for epoch shapes.
    pub pattern: Cow<'static, str>,
    /// A representative value.
    pub example: &'static str,
    /// What that example means, in words.
    pub meaning: &'static str,
    pub shape: Shape,
    pub order: Order,
    pub zone: Zone,
}

use Order::{DayFirst, MonthFirst, Named, NotApplicable, YearFirst};
use Shape::{DateOnly, DateTime as DT, EpochMicros, EpochMillis, EpochSeconds};
use Zone::{EpochUtc, Naive, Offset, Utc};

/// Every built-in format, most specific first.
///
/// Order matters: detection attributes a value to the first format that reads
/// it, so more specific spellings must precede looser ones.
pub fn all() -> &'static [Format] {
    static ALL: std::sync::OnceLock<Vec<Format>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        // (id, pattern, example, meaning, shape, order, zone)
        //
        // No two entries may fully parse the same common value, or a clean
        // column would look ambiguous. `%.f` is optional in chrono, so the
        // fractional patterns already cover their whole-second forms and no
        // separate entry for those exists.
        //
        // The day-first and month-first entries deliberately share the example
        // `03/02/2026`, because showing one string with two meanings is what
        // makes the ambiguity legible to the user.
        const T: &[(&str, &str, &str, &str, Shape, Order, Zone)] = &[
            // ISO 8601 and year-first
            (
                "iso_t_offset_frac",
                "%Y-%m-%dT%H:%M:%S%.f%:z",
                "2026-03-02T14:32:18+05:30",
                "2 March 2026, 14:32:18 at +05:30",
                DT,
                YearFirst,
                Offset,
            ),
            (
                "iso_t_zulu",
                "%Y-%m-%dT%H:%M:%S%.fZ",
                "2026-03-02T14:32:18Z",
                "2 March 2026, 14:32:18 UTC",
                DT,
                YearFirst,
                Utc,
            ),
            (
                "iso_t_seconds",
                "%Y-%m-%dT%H:%M:%S%.f",
                "2026-03-02T14:32:18",
                "2 March 2026, 14:32:18",
                DT,
                YearFirst,
                Naive,
            ),
            (
                "iso_t_minutes",
                "%Y-%m-%dT%H:%M",
                "2026-03-02T14:32",
                "2 March 2026, 14:32",
                DT,
                YearFirst,
                Naive,
            ),
            (
                "iso_space_seconds",
                "%Y-%m-%d %H:%M:%S%.f",
                "2026-03-02 14:32:18",
                "2 March 2026, 14:32:18",
                DT,
                YearFirst,
                Naive,
            ),
            (
                "iso_space_minutes",
                "%Y-%m-%d %H:%M",
                "2026-03-02 14:32",
                "2 March 2026, 14:32",
                DT,
                YearFirst,
                Naive,
            ),
            (
                "iso_space_ampm",
                "%Y-%m-%d %I:%M:%S %p",
                "2026-03-02 02:32:18 PM",
                "2 March 2026, 14:32:18",
                DT,
                YearFirst,
                Naive,
            ),
            (
                "iso_date",
                "%Y-%m-%d",
                "2026-03-02",
                "2 March 2026",
                DateOnly,
                YearFirst,
                Naive,
            ),
            (
                "slash_year_first_seconds",
                "%Y/%m/%d %H:%M:%S",
                "2026/03/02 14:32:18",
                "2 March 2026, 14:32:18",
                DT,
                YearFirst,
                Naive,
            ),
            (
                "slash_year_first_minutes",
                "%Y/%m/%d %H:%M",
                "2026/03/02 14:32",
                "2 March 2026, 14:32",
                DT,
                YearFirst,
                Naive,
            ),
            (
                "slash_year_first_date",
                "%Y/%m/%d",
                "2026/03/02",
                "2 March 2026",
                DateOnly,
                YearFirst,
                Naive,
            ),
            (
                "compact_datetime",
                "%Y%m%d%H%M%S",
                "20260302143218",
                "2 March 2026, 14:32:18",
                DT,
                YearFirst,
                Naive,
            ),
            (
                "compact_date",
                "%Y%m%d",
                "20260302",
                "2 March 2026",
                DateOnly,
                YearFirst,
                Naive,
            ),
            // Month names: unambiguous by construction. Common in SAP and Oracle exports.
            (
                "dash_month_name_seconds",
                "%d-%b-%Y %H:%M:%S",
                "02-Mar-2026 14:32:18",
                "2 March 2026, 14:32:18",
                DT,
                Named,
                Naive,
            ),
            (
                "dash_month_name_date",
                "%d-%b-%Y",
                "02-Mar-2026",
                "2 March 2026",
                DateOnly,
                Named,
                Naive,
            ),
            (
                "space_month_name_seconds",
                "%d %b %Y %H:%M:%S",
                "02 Mar 2026 14:32:18",
                "2 March 2026, 14:32:18",
                DT,
                Named,
                Naive,
            ),
            (
                "space_month_name_date",
                "%d %b %Y",
                "02 Mar 2026",
                "2 March 2026",
                DateOnly,
                Named,
                Naive,
            ),
            (
                "month_name_first_seconds",
                "%b %d, %Y %H:%M:%S",
                "Mar 02, 2026 14:32:18",
                "2 March 2026, 14:32:18",
                DT,
                Named,
                Naive,
            ),
            (
                "month_name_first_date",
                "%b %d, %Y",
                "Mar 02, 2026",
                "2 March 2026",
                DateOnly,
                Named,
                Naive,
            ),
            (
                "full_month_name_date",
                "%d %B %Y",
                "02 March 2026",
                "2 March 2026",
                DateOnly,
                Named,
                Naive,
            ),
            (
                "full_month_name_first_date",
                "%B %d, %Y",
                "March 02, 2026",
                "2 March 2026",
                DateOnly,
                Named,
                Naive,
            ),
            // Day first
            (
                "slash_day_first_ampm",
                "%d/%m/%Y %I:%M:%S %p",
                "03/02/2026 02:32:18 PM",
                "3 February 2026, 14:32:18",
                DT,
                DayFirst,
                Naive,
            ),
            (
                "slash_day_first_ampm_minutes",
                "%d/%m/%Y %I:%M %p",
                "03/02/2026 02:32 PM",
                "3 February 2026, 14:32",
                DT,
                DayFirst,
                Naive,
            ),
            (
                "slash_day_first_seconds",
                "%d/%m/%Y %H:%M:%S",
                "03/02/2026 14:32:18",
                "3 February 2026, 14:32:18",
                DT,
                DayFirst,
                Naive,
            ),
            (
                "slash_day_first_minutes",
                "%d/%m/%Y %H:%M",
                "03/02/2026 14:32",
                "3 February 2026, 14:32",
                DT,
                DayFirst,
                Naive,
            ),
            (
                "slash_day_first_date",
                "%d/%m/%Y",
                "03/02/2026",
                "3 February 2026",
                DateOnly,
                DayFirst,
                Naive,
            ),
            (
                "dot_day_first_seconds",
                "%d.%m.%Y %H:%M:%S",
                "03.02.2026 14:32:18",
                "3 February 2026, 14:32:18",
                DT,
                DayFirst,
                Naive,
            ),
            (
                "dot_day_first_minutes",
                "%d.%m.%Y %H:%M",
                "03.02.2026 14:32",
                "3 February 2026, 14:32",
                DT,
                DayFirst,
                Naive,
            ),
            (
                "dot_day_first_date",
                "%d.%m.%Y",
                "03.02.2026",
                "3 February 2026",
                DateOnly,
                DayFirst,
                Naive,
            ),
            (
                "dash_day_first_seconds",
                "%d-%m-%Y %H:%M:%S",
                "03-02-2026 14:32:18",
                "3 February 2026, 14:32:18",
                DT,
                DayFirst,
                Naive,
            ),
            (
                "dash_day_first_minutes",
                "%d-%m-%Y %H:%M",
                "03-02-2026 14:32",
                "3 February 2026, 14:32",
                DT,
                DayFirst,
                Naive,
            ),
            (
                "dash_day_first_date",
                "%d-%m-%Y",
                "03-02-2026",
                "3 February 2026",
                DateOnly,
                DayFirst,
                Naive,
            ),
            // Month first — same example strings as above, different meanings
            (
                "slash_month_first_ampm",
                "%m/%d/%Y %I:%M:%S %p",
                "03/02/2026 02:32:18 PM",
                "2 March 2026, 14:32:18",
                DT,
                MonthFirst,
                Naive,
            ),
            (
                "slash_month_first_ampm_minutes",
                "%m/%d/%Y %I:%M %p",
                "03/02/2026 02:32 PM",
                "2 March 2026, 14:32",
                DT,
                MonthFirst,
                Naive,
            ),
            (
                "slash_month_first_seconds",
                "%m/%d/%Y %H:%M:%S",
                "03/02/2026 14:32:18",
                "2 March 2026, 14:32:18",
                DT,
                MonthFirst,
                Naive,
            ),
            (
                "slash_month_first_minutes",
                "%m/%d/%Y %H:%M",
                "03/02/2026 14:32",
                "2 March 2026, 14:32",
                DT,
                MonthFirst,
                Naive,
            ),
            (
                "slash_month_first_date",
                "%m/%d/%Y",
                "03/02/2026",
                "2 March 2026",
                DateOnly,
                MonthFirst,
                Naive,
            ),
            (
                "dot_month_first_seconds",
                "%m.%d.%Y %H:%M:%S",
                "03.02.2026 14:32:18",
                "2 March 2026, 14:32:18",
                DT,
                MonthFirst,
                Naive,
            ),
            (
                "dot_month_first_minutes",
                "%m.%d.%Y %H:%M",
                "03.02.2026 14:32",
                "2 March 2026, 14:32",
                DT,
                MonthFirst,
                Naive,
            ),
            (
                "dot_month_first_date",
                "%m.%d.%Y",
                "03.02.2026",
                "2 March 2026",
                DateOnly,
                MonthFirst,
                Naive,
            ),
            (
                "dash_month_first_seconds",
                "%m-%d-%Y %H:%M:%S",
                "03-02-2026 14:32:18",
                "2 March 2026, 14:32:18",
                DT,
                MonthFirst,
                Naive,
            ),
            (
                "dash_month_first_minutes",
                "%m-%d-%Y %H:%M",
                "03-02-2026 14:32",
                "2 March 2026, 14:32",
                DT,
                MonthFirst,
                Naive,
            ),
            (
                "dash_month_first_date",
                "%m-%d-%Y",
                "03-02-2026",
                "2 March 2026",
                DateOnly,
                MonthFirst,
                Naive,
            ),
            // Unix epoch, distinguished by digit width
            (
                "epoch_micros",
                "",
                "1772461938000000",
                "2 March 2026, 14:32:18 UTC",
                EpochMicros,
                NotApplicable,
                EpochUtc,
            ),
            (
                "epoch_millis",
                "",
                "1772461938000",
                "2 March 2026, 14:32:18 UTC",
                EpochMillis,
                NotApplicable,
                EpochUtc,
            ),
            (
                "epoch_seconds",
                "",
                "1772461938",
                "2 March 2026, 14:32:18 UTC",
                EpochSeconds,
                NotApplicable,
                EpochUtc,
            ),
        ];
        T.iter()
            .map(
                |&(id, pattern, example, meaning, shape, order, zone)| Format {
                    id: Cow::Borrowed(id),
                    pattern: Cow::Borrowed(pattern),
                    example,
                    meaning,
                    shape,
                    order,
                    zone,
                },
            )
            .collect()
    })
}

impl Format {
    /// Builds a format from a user-supplied `chrono` pattern.
    ///
    /// The catalogue cannot be exhaustive, so this is the escape hatch for
    /// anything unusual. Day/month order is taken from the pattern itself.
    pub fn custom(pattern: impl Into<String>) -> Format {
        let pattern = pattern.into();
        let order = match (pattern.find("%d"), pattern.find("%m"), pattern.find("%Y")) {
            _ if pattern.contains("%b") || pattern.contains("%B") => Named,
            (Some(d), Some(m), Some(y)) if y < d && y < m => YearFirst,
            (Some(d), Some(m), _) if d < m => DayFirst,
            (Some(d), Some(m), _) if m < d => MonthFirst,
            _ => NotApplicable,
        };
        let shape = if pattern.contains("%H") || pattern.contains("%I") {
            DT
        } else {
            DateOnly
        };
        let zone = if pattern.contains("%:z") || pattern.contains("%z") {
            Offset
        } else {
            Naive
        };
        Format {
            id: Cow::Owned(format!("custom:{pattern}")),
            pattern: Cow::Owned(pattern),
            example: "(custom)",
            meaning: "(custom)",
            shape,
            order,
            zone,
        }
    }

    /// Whether values in this format state their own zone.
    pub fn has_explicit_zone(&self) -> bool {
        matches!(self.zone, Utc | Offset | EpochUtc)
    }

    /// Whether the day is written before the month.
    pub fn is_day_first(&self) -> bool {
        self.order == DayFirst
    }

    /// Whether the month is written before the day.
    pub fn is_month_first(&self) -> bool {
        self.order == MonthFirst
    }

    /// The counterpart format with day and month exchanged, if one exists.
    ///
    /// Derived by swapping `%d` and `%m` in the pattern and looking the result
    /// up, so adding a format automatically gains its counterpart.
    pub fn swapped_day_month(&self) -> Option<&'static Format> {
        if !matches!(self.order, DayFirst | MonthFirst) {
            return None;
        }
        let swapped = self.pattern.replace("%d", "\u{0}").replace("%m", "%d");
        let swapped = swapped.replace('\u{0}', "%m");
        all().iter().find(|f| f.pattern == swapped)
    }

    /// Parses one value.
    pub fn parse(&self, value: &str) -> Option<NaiveDateTime> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        match self.shape {
            EpochSeconds | EpochMillis | EpochMicros => self.parse_epoch(value),
            DateOnly => NaiveDate::parse_from_str(value, &self.pattern)
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0)),
            DT if self.zone == Offset => DateTime::parse_from_str(value, &self.pattern)
                .ok()
                .map(|d| d.naive_utc()),
            DT => NaiveDateTime::parse_from_str(value, &self.pattern).ok(),
        }
    }

    /// Parses a value without applying its numeric UTC offset.
    ///
    /// This is the meaning of the `as_written` timezone policy: `09:00+02:00`
    /// and a naive `09:00` are compared as the same wall-clock time. Epoch
    /// values remain UTC instants because they have no wall-clock spelling.
    pub fn parse_wall_time(&self, value: &str) -> Option<NaiveDateTime> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        match self.shape {
            EpochSeconds | EpochMillis | EpochMicros => self.parse_epoch(value),
            DateOnly => NaiveDate::parse_from_str(value, &self.pattern)
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0)),
            DT if self.zone == Offset => DateTime::parse_from_str(value, &self.pattern)
                .ok()
                .map(|d| d.naive_local()),
            DT => NaiveDateTime::parse_from_str(value, &self.pattern).ok(),
        }
    }

    /// Parses an epoch value.
    ///
    /// Digit count distinguishes the units: a bare integer is otherwise
    /// ambiguous, since `1772461938` is seconds in 2026 but milliseconds in
    /// 1970. Requiring the expected width keeps the reading unambiguous for
    /// any plausible operational date.
    fn parse_epoch(&self, value: &str) -> Option<NaiveDateTime> {
        let digits = value.strip_prefix('-').unwrap_or(value);
        if !digits.chars().all(|c| c.is_ascii_digit()) || digits.is_empty() {
            return None;
        }
        let n: i64 = value.parse().ok()?;
        let (expected_len, secs, nanos) = match self.shape {
            EpochSeconds => (10, n, 0),
            EpochMillis => (
                13,
                n.div_euclid(1_000),
                (n.rem_euclid(1_000) * 1_000_000) as u32,
            ),
            EpochMicros => (
                16,
                n.div_euclid(1_000_000),
                (n.rem_euclid(1_000_000) * 1_000) as u32,
            ),
            _ => return None,
        };
        if digits.len() != expected_len {
            return None;
        }
        DateTime::from_timestamp(secs, nanos).map(|d| d.naive_utc())
    }
}

/// What detection concluded about a column of timestamp values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    /// Formats that read every non-blank sampled value, in preference order.
    pub candidates: Vec<Format>,
    /// Sampled values no format could read.
    pub unparseable: Vec<String>,
    /// Blank or whitespace-only values sampled.
    pub blank_count: usize,
    /// Non-blank values sampled.
    pub sampled: usize,
    /// An unsettled day-versus-month reading, if the data leaves one.
    pub ambiguity: Option<Ambiguity>,
    /// Formats each explaining part of the sample, populated **only** when no
    /// single format explains all of it.
    pub mixed: Vec<Format>,
}

/// An unresolved day-versus-month reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguity {
    pub day_first: Format,
    pub month_first: Format,
    /// A sampled value that reads validly either way.
    pub example: String,
    pub day_first_meaning: String,
    pub month_first_meaning: String,
}

impl Detection {
    /// `true` when exactly one format explains the whole sample.
    pub fn is_conclusive(&self) -> bool {
        self.candidates.len() == 1 && self.unparseable.is_empty() && self.mixed.is_empty()
    }

    /// The single format, if detection was conclusive.
    pub fn resolved(&self) -> Option<&Format> {
        self.is_conclusive().then(|| &self.candidates[0])
    }

    /// `true` when values declaring a zone are mixed with values that do not.
    pub fn mixes_zoned_and_naive(&self) -> bool {
        let mut zoned = false;
        let mut naive = false;
        for f in self.candidates.iter().chain(self.mixed.iter()) {
            if f.has_explicit_zone() {
                zoned = true;
            } else {
                naive = true;
            }
        }
        zoned && naive
    }
}

/// Detects the timestamp format of a sample of values.
pub fn detect(values: &[String]) -> Detection {
    let (blank, present): (Vec<&String>, Vec<&String>) =
        values.iter().partition(|v| v.trim().is_empty());

    let mut full = Vec::new();
    let mut partial = Vec::new();
    for format in all() {
        let parsed = present.iter().filter(|v| format.parse(v).is_some()).count();
        if parsed == present.len() && !present.is_empty() {
            full.push(format.clone());
        } else if parsed > 0 {
            partial.push(format.clone());
        }
    }

    let unparseable: Vec<String> = present
        .iter()
        .filter(|v| !all().iter().any(|f| f.parse(v).is_some()))
        .map(|v| (*v).clone())
        .take(10)
        .collect();

    // Mixed-layout columns still need a day/month decision. Looking only at
    // formats that parse the *whole* sample silently misses `05/01/2026` when
    // the same column also contains ISO values.
    let ambiguity_formats = if full.is_empty() {
        partial.clone()
    } else {
        full.clone()
    };
    let ambiguity = find_ambiguity(&ambiguity_formats, &present);
    let mixed = if full.is_empty() { partial } else { Vec::new() };

    Detection {
        candidates: full,
        unparseable,
        blank_count: blank.len(),
        sampled: present.len(),
        ambiguity,
        mixed,
    }
}

/// Reports an unsettled day/month reading when both orders read the whole
/// sample. A first number above 12 rules the month-first reading out, so the
/// data has answered the question and the user is not asked.
fn find_ambiguity(formats: &[Format], present: &[&String]) -> Option<Ambiguity> {
    for day_first in formats.iter().filter(|f| f.is_day_first()) {
        let Some(counterpart) = day_first.swapped_day_month() else {
            continue;
        };
        let Some(month_first) = formats.iter().find(|f| f.id == counterpart.id) else {
            continue;
        };

        // An ambiguity exists only when a value in the sample has two valid,
        // different readings under matching layouts. A different row with a
        // number above 12 can therefore settle the order for a uniform column.
        let Some(example) =
            present
                .iter()
                .find(|v| match (day_first.parse(v), month_first.parse(v)) {
                    (Some(a), Some(b)) => a != b,
                    _ => false,
                })
        else {
            continue;
        };

        return Some(Ambiguity {
            example: (*example).clone(),
            day_first_meaning: describe(day_first.parse(example)),
            month_first_meaning: describe(month_first.parse(example)),
            day_first: day_first.clone(),
            month_first: month_first.clone(),
        });
    }
    None
}

fn describe(dt: Option<NaiveDateTime>) -> String {
    dt.map(|d| d.format("%-d %B %Y").to_string())
        .unwrap_or_else(|| "unparseable".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    fn by_id(id: &str) -> &'static Format {
        all().iter().find(|f| f.id == id).expect("format exists")
    }

    #[test]
    fn detects_unambiguous_iso() {
        let d = detect(&vals(&["2026-03-02 09:00:00", "2026-03-02 11:30:00"]));
        assert_eq!(
            d.resolved().map(|f| f.id.as_ref()),
            Some("iso_space_seconds")
        );
    }

    /// The central case: refuse to choose when both readings are valid.
    #[test]
    fn refuses_to_guess_ambiguous_day_month() {
        let d = detect(&vals(&["05/01/2026 11:00", "06/01/2026 10:15"]));
        assert!(d.resolved().is_none());
        let a = d.ambiguity.expect("ambiguity must be reported");
        assert_ne!(a.day_first_meaning, a.month_first_meaning);
    }

    #[test]
    fn data_can_settle_day_month_by_itself() {
        let d = detect(&vals(&["13/01/2026 11:00", "05/01/2026 10:15"]));
        assert!(d.ambiguity.is_none(), "13 cannot be a month");
        assert_eq!(
            d.resolved().map(|f| f.id.as_ref()),
            Some("slash_day_first_minutes")
        );
    }

    #[test]
    fn month_first_when_second_number_exceeds_twelve() {
        let d = detect(&vals(&["01/13/2026 11:00", "01/05/2026 10:15"]));
        assert!(d.ambiguity.is_none());
        assert_eq!(
            d.resolved().map(|f| f.id.as_ref()),
            Some("slash_month_first_minutes")
        );
    }

    #[test]
    fn reads_twelve_hour_times_without_seconds() {
        // As a US business user's Excel export writes them.
        let d = detect(&vals(&["3/5/2026 9:41 AM", "3/27/2026 8:48 PM"]));
        assert!(d.ambiguity.is_none());
        let f = d.resolved().unwrap();
        assert_eq!(f.id, "slash_month_first_ampm_minutes");
        assert_eq!(
            f.parse("3/27/2026 8:48 PM").unwrap().to_string(),
            "2026-03-27 20:48:00"
        );
    }

    #[test]
    fn reads_sap_and_oracle_month_name_form() {
        let d = detect(&vals(&["02-Mar-2026 14:32:18", "15-Apr-2026 09:00:00"]));
        assert_eq!(
            d.resolved().map(|f| f.id.as_ref()),
            Some("dash_month_name_seconds")
        );
        assert!(d.ambiguity.is_none(), "month names are never ambiguous");
    }

    #[test]
    fn reads_twelve_hour_clock_with_meridiem() {
        let f = by_id("slash_month_first_ampm");
        let dt = f.parse("03/02/2026 02:32:18 PM").unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-03-02 14:32:18"
        );
        let am = f.parse("03/02/2026 02:32:18 AM").unwrap();
        assert_eq!(am.format("%H").to_string(), "02");
    }

    #[test]
    fn reads_explicit_numeric_offsets_as_utc_instants() {
        let f = by_id("iso_t_offset_frac");
        let dt = f.parse("2026-03-02T14:32:18+05:30").unwrap();
        // 14:32:18 at +05:30 is 09:02:18 UTC.
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-03-02 09:02:18"
        );
    }

    #[test]
    fn can_preserve_the_wall_clock_spelling_of_an_offset_timestamp() {
        let f = by_id("iso_t_offset_frac");
        let dt = f.parse_wall_time("2026-03-02T14:32:18+05:30").unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-03-02 14:32:18"
        );
    }

    #[test]
    fn reads_compact_forms() {
        assert!(by_id("compact_datetime").parse("20260302143218").is_some());
        assert!(by_id("compact_date").parse("20260302").is_some());
    }

    #[test]
    fn reads_epoch_seconds_millis_and_micros() {
        let s = by_id("epoch_seconds").parse("1772461938").unwrap();
        let ms = by_id("epoch_millis").parse("1772461938000").unwrap();
        let us = by_id("epoch_micros").parse("1772461938000000").unwrap();
        assert_eq!(s, ms);
        assert_eq!(s, us);
    }

    /// A bare integer is ambiguous between units, so each epoch format accepts
    /// only its own digit width.
    #[test]
    fn epoch_units_are_distinguished_by_digit_count() {
        assert!(by_id("epoch_millis").parse("1772461938").is_none());
        assert!(by_id("epoch_seconds").parse("1772461938000").is_none());
        assert!(by_id("epoch_micros").parse("1772461938000").is_none());
    }

    #[test]
    fn epoch_rejects_non_numeric() {
        assert!(by_id("epoch_seconds").parse("2026-03-02").is_none());
        assert!(by_id("epoch_seconds").parse("").is_none());
    }

    #[test]
    fn reports_mixed_formats_in_one_column() {
        let d = detect(&vals(&[
            "2026-01-05 09:00:00",
            "05/01/2026 11:00",
            "2026-01-05T14:00:00Z",
        ]));
        assert!(!d.is_conclusive());
        assert!(!d.mixed.is_empty());
        assert!(
            d.ambiguity.is_some(),
            "an ambiguous slash date must not become safe merely because ISO rows share its column"
        );
    }

    #[test]
    fn detects_mixing_of_zoned_and_naive_values() {
        let d = detect(&vals(&["2026-01-05 09:00:00", "2026-01-05T14:00:00Z"]));
        assert!(d.mixes_zoned_and_naive());
    }

    #[test]
    fn counts_blanks_without_treating_them_as_failures() {
        let d = detect(&vals(&[
            "2026-03-02 09:00:00",
            "",
            "   ",
            "2026-03-02 10:00:00",
        ]));
        assert_eq!(d.blank_count, 2);
        assert_eq!(d.sampled, 2);
        assert!(d.unparseable.is_empty());
    }

    #[test]
    fn reports_unparseable_values() {
        let d = detect(&vals(&["2026-03-02 09:00:00", "not a date", "yesterday"]));
        assert_eq!(d.unparseable.len(), 2);
    }

    #[test]
    fn accepts_a_user_supplied_pattern() {
        let f = Format::custom("%d|%m|%Y %H:%M");
        assert!(f.is_day_first());
        let dt = f.parse("02|03|2026 14:32").unwrap();
        assert_eq!(dt.format("%Y-%m-%d %H:%M").to_string(), "2026-03-02 14:32");
    }

    /// Adding a format should gain its day/month counterpart automatically.
    #[test]
    fn day_month_counterparts_are_derived_from_the_pattern() {
        assert_eq!(
            by_id("slash_day_first_seconds")
                .swapped_day_month()
                .map(|f| f.id.as_ref()),
            Some("slash_month_first_seconds")
        );
        assert_eq!(
            by_id("slash_month_first_date")
                .swapped_day_month()
                .map(|f| f.id.as_ref()),
            Some("slash_day_first_date")
        );
        assert!(by_id("iso_space_seconds").swapped_day_month().is_none());
        assert!(by_id("dash_month_name_date").swapped_day_month().is_none());
    }

    #[test]
    fn every_format_parses_its_own_example() {
        for f in all() {
            assert!(
                f.parse(f.example).is_some(),
                "{} cannot parse its own example {:?}",
                f.id,
                f.example
            );
        }
    }

    /// Each example must actually mean what the catalogue claims, or the
    /// choice shown to the user is a lie.
    #[test]
    fn every_example_means_what_it_claims() {
        for f in all() {
            let parsed = f.parse(f.example).expect("example parses");
            let long = parsed.format("%-d %B %Y").to_string();
            assert!(
                f.meaning.starts_with(&long),
                "{}: example {:?} parses to {long} but claims {:?}",
                f.id,
                f.example,
                f.meaning
            );
        }
    }

    #[test]
    fn format_ids_are_unique() {
        let ids: std::collections::BTreeSet<&str> = all().iter().map(|f| f.id.as_ref()).collect();
        assert_eq!(ids.len(), all().len());
    }

    #[test]
    fn catalogue_covers_the_expected_breadth() {
        assert!(all().len() >= 40, "only {} formats", all().len());
        for order in [YearFirst, DayFirst, MonthFirst, Named] {
            assert!(all().iter().any(|f| f.order == order), "missing {order:?}");
        }
        for shape in [DT, DateOnly, EpochSeconds, EpochMillis, EpochMicros] {
            assert!(all().iter().any(|f| f.shape == shape), "missing {shape:?}");
        }
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        assert!(
            by_id("iso_space_seconds")
                .parse("  2026-03-02 09:00:00  ")
                .is_some()
        );
    }

    #[test]
    fn empty_sample_resolves_nothing() {
        let d = detect(&[]);
        assert!(d.resolved().is_none());
        assert_eq!(d.sampled, 0);
    }
}
