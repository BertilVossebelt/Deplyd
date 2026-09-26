//! The self-check is what the shipped binary runs to prove its own guard is intact.
//! These tests prove the self-check itself is worth running.
//!
//! The cases live in the library, not here, so `deplyd check` and this suite assert
//! exactly the same things. Two lists would drift the first time one was corrected.

use deplyd_gateway::git::Verb;
use deplyd_gateway::selfcheck::{self, EXPECTED_VERBS};

#[test]
fn a_clean_build_reports_itself_intact() {
    let results = selfcheck::run();
    let failures: Vec<_> = results.iter().filter(|r| !r.passed).collect();

    assert!(
        failures.is_empty(),
        "the guard in this build is not intact:\n{}",
        failures
            .iter()
            .map(|r| format!("  {}: {}", r.name, r.detail))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(selfcheck::is_intact());
    assert!(selfcheck::failures().is_empty());
}

#[test]
fn every_check_actually_ran() {
    // A self-check that silently checked nothing would pass just as well. Pin the
    // number so removing one is a failure rather than a quieter success.
    let results = selfcheck::run();
    assert_eq!(
        results.len(),
        4,
        "expected four checks, got {}: {:?}",
        results.len(),
        results.iter().map(|r| r.name).collect::<Vec<_>>()
    );
}

#[test]
fn the_verb_set_matches_what_the_check_expects() {
    // The compiler stops `Verb::Push` being *called*. This stops it being *added*
    // without the addition being stated in a second place, in the same diff.
    let actual: Vec<&str> = Verb::ALL.iter().map(|v| v.as_str()).collect();
    assert_eq!(
        actual, EXPECTED_VERBS,
        "the gateway's verb set changed. If that is deliberate, update EXPECTED_VERBS \
         in selfcheck.rs too - and be sure the new verb cannot write."
    );
}

#[test]
fn no_allowed_verb_is_a_writing_verb() {
    // Belt and braces against a plausible-looking addition. These are the verbs that
    // change a repository; none may ever appear in the gateway.
    const WRITES: &[&str] = &[
        "push",
        "commit",
        "reset",
        "checkout",
        "switch",
        "merge",
        "rebase",
        "clean",
        "rm",
        "mv",
        "add",
        "restore",
        "revert",
        "cherry-pick",
        "stash",
        "tag",
        "branch",
        "remote",
        "init",
        "clone",
        "apply",
        "am",
        "gc",
        "prune",
        "update-ref",
        "symbolic-ref",
        "worktree",
        "submodule",
        "filter-branch",
        "replace",
        "notes",
        "reflog",
        "repack",
        "write-tree",
        "commit-tree",
        "hash-object",
        "update-index",
        "send-email",
        "request-pull",
        "daemon",
    ];

    for verb in Verb::ALL {
        assert!(
            !WRITES.contains(&verb.as_str()),
            "'{}' writes and must not be in the gateway",
            verb.as_str()
        );
    }
}

#[test]
fn the_expected_set_is_not_trivially_empty() {
    // An emptied list would make every assertion above vacuous.
    assert!(EXPECTED_VERBS.len() >= 10);
    assert!(Verb::ALL.len() >= 10);
    assert!(
        EXPECTED_VERBS.contains(&"log"),
        "reading commits is essential"
    );
    assert!(
        EXPECTED_VERBS.contains(&"merge-base"),
        "ancestry is how deplyd answers its main question"
    );
}

#[test]
fn fetch_is_the_only_verb_that_writes_anything_at_all() {
    // fetch updates your own remote-tracking refs and nothing else. It is the single
    // exception, and it should stay single.
    let writes_locally: Vec<&str> = Verb::ALL
        .iter()
        .map(|v| v.as_str())
        .filter(|v| *v == "fetch")
        .collect();
    assert_eq!(writes_locally, vec!["fetch"]);
}
