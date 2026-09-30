//! Starting deplyd again, detached, so a watcher outlives the terminal.
//!
//! The child is deplyd itself with the same arguments minus the flag that asked
//! for this. Its output goes to a file, because the first question about a
//! watcher that stopped is what it last said.
//!
//! Detaching uses the safe half of each platform's API - creation flags on
//! Windows, a new process group on unix - since the workspace forbids `unsafe`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP. Together these give the child no
/// console and no share in the terminal's Ctrl-C.
#[cfg(windows)]
const DETACHED: u32 = 0x0000_0008 | 0x0000_0200;

#[derive(Debug)]
pub enum BackgroundError {
    NoExecutable(String),
    CouldNotOpenLog { path: PathBuf, reason: String },
    CouldNotStart(String),
}

impl std::fmt::Display for BackgroundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackgroundError::NoExecutable(reason) => {
                write!(f, "could not find deplyd itself: {reason}")
            }
            BackgroundError::CouldNotOpenLog { path, reason } => {
                write!(f, "could not open {}: {reason}", path.display())
            }
            BackgroundError::CouldNotStart(reason) => write!(f, "could not start it: {reason}"),
        }
    }
}

impl std::error::Error for BackgroundError {}

/// Starts deplyd detached with `args`, writing what it says to `log`. The pid
/// it returns is for reporting, not for control.
#[allow(clippy::disallowed_types)] // the gateway is the only place a process starts
pub fn respawn(args: &[String], log: &Path) -> Result<u32, BackgroundError> {
    let exe = std::env::current_exe()
        .map_err(|error| BackgroundError::NoExecutable(error.to_string()))?;

    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent).map_err(|error| BackgroundError::CouldNotOpenLog {
            path: log.to_path_buf(),
            reason: error.to_string(),
        })?;
    }
    // Appended: a watcher restarted after a crash should not erase the evidence.
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|error| BackgroundError::CouldNotOpenLog {
            path: log.to_path_buf(),
            reason: error.to_string(),
        })?;
    let errors = out
        .try_clone()
        .map_err(|error| BackgroundError::CouldNotOpenLog {
            path: log.to_path_buf(),
            reason: error.to_string(),
        })?;

    let mut command = std::process::Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(errors));

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(DETACHED);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so closing the terminal does not take it too.
        command.process_group(0);
    }

    command
        .spawn()
        .map(|child| child.id())
        .map_err(|error| BackgroundError::CouldNotStart(error.to_string()))
}
