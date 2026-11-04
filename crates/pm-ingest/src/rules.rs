//! User-defined preparation rules.
//!
//! # Decisions, not cleaning
//!
//! These rules never modify the source file. They are recorded with the
//! project and applied at analysis time, so every result stays reproducible
//! and an analyst can always show exactly what was assumed. Editing the data
//! instead would leave a second file to reconcile and no audit trail.
//!
//! # Why explicit, and never fuzzy
//!
//! Automatic merging handles only case and whitespace, because those are
//! provably cosmetic. Anything beyond that is stated by the user:
//!
//! ```text
//! Approve, Approved, APPROVAL_GRANTED  →  Approve
//! STAT_03                              →  Credit Check
//! System Auto-Update                   →  (excluded)
//! ```
//!
//! Similarity scoring is deliberately absent. `Approve` and `Approved` look
//! obviously mergeable; `Cancel` and `Cancelled` may be genuinely distinct
//! states in a real workflow, and no similarity measure can tell the
//! difference. Guessing would silently destroy a real distinction — the exact
//! failure this product exists to avoid.

use pm_core::automation::{ResourceClassification, ResourceKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Why a rules file could not be used.
#[derive(Debug, thiserror::Error)]
pub enum RulesError {
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a valid rules file: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

/// How the user wants the log prepared before analysis.
///
/// Serializable so it can be written to a file, reviewed, version-controlled,
/// and handed between a person and an AI assistant. An assistant reading the
/// activity inventory can *propose* a rule set; the user reviews it; the engine
/// applies it exactly. Proposal is judgment, application is deterministic, and
/// the file is the seam between them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rules {
    /// Raw activity label to the label it should become.
    pub rename: BTreeMap<String, String>,
    /// Activities to drop entirely, matched **after** renaming.
    pub exclude_activities: BTreeSet<String>,
    /// Case identifiers to drop entirely.
    pub exclude_cases: BTreeSet<String>,
    /// Drop cases with fewer than this many events. Zero disables the rule.
    pub min_events_per_case: usize,
    /// Drop cases flagged incomplete because data was missing.
    pub drop_incomplete_cases: bool,
    /// Whether each resource is a person or a machine.
    ///
    /// Recorded here rather than as a separate mechanism because it has the
    /// same lifecycle as everything else on this type: an assistant proposes
    /// it, the user confirms it, the engine applies it exactly, and the
    /// project remembers it. To a user it is simply another statement of how
    /// their data should be read.
    ///
    /// Resources left out are treated as human, which overstates manual work
    /// rather than hiding it.
    pub resource_kinds: BTreeMap<String, ResourceKind>,
    /// The period the source was extracted for. Labels cases against it and
    /// changes no event: see [`pm_core::window`].
    pub observation_window: Option<pm_core::ObservationWindow>,
}

impl Rules {
    /// Rules that change nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Maps `from` onto `to`.
    pub fn rename(mut self, from: &str, to: &str) -> Self {
        self.rename.insert(from.to_owned(), to.to_owned());
        self
    }

    /// Maps several spellings onto one canonical label.
    pub fn merge(mut self, canonical: &str, spellings: &[&str]) -> Self {
        for s in spellings {
            self.rename.insert((*s).to_owned(), canonical.to_owned());
        }
        self
    }

    /// Drops an activity from the analysis.
    pub fn exclude_activity(mut self, activity: &str) -> Self {
        self.exclude_activities.insert(activity.to_owned());
        self
    }

    /// Drops a case from the analysis.
    pub fn exclude_case(mut self, case: &str) -> Self {
        self.exclude_cases.insert(case.to_owned());
        self
    }

    /// Requires at least `n` events for a case to be included.
    pub fn min_events_per_case(mut self, n: usize) -> Self {
        self.min_events_per_case = n;
        self
    }

    /// Drops cases flagged incomplete.
    pub fn drop_incomplete_cases(mut self, drop: bool) -> Self {
        self.drop_incomplete_cases = drop;
        self
    }

    /// Records what a resource is.
    pub fn classify_resource(mut self, label: &str, kind: ResourceKind) -> Self {
        self.resource_kinds.insert(label.to_owned(), kind);
        self
    }

    /// The resource classification these rules express.
    pub fn classification(&self) -> ResourceClassification {
        self.resource_kinds
            .iter()
            .fold(ResourceClassification::new(), |c, (label, kind)| {
                c.assign(label, *kind)
            })
    }

    /// Reads rules from a JSON file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, RulesError> {
        let path = path.as_ref();
        let display = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| RulesError::Io {
            path: display.clone(),
            source,
        })?;
        Self::from_json(&text).map_err(|source| RulesError::Parse {
            path: display,
            source,
        })
    }

    /// Parses rules from JSON.
    ///
    /// Unknown fields are rejected rather than ignored: a typo in a
    /// hand-written or generated rules file should fail loudly, not silently
    /// do nothing.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// Renders these rules as JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Whether these rules change anything at all.
    pub fn is_empty(&self) -> bool {
        self.rename.is_empty()
            && self.exclude_activities.is_empty()
            && self.exclude_cases.is_empty()
            && self.min_events_per_case == 0
            && !self.drop_incomplete_cases
            && self.resource_kinds.is_empty()
            && self.observation_window.is_none()
    }

    /// Merges automatic case/whitespace folding into these rules.
    ///
    /// Explicit user renames win: an automatic suggestion never overrides
    /// something the user stated deliberately.
    pub fn with_automatic_merges(mut self, merges: BTreeMap<String, String>) -> Self {
        for (from, to) in merges {
            self.rename.entry(from).or_insert(to);
        }
        self
    }

    /// The label an activity should be analysed under, or `None` to drop it.
    ///
    /// Renaming happens first, then exclusion, so a user can fold several
    /// spellings together and exclude the result with one name.
    pub fn resolve_activity<'a>(&'a self, raw: &'a str) -> Option<&'a str> {
        let label = self.rename.get(raw).map_or(raw, String::as_str);
        (!self.exclude_activities.contains(label)).then_some(label)
    }

    /// Whether a case should be dropped outright.
    pub fn excludes_case(&self, case: &str) -> bool {
        self.exclude_cases.contains(case)
    }

    /// Whether a case survives the size and completeness rules.
    pub fn keeps_case(&self, event_count: usize, incomplete: bool) -> bool {
        if event_count == 0 {
            return false;
        }
        if self.drop_incomplete_cases && incomplete {
            return false;
        }
        event_count >= self.min_events_per_case
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rules_change_nothing() {
        let r = Rules::new();
        assert!(r.is_empty());
        assert_eq!(r.resolve_activity("Approve"), Some("Approve"));
        assert!(!r.excludes_case("C1"));
        assert!(r.keeps_case(1, false));
    }

    #[test]
    fn renames_an_activity() {
        let r = Rules::new().rename("STAT_03", "Credit Check");
        assert_eq!(r.resolve_activity("STAT_03"), Some("Credit Check"));
        assert_eq!(r.resolve_activity("STAT_04"), Some("STAT_04"));
    }

    #[test]
    fn merges_several_spellings_onto_one_label() {
        let r = Rules::new().merge("Approve", &["Approved", "APPROVAL_GRANTED", "approve"]);
        for spelling in ["Approved", "APPROVAL_GRANTED", "approve"] {
            assert_eq!(r.resolve_activity(spelling), Some("Approve"));
        }
    }

    #[test]
    fn excludes_an_activity() {
        let r = Rules::new().exclude_activity("System Auto-Update");
        assert_eq!(r.resolve_activity("System Auto-Update"), None);
        assert_eq!(r.resolve_activity("Approve"), Some("Approve"));
    }

    /// Renaming happens before exclusion, so several spellings can be folded
    /// together and dropped with one name.
    #[test]
    fn renames_before_excluding() {
        let r = Rules::new()
            .merge("Noise", &["SYS_PING", "SYS_HEARTBEAT"])
            .exclude_activity("Noise");
        assert_eq!(r.resolve_activity("SYS_PING"), None);
        assert_eq!(r.resolve_activity("SYS_HEARTBEAT"), None);
    }

    #[test]
    fn excludes_a_case() {
        let r = Rules::new().exclude_case("TEST-001");
        assert!(r.excludes_case("TEST-001"));
        assert!(!r.excludes_case("ORDER-001"));
    }

    #[test]
    fn drops_cases_below_the_minimum_size() {
        let r = Rules::new().min_events_per_case(3);
        assert!(!r.keeps_case(2, false));
        assert!(r.keeps_case(3, false));
    }

    /// A case with no events left never survives, whatever the rules say.
    #[test]
    fn never_keeps_an_empty_case() {
        assert!(!Rules::new().keeps_case(0, false));
        assert!(!Rules::new().min_events_per_case(0).keeps_case(0, false));
    }

    #[test]
    fn drops_incomplete_cases_only_when_asked() {
        assert!(Rules::new().keeps_case(5, true));
        assert!(!Rules::new().drop_incomplete_cases(true).keeps_case(5, true));
        assert!(
            Rules::new()
                .drop_incomplete_cases(true)
                .keeps_case(5, false)
        );
    }

    /// An automatic suggestion must never override a deliberate choice.
    #[test]
    fn explicit_renames_win_over_automatic_merges() {
        let automatic = BTreeMap::from([
            ("approve".to_owned(), "approve".to_owned()),
            ("Approve".to_owned(), "approve".to_owned()),
        ]);
        let r = Rules::new()
            .rename("Approve", "Approve Order")
            .with_automatic_merges(automatic);
        assert_eq!(r.resolve_activity("Approve"), Some("Approve Order"));
        assert_eq!(r.resolve_activity("approve"), Some("approve"));
    }

    /// Classification belongs to the same mechanism as every other recorded
    /// interpretation decision.
    #[test]
    fn carries_a_resource_classification() {
        let r = Rules::new()
            .classify_resource("WF-BATCH", ResourceKind::System)
            .classify_resource("alice", ResourceKind::Human);
        assert!(!r.is_empty());
        let c = r.classification();
        assert_eq!(c.kind("WF-BATCH"), ResourceKind::System);
        assert_eq!(c.kind("alice"), ResourceKind::Human);
        assert_eq!(
            c.kind("someone-else"),
            ResourceKind::Human,
            "unclassified overstates manual work rather than hiding it"
        );
    }

    #[test]
    fn round_trips_a_classification_through_json() {
        let r = Rules::new().classify_resource("WF-BATCH", ResourceKind::System);
        let back = Rules::from_json(&r.to_json()).unwrap();
        assert_eq!(back, r);
        assert!(r.to_json().contains("\"system\""), "{}", r.to_json());
    }

    #[test]
    fn round_trips_through_json() {
        let rules = Rules::new()
            .merge("Approve", &["Approved", "APPROVAL_GRANTED"])
            .rename("STAT_03", "Credit Check")
            .exclude_activity("System Auto-Update")
            .exclude_case("TEST-001")
            .min_events_per_case(2)
            .drop_incomplete_cases(true);
        assert_eq!(Rules::from_json(&rules.to_json()).unwrap(), rules);
    }

    #[test]
    fn reads_a_partial_rules_file() {
        let r = Rules::from_json(r#"{"rename": {"STAT_03": "Credit Check"}}"#).unwrap();
        assert_eq!(r.resolve_activity("STAT_03"), Some("Credit Check"));
        assert!(r.exclude_activities.is_empty());
    }

    #[test]
    fn empty_json_object_is_valid_and_changes_nothing() {
        assert!(Rules::from_json("{}").unwrap().is_empty());
    }

    /// A typo in a generated rules file must fail loudly rather than silently
    /// doing nothing.
    #[test]
    fn rejects_an_unknown_field() {
        let err = Rules::from_json(r#"{"renames": {"a": "b"}}"#);
        assert!(err.is_err(), "a misspelled field must be rejected");
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(Rules::from_json("not json").is_err());
    }

    #[test]
    fn missing_rules_file_is_an_error_not_a_panic() {
        assert!(Rules::from_file("/nonexistent/rules.json").is_err());
    }

    #[test]
    fn is_deterministic() {
        let build = || {
            Rules::new()
                .merge("Approve", &["Approved", "approve"])
                .exclude_activity("Noise")
                .min_events_per_case(2)
        };
        assert_eq!(build(), build());
    }
}
