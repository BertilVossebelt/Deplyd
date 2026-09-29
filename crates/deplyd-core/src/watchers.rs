//! Watchers running in the background, and the file each one leaves behind.
//!
//! Two of deplyd's own rules shape this more than anything else.
//!
//! It never deletes, so stopping a watcher does not remove its record - the
//! record is marked stopped and stays. `watchers` shows the running ones;
//! nothing is ever quietly gone.
//!
//! It does not kill processes either. Stopping is a request written into the
//! file, which the watcher reads on its next look and then exits. That costs up
//! to one interval, and buys not needing to reach into another process at all.
//!
//! Liveness is a heartbeat rather than a pid check: a watcher stamps the file
//! every look, and one that has missed several is presumed gone. A pid says
//! nothing useful anyway once the number has been handed to something else.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Missed heartbeats before a watcher is presumed gone. Three intervals is long
/// enough to survive a slow look and short enough to notice a crash.
const MISSED_BEFORE_GONE: u32 = 3;

/// The shortest grace, whatever the interval.
///
/// Three times ten seconds is not long enough to survive one slow request, let
/// alone a hook taking its full timeout. Being wrongly called lost is not a
/// cosmetic error: `request_stop` refuses anything that is not live, so a
/// healthy watcher would become unstoppable.
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

    /// Takes on whatever another process has written since this one loaded the
    /// record, and stamps the time.
    ///
    /// Split out from the writing so it can be tested without a config
    /// directory, because this is the whole of why stopping works: writing back
    /// what we remember would wipe the very request we are about to look for.
    pub fn absorb(&mut self, on_disk: Option<&Watcher>, at: i64) {
        if let Some(fresh) = on_disk {
            self.stop_requested = fresh.stop_requested;
            self.stopped_at = fresh.stopped_at;
        }
        self.last_seen = at;
    }

    /// Stamps the file so `watchers` can tell it is still going, and says
    /// whether someone has asked it to stop. One look at the file, not two:
    /// reading after writing would only ever see what we just wrote.
    pub fn beat(&mut self) -> bool {
        let on_disk = load(&self.path());
        self.absorb(on_disk.as_ref(), now());
        let _ = self.save();
        self.stop_requested
    }

    /// Marks it finished. The record stays; deplyd does not delete.
    pub fn mark_stopped(&mut self) {
        self.stopped_at = Some(now());
        let _ = self.save();
    }
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

/// Every record, newest first. Unreadable ones are skipped rather than fatal:
/// one bad file should not hide the rest.
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

/// Asks a watcher to stop. It notices on its next look, which is why this says
/// how long that could be rather than pretending it is immediate.
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

/// A short, unique-enough name. Time and pid together, which cannot collide on
/// one machine: two watchers started in the same second have different pids.
pub fn new_id(pid: u32) -> String {
    format!("{:x}{:04x}", now(), pid & 0xffff)
}
