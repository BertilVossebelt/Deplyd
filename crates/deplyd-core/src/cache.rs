//! What deplyd remembers between runs: only facts that cannot change. A completed
//! run is frozen. The run list is never cached, which is how new deploys arrive.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::settings::config_directory;

/// Entries kept per repository before the oldest are dropped.
const MAX_ENTRIES: usize = 2000;
const VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedSha {
    pub sha: String,
    /// Only exact resolutions are stored; a guess could read differently once the
    /// clone has more history.
    pub warning: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Document {
    #[serde(default)]
    version: u32,
    /// job id -> what its checkout resolved to.
    #[serde(default)]
    shas: BTreeMap<String, ResolvedSha>,
    /// Insertion order, oldest first, for pruning.
    #[serde(default)]
    order: Vec<String>,
}

pub struct Cache {
    path: PathBuf,
    document: Document,
    dirty: bool,
}

impl Cache {
    pub fn open(owner: &str, repo: &str) -> Self {
        let leaf = format!("{owner}-{repo}.json").replace(
            |c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-' && c != '_',
            "-",
        );
        let path = config_directory().join("cache").join(leaf);

        let document = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Document>(&text).ok())
            .filter(|document| document.version == VERSION)
            .unwrap_or_default();

        Self {
            path,
            document,
            dirty: false,
        }
    }

    /// A cache at a path of the caller's choosing, for tests.
    pub fn at(path: PathBuf) -> Self {
        let document = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Document>(&text).ok())
            .filter(|document| document.version == VERSION)
            .unwrap_or_default();
        Self {
            path,
            document,
            dirty: false,
        }
    }

    /// Nowhere to write, so nothing is kept. Used by tests and by anything that must
    /// not touch the disk.
    pub fn disabled() -> Self {
        Self {
            path: PathBuf::new(),
            document: Document::default(),
            dirty: false,
        }
    }

    pub fn sha_for_job(&self, job_id: u64) -> Option<&ResolvedSha> {
        self.document.shas.get(&job_id.to_string())
    }

    /// Only call for a completed run and an exact resolution.
    pub fn remember_sha(&mut self, job_id: u64, sha: &str, warning: &str) {
        if self.path.as_os_str().is_empty() || sha.is_empty() {
            return;
        }
        let key = job_id.to_string();
        if self.document.shas.contains_key(&key) {
            return;
        }
        self.document.shas.insert(
            key.clone(),
            ResolvedSha {
                sha: sha.to_string(),
                warning: warning.to_string(),
            },
        );
        self.document.order.push(key);
        self.dirty = true;
    }

    pub fn save(&mut self) {
        if !self.dirty || self.path.as_os_str().is_empty() {
            return;
        }
        self.prune();
        self.document.version = VERSION;

        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(&self.document) {
            let _ = std::fs::write(&self.path, text);
        }
        self.dirty = false;
    }

    fn prune(&mut self) {
        while self.document.order.len() > MAX_ENTRIES {
            let oldest = self.document.order.remove(0);
            self.document.shas.remove(&oldest);
        }
    }

    pub fn len(&self) -> usize {
        self.document.shas.len()
    }

    pub fn is_empty(&self) -> bool {
        self.document.shas.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_cache_keeps_nothing() {
        let mut cache = Cache::disabled();
        cache.remember_sha(1, "a".repeat(40).as_str(), "");
        assert!(cache.sha_for_job(1).is_none());
        cache.save();
    }

    #[test]
    fn the_first_answer_for_a_job_wins() {
        let mut cache = Cache::disabled();
        cache.path = std::env::temp_dir().join("deplyd-cache-test-unused.json");
        cache.remember_sha(1, "a", "");
        cache.remember_sha(1, "b", "");
        assert_eq!(cache.sha_for_job(1).map(|r| r.sha.as_str()), Some("a"));
    }

    #[test]
    fn pruning_drops_the_oldest_first() {
        let mut cache = Cache::disabled();
        cache.path = std::env::temp_dir().join("deplyd-cache-test-unused.json");
        for id in 0..(MAX_ENTRIES as u64 + 10) {
            cache.remember_sha(id, "sha", "");
        }
        cache.prune();
        assert_eq!(cache.len(), MAX_ENTRIES);
        assert!(cache.sha_for_job(0).is_none(), "oldest gone");
        assert!(
            cache.sha_for_job(MAX_ENTRIES as u64 + 9).is_some(),
            "newest kept"
        );
    }
}
