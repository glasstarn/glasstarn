//! Turns simulated traces into timestamped event logs.
//!
//! A [`Model`] produces activity *sequences*. A real event log also has case
//! identifiers, timestamps, and the waiting time between steps — which is
//! where bottlenecks live. This module adds those, deterministically.

use crate::model::Model;
use crate::rng::Rng;
use chrono::{Duration, NaiveDateTime};
use std::collections::HashMap;

/// One generated event, in canonical long format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub case_id: String,
    pub activity: String,
    pub timestamp: NaiveDateTime,
}

/// How long an activity waits before starting, in seconds.
///
/// Waiting time — the gap between the previous activity finishing and this one
/// starting — is where process loss usually is, so it is what the generator
/// models. Values are drawn uniformly from `min..=max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wait {
    pub min_secs: i64,
    pub max_secs: i64,
}

impl Wait {
    /// A fixed waiting time.
    pub fn fixed(secs: i64) -> Self {
        Self {
            min_secs: secs,
            max_secs: secs,
        }
    }

    /// A waiting time drawn from a range.
    pub fn range(min_secs: i64, max_secs: i64) -> Self {
        assert!(min_secs <= max_secs, "wait range is inverted");
        Self { min_secs, max_secs }
    }

    fn draw(&self, rng: &mut Rng) -> Duration {
        let span = (self.max_secs - self.min_secs).unsigned_abs() + 1;
        Duration::seconds(self.min_secs + rng.below(span) as i64)
    }
}

/// Builds an event log from a model.
#[derive(Debug, Clone)]
pub struct LogBuilder {
    model: Model,
    seed: u64,
    case_prefix: String,
    first_case_start: NaiveDateTime,
    case_interval: Wait,
    default_wait: Wait,
    waits: HashMap<String, Wait>,
}

impl LogBuilder {
    /// Creates a builder for `model` starting at `first_case_start`.
    pub fn new(model: Model, first_case_start: NaiveDateTime) -> Self {
        Self {
            model,
            seed: 1,
            case_prefix: "CASE-".to_owned(),
            first_case_start,
            case_interval: Wait::range(600, 7_200),
            default_wait: Wait::range(300, 3_600),
            waits: HashMap::new(),
        }
    }

    /// Sets the generation seed. The same seed always produces the same log.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Sets the case identifier prefix, e.g. `"ORDER-"`.
    pub fn case_prefix(mut self, prefix: &str) -> Self {
        self.case_prefix = prefix.to_owned();
        self
    }

    /// Sets how long to wait between the start of one case and the next.
    pub fn case_interval(mut self, wait: Wait) -> Self {
        self.case_interval = wait;
        self
    }

    /// Sets the waiting time used for activities with no specific setting.
    pub fn default_wait(mut self, wait: Wait) -> Self {
        self.default_wait = wait;
        self
    }

    /// Sets the waiting time before a named activity.
    ///
    /// This is how a deliberate bottleneck is planted: give one activity a
    /// much larger wait than the rest, and the analysis must find it.
    pub fn wait_for(mut self, activity: &str, wait: Wait) -> Self {
        self.waits.insert(activity.to_owned(), wait);
        self
    }

    /// Generates `case_count` cases.
    pub fn build(&self, case_count: usize) -> Vec<Event> {
        let mut rng = Rng::new(self.seed);
        let mut events = Vec::new();
        let mut case_start = self.first_case_start;

        for i in 0..case_count {
            let case_id = format!("{}{:05}", self.case_prefix, i + 1);
            let trace = self.model.simulate(&mut rng);

            let mut at = case_start;
            for activity in trace {
                let wait = self.waits.get(&activity).unwrap_or(&self.default_wait);
                at += wait.draw(&mut rng);
                events.push(Event {
                    case_id: case_id.clone(),
                    activity,
                    timestamp: at,
                });
            }

            case_start += self.case_interval.draw(&mut rng);
        }

        events
    }

    /// Generates `case_count` cases and renders them as CSV.
    pub fn build_csv(&self, case_count: usize) -> String {
        to_csv(&self.build(case_count))
    }
}

/// Renders events as CSV with a `case_id,activity,timestamp` header.
///
/// Timestamps use `YYYY-MM-DD HH:MM:SS`, the format most operational exports
/// produce.
pub fn to_csv(events: &[Event]) -> String {
    let mut out = String::from("case_id,activity,timestamp\n");
    for e in events {
        out.push_str(&format!(
            "{},{},{}\n",
            csv_field(&e.case_id),
            csv_field(&e.activity),
            e.timestamp.format("%Y-%m-%d %H:%M:%S")
        ));
    }
    out
}

/// Quotes a field if it contains a comma, quote, or newline.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn start() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 3, 2)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
    }

    fn simple() -> LogBuilder {
        LogBuilder::new(Model::seq(&["Receive", "Check", "Ship"]), start()).seed(42)
    }

    #[test]
    fn generates_expected_event_count() {
        // 10 cases through a 3-activity sequence is exactly 30 events.
        assert_eq!(simple().build(10).len(), 30);
    }

    #[test]
    fn generates_expected_case_count() {
        let events = simple().build(10);
        let cases: std::collections::HashSet<&str> =
            events.iter().map(|e| e.case_id.as_str()).collect();
        assert_eq!(cases.len(), 10);
    }

    #[test]
    fn timestamps_increase_within_a_case() {
        let events = simple().build(20);
        let mut by_case: HashMap<&str, Vec<NaiveDateTime>> = HashMap::new();
        for e in &events {
            by_case.entry(&e.case_id).or_default().push(e.timestamp);
        }
        for (case, times) in by_case {
            assert!(
                times.windows(2).all(|w| w[0] < w[1]),
                "case {case} has non-increasing timestamps: {times:?}"
            );
        }
    }

    #[test]
    fn events_are_emitted_grouped_by_case_in_order() {
        let events = simple().build(5);
        let ids: Vec<&str> = events.iter().map(|e| e.case_id.as_str()).collect();
        let mut expected = Vec::new();
        for i in 1..=5 {
            let id = format!("CASE-{i:05}");
            expected.extend(std::iter::repeat_n(id, 3));
        }
        assert_eq!(ids, expected);
    }

    #[test]
    fn is_deterministic_for_a_seed() {
        assert_eq!(simple().build(50), simple().build(50));
    }

    #[test]
    fn different_seeds_produce_different_logs() {
        let a = simple().build(50);
        let b = simple().seed(999).build(50);
        assert_ne!(a, b);
    }

    /// A planted bottleneck must actually dominate. This is the property that
    /// makes generated logs usable for testing bottleneck detection.
    #[test]
    fn planted_bottleneck_dominates_waiting_time() {
        let events = LogBuilder::new(Model::seq(&["Receive", "Approve", "Ship"]), start())
            .seed(7)
            .default_wait(Wait::range(60, 120))
            .wait_for("Approve", Wait::range(86_400, 172_800))
            .build(100);

        let mut total: HashMap<&str, i64> = HashMap::new();
        for pair in events.chunks(3) {
            total
                .entry("Approve")
                .and_modify(|t| *t += (pair[1].timestamp - pair[0].timestamp).num_seconds())
                .or_insert((pair[1].timestamp - pair[0].timestamp).num_seconds());
            total
                .entry("Ship")
                .and_modify(|t| *t += (pair[2].timestamp - pair[1].timestamp).num_seconds())
                .or_insert((pair[2].timestamp - pair[1].timestamp).num_seconds());
        }
        assert!(
            total["Approve"] > total["Ship"] * 100,
            "planted bottleneck did not dominate: {total:?}"
        );
    }

    #[test]
    fn csv_has_header_and_one_line_per_event() {
        let csv = simple().build_csv(4);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "case_id,activity,timestamp");
        assert_eq!(lines.len(), 1 + 12);
    }

    #[test]
    fn csv_timestamp_format_is_stable() {
        let csv = simple().build_csv(1);
        let first = csv.lines().nth(1).unwrap();
        let ts = first.split(',').nth(2).unwrap();
        assert_eq!(ts.len(), 19, "expected YYYY-MM-DD HH:MM:SS, got {ts}");
        assert!(ts.starts_with("2026-03-02"), "unexpected date: {ts}");
    }

    #[test]
    fn csv_quotes_fields_containing_commas() {
        let events = [Event {
            case_id: "C-1".to_owned(),
            activity: "Approve, then Sign".to_owned(),
            timestamp: start(),
        }];
        let csv = to_csv(&events);
        assert!(csv.contains("\"Approve, then Sign\""), "got: {csv}");
    }

    #[test]
    fn csv_escapes_embedded_quotes() {
        let events = [Event {
            case_id: "C-1".to_owned(),
            activity: "Mark \"urgent\"".to_owned(),
            timestamp: start(),
        }];
        let csv = to_csv(&events);
        assert!(csv.contains("\"Mark \"\"urgent\"\"\""), "got: {csv}");
    }
}
