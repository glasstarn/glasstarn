//! Immutable, content-addressed analysis result caching.

use crate::Analysis;
use pm_core::filter::Filter;
use pm_ingest::validate::Mapping;
use pm_ingest::{Policy, Rules};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// 2: summaries carry the P90 completion time. 3: analyses carry timing.
const CACHE_FORMAT: u32 = 3;
const ANALYSIS_KIND: &str = "default-analysis";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey(String);

#[derive(Serialize)]
struct KeyMaterial<'a> {
    cache_format: u32,
    engine_version: &'static str,
    analysis_kind: &'static str,
    source_path: &'a str,
    source_sha256: &'a str,
    mapping: &'a Mapping,
    policy: &'a Policy,
    rules: &'a Rules,
    filter: &'a Filter,
}

impl CacheKey {
    pub fn new(
        source_path: &str,
        source_sha256: &str,
        mapping: &Mapping,
        policy: &Policy,
        rules: &Rules,
        filter: &Filter,
    ) -> Self {
        let material = KeyMaterial {
            cache_format: CACHE_FORMAT,
            engine_version: env!("CARGO_PKG_VERSION"),
            analysis_kind: ANALYSIS_KIND,
            source_path,
            source_sha256,
            mapping,
            policy,
            rules,
            filter,
        };
        let encoded = serde_json::to_vec(&material).expect("cache key material is serializable");
        Self(format!("{:x}", Sha256::digest(encoded)))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    format: u32,
    key: String,
    analysis: Analysis,
}

#[derive(Debug, Clone)]
pub struct AnalysisCache {
    directory: PathBuf,
}

impl AnalysisCache {
    /// Places immutable cache entries beside the reviewable project file.
    pub fn beside(project: impl AsRef<Path>) -> Self {
        let mut directory = project.as_ref().as_os_str().to_os_string();
        directory.push(".cache");
        Self {
            directory: PathBuf::from(directory),
        }
    }

    fn entry_path(&self, key: &CacheKey) -> PathBuf {
        self.directory.join(format!("{}.json", key.0))
    }

    /// Returns a miss for unreadable, damaged, old, or mismatched entries.
    pub fn load(&self, key: &CacheKey) -> Option<Analysis> {
        let bytes = std::fs::read(self.entry_path(key)).ok()?;
        let entry: Entry = serde_json::from_slice(&bytes).ok()?;
        (entry.format == CACHE_FORMAT && entry.key == key.0).then_some(entry.analysis)
    }

    /// Writes through a unique temporary file, then publishes atomically.
    pub fn store(&self, key: &CacheKey, analysis: &Analysis) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.directory)?;
        let target = self.entry_path(key);
        if target.exists() {
            return Ok(());
        }
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary =
            self.directory
                .join(format!(".{}.{}.{sequence}.tmp", key.0, std::process::id()));
        let entry = Entry {
            format: CACHE_FORMAT,
            key: key.0.clone(),
            analysis: analysis.clone(),
        };
        let bytes = serde_json::to_vec(&entry).expect("analysis cache entry is serializable");
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        match std::fs::rename(&temporary, &target) {
            Ok(()) => Ok(()),
            Err(_error) if target.exists() => {
                let _ = std::fs::remove_file(&temporary);
                Ok(())
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_ingest::finding::Kind;

    fn mapping() -> Mapping {
        Mapping::new("case", "activity", "timestamp")
    }

    #[test]
    fn every_analysis_input_changes_the_key() {
        let base = CacheKey::new(
            "log.csv",
            "abc",
            &mapping(),
            &Policy::new(),
            &Rules::new(),
            &Filter::new(),
        );
        let changed_source = CacheKey::new(
            "log.csv",
            "def",
            &mapping(),
            &Policy::new(),
            &Rules::new(),
            &Filter::new(),
        );
        let changed_path = CacheKey::new(
            "moved.csv",
            "abc",
            &mapping(),
            &Policy::new(),
            &Rules::new(),
            &Filter::new(),
        );
        let changed_mapping = CacheKey::new(
            "log.csv",
            "abc",
            &Mapping::new("case", "step", "timestamp"),
            &Policy::new(),
            &Rules::new(),
            &Filter::new(),
        );
        let changed_policy = CacheKey::new(
            "log.csv",
            "abc",
            &mapping(),
            &Policy::new().answer(Kind::DuplicateRows, "deduplicate"),
            &Rules::new(),
            &Filter::new(),
        );
        let changed_rules = CacheKey::new(
            "log.csv",
            "abc",
            &mapping(),
            &Policy::new(),
            &Rules::new().exclude_activity("Noise"),
            &Filter::new(),
        );
        let changed_filter = CacheKey::new(
            "log.csv",
            "abc",
            &mapping(),
            &Policy::new(),
            &Rules::new(),
            &Filter::new().with_activity("Approve"),
        );
        assert_ne!(base, changed_source);
        assert_ne!(base, changed_path);
        assert_ne!(base, changed_mapping);
        assert_ne!(base, changed_policy);
        assert_ne!(base, changed_rules);
        assert_ne!(base, changed_filter);
    }

    #[test]
    fn stores_round_trips_and_ignores_damage() {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pm_query_cache_test_{}_{}",
            std::process::id(),
            sequence
        ));
        let source = root.with_extension("csv");
        std::fs::write(
            &source,
            "case_id,activity,timestamp\nC1,A,2026-03-02 09:00:00\nC1,B,2026-03-02 10:00:00\n",
        )
        .unwrap();
        let mapping = Mapping::new("case_id", "activity", "timestamp");
        let crate::Outcome::Complete(analysis) =
            crate::analyze(&source, Some(mapping.clone()), &Policy::new()).unwrap()
        else {
            panic!("fixture should analyze");
        };
        let cache = AnalysisCache::beside(root.with_extension("json"));
        let key = CacheKey::new(
            source.to_str().unwrap(),
            "fixture-sha",
            &mapping,
            &Policy::new(),
            &Rules::new(),
            &Filter::new(),
        );

        cache.store(&key, &analysis).unwrap();
        assert_eq!(cache.load(&key).as_ref(), Some(analysis.as_ref()));

        std::fs::write(cache.entry_path(&key), b"damaged").unwrap();
        assert_eq!(cache.load(&key), None);
    }
}
