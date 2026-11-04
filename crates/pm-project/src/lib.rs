//! Saved analysis projects.
//!
//! # Why this exists
//!
//! Every ambiguity in a log is resolved by a recorded decision — how to read
//! ambiguous dates, what a blank timestamp means, which labels are the same
//! activity. Without somewhere to keep those decisions they evaporate when the
//! command exits, and the next run either asks everything again or, worse,
//! answers differently.
//!
//! A project records **what was analysed and how it was interpreted**, so
//! reopening it reproduces the same numbers.
//!
//! # Reproducibility is verified, not assumed
//!
//! The source file's size and cryptographic hash are recorded. Reopening a
//! project whose source has changed is not silently accepted: the mismatch is
//! reported, because results computed from different data are not the same
//! results, however similar they look.

pub mod audit;
pub mod discover;
pub mod extract;
pub mod extract_hubspot;
pub mod extract_jira;
pub mod extract_servicenow;
pub mod folders;
pub mod improvement;
pub mod workspace;

use pm_ingest::finding::Policy;
use pm_ingest::rules::Rules;
use pm_ingest::validate::Mapping;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The engine version a project was created with.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The project file format version.
///
/// Separate from the engine version: the format changes far less often, and a
/// project written by an older engine should still open.
pub const FORMAT_VERSION: u32 = 1;

/// Why a project could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("could not read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a valid project file: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "this project was written in format version {found}, but this engine understands \
         version {understood}"
    )]
    UnsupportedFormat { found: u32, understood: u32 },
    #[error("{path} is already open for writing in another process")]
    Locked { path: String },
    #[error("the project lock for {locked} cannot write {requested}")]
    LockMismatch { locked: String, requested: String },
    #[error("refusing to write through the symbolic-link project path {path}")]
    Symlink { path: String },
}

/// Exclusive cross-process ownership of one writable project.
///
/// The operating system releases the advisory lock if the process exits or
/// crashes. The small sidecar may remain, but its presence alone never means a
/// project is locked; only the kernel lock does.
#[derive(Debug)]
pub struct ProjectLock {
    _file: std::fs::File,
    project_path: PathBuf,
    identity: PathBuf,
}

impl ProjectLock {
    /// Attempts to become the single writer without waiting.
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        use fs2::FileExt;

        let project_path = path.as_ref().to_path_buf();
        let identity = project_identity(&project_path)?;
        let lock_path = sidecar_lock_path(&identity);
        let display = project_path.display().to_string();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| ProjectError::Io {
                path: lock_path.display().to_string(),
                source,
            })?;
        if let Err(source) = file.try_lock_exclusive() {
            if source.kind() == fs2::lock_contended_error().kind() {
                return Err(ProjectError::Locked { path: display });
            }
            return Err(ProjectError::Io {
                path: lock_path.display().to_string(),
                source,
            });
        }
        Ok(Self {
            _file: file,
            project_path,
            identity,
        })
    }

    pub fn project_path(&self) -> &Path {
        &self.project_path
    }
}

fn sidecar_lock_path(path: &Path) -> PathBuf {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    PathBuf::from(lock_path)
}

fn project_identity(path: &Path) -> Result<PathBuf, ProjectError> {
    let display = path.display().to_string();
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(ProjectError::Symlink { path: display });
    }
    let file_name = path.file_name().ok_or_else(|| ProjectError::Io {
        path: display.clone(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "project path has no file name",
        ),
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = parent.canonicalize().map_err(|source| ProjectError::Io {
        path: parent.display().to_string(),
        source,
    })?;
    Ok(parent.join(file_name))
}

/// What the source file looked like when the project was saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// Path as given. Recorded for orientation; the fingerprint is what counts.
    pub path: String,
    pub bytes: u64,
    /// SHA-256 of the file contents.
    pub sha256: String,
}

impl Source {
    /// Fingerprints a file.
    pub fn fingerprint(path: impl AsRef<Path>) -> Result<Source, ProjectError> {
        let path = path.as_ref();
        let display = path.display().to_string();
        let mut file = std::fs::File::open(path).map_err(|source| ProjectError::Io {
            path: display.clone(),
            source,
        })?;
        let mut hasher = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|source| ProjectError::Io {
                path: display.clone(),
                source,
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            bytes += read as u64;
        }
        Ok(Source {
            path: display,
            bytes,
            sha256: format!("{:x}", hasher.finalize()),
        })
    }
}

/// Whether a source file still matches what a project recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceState {
    /// Byte-for-byte identical. Results will reproduce.
    Unchanged,
    /// The file exists but its contents differ.
    Changed { was_bytes: u64, now_bytes: u64 },
    /// The file is not where the project recorded it.
    Missing,
}

impl SourceState {
    /// Whether results can be expected to reproduce.
    pub fn reproduces(&self) -> bool {
        matches!(self, SourceState::Unchanged)
    }

    /// A plain-language explanation.
    pub fn explain(&self) -> String {
        match self {
            SourceState::Unchanged => {
                "The source file is unchanged, so results will reproduce exactly.".to_owned()
            }
            SourceState::Changed {
                was_bytes,
                now_bytes,
            } => format!(
                "The source file has changed since this project was saved ({was_bytes} bytes \
                 then, {now_bytes} now). Results will differ, and any earlier finding no longer \
                 refers to this data."
            ),
            SourceState::Missing => {
                "The source file is not where this project recorded it, so nothing can be \
                 reproduced until it is restored or re-imported."
                    .to_owned()
            }
        }
    }
}

/// A saved analysis project.
///
/// Everything needed to reproduce a result, and nothing else. The event data
/// itself stays in the source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Project file format version.
    pub format_version: u32,
    /// Engine version that wrote it, so a changed engine can be detected.
    pub engine_version: String,
    /// A human name for the analysis.
    #[serde(default)]
    pub name: String,
    pub source: Source,
    /// Which columns hold which fields.
    pub mapping: Mapping,
    /// Answers to the decisions validation raised.
    #[serde(default)]
    pub policy: Policy,
    /// Renames, exclusions, and case rules.
    #[serde(default)]
    pub rules: Rules,
    /// The active exploratory case view when the project was saved.
    #[serde(default)]
    pub view_filter: pm_core::Filter,
}

impl Project {
    /// Creates a project describing an analysis of `path`.
    pub fn new(
        path: impl AsRef<Path>,
        mapping: Mapping,
        policy: Policy,
        rules: Rules,
    ) -> Result<Project, ProjectError> {
        Ok(Project {
            format_version: FORMAT_VERSION,
            engine_version: ENGINE_VERSION.to_owned(),
            name: String::new(),
            source: Source::fingerprint(path)?,
            mapping,
            policy,
            rules,
            view_filter: pm_core::Filter::new(),
        })
    }

    /// Names the analysis.
    pub fn named(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self
    }

    /// Reads a project file.
    pub fn load(path: impl AsRef<Path>) -> Result<Project, ProjectError> {
        let path = path.as_ref();
        let display = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| ProjectError::Io {
            path: display.clone(),
            source,
        })?;
        Self::from_json(&text, &display)
    }

    /// Reads a project from JSON supplied by a browser or another in-memory
    /// transport. `label` is used only to make parse errors understandable.
    pub fn from_json(text: &str, label: &str) -> Result<Project, ProjectError> {
        let project: Project =
            serde_json::from_str(text).map_err(|source| ProjectError::Parse {
                path: label.to_owned(),
                source,
            })?;

        project.validate()?;
        Ok(project)
    }

    /// Confirms that this engine understands the deserialized project format.
    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.format_version > FORMAT_VERSION {
            return Err(ProjectError::UnsupportedFormat {
                found: self.format_version,
                understood: FORMAT_VERSION,
            });
        }
        Ok(())
    }

    /// Writes the project to a file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ProjectError> {
        let path = path.as_ref();
        let lock = ProjectLock::acquire(path)?;
        self.save_with_lock(path, &lock)
    }

    /// Atomically saves through a lock retained by a longer-running owner.
    /// This is the path a browser service will use while MCP delegates to it.
    pub fn save_with_lock(
        &self,
        path: impl AsRef<Path>,
        lock: &ProjectLock,
    ) -> Result<(), ProjectError> {
        let path = path.as_ref();
        let requested_identity = project_identity(path)?;
        if requested_identity != lock.identity {
            return Err(ProjectError::LockMismatch {
                locked: lock.project_path().display().to_string(),
                requested: path.display().to_string(),
            });
        }
        let display = path.display().to_string();
        let mut file =
            atomic_write_file::AtomicWriteFile::open(path).map_err(|source| ProjectError::Io {
                path: display.clone(),
                source,
            })?;
        file.write_all(self.to_json().as_bytes())
            .map_err(|source| ProjectError::Io {
                path: display.clone(),
                source,
            })?;
        file.commit().map_err(|source| ProjectError::Io {
            path: display,
            source,
        })
    }

    /// Renders the project as JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Checks whether a source file still matches what was recorded.
    pub fn check_source(&self, path: impl AsRef<Path>) -> SourceState {
        match Source::fingerprint(path) {
            Err(_) => SourceState::Missing,
            Ok(now) => self.check_fingerprint(&now),
        }
    }

    /// Checks an already-computed fingerprint, avoiding another source scan.
    pub fn check_fingerprint(&self, now: &Source) -> SourceState {
        if now.sha256 == self.source.sha256 {
            SourceState::Unchanged
        } else {
            SourceState::Changed {
                was_bytes: self.source.bytes,
                now_bytes: now.bytes,
            }
        }
    }

    /// Checks the source at the path recorded in the project.
    pub fn check_recorded_source(&self) -> SourceState {
        self.check_source(&self.source.path)
    }

    /// Whether the engine has changed since the project was written.
    ///
    /// Not an error — a newer engine may legitimately produce better results —
    /// but the user should know before treating numbers as identical.
    pub fn engine_changed(&self) -> bool {
        self.engine_version != ENGINE_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_ingest::finding::Kind;
    use std::io::Write;

    fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("pm_project_{name}"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    const LOG: &str = "case_id,activity,timestamp\nC1,Start,2026-03-02 09:00:00\n";

    fn project(source: &std::path::Path) -> Project {
        Project::new(
            source,
            Mapping::new("case_id", "activity", "timestamp"),
            Policy::new().answer(Kind::DuplicateRows, "deduplicate"),
            Rules::new().rename("STAT_03", "Credit Check"),
        )
        .unwrap()
        .named("Order to cash")
    }

    #[test]
    fn records_everything_needed_to_reproduce() {
        let source = write_temp("record.csv", LOG);
        let p = project(&source);
        assert_eq!(p.mapping.case_id, "case_id");
        assert_eq!(p.policy.get(&Kind::DuplicateRows), Some("deduplicate"));
        assert_eq!(p.rules.resolve_activity("STAT_03"), Some("Credit Check"));
        assert_eq!(p.engine_version, ENGINE_VERSION);
        assert_eq!(p.name, "Order to cash");
    }

    #[test]
    fn round_trips_through_a_file() {
        let source = write_temp("roundtrip.csv", LOG);
        let mut p = project(&source);
        p.view_filter = pm_core::Filter::new()
            .starts_with("Start")
            .ends_with("Archive")
            .directly_follows("Start", "Review")
            .eventually_follows("Review", "Archive");
        let path = write_temp("roundtrip.json", "");
        p.save(&path).unwrap();
        assert_eq!(Project::load(&path).unwrap(), p);
    }

    #[test]
    fn projects_saved_before_trace_filters_keep_an_empty_compatible_view() {
        let source = write_temp("legacy-filter.csv", LOG);
        let json = project(&source).to_json();
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let filter = value["view_filter"].as_object_mut().unwrap();
        for field in [
            "starts_with",
            "ends_with",
            "directly_follows",
            "eventually_follows",
        ] {
            filter.remove(field);
        }
        let restored = Project::from_json(&value.to_string(), "legacy project").unwrap();
        assert!(restored.view_filter.is_empty());
    }

    #[test]
    fn one_writer_owns_a_project_and_drop_recovers_the_lock() {
        let path = write_temp("exclusive-lock.json", "");
        let first = ProjectLock::acquire(&path).unwrap();
        assert!(matches!(
            ProjectLock::acquire(&path),
            Err(ProjectError::Locked { .. })
        ));

        drop(first);
        ProjectLock::acquire(&path).unwrap();
        assert!(sidecar_lock_path(&path).is_file());
    }

    #[test]
    fn a_competing_save_keeps_the_existing_project_readable() {
        let source = write_temp("locked-source.csv", LOG);
        let path = write_temp("locked-project.json", "");
        let original = project(&source);
        original.save(&path).unwrap();
        let owner = ProjectLock::acquire(&path).unwrap();
        let changed = original.clone().named("Changed by a second writer");

        assert!(matches!(
            changed.save(&path),
            Err(ProjectError::Locked { .. })
        ));
        assert_eq!(Project::load(&path).unwrap(), original);
        drop(owner);
    }

    #[test]
    fn a_long_running_owner_can_atomically_replace_its_project() {
        let source = write_temp("owned-source.csv", LOG);
        let path = write_temp("owned-project.json", "");
        let original = project(&source);
        original.save(&path).unwrap();
        let owner = ProjectLock::acquire(&path).unwrap();
        let changed = original.clone().named("Updated by owner");

        changed.save_with_lock(&path, &owner).unwrap();
        assert_eq!(Project::load(&path).unwrap(), changed);
    }

    #[test]
    fn a_lock_cannot_authorize_writing_a_different_project() {
        let source = write_temp("lock-scope-source.csv", LOG);
        let locked_path = write_temp("lock-scope-a.json", "");
        let other_path = write_temp("lock-scope-b.json", "unchanged");
        let owner = ProjectLock::acquire(&locked_path).unwrap();

        assert!(matches!(
            project(&source).save_with_lock(&other_path, &owner),
            Err(ProjectError::LockMismatch { .. })
        ));
        assert_eq!(std::fs::read_to_string(other_path).unwrap(), "unchanged");
    }

    #[cfg(unix)]
    #[test]
    fn parent_directory_aliases_share_one_lock_and_file_symlinks_are_refused() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pm_project_lock_alias_{}_{}",
            std::process::id(),
            unique
        ));
        let real = root.join("real");
        let alias = root.join("alias");
        std::fs::create_dir_all(&real).unwrap();
        symlink(&real, &alias).unwrap();
        let target = real.join("project.json");
        std::fs::write(&target, "{}").unwrap();
        let owner = ProjectLock::acquire(&target).unwrap();
        assert!(matches!(
            ProjectLock::acquire(alias.join("project.json")),
            Err(ProjectError::Locked { .. })
        ));

        let file_alias = root.join("project-link.json");
        symlink(&target, &file_alias).unwrap();
        assert!(matches!(
            ProjectLock::acquire(file_alias),
            Err(ProjectError::Symlink { .. })
        ));
        drop(owner);
    }

    #[test]
    fn fingerprints_the_source() {
        let source = write_temp("fingerprint.csv", LOG);
        let f = Source::fingerprint(&source).unwrap();
        assert_eq!(f.bytes as usize, LOG.len());
        assert_eq!(f.sha256.len(), 64, "sha-256 is 64 hex characters");
    }

    #[test]
    fn identical_contents_fingerprint_identically() {
        let a = write_temp("same_a.csv", LOG);
        let b = write_temp("same_b.csv", LOG);
        assert_eq!(
            Source::fingerprint(&a).unwrap().sha256,
            Source::fingerprint(&b).unwrap().sha256
        );
    }

    /// The point of the fingerprint: an unchanged source reproduces.
    #[test]
    fn detects_an_unchanged_source() {
        let source = write_temp("unchanged.csv", LOG);
        let p = project(&source);
        assert_eq!(p.check_source(&source), SourceState::Unchanged);
        assert!(p.check_source(&source).reproduces());
    }

    /// And a changed one must not be silently accepted — results computed from
    /// different data are not the same results.
    #[test]
    fn detects_a_changed_source() {
        let source = write_temp("changed.csv", LOG);
        let p = project(&source);
        let mut f = std::fs::File::create(&source).unwrap();
        f.write_all(b"case_id,activity,timestamp\nC1,Start,2026-03-02 09:00:00\nC2,Start,2026-03-02 10:00:00\n")
            .unwrap();
        let state = p.check_source(&source);
        assert!(!state.reproduces());
        assert!(matches!(state, SourceState::Changed { .. }));
        assert!(state.explain().contains("changed"));
    }

    /// A same-size edit must still be caught — length alone is not a
    /// fingerprint.
    #[test]
    fn detects_a_change_that_preserves_file_size() {
        let source = write_temp("samesize.csv", LOG);
        let p = project(&source);
        let altered = LOG.replace("Start", "Sturt");
        assert_eq!(altered.len(), LOG.len());
        std::fs::write(&source, &altered).unwrap();
        assert!(!p.check_source(&source).reproduces());
    }

    #[test]
    fn detects_a_missing_source() {
        let source = write_temp("missing.csv", LOG);
        let p = project(&source);
        std::fs::remove_file(&source).unwrap();
        assert_eq!(p.check_source(&source), SourceState::Missing);
        assert!(p.check_recorded_source().explain().contains("not where"));
    }

    /// A project from a future engine must be refused rather than
    /// misinterpreted.
    #[test]
    fn refuses_a_newer_format_version() {
        let path = write_temp("future.json", "");
        let source = write_temp("future.csv", LOG);
        let mut p = project(&source);
        p.format_version = FORMAT_VERSION + 1;
        p.save(&path).unwrap();
        assert!(matches!(
            Project::load(&path),
            Err(ProjectError::UnsupportedFormat { .. })
        ));
    }

    #[test]
    fn notices_a_different_engine_version() {
        let source = write_temp("engine.csv", LOG);
        let mut p = project(&source);
        assert!(!p.engine_changed());
        p.engine_version = "0.0.1-ancient".to_owned();
        assert!(p.engine_changed());
    }

    #[test]
    fn rejects_a_malformed_project_file() {
        let path = write_temp("bad.json", "{ not json");
        assert!(matches!(
            Project::load(&path),
            Err(ProjectError::Parse { .. })
        ));
    }

    #[test]
    fn missing_project_file_is_an_error_not_a_panic() {
        assert!(Project::load("/nonexistent/project.json").is_err());
    }

    #[test]
    fn is_deterministic() {
        let source = write_temp("determinism.csv", LOG);
        assert_eq!(project(&source).to_json(), project(&source).to_json());
    }
}
