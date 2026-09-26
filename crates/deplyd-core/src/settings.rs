//! Everything deplyd writes, and the corrections it reads. Nothing lands in your
//! working tree unless `.deplyd.json` is already there.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where deplyd keeps its own files. A binary cannot write beside itself, so this
/// is the platform's config directory; `deplyd check` prints it.
pub fn config_directory() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);

    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));

    base.unwrap_or_else(std::env::temp_dir).join("deplyd")
}

pub fn settings_path() -> PathBuf {
    config_directory().join("deplyd.settings.json")
}

/// Defaults kept by `deplyd remember`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_path: Option<String>,
}

impl Settings {
    pub fn load() -> Self {
        let path = settings_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<PathBuf> {
        let path = settings_path();
        write_json(&path, self)?;
        Ok(path)
    }
}

/// One environment's corrections. An explicit workflow list always wins.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EnvironmentOverride {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub workflows: Vec<String>,
}

/// `.deplyd.json`. Every key optional; between them they cover what detection
/// misses.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Override {
    /// Matched against workflow file name and `name:` to decide what deploys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deploy_pattern: Option<String>,
    /// Replaces the detected environment list entirely.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub environments: BTreeMap<String, EnvironmentOverride>,
    /// Substrings of a job name that keep it from being a target.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignore_jobs: Vec<String>,
    /// Which paths a target covers, when its job sets no `working-directory`.
    /// Keyed by the label deplyd reports.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub scopes: BTreeMap<String, Vec<String>>,
}

impl Override {
    /// `Ok(None)` when there is no file, `Err` when there is one that will not parse.
    /// Ignoring a broken one would drop the corrections it exists to hold and say
    /// nothing about it.
    pub fn load(path: &Path) -> Result<Option<Self>, String> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(None);
        };
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|error| error.to_string())
    }
}

/// Where this repository's override lives, and whether it is the shared one.
pub struct OverrideLocation {
    pub path: PathBuf,
    /// True when it is `.deplyd.json` at the repository root: someone put it there
    /// on purpose, and it is the one every teammate sees.
    pub shared: bool,
}

pub fn override_location(repo_root: &Path) -> OverrideLocation {
    let shared = repo_root.join(".deplyd.json");
    if shared.exists() {
        return OverrideLocation {
            path: shared,
            shared: true,
        };
    }
    OverrideLocation {
        path: private_override_path(repo_root),
        shared: false,
    }
}

/// Named after the repository plus a short hash of its path, so two clones sharing
/// a name stay apart. The path is normalised first: git answers in forward slashes
/// while `--repo-path` arrives in backslashes. The hash is not cryptographic.
fn private_override_path(repo_root: &Path) -> PathBuf {
    let mut full = repo_root.to_string_lossy().replace('\\', "/");
    while full.ends_with('/') {
        full.pop();
    }
    if cfg!(windows) {
        full = full.to_lowercase();
    }

    let digest = fnv1a(full.as_bytes());
    let leaf: String = Path::new(&full)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "repo".into())
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();

    config_directory()
        .join("overrides")
        .join(format!("{leaf}-{digest:08x}.json"))
}

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Writes pretty JSON with a trailing newline, and the same bytes everywhere.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    // A line feed, not the platform's terminator: the body is joined with line feeds,
    // so a platform-dependent ending would put a difference in the last byte alone.
    text.push('\n');
    std::fs::write(path, text)
}
