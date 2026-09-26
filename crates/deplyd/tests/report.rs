//! The report, end to end: a real repository, a GitHub made of files, and the
//! binary driven the way a person drives it.
//!
//! These cover what only running the whole thing can: the wording, the exit codes,
//! and whether the JSON a script reads says the same as the text a person reads.

mod support;

use support::{API_WORKFLOW, Sandbox, jobs_json, merged_pr_json, runs_json};

/// A repository with one deploy, one merged change inside it, and one after it.
struct Deployed {
    sandbox: Sandbox,
    inside: String,
    after: String,
}

fn deployed(skipped: &[&str]) -> Deployed {
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
    sandbox.stub("jobs-500.json", &jobs_json(9000, "deploy-api", skipped));
    sandbox.stub(
        "pr-101.json",
        &merged_pr_json(101, "the first thing", &inside),
    );
    sandbox.stub(
        "pr-102.json",
        &merged_pr_json(102, "the second thing", &after),
    );

    Deployed {
        sandbox,
        inside,
        after,
    }
}

#[test]
fn a_change_inside_the_deploy_reads_as_deployd_and_exits_zero() {
    let world = deployed(&[]);
    let (output, code) = world
        .sandbox
        .deplyd_stubbed(&["pr", "101", "-A", "Ada Lovelace"]);

    assert!(output.contains("DEPLYD"), "got:\n{output}");
    assert!(!output.contains("NOT DEPLYD"), "got:\n{output}");
    assert_eq!(code, 0);
}

#[test]
fn a_change_after_the_deploy_reads_as_not_deployd_and_exits_two() {
    let world = deployed(&[]);
    let (output, code) = world
        .sandbox
        .deplyd_stubbed(&["pr", "102", "-A", "Ada Lovelace"]);

    assert!(output.contains("NOT DEPLYD"), "got:\n{output}");
    assert_eq!(code, 2);
}

#[test]
fn the_same_question_about_a_commit_gives_the_same_answer() {
    let world = deployed(&[]);

    let (inside, inside_code) =
        world
            .sandbox
            .deplyd_stubbed(&["commit", &world.inside, "-A", "Ada Lovelace"]);
    assert!(inside.contains("DEPLYD"), "got:\n{inside}");
    assert!(
        inside.contains(&format!("commit {}", &world.inside[..7])),
        "a commit is labelled as a commit:\n{inside}"
    );
    assert!(!inside.contains("PR #"), "and never as a pull request");
    assert_eq!(inside_code, 0);

    let (after, after_code) =
        world
            .sandbox
            .deplyd_stubbed(&["commit", &world.after, "-A", "Ada Lovelace"]);
    assert!(after.contains("NOT DEPLYD"), "got:\n{after}");
    assert_eq!(after_code, 2);
}

#[test]
fn skipped_steps_are_listed_one_per_line() {
    let world = deployed(&["Build the front-end bundle", "Warm the cache"]);
    let (output, _) = world
        .sandbox
        .deplyd_stubbed(&["pr", "101", "-A", "Ada Lovelace"]);

    let steps = ["Build the front-end bundle", "Warm the cache"];
    let mut seen: Vec<&str> = Vec::new();
    for step in steps {
        let line = output
            .lines()
            .find(|line| line.contains(step))
            .unwrap_or_else(|| panic!("{step} missing from:\n{output}"));
        // Alone on its line: only the field label may come before it.
        assert!(
            line.trim_end().ends_with(step),
            "a skipped step should end its line, got {line:?}"
        );
        seen.push(line);
    }
    assert!(seen[0] != seen[1], "both steps landed on the same line");
    assert!(
        seen[0].contains("skipped"),
        "the first line should carry the label, got {:?}",
        seen[0]
    );
}

#[test]
fn the_status_report_dates_and_labels_every_change() {
    let world = deployed(&[]);
    let (output, code) = world
        .sandbox
        .deplyd_stubbed(&["status", "-A", "Ada Lovelace"]);

    assert!(
        output.contains("deplyd changes") && output.contains("Ada Lovelace"),
        "got:\n{output}"
    );
    assert!(output.contains("PR #101"), "got:\n{output}");
    // A date column, as YYYY-MM-DD HH:MM.
    assert!(
        output
            .lines()
            .any(|line| { line.contains("PR #101") && line.contains("2026-") }),
        "every row should carry a date:\n{output}"
    );
    assert_eq!(code, 0, "status answers whenever it can answer at all");
}

#[test]
fn json_and_text_agree_about_the_verdict() {
    let world = deployed(&[]);

    let (text, text_code) = world
        .sandbox
        .deplyd_stubbed(&["pr", "101", "-A", "Ada Lovelace"]);
    let (json, json_code) =
        world
            .sandbox
            .deplyd_stubbed(&["pr", "101", "-A", "Ada Lovelace", "--json"]);

    assert_eq!(
        text_code, json_code,
        "the exit code cannot depend on the format"
    );

    let parsed: serde_json::Value =
        serde_json::from_str(json.trim()).unwrap_or_else(|e| panic!("{e}\n{json}"));
    assert_eq!(parsed["change"]["status"], "deplyd");
    assert_eq!(parsed["change"]["uncertain"], false);
    assert!(text.contains("DEPLYD"));
}

#[test]
fn json_output_is_the_only_thing_on_stdout() {
    // A script redirects stdout. Progress chatter belongs on stderr or it lands in
    // the middle of the document.
    let world = deployed(&[]);
    let (_, _) = world
        .sandbox
        .deplyd_stubbed(&["status", "-A", "Ada Lovelace"]);

    let stdout_only = world
        .sandbox
        .deplyd_stdout(&["status", "-A", "Ada Lovelace", "--json"]);
    serde_json::from_str::<serde_json::Value>(stdout_only.trim())
        .unwrap_or_else(|e| panic!("stdout was not one JSON document: {e}\n{stdout_only}"));
}

#[test]
fn the_status_json_lists_targets_and_changes() {
    let world = deployed(&[]);
    let stdout = world
        .sandbox
        .deplyd_stdout(&["status", "-A", "Ada Lovelace", "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json");

    assert_eq!(parsed["author"], "Ada Lovelace");
    assert_eq!(parsed["targets"][0]["label"], "API");
    assert_eq!(parsed["targets"][0]["state"], "deplyd");
    assert!(
        parsed["changes"]
            .as_array()
            .is_some_and(|changes| !changes.is_empty()),
        "changes should be listed: {parsed}"
    );
}

#[test]
fn paging_past_the_end_says_so_rather_than_counting_backwards() {
    let world = deployed(&[]);
    let (output, _) =
        world
            .sandbox
            .deplyd_stubbed(&["status", "-A", "Ada Lovelace", "--skip", "500"]);

    assert!(
        output.contains("nothing left after skipping 500"),
        "got:\n{output}"
    );
}

#[test]
fn a_pull_request_that_does_not_exist_exits_five() {
    let world = deployed(&[]);
    let (output, code) = world
        .sandbox
        .deplyd_stubbed(&["pr", "999", "-A", "Ada Lovelace"]);

    assert!(output.contains("not found"), "got:\n{output}");
    assert_eq!(code, 5);
}

#[test]
fn a_commit_this_clone_does_not_have_is_refused_clearly() {
    let world = deployed(&[]);
    let (output, code) = world.sandbox.deplyd_stubbed(&[
        "commit",
        "0000000000000000000000000000000000000000",
        "-A",
        "Ada",
    ]);

    assert!(output.contains("No such commit"), "got:\n{output}");
    assert_eq!(code, 1, "deplyd could not answer, which is not a verdict");
}

#[test]
fn a_change_outside_every_scope_shows_what_was_compared() {
    let sandbox = Sandbox::empty();
    sandbox.workflow("deploy-production.yml", API_WORKFLOW);
    sandbox.write("services/api/a.txt", "a");
    sandbox.commit("first");
    sandbox.write("docs/readme.md", "words");
    let elsewhere = sandbox.commit("docs: tidy (#300)");
    let deployed_sha = sandbox.commit("Merge pull request #300");

    sandbox.set_origin("acme/widgets");
    sandbox.stub(
        "runs-deploy-production.yml.json",
        &runs_json(600, &deployed_sha),
    );
    sandbox.stub("jobs-600.json", &jobs_json(9100, "deploy-api", &[]));
    sandbox.stub(
        "pr-300.json",
        &merged_pr_json(300, "docs: tidy", &elsewhere),
    );

    let (output, code) = sandbox.deplyd_stubbed(&["pr", "300", "-A", "Ada Lovelace"]);

    assert!(output.contains("NOT COVERED"), "got:\n{output}");
    assert!(
        output.contains("covers services/api"),
        "it names what each target covers"
    );
    assert!(
        output.contains("docs/readme.md"),
        "and the files it compared"
    );
    assert!(output.contains("\"scopes\""), "and how to fix it");
    assert_eq!(code, 2);
}

#[test]
fn an_unreadable_settings_file_stops_rather_than_being_ignored() {
    let sandbox = Sandbox::empty();
    sandbox.workflow("deploy-production.yml", API_WORKFLOW);
    sandbox.write("services/api/a.txt", "a");
    sandbox.commit("first");
    sandbox.write(".deplyd.json", "{ this is not json");
    sandbox.commit("add a broken settings file");

    let (output, code) = sandbox.deplyd_stubbed(&["config"]);
    assert!(output.contains("could not be read"), "got:\n{output}");
    assert!(output.contains(".deplyd.json"), "and names the file");
    assert_eq!(code, 1);
}

#[test]
fn runs_that_never_succeeded_are_not_reported_as_unrecognised_jobs() {
    // "No deploy jobs recognised" sends people to look at job names. When nothing
    // succeeded, the job names were never the problem.
    let sandbox = Sandbox::empty();
    sandbox.workflow("deploy-production.yml", API_WORKFLOW);
    sandbox.write("services/api/a.txt", "a");
    let head = sandbox.commit("first");
    sandbox.set_origin("acme/widgets");

    sandbox.stub(
        "runs-deploy-production.yml.json",
        &format!(
            "{{\"workflow_runs\":[{{\"id\":700,\"created_at\":\"2026-09-24T10:00:00Z\",\
              \"html_url\":\"https://github.invalid/x\",\"status\":\"completed\",\
              \"conclusion\":\"failure\",\"head_sha\":\"{head}\"}}]}}"
        ),
    );

    let (output, code) = sandbox.deplyd_stubbed(&["status", "-A", "Ada Lovelace"]);
    assert!(
        output.contains("No successful") && output.contains("none of them succeeded"),
        "got:\n{output}"
    );
    assert!(
        !output.contains("recognised"),
        "it should not send them looking at job names:\n{output}"
    );
    assert_eq!(code, 1);
}

#[test]
fn a_workflow_that_never_ran_says_that_instead() {
    let sandbox = Sandbox::empty();
    sandbox.workflow("deploy-production.yml", API_WORKFLOW);
    sandbox.write("services/api/a.txt", "a");
    sandbox.commit("first");
    sandbox.set_origin("acme/widgets");
    sandbox.stub("runs-deploy-production.yml.json", "{\"workflow_runs\":[]}");

    let (output, code) = sandbox.deplyd_stubbed(&["status", "-A", "Ada Lovelace"]);
    assert!(
        output.contains("No production deploy runs found"),
        "got:\n{output}"
    );
    assert_eq!(code, 1);
}

#[test]
fn init_writes_what_detection_found_and_leaves_the_repo_alone() {
    let sandbox = Sandbox::empty();
    sandbox.workflow("deploy-production.yml", API_WORKFLOW);
    sandbox.write("services/api/a.txt", "a");
    sandbox.commit("first");

    let (output, code) = sandbox.deplyd_stubbed(&["init"]);
    assert!(output.contains("Wrote "), "got:\n{output}");
    assert!(
        output.contains("your repository is left alone"),
        "got:\n{output}"
    );
    assert_eq!(code, 0);

    // Nothing landed in the working tree.
    assert!(
        !sandbox.repo().join(".deplyd.json").exists(),
        "init must not write into the repository"
    );

    // And running it again refuses rather than overwriting.
    let (again, again_code) = sandbox.deplyd_stubbed(&["init"]);
    assert!(again.contains("already a settings file"), "got:\n{again}");
    assert_eq!(again_code, 1);

    // --force rewrites it.
    let (forced, forced_code) = sandbox.deplyd_stubbed(&["init", "--force"]);
    assert!(forced.contains("Wrote "), "got:\n{forced}");
    assert_eq!(forced_code, 0);
}

#[test]
fn remember_keeps_a_default_and_says_where() {
    let sandbox = Sandbox::empty();
    sandbox.workflow("deploy-production.yml", API_WORKFLOW);
    sandbox.commit("first");

    let (output, code) = sandbox.deplyd_stubbed(&["remember", "author", "Ada Lovelace"]);
    assert!(output.contains("Saved to"), "got:\n{output}");
    assert!(output.contains("author = Ada Lovelace"), "got:\n{output}");
    assert_eq!(code, 0);

    let (bad, bad_code) = sandbox.deplyd_stubbed(&["remember", "nonsense", "x"]);
    assert!(bad.contains("What should be remembered?"), "got:\n{bad}");
    assert_eq!(bad_code, 1);
}

#[test]
fn completions_are_generated_for_every_shell_clap_knows() {
    let sandbox = Sandbox::empty();
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let script = sandbox.deplyd_stdout(&["completions", shell]);
        assert!(
            script.len() > 200 && script.contains("deplyd"),
            "{shell} produced nothing usable"
        );
    }
}

#[test]
fn the_json_uses_the_same_vocabulary_as_the_report() {
    let world = deployed(&[]);

    let deployd = world
        .sandbox
        .deplyd_stdout(&["pr", "101", "-A", "Ada Lovelace", "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(deployd.trim()).expect("json");

    assert_eq!(parsed["change"]["kind"], "pullRequest");
    assert_eq!(parsed["change"]["status"], "deplyd");
    assert_eq!(parsed["change"]["targets"][0]["status"], "deplyd");
    assert_eq!(parsed["targets"][0]["state"], "deplyd");

    let not_deployd = world
        .sandbox
        .deplyd_stdout(&["pr", "102", "-A", "Ada Lovelace", "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(not_deployd.trim()).expect("json");
    assert_eq!(parsed["change"]["status"], "not-deplyd");
    assert_eq!(parsed["change"]["targets"][0]["status"], "not-deplyd");

    // Nothing anywhere still says live.
    for document in [deployd, not_deployd] {
        assert!(
            !document.contains("\"live\"") && !document.contains("\"not-live\""),
            "the old vocabulary survives in:\n{document}"
        );
    }
}

#[test]
fn a_commit_is_not_given_fields_a_commit_does_not_have() {
    let world = deployed(&[]);
    let stdout =
        world
            .sandbox
            .deplyd_stdout(&["commit", &world.inside, "-A", "Ada Lovelace", "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json");

    assert_eq!(parsed["change"]["kind"], "commit");
    assert_eq!(parsed["change"]["status"], "deplyd");
    // A commit has no pull request number, no merge state and no branch, so it is
    // given none rather than zero and empty string.
    for absent in ["number", "state", "branch", "commitSource"] {
        assert!(
            parsed["change"][absent].is_null(),
            "a commit should not carry {absent}: {parsed}"
        );
    }
}
