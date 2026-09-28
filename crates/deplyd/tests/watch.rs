//! Watching, end to end. The loop, the exits and the stream, against a GitHub
//! made of files so nothing here waits on a network or a real deploy.

mod support;

use support::{API_WORKFLOW, Sandbox, jobs_json, merged_pr_json, runs_json};

/// One deploy, one change inside it, one after it. The same shape the report
/// tests use, because watching answers the same question repeatedly.
fn world() -> (Sandbox, String) {
    let sandbox = Sandbox::empty();
    sandbox.workflow("deploy-production.yml", API_WORKFLOW);
    sandbox.write("services/api/one.txt", "one");
    let inside = sandbox.commit("feat: the first thing (#101)");
    let deployed_sha = sandbox.commit("Merge pull request #101");

    sandbox.write("services/api/two.txt", "two");
    let after = sandbox.commit("feat: the second thing (#102)");

    sandbox.set_origin("acme/widgets");
    sandbox.stub(
        "runs-deploy-production.yml.json",
        &runs_json(500, &deployed_sha),
    );
    sandbox.stub("jobs-500.json", &jobs_json(9000, "deploy-api", &[]));
    sandbox.stub(
        "pr-101.json",
        &merged_pr_json(101, "the first thing", &inside),
    );
    sandbox.stub(
        "pr-102.json",
        &merged_pr_json(102, "the second thing", &after),
    );
    (sandbox, after)
}

#[test]
fn waiting_for_something_already_live_stops_at_once() {
    let (sandbox, _) = world();
    let (output, code) =
        sandbox.deplyd_stubbed(&["watch", "pr", "101", "--every", "10s", "-A", "Ada Lovelace"]);

    assert!(
        output.contains("is live"),
        "expected it to say what it stopped for, got:\n{output}"
    );
    assert_eq!(code, 0, "live is a success, got:\n{output}");
}

#[test]
fn waiting_for_something_that_never_arrives_gives_up_when_told_to() {
    // PR #102 is after the deploy, so it will not go live while this runs. The
    // point is that --for ends the loop rather than it running until killed.
    let (sandbox, _) = world();
    let (output, code) = sandbox.deplyd_stubbed(&[
        "watch",
        "pr",
        "102",
        "--for",
        "1s",
        "--every",
        "10s",
        "-A",
        "Ada Lovelace",
    ]);

    assert!(
        output.contains("time is up"),
        "expected the deadline to be named, got:\n{output}"
    );
    assert_eq!(code, 0, "a deadline is not a failure, got:\n{output}");
}

#[test]
fn a_length_of_time_that_is_not_one_is_refused_before_anything_runs() {
    let (sandbox, _) = world();
    let (output, code) = sandbox.deplyd_stubbed(&["watch", "--every", "soon", "--anyone"]);

    assert!(
        output.contains("not a length of time"),
        "expected a refusal naming the problem, got:\n{output}"
    );
    assert_eq!(code, 1, "bad input is not a verdict, got:\n{output}");
}

#[test]
fn asking_to_wait_for_two_things_at_once_cannot_be_spelled() {
    // What it waits for is a subcommand now, so "both" is a parse error rather
    // than a check that has to be remembered.
    let (sandbox, _) = world();
    let (output, code) =
        sandbox.deplyd_stubbed(&["watch", "pr", "101", "commit", "HEAD", "--anyone"]);

    assert!(
        !output.contains("Inspecting"),
        "expected it to refuse before looking at anything, got:\n{output}"
    );
    assert_eq!(code, 2, "clap refuses an argument it has no room for");
}

#[test]
fn the_json_stream_carries_no_chatter() {
    // Something reading this parses a line at a time. An opening banner or a
    // closing note in the stream would be a parse error at the far end.
    let (sandbox, _) = world();
    let (output, code) = sandbox.deplyd_stubbed(&[
        "watch",
        "pr",
        "101",
        "--every",
        "10s",
        "--json",
        "-A",
        "Ada Lovelace",
    ]);

    assert_eq!(code, 0, "got:\n{output}");
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        assert!(
            serde_json::from_str::<serde_json::Value>(line).is_ok(),
            "every line should be JSON, this one is not: {line:?}\nfull output:\n{output}"
        );
    }
}

#[test]
fn looking_faster_than_the_floor_is_refused_before_anything_is_fetched() {
    // The order matters: the same principle as a bad pull request number. A
    // typo is the user's to fix and is worth saying before a round trip.
    let (sandbox, _) = world();
    let (output, code) = sandbox.deplyd_stubbed(&["watch", "--every", "5s", "--anyone"]);

    assert!(
        output.contains("faster than deplyd will go"),
        "expected the floor to be named, got:\n{output}"
    );
    assert!(
        !output.contains("Inspecting"),
        "it should refuse before looking at anything, got:\n{output}"
    );
    assert_eq!(code, 1);
}

#[test]
fn the_floor_itself_is_allowed() {
    let (sandbox, _) = world();
    let (output, code) = sandbox.deplyd_stubbed(&[
        "watch",
        "--every",
        "10s",
        "--for",
        "1s",
        "-A",
        "Ada Lovelace",
    ]);

    assert!(
        output.contains("time is up"),
        "10s is the floor, not past it, got:\n{output}"
    );
    assert_eq!(code, 0);
}

#[test]
fn a_remembered_interval_is_used_and_the_flag_still_wins() {
    let (sandbox, _) = world();
    let (saved, code) = sandbox.deplyd_stubbed(&["remember", "every", "5m"]);
    assert_eq!(code, 0, "got:\n{saved}");

    let (kept, _) = sandbox.deplyd_stubbed(&["watch", "--for", "1s", "-A", "Ada Lovelace"]);
    assert!(
        kept.contains("every 5m"),
        "the kept default should be used, got:\n{kept}"
    );

    let (flagged, _) = sandbox.deplyd_stubbed(&[
        "watch",
        "--for",
        "1s",
        "--every",
        "30s",
        "-A",
        "Ada Lovelace",
    ]);
    assert!(
        flagged.contains("every 30s"),
        "the flag should beat the kept default, got:\n{flagged}"
    );
}
