//! Proof that the git gateway refuses what it should.
//!
//! Every test here asserts on a *refusal*. None of them runs the call to find out
//! what would happen, and nothing in this file touches a repository, a remote or the
//! network. A guard tested by performing the dangerous operation is the accident the
//! guard exists to prevent.
//!
//! The strongest cases are not here at all, because they cannot be written: there is
//! no `Verb::Push`, so `git push` is not a refusal, it is a compile error. What
//! remains to test is the second layer - a permitted verb handed a writing argument.

use deplyd_gateway::git::{ReadOnlyGit, Verb};

fn refusal(verb: Verb, args: &[&str]) -> String {
    match ReadOnlyGit::new(verb, args) {
        Ok(allowed) => panic!(
            "gateway allowed a write: {}\nit should have been refused",
            allowed.rendered()
        ),
        Err(denied) => denied.reason,
    }
}

fn allowed(verb: Verb, args: &[&str]) -> ReadOnlyGit {
    ReadOnlyGit::new(verb, args).unwrap_or_else(|denied| panic!("gateway refused a read: {denied}"))
}

#[test]
fn config_reading_is_allowed() {
    let call = allowed(Verb::Config, &["user.name"]);
    assert_eq!(call.rendered(), "git config user.name");
}

#[test]
fn config_assigning_a_value_is_refused() {
    let reason = refusal(Verb::Config, &["user.name", "Mallory"]);
    assert!(
        reason.contains("would write"),
        "unexpected reason: {reason}"
    );
}

#[test]
fn config_write_flags_are_refused() {
    for flag in [
        "--add",
        "--unset",
        "--unset-all",
        "--replace-all",
        "--edit",
        "--rename-section",
        "--remove-section",
    ] {
        let reason = refusal(Verb::Config, &[flag, "some.key"]);
        assert!(
            reason.contains("writes") || reason.contains("would write"),
            "{flag} gave: {reason}"
        );
    }
}

#[test]
fn fetch_takes_only_its_known_shape() {
    allowed(Verb::Fetch, &["origin", "--quiet"]);
}

#[test]
fn fetch_prune_is_refused() {
    // --prune deletes local remote-tracking refs, which is the one way fetch can
    // destroy something the user had.
    let reason = refusal(Verb::Fetch, &["origin", "--prune"]);
    assert!(reason.contains("--prune"), "unexpected reason: {reason}");
}

#[test]
fn fetch_refspec_that_would_overwrite_branches_is_refused() {
    let reason = refusal(Verb::Fetch, &["origin", "+refs/heads/*:refs/heads/*"]);
    assert!(reason.contains("rewrite"), "unexpected reason: {reason}");
}

#[test]
fn fetch_of_another_remote_is_refused() {
    // Not because another remote is dangerous in itself, but because the allowlist
    // is by exact spelling: anything unrecognised stops rather than being guessed at.
    refusal(Verb::Fetch, &["some-other-remote"]);
}

#[test]
fn dash_c_config_injection_is_refused_on_every_verb() {
    // -c core.sshCommand / core.pager / alias.x=!sh all reach a shell, which would
    // put arbitrary execution behind a verb that looks like a read.
    for verb in [Verb::Log, Verb::Status, Verb::Fetch, Verb::RevParse] {
        let reason = refusal(verb, &["-c", "core.pager=sh -c 'echo pwned'"]);
        assert!(
            reason.contains("cannot see"),
            "{} gave: {reason}",
            verb.as_str()
        );
    }
}

#[test]
fn output_redirection_is_refused() {
    // --output writes a file of the caller's choosing, which is a write however
    // read-only the subcommand is.
    let reason = refusal(Verb::Diff, &["--output=/tmp/anywhere", "HEAD"]);
    assert!(reason.contains("cannot see"), "unexpected reason: {reason}");
}

#[test]
fn external_diff_and_exec_path_are_refused() {
    for arg in [
        "--ext-diff",
        "--exec-path=/tmp/fake-git",
        "--upload-pack=sh",
    ] {
        refusal(Verb::Diff, &[arg]);
    }
}

#[test]
fn git_dir_redirection_is_refused() {
    // Pointing git at another repository sidesteps every assumption the caller made
    // about which repository is being read.
    refusal(Verb::Log, &["--git-dir=/elsewhere/.git"]);
}

#[test]
fn ordinary_reads_still_pass() {
    allowed(Verb::RevParse, &["--show-toplevel"]);
    allowed(Verb::Log, &["-1", "--format=%H", "HEAD"]);
    allowed(Verb::MergeBase, &["--is-ancestor", "abc123", "def456"]);
    allowed(Verb::Cherry, &["main", "feature"]);
    allowed(Verb::LsFiles, &["--error-unmatch", "--", ".deplyd.json"]);
    allowed(Verb::Shortlog, &["-sn", "--no-merges", "origin/main"]);
}

#[test]
fn rendered_never_loses_an_argument() {
    // The refusal messages quote this, and a call that under-reports what it would
    // run would make the audit trail a lie.
    let call = allowed(Verb::Log, &["-1", "--format=%H"]);
    assert_eq!(call.rendered(), "git log -1 --format=%H");
    assert_eq!(call.argv().len(), 3);
}
