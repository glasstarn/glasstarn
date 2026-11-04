//! Durable investigation folders. A folder has one owner and immutable assets;
//! atomically replacing the manifest is the only commit point for a saved view.
use crate::{ProjectError, ProjectLock, Source};
use pm_core::OcelFilter;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

const MANIFEST: &str = "workspace.json";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationPurpose {
    Audit,
    #[default]
    Improvement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceManifest {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub purpose: InvestigationPurpose,
    #[serde(default)]
    pub state: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceAsset {
    pub relative_path: String,
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowedView {
    pub object_type: String,
    pub filter: pm_core::Filter,
    pub base_filter: OcelFilter,
}

/// Persisted active analysis shared by browser-first and headless workflows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceState {
    pub source: Option<WorkspaceAsset>,
    #[serde(default)]
    pub preparation_sources: Vec<WorkspaceAsset>,
    pub case_project: Option<crate::Project>,
    pub ocel_filter: Option<OcelFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followed_view: Option<FollowedView>,
    pub case_model: Option<WorkspaceAsset>,
    pub ocel_model: Option<WorkspaceAsset>,
    /// The period the project's extract covers, set by extraction and
    /// carried into the case project's rules so cases are labelled against it.
    #[serde(default)]
    pub observation_window: Option<pm_core::ObservationWindow>,
    /// Business names for item types of a connected log, as plurals:
    /// "opportunity" → "Deals". Display only; analysis uses the source names.
    #[serde(default)]
    pub item_names: std::collections::BTreeMap<String, String>,
}

pub struct WorkspaceFolder {
    pub root: PathBuf,
    pub manifest: WorkspaceManifest,
    _lock: ProjectLock,
    append_only: bool,
}

fn io(path: &Path, source: std::io::Error) -> ProjectError {
    ProjectError::Io {
        path: path.display().to_string(),
        source,
    }
}
fn invalid(path: &Path, message: &str) -> ProjectError {
    io(
        path,
        std::io::Error::new(std::io::ErrorKind::InvalidData, message),
    )
}

pub fn valid_workspace_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

impl WorkspaceFolder {
    pub fn create_named(
        parent: &Path,
        name: &str,
        purpose: InvestigationPurpose,
    ) -> Result<Self, ProjectError> {
        std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        for _ in 0..16 {
            let mut entropy = [0u8; 16];
            OsRng.fill_bytes(&mut entropy);
            let id = entropy
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let root = parent.join(&id);
            match Self::create(&root, &id, name, purpose) {
                Ok(folder) => return Ok(folder),
                Err(ProjectError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(invalid(
            parent,
            "could not allocate a unique project identifier",
        ))
    }

    /// Private runtime/config files are not analytical source artifacts.
    pub fn read_agent_file(&self, name: &str) -> Result<Option<serde_json::Value>, ProjectError> {
        let path = self.agent_path(name)?;
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(&path, e)),
            Ok(meta) if !meta.is_file() || meta.len() > 64 * 1024 => {
                return Err(invalid(&path, "invalid agent configuration file"));
            }
            Ok(_) => {}
        }
        let text = std::fs::read(&path).map_err(|e| io(&path, e))?;
        serde_json::from_slice(&text)
            .map(Some)
            .map_err(|source| ProjectError::Parse {
                path: path.display().to_string(),
                source,
            })
    }

    pub fn write_agent_file(
        &self,
        name: &str,
        value: &serde_json::Value,
    ) -> Result<(), ProjectError> {
        if self.append_only {
            return Err(invalid(
                &self.root,
                "investigation access cannot change agent access",
            ));
        }
        let path = self.agent_path(name)?;
        if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
            return Err(invalid(&path, "invalid agent configuration file"));
        }
        let bytes = serde_json::to_vec_pretty(value).map_err(|source| ProjectError::Parse {
            path: path.display().to_string(),
            source,
        })?;
        if bytes.len() > 64 * 1024 {
            return Err(invalid(&path, "agent configuration exceeds size limit"));
        }
        let mut file = atomic_write_file::AtomicWriteFile::open(&path).map_err(|e| io(&path, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| io(&path, e))?;
        }
        file.write_all(&bytes).map_err(|e| io(&path, e))?;
        file.commit().map_err(|e| io(&path, e))
    }

    fn agent_path(&self, name: &str) -> Result<PathBuf, ProjectError> {
        if ![".agent-access.json", ".agent-session.json"].contains(&name) {
            return Err(invalid(&self.root, "invalid agent file name"));
        }
        Ok(self.root.join(name))
    }

    pub fn create(
        root: &Path,
        id: &str,
        name: &str,
        purpose: InvestigationPurpose,
    ) -> Result<Self, ProjectError> {
        if !valid_workspace_id(id) || name.trim().is_empty() || name.len() > 200 {
            return Err(invalid(
                root,
                "a project requires an identifier and a name of 1–200 bytes",
            ));
        }
        std::fs::create_dir(root).map_err(|e| io(root, e))?;
        // The name is display-only. Untrusted names never become paths.
        let root = root.canonicalize().map_err(|e| io(root, e))?;
        let lock = ProjectLock::acquire(root.join(MANIFEST))?;
        for directory in [
            "sources",
            "models",
            "references",
            "preparation",
            "findings",
            "exports",
        ] {
            let path = root.join(directory);
            std::fs::create_dir(&path).map_err(|e| io(&path, e))?;
        }
        let folder = Self {
            root,
            manifest: WorkspaceManifest {
                version: 1,
                id: id.to_owned(),
                name: name.trim().to_owned(),
                purpose,
                state: serde_json::Value::Null,
            },
            _lock: lock,
            append_only: false,
        };
        folder.write(&folder.manifest)?;
        Ok(folder)
    }

    pub fn inspect(root: &Path) -> Result<WorkspaceManifest, ProjectError> {
        let path = root.join(MANIFEST);
        if std::fs::symlink_metadata(&path)
            .map_err(|e| io(&path, e))?
            .file_type()
            .is_symlink()
        {
            return Err(ProjectError::Symlink {
                path: path.display().to_string(),
            });
        }
        if std::fs::metadata(&path).map_err(|e| io(&path, e))?.len() > 4 * 1024 * 1024 {
            return Err(invalid(&path, "project manifest exceeds 4 MiB"));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| io(&path, e))?;
        let manifest: WorkspaceManifest =
            serde_json::from_str(&text).map_err(|source| ProjectError::Parse {
                path: path.display().to_string(),
                source,
            })?;
        if manifest.version != 1
            || !valid_workspace_id(&manifest.id)
            || manifest.name.trim().is_empty()
            || manifest.name.len() > 200
        {
            return Err(invalid(&path, "invalid or unsupported project manifest"));
        }
        Ok(manifest)
    }

    pub fn open(root: &Path) -> Result<Self, ProjectError> {
        Self::open_mode(root, false)
    }

    /// Whether another process owns this project's settings (its open page),
    /// so this one may only add evidence and answers.
    pub fn is_append_only(&self) -> bool {
        self.append_only
    }

    /// Read a pinned settings snapshot and append unique evidence/answers while
    /// another process owns settings. Never permits source, settings, model or
    /// access changes. A separate lock serializes concurrent artifact writers.
    pub fn open_for_investigation(root: &Path) -> Result<Self, ProjectError> {
        Self::open_mode(root, true)
    }

    pub fn settings_are_current(&self) -> Result<bool, ProjectError> {
        let current = Self::inspect(&self.root)?;
        Ok(current.id == self.manifest.id && current.state == self.manifest.state)
    }

    fn open_mode(root: &Path, append_only: bool) -> Result<Self, ProjectError> {
        if std::fs::symlink_metadata(root)
            .map_err(|e| io(root, e))?
            .file_type()
            .is_symlink()
        {
            return Err(ProjectError::Symlink {
                path: root.display().to_string(),
            });
        }
        let root = root.canonicalize().map_err(|e| io(root, e))?;
        let lock = ProjectLock::acquire(root.join(if append_only {
            ".investigation-artifacts"
        } else {
            MANIFEST
        }))?;
        let manifest = Self::inspect(&root)?;
        Ok(Self {
            root,
            manifest,
            _lock: lock,
            append_only,
        })
    }

    fn write(&self, manifest: &WorkspaceManifest) -> Result<(), ProjectError> {
        if self.append_only {
            return Err(invalid(
                &self.root,
                "investigation access cannot change project settings",
            ));
        }
        let path = self.root.join(MANIFEST);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(ProjectError::Symlink {
                path: path.display().to_string(),
            });
        }
        let bytes = serde_json::to_vec_pretty(manifest).map_err(|source| ProjectError::Parse {
            path: path.display().to_string(),
            source,
        })?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(invalid(&path, "project manifest exceeds 4 MiB"));
        }
        let mut file = atomic_write_file::AtomicWriteFile::open(&path).map_err(|e| io(&path, e))?;
        file.write_all(&bytes).map_err(|e| io(&path, e))?;
        file.commit().map_err(|e| io(&path, e))
    }

    pub fn save(&mut self, state: serde_json::Value) -> Result<(), ProjectError> {
        let mut manifest = self.manifest.clone();
        manifest.state = state;
        self.write(&manifest)?;
        self.manifest = manifest;
        Ok(())
    }

    /// Gives the project a new name, keeping everything else.
    pub fn rename(&mut self, name: &str) -> Result<(), ProjectError> {
        let mut manifest = self.manifest.clone();
        manifest.name = name.to_owned();
        self.write(&manifest)?;
        self.manifest = manifest;
        Ok(())
    }

    pub fn state(&self) -> Result<WorkspaceState, ProjectError> {
        if self.manifest.state.is_null() {
            return Ok(WorkspaceState::default());
        }
        serde_json::from_value(self.manifest.state.clone()).map_err(|source| ProjectError::Parse {
            path: self.root.join(MANIFEST).display().to_string(),
            source,
        })
    }

    pub fn save_state(&mut self, state: &WorkspaceState) -> Result<(), ProjectError> {
        let value = serde_json::to_value(state).map_err(|source| ProjectError::Parse {
            path: self.root.join(MANIFEST).display().to_string(),
            source,
        })?;
        self.save(value)
    }

    /// Bounded saved question definitions, available to browser and headless agents.
    /// These contain scope and query choices, never client-supplied statistics.
    pub fn saved_views(&self) -> Result<Vec<serde_json::Value>, ProjectError> {
        let directory = self.root.join("exports");
        if directory.canonicalize().map_err(|e| io(&directory, e))? != directory {
            return Err(invalid(
                &directory,
                "artifact directory cannot be a symbolic link",
            ));
        }
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(&directory).map_err(|e| io(&directory, e))? {
            let entry = entry.map_err(|e| io(&directory, e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("saved-view-")
                && name.ends_with(".json")
                && entry
                    .file_type()
                    .map_err(|e| io(&entry.path(), e))?
                    .is_file()
                && entry.metadata().map_err(|e| io(&entry.path(), e))?.len() <= 128 * 1024
            {
                paths.push(entry.path());
            }
        }
        paths.sort();
        let state =
            serde_json::to_value(self.state()?).map_err(|e| invalid(&directory, &e.to_string()))?;
        let source = &state["source"];
        let mut result = Vec::new();
        for path in paths.into_iter().rev().take(100) {
            let bytes = std::fs::read(&path).map_err(|e| io(&path, e))?;
            let Ok(mut view) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if view["version"] != 1
                || view["project"] != self.manifest.id
                || !view["exploration"].is_object()
                || !view["title"].is_string()
            {
                continue;
            }
            if let Some(stem) = path.file_stem() {
                view["id"] = serde_json::json!(stem.to_string_lossy());
            }
            view["same_source"] =
                serde_json::json!(!source.is_null() && view["state"]["source"] == *source);
            view["compatible"] = serde_json::json!(
                view["same_source"] == true
                    && ["mapping", "policy", "rules"]
                        .iter()
                        .all(|key| view["state"]["case_project"][key] == state["case_project"][key])
                    && view["state"]["observation_window"] == state["observation_window"]
            );
            result.push(view);
        }
        Ok(result)
    }

    /// Takes a saved view off the list. The file is renamed, not erased, so
    /// it can still be recovered from the project's exports folder.
    pub fn remove_saved_view(&self, id: &str) -> Result<(), ProjectError> {
        let directory = self.root.join("exports");
        if self.append_only
            || !id.starts_with("saved-view-")
            || id.len() > 120
            || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(invalid(&directory, "invalid saved view"));
        }
        let path = directory.join(format!("{id}.json"));
        if !std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
            return Err(invalid(&path, "saved view not found"));
        }
        let removed = directory.join(format!("removed-{id}.json"));
        std::fs::rename(&path, &removed).map_err(|e| io(&path, e))
    }

    /// Atomically writes a bounded, non-source artifact to a reserved folder.
    pub fn write_artifact(
        &self,
        category: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, ProjectError> {
        if !["preparation", "findings", "exports"].contains(&category)
            || name.is_empty()
            || name.len() > 120
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
            || bytes.len() > 64 * 1024 * 1024
        {
            return Err(invalid(&self.root, "invalid or oversized project artifact"));
        }
        if self.append_only && !["exports", "findings"].contains(&category) {
            return Err(invalid(
                &self.root,
                "investigation access can append only query evidence and findings",
            ));
        }
        let directory = self.root.join(category);
        if directory.canonicalize().map_err(|e| io(&directory, e))? != directory {
            return Err(invalid(
                &directory,
                "project artifact directories cannot be symbolic links",
            ));
        }
        let path = directory.join(name);
        if self.append_only && path.exists() {
            return Err(invalid(&path, "investigation artifacts are append-only"));
        }
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_file()) {
            return Err(invalid(&path, "invalid project artifact"));
        }
        let mut file = atomic_write_file::AtomicWriteFile::open(&path).map_err(|e| io(&path, e))?;
        file.write_all(bytes).map_err(|e| io(&path, e))?;
        file.commit().map_err(|e| io(&path, e))?;
        Ok(path)
    }

    /// Imports an already fingerprinted upload as a content-addressed asset.
    /// Earlier revisions stay on disk; a failed save never damages the old view.
    pub fn store_asset(
        &self,
        source: &Path,
        name: &str,
        suffix: &str,
        category: &str,
    ) -> Result<WorkspaceAsset, ProjectError> {
        if self.append_only {
            return Err(invalid(
                &self.root,
                "investigation access cannot change sources or models",
            ));
        }
        if !["sources", "models"].contains(&category)
            || !suffix.starts_with('.')
            || !suffix
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'.')
        {
            return Err(invalid(&self.root, "invalid project asset type"));
        }
        let fingerprint = Source::fingerprint(source)?;
        let asset = WorkspaceAsset {
            relative_path: format!("{category}/{}{suffix}", fingerprint.sha256),
            name: name.to_owned(),
            bytes: fingerprint.bytes,
            sha256: fingerprint.sha256,
        };
        let directory = self.root.join(category);
        if directory.canonicalize().map_err(|e| io(&directory, e))? != directory {
            return Err(invalid(
                &directory,
                "project asset directories cannot be symbolic links",
            ));
        }
        let destination = self.root.join(&asset.relative_path);
        if destination.exists() {
            self.resolve_asset(&asset)?;
        } else {
            if std::fs::symlink_metadata(&destination).is_ok() {
                return Err(invalid(&destination, "invalid project asset"));
            }
            let mut input = std::fs::File::open(source).map_err(|e| io(source, e))?;
            let mut output = atomic_write_file::AtomicWriteFile::open(&destination)
                .map_err(|e| io(&destination, e))?;
            std::io::copy(&mut input, &mut output).map_err(|e| io(&destination, e))?;
            output.commit().map_err(|e| io(&destination, e))?;
            self.resolve_asset(&asset)?;
        }
        Ok(asset)
    }

    pub fn resolve_asset(&self, asset: &WorkspaceAsset) -> Result<PathBuf, ProjectError> {
        let path = Path::new(&asset.relative_path);
        let components: Vec<_> = path.components().collect();
        if components.len() != 2
            || !components
                .iter()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            || !matches!(
                components[0].as_os_str().to_str(),
                Some("sources" | "models")
            )
        {
            return Err(invalid(
                path,
                "project asset must be inside sources or models",
            ));
        }
        let expected = self.root.join(path);
        let resolved = expected.canonicalize().map_err(|e| io(&expected, e))?;
        if resolved != expected || !resolved.starts_with(&self.root) {
            return Err(invalid(path, "project asset cannot follow symbolic links"));
        }
        let actual = Source::fingerprint(&resolved)?;
        if actual.sha256 != asset.sha256 || actual.bytes != asset.bytes {
            return Err(invalid(
                path,
                "project asset changed; original evidence cannot be reproduced",
            ));
        }
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pm-workspace-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    const ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn investigation_append_access_cannot_change_settings_sources_or_permissions() {
        let path = root("append-capability");
        let mut owner = WorkspaceFolder::create_named(
            &path,
            "Append boundary",
            InvestigationPurpose::Improvement,
        )
        .unwrap();
        let mut append = WorkspaceFolder::open_for_investigation(&owner.root).unwrap();
        assert!(WorkspaceFolder::open(&owner.root).is_err());
        assert!(WorkspaceFolder::open_for_investigation(&owner.root).is_err());
        assert!(append.save(serde_json::json!({})).is_err());
        assert!(
            append
                .write_agent_file(".agent-access.json", &serde_json::json!({}))
                .is_err()
        );
        assert!(
            append
                .store_asset(
                    &owner.root.join("workspace.json"),
                    "new.csv",
                    ".csv",
                    "sources"
                )
                .is_err()
        );
        assert!(
            append
                .write_artifact("preparation", "recipe.json", b"{}")
                .is_err()
        );
        append
            .write_artifact("findings", "investigation-1.json", b"{}")
            .unwrap();
        assert!(
            append
                .write_artifact("findings", "investigation-1.json", b"{}")
                .is_err()
        );
        assert!(append.settings_are_current().unwrap());
        owner.save(serde_json::json!({"case_project":null,"source":null,"ocel_filter":null,"case_model":null,"ocel_model":null})).unwrap();
        assert!(!append.settings_are_current().unwrap());
        drop(append);
        drop(owner);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn workspace_roundtrip_locks_and_moves_with_its_evidence() {
        let root = root("roundtrip");
        let mut folder =
            WorkspaceFolder::create(&root, ID, "Audit / quarter 1", InvestigationPurpose::Audit)
                .unwrap();
        assert!(WorkspaceFolder::open(&root).is_err());
        let input = root.join("input.csv");
        std::fs::write(&input, "case,activity\n1,Approved\n").unwrap();
        let asset = folder
            .store_asset(&input, "original.csv", ".csv", "sources")
            .unwrap();
        folder.save(serde_json::json!({"source":asset})).unwrap();
        drop(folder);
        std::fs::remove_file(input).unwrap();
        let moved = root.with_extension("moved");
        std::fs::rename(&root, &moved).unwrap();
        let folder = WorkspaceFolder::open(&moved).unwrap();
        assert_eq!(folder.manifest.purpose, InvestigationPurpose::Audit);
        assert_eq!(folder.manifest.name, "Audit / quarter 1");
        let asset: WorkspaceAsset =
            serde_json::from_value(folder.manifest.state["source"].clone()).unwrap();
        let path = folder.resolve_asset(&asset).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "case,activity\n1,Approved\n"
        );
        std::fs::write(&path, "tampered").unwrap();
        assert!(
            folder
                .resolve_asset(&asset)
                .unwrap_err()
                .to_string()
                .contains("changed")
        );
        drop(folder);
        std::fs::remove_dir_all(moved).unwrap();
    }

    #[test]
    fn rejects_asset_path_escape_and_invalid_names() {
        let root = root("paths");
        assert!(
            WorkspaceFolder::create(&root, "../escape", "A", InvestigationPurpose::Audit).is_err()
        );
        let folder = WorkspaceFolder::create(&root, ID, "A", InvestigationPurpose::Audit).unwrap();
        for path in [
            "../outside.csv",
            "/tmp/source.csv",
            "sources/../../outside.csv",
        ] {
            let asset = WorkspaceAsset {
                relative_path: path.into(),
                name: "x".into(),
                bytes: 0,
                sha256: "x".into(),
            };
            assert!(folder.resolve_asset(&asset).is_err());
        }
        drop(folder);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_sources_and_manifest() {
        let root = root("symlinks");
        let mut folder =
            WorkspaceFolder::create(&root, ID, "A", InvestigationPurpose::Audit).unwrap();
        let source = root.join("outside.csv");
        std::fs::write(&source, "data").unwrap();
        std::os::unix::fs::symlink(&source, root.join("sources/escape.csv")).unwrap();
        let asset = WorkspaceAsset {
            relative_path: "sources/escape.csv".into(),
            name: "x".into(),
            bytes: 4,
            sha256: Source::fingerprint(&source).unwrap().sha256,
        };
        assert!(folder.resolve_asset(&asset).is_err());
        std::fs::remove_file(root.join(MANIFEST)).unwrap();
        std::os::unix::fs::symlink(&source, root.join(MANIFEST)).unwrap();
        assert!(folder.save(serde_json::Value::Null).is_err());
        assert_eq!(std::fs::read_to_string(source).unwrap(), "data");
        drop(folder);
        std::fs::remove_dir_all(root).unwrap();
    }
}
