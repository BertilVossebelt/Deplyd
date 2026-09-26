//! What `deplyd check` runs, and what every invocation runs first. A binary has no
//! source to scan, so this exercises the paths compiled into it.

use crate::git::{ReadOnlyGit, Verb};
use crate::http;

/// The verbs this build may run, frozen. The last of four gates against a new one
/// being added quietly; the other three are in `git.rs`.
pub const EXPECTED_VERBS: &[&str] = &[
    "rev-parse",
    "rev-list",
    "log",
    "show",
    "merge-base",
    "shortlog",
    "cat-file",
    "diff",
    "status",
    "cherry",
    "ls-files",
    "config",
    "fetch",
];

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub name: &'static str,
    pub detail: String,
    pub passed: bool,
}

impl CheckResult {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            detail: detail.into(),
            passed: true,
        }
    }

    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            detail: detail.into(),
            passed: false,
        }
    }
}

/// Calls that must be refused. Never executed: proving a guard by performing the
/// dangerous call is the accident it exists to prevent.
const MUST_REFUSE: &[(Verb, &[&str], &str)] = &[
    (
        Verb::Config,
        &["user.name", "someone else"],
        "git config assigning a value",
    ),
    (
        Verb::Config,
        &["--unset", "user.name"],
        "git config --unset",
    ),
    (
        Verb::Fetch,
        &["origin", "--prune"],
        "git fetch --prune, which deletes refs",
    ),
    (
        Verb::Fetch,
        &["origin", "+refs/heads/*:refs/heads/*"],
        "git fetch with a refspec that overwrites branches",
    ),
    (
        Verb::Log,
        &["-c", "core.pager=sh -c id"],
        "git -c config injection, which reaches a shell",
    ),
    (
        Verb::Diff,
        &["--output=/tmp/anywhere"],
        "git --output, which writes a file",
    ),
    (
        Verb::Log,
        &["--git-dir=/elsewhere/.git"],
        "git --git-dir, which reads another repository",
    ),
    (
        Verb::Diff,
        &["--ext-diff"],
        "git --ext-diff, which runs another program",
    ),
];

/// Reads that must still work, or a guard refusing everything would pass.
const MUST_ALLOW: &[(Verb, &[&str], &str)] = &[
    (
        Verb::RevParse,
        &["--show-toplevel"],
        "finding the repo root",
    ),
    (Verb::Log, &["-1", "--format=%H"], "reading a commit"),
    (
        Verb::MergeBase,
        &["--is-ancestor", "a", "b"],
        "testing ancestry",
    ),
    (Verb::Fetch, &["origin", "--quiet"], "the one allowed fetch"),
    (Verb::Config, &["user.name"], "reading a config value"),
];

pub fn run() -> Vec<CheckResult> {
    let mut results = Vec::new();

    let actual: Vec<&str> = Verb::ALL.iter().map(|v| v.as_str()).collect();
    if actual == EXPECTED_VERBS {
        results.push(CheckResult::pass(
            "git verbs",
            format!(
                "{} allowed, none of them write: {}",
                actual.len(),
                actual.join(", ")
            ),
        ));
    } else {
        results.push(CheckResult::fail(
            "git verbs",
            format!(
                "the allowed set has changed.\n    expected: {}\n    found:    {}",
                EXPECTED_VERBS.join(", "),
                actual.join(", ")
            ),
        ));
    }

    let mut refused = 0;
    let mut leaked = Vec::new();
    for (verb, args, description) in MUST_REFUSE {
        match ReadOnlyGit::new(*verb, args) {
            Err(_) => refused += 1,
            Ok(allowed) => {
                leaked.push(format!("{description} was allowed: {}", allowed.rendered()))
            }
        }
    }
    if leaked.is_empty() {
        results.push(CheckResult::pass(
            "write refusals",
            format!("{refused} of {} refused", MUST_REFUSE.len()),
        ));
    } else {
        results.push(CheckResult::fail("write refusals", leaked.join("\n    ")));
    }

    let mut blocked = Vec::new();
    for (verb, args, description) in MUST_ALLOW {
        if let Err(denied) = ReadOnlyGit::new(*verb, args) {
            blocked.push(format!("{description} was refused: {}", denied.reason));
        }
    }
    if blocked.is_empty() {
        results.push(CheckResult::pass(
            "reads still work",
            format!("{} of {} allowed", MUST_ALLOW.len(), MUST_ALLOW.len()),
        ));
    } else {
        results.push(CheckResult::fail(
            "reads still work",
            blocked.join("\n    "),
        ));
    }

    match http::routes_are_read_only() {
        Ok(routes) => results.push(CheckResult::pass(
            "github routes",
            format!("{} routes, all GET, all inside the repo", routes.len()),
        )),
        Err(denied) => results.push(CheckResult::fail(
            "github routes",
            format!("{}: {}", denied.attempted, denied.reason),
        )),
    }

    results
}

/// Whether this build's guard is intact.
pub fn is_intact() -> bool {
    run().iter().all(|r| r.passed)
}

/// The failures, for a caller that wants to report before stopping.
pub fn failures() -> Vec<CheckResult> {
    run().into_iter().filter(|r| !r.passed).collect()
}
