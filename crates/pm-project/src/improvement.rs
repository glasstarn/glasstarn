//! What the company has decided about improving its process: which accounts
//! are systems and which are people, which rules the process should follow,
//! and which steps are candidates for automation.
//!
//! An agent suggests; a person confirms. Every entry says which it is, who
//! said so and when, and every change is written to the project's audit log.
//! The file is `preparation/improvement.json`; its revision goes up by one
//! with every change, so an open results page can see that something changed
//! and refresh.

use pm_core::ResourceKind;
use pm_core::declare::Template;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Whether an entry is the agent's suggestion or the person's decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Suggested,
    Confirmed,
    Rejected,
}

/// Where a step stands for automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    /// Already done by systems.
    AlreadyAutomated,
    /// A decision that always goes the same way: a simple rule will do.
    RuleCandidate,
    /// Frequent, repetitive human work with some variation: an AI agent.
    AgentCandidate,
    /// Rare, or needs judgement.
    StaysHuman,
}

impl Bucket {
    pub fn label(self) -> &'static str {
        match self {
            Self::AlreadyAutomated => "Already automated",
            Self::RuleCandidate => "Rule candidate",
            Self::AgentCandidate => "Agent candidate",
            Self::StaysHuman => "Stays human",
        }
    }
}

/// Who said something, and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Said {
    pub status: Status,
    /// The agent for a suggestion; the person's name or role for a decision.
    pub by: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A rule over activity names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedRule {
    pub template: Template,
    pub a: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<String>,
    #[serde(flatten)]
    pub said: Said,
}

impl SavedRule {
    /// Whether two entries are about the same rule.
    pub fn same_rule(&self, template: Template, a: &str, b: Option<&str>) -> bool {
        self.template == template && self.a == a && self.b.as_deref() == b
    }
}

/// A step's bucket, and how long the step takes a person when known.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedBucket {
    pub bucket: Bucket,
    #[serde(flatten)]
    pub said: Said,
}

/// What a performer is, as decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedPerformer {
    pub kind: ResourceKind,
    #[serde(flatten)]
    pub said: Said,
}

/// Minutes of a person's work one occurrence of a step takes, as the company
/// estimated it; money needs it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedEffort {
    pub minutes: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_hour: Option<f64>,
    #[serde(flatten)]
    pub said: Said,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Improvement {
    /// Goes up by one with every change.
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub performers: BTreeMap<String, SavedPerformer>,
    #[serde(default)]
    pub rules: Vec<SavedRule>,
    #[serde(default)]
    pub buckets: BTreeMap<String, SavedBucket>,
    #[serde(default)]
    pub efforts: BTreeMap<String, SavedEffort>,
    /// A number field on each case (an amount) to total the cases that break
    /// a rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_field: Option<String>,
    /// The latest change in words, for an open page to show, e.g. "Priya (AP
    /// lead) confirmed: “Pay invoice” happens only after “Approve invoice”".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_change: Option<String>,
}

/// The file inside a project folder.
pub fn path(root: &Path) -> PathBuf {
    root.join("preparation").join("improvement.json")
}

impl Improvement {
    /// The project's decisions, or none yet.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = path(root);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("{} is unreadable: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        }
    }

    /// Saves with the next revision and records the change in the audit log.
    pub fn save(
        &mut self,
        root: &Path,
        action: &str,
        details: serde_json::Value,
    ) -> Result<(), String> {
        self.revision += 1;
        let path = path(root);
        std::fs::create_dir_all(path.parent().expect("a folder")).map_err(|e| e.to_string())?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
        let mut details = details;
        details["revision"] = self.revision.into();
        crate::audit::append(root, action, details)
    }

    /// Loads, changes and saves under the decisions' own lock, so two chats
    /// deciding at once both land. It does not take the project's lock: a
    /// person confirms in chat while the project is open in the app.
    pub fn change<T>(
        root: &Path,
        action: &str,
        change: impl FnOnce(&mut Self) -> Result<(T, serde_json::Value), String>,
    ) -> Result<(Self, T), String> {
        use fs2::FileExt;
        let path = path(root);
        std::fs::create_dir_all(path.parent().expect("a folder")).map_err(|e| e.to_string())?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.with_extension("lock"))
            .map_err(|e| format!("cannot lock the decisions: {e}"))?;
        lock.lock_exclusive()
            .map_err(|e| format!("cannot lock the decisions: {e}"))?;
        let mut improvement = Self::load(root)?;
        let (value, details) = change(&mut improvement)?;
        improvement.save(root, action, details)?;
        Ok((improvement, value))
    }

    /// The performer kinds decided so far (not merely suggested).
    pub fn confirmed_performers(&self) -> BTreeMap<String, ResourceKind> {
        self.performers
            .iter()
            .filter(|(_, p)| p.said.status == Status::Confirmed)
            .map(|(name, p)| (name.clone(), p.kind))
            .collect()
    }

    /// Adds or replaces a rule entry.
    pub fn put_rule(&mut self, rule: SavedRule) {
        match self
            .rules
            .iter_mut()
            .find(|r| r.same_rule(rule.template, &rule.a, rule.b.as_deref()))
        {
            Some(existing) => *existing = rule,
            None => self.rules.push(rule),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(status: Status) -> Said {
        Said {
            status,
            by: "agent".into(),
            at: "2026-10-14T00:00:00Z".into(),
            reason: Some("holds in 99% of cases".into()),
        }
    }

    #[test]
    fn saves_counts_revisions_and_audits() {
        let dir = tempfile::tempdir().unwrap();
        let mut improvement = Improvement::load(dir.path()).unwrap();
        assert_eq!(improvement.revision, 0);
        improvement.put_rule(SavedRule {
            template: Template::Precedence,
            a: "Approve".into(),
            b: Some("Pay".into()),
            said: said(Status::Suggested),
        });
        improvement
            .save(
                dir.path(),
                "rule_suggested",
                serde_json::json!({"rule": "precedence"}),
            )
            .unwrap();
        // Confirming the same rule replaces the suggestion.
        improvement.put_rule(SavedRule {
            template: Template::Precedence,
            a: "Approve".into(),
            b: Some("Pay".into()),
            said: said(Status::Confirmed),
        });
        improvement
            .save(dir.path(), "rule_confirmed", serde_json::json!({}))
            .unwrap();
        let again = Improvement::load(dir.path()).unwrap();
        assert_eq!(again.revision, 2);
        assert_eq!(again.rules.len(), 1);
        assert_eq!(again.rules[0].said.status, Status::Confirmed);
        let log = std::fs::read_to_string(crate::audit::path(dir.path())).unwrap();
        assert_eq!(log.lines().count(), 2);
        assert!(log.contains("rule_confirmed"));
    }

    #[test]
    fn changes_from_two_threads_both_land() {
        let dir = tempfile::tempdir().unwrap();
        let threads = (0..8)
            .map(|n| {
                let root = dir.path().to_owned();
                std::thread::spawn(move || {
                    Improvement::change(&root, "decide_effort", |improvement| {
                        improvement.efforts.insert(
                            format!("step {n}"),
                            SavedEffort {
                                minutes: 1.0,
                                cost_per_hour: None,
                                said: said(Status::Confirmed),
                            },
                        );
                        Ok(((), serde_json::json!({})))
                    })
                    .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        let saved = Improvement::load(dir.path()).unwrap();
        assert_eq!((saved.revision, saved.efforts.len()), (8, 8));
    }

    #[test]
    fn only_confirmed_performers_count() {
        let mut improvement = Improvement::default();
        improvement.performers.insert(
            "batch_01".into(),
            SavedPerformer {
                kind: ResourceKind::System,
                said: said(Status::Suggested),
            },
        );
        improvement.performers.insert(
            "batch_02".into(),
            SavedPerformer {
                kind: ResourceKind::System,
                said: said(Status::Confirmed),
            },
        );
        let confirmed = improvement.confirmed_performers();
        assert_eq!(confirmed.len(), 1);
        assert!(confirmed.contains_key("batch_02"));
    }
}
