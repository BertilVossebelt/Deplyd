//! Running a hook: the one place deplyd starts a program it did not choose.
//!
//! Everything else here is a fixed verb against `git` or `gh`. A hook is the
//! user's own script, so the contract is narrow on purpose:
//!
//! - the script is executed directly, never through a shell, so nothing in the
//!   payload can become a command;
//! - the payload goes on stdin, never in the arguments or the environment, both
//!   of which are visible to every other process on the machine;
//! - a hook that hangs is killed, because a watcher that stops watching while it
//!   waits is worse than a hook that does not finish;
//! - a hook that fails is reported, not fatal. It is a notification, not a step.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

/// How long a hook gets before it is killed.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum HookError {
    Missing(PathBuf),
    CouldNotStart { path: PathBuf, reason: String },
    TimedOut { path: PathBuf, after: Duration },
}

impl std::fmt::Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookError::Missing(path) => write!(f, "no such hook: {}", path.display()),
            HookError::CouldNotStart { path, reason } => {
                write!(f, "could not start {}: {reason}", path.display())
            }
            HookError::TimedOut { path, after } => write!(
                f,
                "{} did not finish within {}s and was stopped",
                path.display(),
                after.as_secs()
            ),
        }
    }
}

impl std::error::Error for HookError {}

/// What running one hook came to.
#[derive(Debug)]
pub struct Outcome {
    pub code: Option<i32>,
    pub stderr: String,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// A `.ps1` cannot be executed directly on Windows and a `.sh` is not executable
/// there either, so those name an interpreter - chosen from the extension, never
/// from the payload. Anything else is run as itself and the OS decides.
fn program_for(path: &Path) -> (String, Vec<String>) {
    let script = path.display().to_string();
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("ps1") => (
            "powershell".into(),
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-File".into(),
                script,
            ],
        ),
        Some("sh") if cfg!(windows) => ("sh".into(), vec![script]),
        _ => (script, Vec::new()),
    }
}

/// Starts the hook, hands it `payload` on stdin, and waits for it.
#[allow(clippy::disallowed_types)] // the gateway is the only place a process starts
pub fn run(path: &Path, payload: &str, timeout: Duration) -> Result<Outcome, HookError> {
    if !path.is_file() {
        return Err(HookError::Missing(path.to_path_buf()));
    }

    let (program, args) = program_for(path);
    let mut child = std::process::Command::new(&program)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| HookError::CouldNotStart {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;

    // Dropped after writing, so the hook sees end-of-input. One event is far
    // smaller than a pipe buffer, so this cannot block.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
    }

    // Drained on its own thread: a pipe nobody reads fills and blocks the
    // writer, so a chatty hook would hang before the timeout could catch it.
    let draining = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut said = String::new();
            let _ = pipe.read_to_string(&mut said);
            said
        })
    });
    let collect = |handle: Option<std::thread::JoinHandle<String>>| {
        handle
            .and_then(|h| h.join().ok())
            .unwrap_or_default()
            .trim()
            .to_string()
    };

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(Outcome {
                    code: status.code(),
                    stderr: collect(draining),
                });
            }
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                // Joined after the kill, so no thread holds a dead pipe.
                let _ = collect(draining);
                return Err(HookError::TimedOut {
                    path: path.to_path_buf(),
                    after: timeout,
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                let _ = collect(draining);
                return Err(HookError::CouldNotStart {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                });
            }
        }
    }
}
