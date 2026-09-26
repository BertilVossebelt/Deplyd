//! Labels, ignore matching, and reading a commit out of a job's log.
//!
//! The label cases are the PowerShell suite's own, spelling for spelling.

use deplyd_core::targets::{matched_ignore_words, resolve_checkout_sha, target_label};

#[test]
fn labels_match_the_powershell_suite() {
    let cases = [
        ("deploy (api)", "API"),
        ("deploy-api (eu-west-1)", "API-EU-WEST-1"),
        ("quick-deploy (api, staging)", "API-STAGING"),
        ("Deploy Back-End / deploy-back-end", "BACK-END"),
        ("deploy", "DEPLOY"),
        ("build-and-deploy", "BUILD"),
        ("deploy-to-production", "PRODUCTION"),
    ];

    for (job, expected) in cases {
        assert_eq!(target_label(job), expected, "label for {job:?}");
    }
}

#[test]
fn a_label_stripped_to_nothing_keeps_the_job_name() {
    // Every word is noise, so there is nothing left to name it by. The job's own name
    // is better than an empty column.
    assert_eq!(target_label("deploy"), "DEPLOY");
    assert_eq!(target_label("deployment"), "DEPLOYMENT");
}

#[test]
fn noise_words_are_stripped_whole_not_as_substrings() {
    // Removing "deploy" as a substring would leave "build-and" and "to-production".
    assert_eq!(target_label("build-and-deploy"), "BUILD");
    assert_eq!(target_label("deploy-to-production"), "PRODUCTION");
    // "redeploy" contains "deploy" but is not the word, so it survives.
    assert_eq!(target_label("redeploy-api"), "REDEPLOY-API");
}

#[test]
fn matrix_values_are_what_tell_legs_apart() {
    assert_ne!(
        target_label("deploy (api)"),
        target_label("deploy (api, eu-west-1)"),
        "two legs must not share a label"
    );
}

#[test]
fn ignore_words_are_matched_on_a_tokenised_name() {
    let ignore: Vec<String> = ["merge", "notify", "test"]
        .iter()
        .map(|w| (*w).to_string())
        .collect();

    assert_eq!(matched_ignore_words("merge-queue", &ignore), vec!["merge"]);
    assert_eq!(
        matched_ignore_words("Notify Slack", &ignore),
        vec!["notify"]
    );
    // Known cost of the approach, and why ignoreJobs exists in .deplyd.json.
    assert_eq!(
        matched_ignore_words("deploy-test-api", &ignore),
        vec!["test"]
    );
    assert!(matched_ignore_words("deploy-api", &ignore).is_empty());
}

/// A log line as GitHub writes it: an ISO timestamp, then the content.
fn log_line(content: &str) -> String {
    format!("2026-09-25T10:11:12.1234567Z {content}")
}

#[test]
fn reads_a_bare_checkout_sha() {
    let repo = holding(&["a".repeat(40)]);
    let log = [
        log_line("Syncing repository: acme/widgets"),
        log_line(&"a".repeat(40)),
        log_line("Deploying"),
    ]
    .join("\n");

    let resolved = resolve_checkout_sha(&log, &repo).expect("a sha");
    assert_eq!(resolved.sha, "a".repeat(40));
    assert!(resolved.exact);
    assert!(resolved.warning.is_empty());
}

#[test]
fn a_sha_embedded_in_prose_is_a_guess_and_says_so() {
    // A pinned action sha and a cache key are forty hex characters too, so anything
    // not on a line of its own is reported as inference rather than fact.
    let repo = holding(&["b".repeat(40)]);
    let log = [
        log_line("Syncing repository"),
        log_line(&format!("checked out {} for you", "b".repeat(40))),
    ]
    .join("\n");

    let resolved = resolve_checkout_sha(&log, &repo).expect("a sha");
    assert_eq!(resolved.sha, "b".repeat(40));
    assert!(!resolved.exact, "a sha found in prose is not exact");
    assert!(resolved.warning.contains("no checkout line"));
}

#[test]
fn returns_nothing_when_no_known_commit_appears() {
    let repo = holding(&[]);
    let log = [log_line("Syncing repository"), log_line(&"c".repeat(40))].join("\n");
    assert!(resolve_checkout_sha(&log, &repo).is_none());
}

#[test]
fn a_downloaded_action_sha_is_not_mistaken_for_a_checkout() {
    let repo = holding(&["d".repeat(40)]);
    let log = log_line(&format!(
        "Download action repository 'actions/checkout@{}'",
        "d".repeat(40)
    ));
    assert!(
        resolve_checkout_sha(&log, &repo).is_none(),
        "an action download names a sha that was never deployed"
    );
}

#[test]
fn several_checkouts_take_the_first_and_say_so() {
    let repo = holding(&["e".repeat(40), "f".repeat(40)]);
    let log = [
        log_line(&"e".repeat(40)),
        log_line("and later"),
        log_line(&"f".repeat(40)),
    ]
    .join("\n");

    let resolved = resolve_checkout_sha(&log, &repo).expect("a sha");
    assert_eq!(
        resolved.sha,
        "e".repeat(40),
        "the deploy steps ran against the first"
    );
    assert!(resolved.warning.contains("2 different commits"));
}

/// A repository containing exactly the commits named, and nothing else.
///
/// Real commits cannot be made to have chosen shas, so the earlier version of this
/// helper quietly claimed to hold shas it did not. Since resolving a commit asks the
/// repository only one question, answering that question directly is both honest and
/// enough.
struct Present(Vec<String>);

impl deplyd_core::targets::CommitLookup for Present {
    fn commit_exists(&self, sha: &str, _allow_fetch: bool) -> bool {
        self.0.iter().any(|known| known == sha)
    }
}

fn holding(shas: &[String]) -> Present {
    Present(shas.to_vec())
}
