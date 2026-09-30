//! Watchers that come back when the machine does.
//!
//! Each platform's way in is a file: a `.cmd` in the Startup folder on Windows,
//! a LaunchAgent plist on macOS, a systemd user unit on Linux. No installer, no
//! privileges.
//!
//! The file runs `deplyd startup run <id>`, not the watch itself. deplyd never
//! deletes, so turning one off is a line in the record it consults rather than
//! a deletion nobody can undo.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// The watch as it was asked for, ready to hand back to deplyd.
    pub args: Vec<String>,
    pub repo: String,
    pub enabled: bool,
    pub created_at: i64,
    /// What was written, and where. The file stays, so it can be named for
    /// anyone who wants to remove it by hand.
    pub os_file: String,
}

impl Entry {
    pub fn path(&self) -> PathBuf {
        directory().join(format!("{}.json", self.id))
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(directory())?;
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        std::fs::write(self.path(), text)
    }
}

pub fn directory() -> PathBuf {
    crate::settings::config_directory().join("startup")
}

pub fn all() -> Vec<Entry> {
    let Ok(entries) = std::fs::read_dir(directory()) else {
        return Vec::new();
    };
    let mut found: Vec<Entry> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .filter_map(|path| serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok())
        .collect();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
    found
}

/// By id, or a unique prefix of it.
pub fn find(id: &str) -> Result<Entry, String> {
    let held = all();
    if let Some(exact) = held.iter().find(|entry| entry.id == id) {
        return Ok(exact.clone());
    }
    let mut matching = held.iter().filter(|entry| entry.id.starts_with(id));
    let Some(first) = matching.next() else {
        return Err(format!("Nothing starts at boot called {id}."));
    };
    match matching.next() {
        None => Ok(first.clone()),
        Some(_) => Err(format!("{id} names more than one.")),
    }
}

/// Turns one on or off. The file stays either way.
pub fn set_enabled(id: &str, enabled: bool) -> Result<Entry, String> {
    let mut entry = find(id)?;
    entry.enabled = enabled;
    entry
        .save()
        .map_err(|error| format!("could not write {}: {error}", entry.path().display()))?;
    Ok(entry)
}

/// Which of the three shapes to write. Passed rather than asked, so the other
/// two can be tested from whichever one you are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    pub fn here() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        }
    }
}

/// Where this platform looks for things to start at login.
pub fn os_location() -> Result<PathBuf, String> {
    location_for(Platform::here())
}

pub fn location_for(platform: Platform) -> Result<PathBuf, String> {
    if platform == Platform::Windows {
        let appdata = std::env::var_os("APPDATA").ok_or_else(|| {
            "APPDATA is not set, so the Startup folder cannot be found".to_string()
        })?;
        // A component at a time, so the path does not mix both slashes.
        return Ok(PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup"));
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| "HOME is not set, so there is nowhere to put it".to_string())?;
    let home = PathBuf::from(home);
    if platform == Platform::MacOs {
        return Ok(home.join("Library").join("LaunchAgents"));
    }
    Ok(home.join(".config").join("systemd").join("user"))
}

/// The file this platform wants, and what goes in it.
pub fn os_file(platform: Platform, id: &str, exe: &str) -> (String, String) {
    if platform == Platform::Windows {
        // Windowless, so logging in does not flash a console at you.
        return (
            format!("deplyd-{id}.cmd"),
            format!("@echo off\r\nstart \"\" /b \"{exe}\" watch startup run {id}\r\n"),
        );
    }
    if platform == Platform::MacOs {
        return (
            format!("com.deplyd.{id}.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.deplyd.{id}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>watch</string>
    <string>startup</string>
    <string>run</string>
    <string>{id}</string>
  </array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#
            ),
        );
    }
    // RemainAfterExit and KillMode are load-bearing: ExecStart returns as soon
    // as the watcher is started, and systemd reads that as the unit having
    // finished, then tears the cgroup down with the watcher inside it.
    (
        format!("deplyd-{id}.service"),
        format!(
            "[Unit]\n\
             Description=deplyd watcher {id}\n\
             After=network-online.target\n\n\
             [Service]\n\
             Type=oneshot\n\
             RemainAfterExit=yes\n\
             KillMode=process\n\
             ExecStart={exe} watch startup run {id}\n\n\
             [Install]\n\
             WantedBy=default.target\n"
        ),
    )
}

/// Writes the platform's file and returns where it went. On Linux it also makes
/// the `default.target.wants` link `systemctl --user enable` would, so no
/// service manager has to be talked to.
pub fn install(id: &str, exe: &str) -> Result<PathBuf, String> {
    let platform = Platform::here();
    let directory = location_for(platform)?;
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not make {}: {error}", directory.display()))?;

    let (name, body) = os_file(platform, id, exe);
    let path = directory.join(&name);
    std::fs::write(&path, body)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let wants = directory.join("default.target.wants");
        if std::fs::create_dir_all(&wants).is_ok() {
            let link = wants.join(&name);
            // symlink_metadata, not exists: a link pointing at a unit that has
            // moved reads as absent, and creating it then fails as already
            // there - leaving it enabled-looking and never starting.
            if std::fs::symlink_metadata(&link).is_err() {
                let _ = std::os::unix::fs::symlink(&path, &link);
            }
        }
    }

    Ok(path)
}

/// Time and pid together. Time alone collides within a second, and the loser
/// leaves a startup file behind that runs a watch nothing knows about.
pub fn new_id(pid: u32) -> String {
    format!("{:x}{:04x}", crate::watchers::now(), pid & 0xffff)
}

/// An entry already asking for this exact watch on this exact repo. Without it
/// the startup folder collects one file per run of the command, each starting
/// its own watcher at every boot - and deplyd cannot delete them again.
pub fn already_registered(repo: &str, args: &[String]) -> Option<Entry> {
    matching(&all(), repo, args).cloned()
}

/// The same question against a list, so it can be tested without a config
/// directory.
pub fn matching<'a>(held: &'a [Entry], repo: &str, args: &[String]) -> Option<&'a Entry> {
    held.iter()
        .find(|entry| entry.repo == repo && entry.args == args)
}
