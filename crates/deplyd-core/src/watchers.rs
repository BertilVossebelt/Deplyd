//! Watchers running in the background, and the file each one leaves behind.
//!
//! Stopping is a request written into the file, not a signal: the watcher reads
//! it on its next look and exits, which costs up to one interval and reaches
//! into no other process. Liveness is a heartbeat, since a pid says nothing once
//! the number has been handed on.
//!
//! A stopped watcher's record stays, so `watch log` still answers. The one thing
//! deplyd removes anywhere is a finished record over a month old and not among
//! the newest ten - see [`tidy`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Long enough to survive a slow look, short enough to notice a crash.
const MISSED_BEFORE_GONE: u32 = 3;

/// Finished records kept however old they are, so the last few logs stay
/// readable.
pub const KEEP_FINISHED: usize = 10;

/// How long a finished record is kept past those, in seconds. Thirty days.
pub const KEEP_FOR: i64 = 30 * 24 * 60 * 60;

/// The shortest grace, whatever the interval. Three times ten seconds does not
/// survive one hook taking its full timeout, and a watcher wrongly called lost
/// is one `request_stop` then refuses to stop.
const SHORTEST_GRACE: i64 = 90;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watcher {
    pub id: String,
    /// Informational. Liveness comes from the heartbeat, not from this.
    pub pid: u32,
    pub repo: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub environment: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    pub every_secs: u64,
    pub started_at: i64,
    /// Bumped every look, which is what says it is still going.
    pub last_seen: i64,
    /// Set by `watchers stop`, read by the watcher itself.
    #[serde(default)]
    pub stop_requested: bool,
    #[serde(default)]
    pub stopped_at: Option<i64>,
    pub log: String,
}

/// What a watcher is doing, as far as its file can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Stopping,
    Stopped,
    /// Heartbeat went quiet without a clean stop.
    Lost,
}

impl Watcher {
    pub fn state(&self) -> State {
        if self.stopped_at.is_some() {
            return State::Stopped;
        }
        let quiet_for = now().saturating_sub(self.last_seen);
        let allowed =
            ((self.every_secs * u64::from(MISSED_BEFORE_GONE)) as i64).max(SHORTEST_GRACE);
        if quiet_for > allowed {
            return State::Lost;
        }
        if self.stop_requested {
            return State::Stopping;
        }
        State::Running
    }

    /// Still worth showing as something you could stop.
    pub fn is_live(&self) -> bool {
        matches!(self.state(), State::Running | State::Stopping)
    }

    pub fn path(&self) -> PathBuf {
        directory().join(format!("{}.json", self.id))
    }

    pub fn save(&self) -> std::io::Result<()> {
        let directory = directory();
        std::fs::create_dir_all(&directory)?;
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        std::fs::write(self.path(), text)
    }

    /// Takes on whatever another process wrote since this one loaded the record,
    /// and stamps the time. Writing back what we remember instead would wipe the
    /// very stop request we are about to look for.
    pub fn absorb(&mut self, on_disk: Option<&Watcher>, at: i64) {
        if let Some(fresh) = on_disk {
            self.stop_requested = fresh.stop_requested;
            self.stopped_at = fresh.stopped_at;
        }
        self.last_seen = at;
    }

    /// Stamps the file and says whether someone has asked it to stop. Read
    /// before the write: the other order only ever sees what we just wrote.
    pub fn beat(&mut self) -> bool {
        let on_disk = load(&self.path());
        self.absorb(on_disk.as_ref(), now());
        let _ = self.save();
        self.stop_requested
    }

    /// Marks it finished. The record stays, until `tidy` decides otherwise.
    pub fn mark_stopped(&mut self) {
        self.stopped_at = Some(now());
        let _ = self.save();
    }

    /// When it finished, as far as the record can say: when it stopped, or the
    /// last heartbeat of one that went quiet.
    pub fn finished_at(&self) -> i64 {
        self.stopped_at.unwrap_or(self.last_seen)
    }
}

/// The records that have served their purpose: finished, not among the newest
/// `KEEP_FINISHED` finished ones, and finished more than `KEEP_FOR` ago. A live
/// one is never named, however old its file. Pure, so the rule can be tested
/// without a config directory.
pub fn stale(held: &[Watcher], at: i64) -> Vec<&Watcher> {
    let mut finished: Vec<&Watcher> = held.iter().filter(|w| !w.is_live()).collect();
    finished.sort_by_key(|w| std::cmp::Reverse(w.finished_at()));
    finished
        .into_iter()
        .skip(KEEP_FINISHED)
        .filter(|w| at.saturating_sub(w.finished_at()) > KEEP_FOR)
        .collect()
}

/// Removes the stale records and their logs, and says how many records went.
///
/// Housekeeping, so a file that will not go is skipped rather than fatal. The
/// log is named from the id rather than read from the record, so what is
/// removed is always beside the record and never wherever the record says.
pub fn tidy() -> usize {
    let directory = directory();
    let held = all();
    let mut removed = 0;
    for watcher in stale(&held, now()) {
        let log = directory.join(format!("{}.log", watcher.id));
        let _ = crate::gateway::tidy::remove_watcher_file(&directory, &log);
        if crate::gateway::tidy::remove_watcher_file(&directory, &watcher.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// Where the records live, beside the other things deplyd keeps for itself.
pub fn directory() -> PathBuf {
    crate::settings::config_directory().join("watchers")
}

pub fn load(path: &Path) -> Option<Watcher> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Every record, newest first. One bad file should not hide the rest.
pub fn all() -> Vec<Watcher> {
    let Ok(entries) = std::fs::read_dir(directory()) else {
        return Vec::new();
    };
    let mut found: Vec<Watcher> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .filter_map(|path| load(&path))
        .collect();
    found.sort_by_key(|watcher| std::cmp::Reverse(watcher.started_at));
    found
}

pub fn live() -> Vec<Watcher> {
    all().into_iter().filter(Watcher::is_live).collect()
}

/// Finds one by id, or by a unique prefix of it, the way git takes a short sha.
pub fn find(id: &str) -> Result<Watcher, String> {
    let held = all();
    if let Some(exact) = held.iter().find(|watcher| watcher.id == id) {
        return Ok(exact.clone());
    }
    let mut matching = held.iter().filter(|watcher| watcher.id.starts_with(id));
    let Some(first) = matching.next() else {
        return Err(format!("No watcher here called {id}."));
    };
    match matching.next() {
        None => Ok(first.clone()),
        Some(_) => Err(format!("{id} names more than one watcher.")),
    }
}

/// Asks a watcher to stop. It notices on its next look, not now.
pub fn request_stop(id: &str) -> Result<Watcher, String> {
    let mut watcher = find(id)?;
    if !watcher.is_live() {
        return Err(format!("{} is not running.", watcher.id));
    }
    watcher.stop_requested = true;
    watcher
        .save()
        .map_err(|error| format!("could not write to {}: {error}", watcher.path().display()))?;
    Ok(watcher)
}

/// Time and pid together: two watchers started in the same second differ.
pub fn new_id(pid: u32) -> String {
    format!("{:x}{:04x}", now(), pid & 0xffff)
}
