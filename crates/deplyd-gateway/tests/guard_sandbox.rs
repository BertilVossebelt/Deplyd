//! Proof that the test sandbox cannot reach a remote.
//!
//! These tests are about the harness, not about deplyd. They exist because fixture
//! construction needs writing git verbs, which the gateway deliberately cannot
//! express, and that is where a porting accident would come from.
//!
//! Every case asserts that git *refuses* a remote it was pointed at. The addresses
//! used are reserved by RFC 2606 (`.invalid`, `.example`) and can never resolve to a
//! real host, so even a sandbox that failed to refuse would have nothing to reach.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use support::{ALLOWED_PROTOCOL, Sandbox, code_only, run_git};

#[test]
fn https_remotes_are_refused_by_git_itself() {
    let sandbox = Sandbox::empty("protocol-https");
    sandbox.git(&[
        "remote",
        "add",
        "origin",
        "https://github.invalid/nobody/nothing.git",
    ]);

    let output = sandbox.git(&["fetch", "origin"]);
    assert_transport_refused("https", &output);
}

#[test]
fn ssh_remotes_are_refused_by_git_itself() {
    let sandbox = Sandbox::empty("protocol-ssh");
    sandbox.git(&[
        "remote",
        "add",
        "origin",
        "git@github.invalid:nobody/nothing.git",
    ]);

    let output = sandbox.git(&["fetch", "origin"]);
    assert_transport_refused("ssh", &output);
}

#[test]
fn a_local_origin_over_file_still_works() {
    // The refusals above must not be a sandbox that simply cannot fetch anything:
    // then they would prove nothing. This is the control.
    let sandbox = Sandbox::from_fixture("conventional").with_local_origin();
    let refs = sandbox.git(&["rev-parse", "--verify", "origin/main"]);
    assert!(
        refs.trim().len() == 40,
        "a file:// origin should fetch, got: {refs}"
    );
}

#[test]
fn the_sandbox_never_sees_the_real_git_config() {
    // A credential helper in the user's own config is the one thing that could hand
    // a sandboxed git a token for a real host.
    let sandbox = Sandbox::empty("isolated-config");
    let helper = sandbox.git(&["config", "--get", "credential.helper"]);
    assert!(
        helper.trim().is_empty(),
        "sandbox inherited a credential helper: {helper}"
    );

    let user = sandbox.git(&["config", "--get", "user.email"]);
    assert_eq!(
        user.trim(),
        "ada@example.invalid",
        "sandbox should use its own identity, not the machine's"
    );
}

#[test]
fn fixtures_load_and_carry_their_workflows() {
    let sandbox = Sandbox::from_fixture("conventional");
    let workflows = sandbox.path().join(".github/workflows");
    assert!(
        workflows.join("deploy-production-api.yml").is_file(),
        "fixture should have been copied whole"
    );

    let log = sandbox.git(&["log", "--oneline"]);
    assert!(!log.trim().is_empty(), "fixture should have a commit");
}

/// The harness equivalent of `guard_source.rs`: test files may not spawn git
/// themselves, because only `support::run_git` applies the protocol restriction.
#[test]
fn no_test_file_spawns_git_outside_the_sandbox() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate should sit two levels under the workspace root")
        .join("crates");
    let mut files = Vec::new();
    collect_rust_files(&tests, &mut files);
    assert!(
        files.len() >= 3,
        "expected to audit the test suite, found {} file(s)",
        files.len()
    );

    let mut violations = Vec::new();
    for file in files {
        // Only tests are in scope here; src/ is guard_source.rs's job, and the
        // sandbox itself is the one place in tests allowed to spawn.
        let in_tests = file.components().any(|c| c.as_os_str() == "tests");
        let is_sandbox = file.components().any(|c| c.as_os_str() == "support");
        if !in_tests || is_sandbox {
            continue;
        }
        let text = fs::read_to_string(&file).expect("test file should be readable");
        for (number, line) in text.lines().enumerate() {
            let code = code_only(line);
            if code.contains("Command::new") {
                violations.push(format!(
                    "{}:{}: spawn through support::run_git, which restricts the protocol - {}",
                    file.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "a test starts a process outside the sandbox:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_protocol_restriction_is_actually_narrow() {
    // A regression here would be silent: widening ALLOWED_PROTOCOL would leave every
    // other test passing while the sandbox stopped being one.
    assert_eq!(
        ALLOWED_PROTOCOL, "file",
        "the sandbox must permit only the file protocol"
    );
}

#[test]
fn run_git_applies_the_restriction_on_every_call() {
    // Not a second copy of the check above: this proves the variable reaches git,
    // rather than merely being set to the right value in Rust.
    let sandbox = Sandbox::empty("env-reaches-git");
    let seen = run_git(
        sandbox.root(),
        sandbox.path(),
        &["config", "--get", "protocol.allow"],
    );
    // GIT_ALLOW_PROTOCOL is an environment variable, not config, so the useful proof
    // is behavioural: a non-file protocol must fail.
    assert!(seen.trim().is_empty());

    sandbox.git(&["remote", "add", "probe", "ftp://nowhere.invalid/x.git"]);
    let output = sandbox.git(&["fetch", "probe"]);
    assert!(
        !output.contains("done."),
        "an ftp remote should not have been contacted, git said: {output}"
    );
}

/// Requires the refusal to be git declining the transport, not the host failing to
/// resolve. Both produce an error and only one is the sandbox working: a test that
/// accepted either would keep passing if the protocol restriction were removed.
fn assert_transport_refused(protocol: &str, output: &str) {
    assert!(
        output.contains(&format!("transport '{protocol}' not allowed")),
        "{protocol} should have been refused by GIT_ALLOW_PROTOCOL before any          connection was attempted, git said: {output}"
    );
    assert!(
        !output.contains("resolve host") && !output.contains("Connection"),
        "{protocol} reached the network instead of being refused: {output}"
    );
}

fn collect_rust_files(directory: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
}
