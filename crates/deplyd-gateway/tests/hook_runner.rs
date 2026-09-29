//! What a hook may do to deplyd, and what it may not.
//!
//! A hook is the user's own script, so it is the one thing here deplyd cannot
//! reason about. These cover the ways a badly behaved one could take the watcher
//! with it.

use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use deplyd_gateway::hook;

/// Every script these tests run, written in one go before any of them starts
/// a process.
///
/// Not laziness per test: on Linux a file cannot be executed while something
/// holds it open for writing, and a child one test is starting inherits, for
/// the moment between fork and exec, whatever another test's thread has open.
/// Written by test A while test B spawns, a script came back "Text file busy"
/// once in CI and not the next hundred times. Writing them all under one lock
/// before the first spawn leaves no write for a fork to overlap.
struct Scripts {
    reads_stdin: PathBuf,
    /// What `reads_stdin` copies its input into.
    seen: PathBuf,
    hangs: PathBuf,
    chatty: PathBuf,
}

static SCRIPTS: LazyLock<Scripts> = LazyLock::new(|| {
    let seen = std::env::temp_dir().join(format!("deplyd-hook-seen-{}.txt", std::process::id()));
    Scripts {
        reads_stdin: script(
            "reads-stdin",
            &format!("cat > '{}'", seen.display()),
            &format!(
                "[Console]::In.ReadToEnd() | Set-Content -Encoding utf8 '{}'",
                seen.display()
            ),
        ),
        seen,
        hangs: script("hangs", "sleep 30", "Start-Sleep -Seconds 30"),
        chatty: script(
            "chatty",
            "i=0; while [ $i -lt 4000 ]; do echo 'a line of complaint' >&2; i=$((i+1)); done",
            "1..4000 | ForEach-Object { [Console]::Error.WriteLine('a line of complaint') }",
        ),
    }
});

fn scratch(name: &str, body: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("deplyd-hook-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("scratch");
    let path = directory.join(name);
    std::fs::write(&path, body).expect("write");
    path
}

/// A script every platform in CI can run: sh on unix, powershell on Windows.
fn script(name: &str, sh: &str, ps: &str) -> PathBuf {
    if cfg!(windows) {
        scratch(&format!("{name}.ps1"), ps)
    } else {
        let path = scratch(&format!("{name}.sh"), &format!("#!/bin/sh\n{sh}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
        path
    }
}

#[test]
fn a_hook_is_handed_the_payload_on_stdin() {
    let outcome = hook::run(
        &SCRIPTS.reads_stdin,
        "{\"kind\":\"deploy.succeeded\"}",
        hook::DEFAULT_TIMEOUT,
    )
    .expect("it should run");
    assert!(outcome.ok(), "stderr: {}", outcome.stderr);

    let written = std::fs::read_to_string(&SCRIPTS.seen).expect("the hook should have written it");
    assert!(
        written.contains("deploy.succeeded"),
        "the hook should have been given the event, saw: {written:?}"
    );
}

#[test]
fn a_hook_that_never_finishes_is_stopped() {
    // Otherwise a watcher stops watching the first time a hook waits on
    // something, which is the failure nobody would attribute to the hook.
    let started = Instant::now();
    let error =
        hook::run(&SCRIPTS.hangs, "{}", Duration::from_secs(2)).expect_err("it should be stopped");

    assert!(
        matches!(error, hook::HookError::TimedOut { .. }),
        "got: {error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "it should be killed near the timeout, took {:?}",
        started.elapsed()
    );
}

#[test]
fn a_hook_that_says_a_great_deal_does_not_wedge_us() {
    // A pipe nobody drains fills and blocks the writer. Read after the wait
    // rather than during it, this hangs forever instead of finishing.
    let outcome = hook::run(&SCRIPTS.chatty, "{}", Duration::from_secs(60))
        .expect("it should finish, not hang");
    assert!(
        outcome.ok(),
        "it exited badly: {}",
        outcome.code.unwrap_or(-1)
    );
    assert!(
        outcome.stderr.len() > 10_000,
        "all of it should have been read, got {} bytes",
        outcome.stderr.len()
    );
}

#[test]
fn a_hook_that_is_not_there_is_named_rather_than_ignored() {
    let missing = std::env::temp_dir().join("deplyd-no-such-hook.sh");
    let error = hook::run(&missing, "{}", hook::DEFAULT_TIMEOUT).expect_err("no such file");
    assert!(matches!(error, hook::HookError::Missing(_)), "got: {error}");
}
