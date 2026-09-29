//! Removing a finished watcher's record and log: the one place deplyd deletes.
//!
//! Everything else deplyd writes, it keeps. These two files are the exception
//! because nothing else bounds them: every background watch leaves a record
//! and a log behind, and a machine that has watched for a year has hundreds,
//! of which the newest few are the only ones anyone looks at.
//!
//! The contract is as narrow as the hook runner's. The directory must be the
//! one deplyd keeps for watchers - the last two parts of its path are `deplyd`
//! and `watchers` - the file must sit directly inside it, and the file must be
//! a record or a log by name. Anything else is refused before the filesystem
//! is touched, so a path that arrives here by mistake cannot take something
//! that is not deplyd's own.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TidyError {
    /// The directory is not one deplyd keeps for itself.
    NotOurs(PathBuf),
    /// Not directly inside the directory it was supposed to be in.
    Outside {
        path: PathBuf,
        directory: PathBuf,
    },
    /// Not a record or a log.
    NotARecord(PathBuf),
    CouldNotRemove {
        path: PathBuf,
        reason: String,
    },
}

impl std::fmt::Display for TidyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TidyError::NotOurs(directory) => {
                write!(
                    f,
                    "{} is not deplyd's watchers directory",
                    directory.display()
                )
            }
            TidyError::Outside { path, directory } => write!(
                f,
                "{} is not directly inside {}",
                path.display(),
                directory.display()
            ),
            TidyError::NotARecord(path) => {
                write!(f, "{} is not a watcher record or log", path.display())
            }
            TidyError::CouldNotRemove { path, reason } => {
                write!(f, "could not remove {}: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for TidyError {}

/// Whether `directory` is a watchers directory of deplyd's own: `.../deplyd/watchers`.
pub fn is_watchers_directory(directory: &Path) -> bool {
    let mut parts = directory.components().rev();
    parts.next().is_some_and(|c| c.as_os_str() == "watchers")
        && parts.next().is_some_and(|c| c.as_os_str() == "deplyd")
}

/// A record or a log: a hex name with the one extension or the other.
fn is_record_or_log(path: &Path) -> bool {
    let named_like_one = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| !stem.is_empty() && stem.chars().all(|c| c.is_ascii_hexdigit()));
    let ends_like_one = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension == "json" || extension == "log");
    named_like_one && ends_like_one
}

/// Removes one record or log from `directory`, and nothing else.
///
/// `parent` is lexical, which is the point: a path that climbs out through
/// `..` has a parent that is not `directory`, and is refused for it.
#[allow(clippy::disallowed_methods)] // the gateway is the only place a file is removed
pub fn remove_watcher_file(directory: &Path, path: &Path) -> Result<(), TidyError> {
    if !is_watchers_directory(directory) {
        return Err(TidyError::NotOurs(directory.to_path_buf()));
    }
    if path.parent() != Some(directory) {
        return Err(TidyError::Outside {
            path: path.to_path_buf(),
            directory: directory.to_path_buf(),
        });
    }
    if !is_record_or_log(path) {
        return Err(TidyError::NotARecord(path.to_path_buf()));
    }
    std::fs::remove_file(path).map_err(|error| TidyError::CouldNotRemove {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}
