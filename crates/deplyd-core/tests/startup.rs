//! What gets written into each platform's startup folder.
//!
//! Two of the three can never be exercised on the machine running the tests, so
//! the shape is asserted: a bad plist or a unit with no [Install] section fails
//! silently at a login nobody is watching.

use deplyd_core::startup::{Platform, os_file};

const EXE: &str = "/usr/local/bin/deplyd";

#[test]
fn every_platform_calls_deplyd_back_with_the_id() {
    // The file runs `startup run <id>`, never the watch itself: that
    // indirection is what lets one be turned off without deleting anything.
    for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
        let (name, body) = os_file(platform, "abc123", EXE);
        assert!(
            body.contains("abc123"),
            "{platform:?} must pass the id along: {body}"
        );
        assert!(
            body.contains("watch") && body.contains("startup"),
            "{platform:?} must call watch startup run: {body}"
        );
        assert!(
            name.contains("abc123"),
            "{platform:?} should name the file after it: {name}"
        );
    }
}

#[test]
fn the_windows_file_is_a_cmd_that_opens_no_window() {
    let (name, body) = os_file(Platform::Windows, "abc123", r"C:\deplyd.exe");
    assert!(name.ends_with(".cmd"), "got {name}");
    assert!(body.contains("@echo off"), "got {body}");
    // /b is what keeps a console from flashing up at every login.
    assert!(body.contains("/b"), "it should start windowless: {body}");
    assert!(body.contains("\r\n"), "a .cmd wants CRLF: {body:?}");
}

#[test]
fn the_macos_file_is_a_plist_that_runs_at_load() {
    let (name, body) = os_file(Platform::MacOs, "abc123", EXE);
    assert!(name.ends_with(".plist"), "got {name}");
    assert!(body.starts_with("<?xml"), "got {body}");
    assert!(body.contains("<key>RunAtLoad</key>"), "got {body}");
    assert!(body.contains("com.deplyd.abc123"), "the label: {body}");

    // Each argument is its own <string>, or launchd runs one long filename.
    for piece in ["<string>startup</string>", "<string>run</string>"] {
        assert!(body.contains(piece), "missing {piece} in {body}");
    }
    assert_eq!(
        body.matches("<string>").count(),
        body.matches("</string>").count(),
        "tags must balance or the plist will not parse"
    );
}

#[test]
fn the_linux_file_is_a_unit_systemd_will_enable() {
    let (name, body) = os_file(Platform::Linux, "abc123", EXE);
    assert!(name.ends_with(".service"), "got {name}");
    for section in ["[Unit]", "[Service]", "[Install]"] {
        assert!(body.contains(section), "missing {section} in {body}");
    }
    // Without WantedBy there is nothing for the link to point at, and the unit
    // sits there enabled-looking but never started.
    assert!(
        body.contains("WantedBy=default.target"),
        "it would never start: {body}"
    );
    // ExecStart returns as soon as the watcher is started. Without these,
    // systemd reads that as the unit finishing and tears down the cgroup.
    for directive in ["RemainAfterExit=yes", "KillMode=process"] {
        assert!(
            body.contains(directive),
            "missing {directive}, so the watcher would be killed at boot: {body}"
        );
    }
    assert!(
        body.contains(&format!("ExecStart={EXE} watch startup run abc123")),
        "got {body}"
    );
}

#[test]
fn each_platform_looks_somewhere_different() {
    // Reading the same folder on two platforms would mean one of them is wrong.
    let paths: Vec<_> = [Platform::Windows, Platform::MacOs, Platform::Linux]
        .into_iter()
        .filter_map(|p| deplyd_core::startup::location_for(p).ok())
        .collect();
    for pair in paths.windows(2) {
        assert_ne!(pair[0], pair[1], "two platforms share a location");
    }
}

#[test]
fn two_registrations_in_one_second_are_still_two_different_entries() {
    // Time alone collides, and the loser's record is overwritten while its
    // startup file stays behind - running a watch nothing has a record of.
    let first = deplyd_core::startup::new_id(1000);
    let second = deplyd_core::startup::new_id(1001);
    assert_ne!(first, second, "the pid is what tells them apart");
}

#[test]
fn asking_for_the_same_watch_twice_finds_the_one_already_there() {
    use deplyd_core::startup::{Entry, matching};

    let entry = |id: &str, repo: &str, args: &[&str]| Entry {
        id: id.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        repo: repo.into(),
        enabled: true,
        created_at: 0,
        os_file: format!("/startup/deplyd-{id}.cmd"),
    };

    let held = vec![
        entry("aaa", "/repo", &["watch", "--anyone", "--background"]),
        entry("bbb", "/other", &["watch", "--anyone", "--background"]),
    ];

    let same: Vec<String> = ["watch", "--anyone", "--background"]
        .iter()
        .map(|a| a.to_string())
        .collect();

    // Asking again for what is already there finds it, so nothing new is
    // written into a startup folder deplyd cannot clean up afterwards.
    assert_eq!(
        matching(&held, "/repo", &same).map(|e| e.id.as_str()),
        Some("aaa")
    );

    // A different repo is a different watch, even with the same arguments.
    assert!(matching(&held, "/elsewhere", &same).is_none());

    // So is a different watch on the same repo.
    let narrower: Vec<String> = ["watch", "-E", "staging", "--background"]
        .iter()
        .map(|a| a.to_string())
        .collect();
    assert!(matching(&held, "/repo", &narrower).is_none());
}
