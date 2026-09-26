//! Targets and verdicts, end to end against a real repository and a stubbed GitHub.
//!
//! These are the paths that had no coverage at all: resolving what a run deployed,
//! deciding whether a change is in it, and the cross-check between the run log and
//! the deployment record. Everything except the network runs for real.

mod support;

use deplyd_core::context::Context;
use deplyd_core::github::GitHub;
use deplyd_core::repo::Repo;
use deplyd_core::settings::Settings;
use deplyd_core::targets::{self, ShaSource};
use deplyd_core::verdict::{self, Status, TargetStatus};

use support::{
    INPUT_REF_WORKFLOW, PLAIN_WORKFLOW, Sandbox, StubGitHub, StubJob, StubRun, TempCache,
};

struct World {
    sandbox: Sandbox,
    repo: Repo,
}

impl World {
    fn new(label: &str, workflow: &str) -> Self {
        let sandbox = Sandbox::new(label);
        sandbox.workflow("deploy-production.yml", workflow);
        sandbox.commit("services/api/start.txt", "one", "first");
        sandbox.set_origin("acme/widgets");
        let repo = Repo::discover(&sandbox.path()).expect("a repository");
        Self { sandbox, repo }
    }

    fn context(&self, environment: &str) -> Context {
        let mut context = Context::build(
            self.repo.root(),
            "Ada Lovelace".to_string(),
            Settings::default(),
        )
        .expect("workflows should be readable");
        context
            .select_environment(environment)
            .expect("environment should resolve");
        context
    }
}

fn build_targets(
    context: &Context,
    repo: &Repo,
    github: &GitHub,
    runs: &[deplyd_core::github::Run],
) -> targets::TargetSet {
    targets::build(
        context,
        repo,
        github,
        runs,
        &mut deplyd_core::cache::Cache::disabled(),
        |_| {},
    )
}

fn runs_for(github: &GitHub, file: &str, needs_log: bool) -> Vec<deplyd_core::github::Run> {
    let mut runs = github.runs_for_workflow(file).expect("runs");
    for run in &mut runs {
        run.workflow_file = file.to_string();
        run.workflow_token = "deployproduction".into();
        run.needs_log = needs_log;
    }
    runs
}

#[test]
fn a_plain_workflow_deploys_the_runs_own_commit() {
    let world = World::new("plain", PLAIN_WORKFLOW);
    let deployed = world.sandbox.head();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(100, "2026-01-02T00:00:00Z", &deployed)],
        )
        .jobs(100, &[StubJob::deploy(900, "deploy-api")]);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);

    assert_eq!(set.targets.len(), 1, "one deploy job, one target");
    let target = &set.targets[0];
    assert_eq!(target.label, "API");
    assert_eq!(target.sha, deployed);
    assert_eq!(target.sha_source, ShaSource::RunRef);
    assert!(target.sha_is_exact);
    assert_eq!(target.scope, vec!["services/api"]);
    assert!(!target.uncertain());
}

#[test]
fn a_branch_input_workflow_takes_the_commit_from_the_job_log() {
    let world = World::new("input-ref", INPUT_REF_WORKFLOW);
    let older = world.sandbox.head();
    world
        .sandbox
        .commit("services/api/later.txt", "two", "second");
    let newest = world.sandbox.head();

    // The run was triggered on the newest commit, but the checkout took the older
    // one. Only the log says so, which is the whole reason this path exists.
    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(101, "2026-01-02T00:00:00Z", &newest)],
        )
        .jobs(101, &[StubJob::deploy(901, "deploy-api")])
        .log(
            901,
            &["Syncing repository: acme/widgets", &older, "Deploying"],
        );

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", true);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let target = &set.targets[0];
    assert_eq!(target.sha, older, "the log wins over the run's own ref");
    assert_eq!(target.sha_source, ShaSource::RunLog);
    assert!(target.sha_is_exact);
}

#[test]
fn a_log_that_names_no_known_commit_falls_back_to_the_deployment_record() {
    let world = World::new("fallback", INPUT_REF_WORKFLOW);
    let deployed = world.sandbox.head();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(
                102,
                "2026-01-02T00:00:00Z",
                &"f".repeat(40),
            )],
        )
        .jobs(102, &[StubJob::deploy(902, "deploy-api")])
        .log(902, &["Syncing repository", "nothing useful here"])
        .deployments("production", &[(7, &deployed)])
        .deployment_statuses(7, 102);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", true);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let target = &set.targets[0];
    assert_eq!(target.sha, deployed);
    assert_eq!(target.sha_source, ShaSource::DeploymentRecord);
    assert!(
        !target.sha_is_exact,
        "a commit from the record rather than the checkout is not exact"
    );
    assert!(target.uncertain());
}

#[test]
fn a_deployment_record_naming_the_same_commit_is_reported_as_corroboration() {
    let world = World::new("agree", INPUT_REF_WORKFLOW);
    let deployed = world.sandbox.head();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(103, "2026-01-02T00:00:00Z", &deployed)],
        )
        .jobs(103, &[StubJob::deploy(903, "deploy-api")])
        .log(903, &[&deployed])
        .deployments("production", &[(8, &deployed)])
        .deployment_statuses(8, 103);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    // The walk happens because an environment had to be matched, so the record is
    // already in hand and costs nothing to compare against.
    let _ = github.deployments("production");
    let runs = runs_for(&github, "deploy-production.yml", true);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let target = &set.targets[0];
    assert!(target.corroborated, "two records agreeing should say so");
    assert!(target.sha_is_exact);
    assert!(!target.uncertain());
}

#[test]
fn two_records_disagreeing_drops_the_certainty_rather_than_picking_a_winner() {
    let world = World::new("disagree", INPUT_REF_WORKFLOW);
    let checked_out = world.sandbox.head();
    let recorded = world
        .sandbox
        .commit("services/api/other.txt", "x", "moved on");

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(104, "2026-01-02T00:00:00Z", &checked_out)],
        )
        .jobs(104, &[StubJob::deploy(904, "deploy-api")])
        .log(904, &[&checked_out])
        .deployments("production", &[(9, &recorded)])
        .deployment_statuses(9, 104);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let _ = github.deployments("production");
    let runs = runs_for(&github, "deploy-production.yml", true);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let target = &set.targets[0];
    assert_eq!(
        target.sha, checked_out,
        "the checkout is still what is reported"
    );
    assert!(!target.sha_is_exact, "but the disagreement is surfaced");
    assert!(!target.corroborated);
    assert!(
        target.sha_warning.contains("deployment record names"),
        "the reader should be told what the other record said: {}",
        target.sha_warning
    );
}

#[test]
fn plumbing_jobs_and_short_jobs_are_not_targets() {
    let world = World::new("plumbing", PLAIN_WORKFLOW);
    let deployed = world.sandbox.head();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(105, "2026-01-02T00:00:00Z", &deployed)],
        )
        .jobs(
            105,
            &[
                StubJob::deploy(905, "notify-slack"),
                StubJob::deploy(906, "deploy-api"),
                StubJob::deploy(907, "tiny").with_steps(vec![("only", "success")]),
                StubJob::deploy(908, "deploy-web").concluded("failure"),
            ],
        );

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let labels: Vec<&str> = set.targets.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(labels, vec!["API"], "only the real deploy job");

    assert!(set.ignored_jobs.contains_key("notify-slack"));
    assert!(set.ignored_jobs["tiny"].contains("three steps"));
    // A failed job is not a target and is not an explained skip either: it simply did
    // not deploy.
    assert!(!set.ignored_jobs.contains_key("deploy-web"));
}

#[test]
fn skipped_steps_are_recorded_on_the_target() {
    let world = World::new("skipped", PLAIN_WORKFLOW);
    let deployed = world.sandbox.head();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(106, "2026-01-02T00:00:00Z", &deployed)],
        )
        .jobs(
            106,
            &[StubJob::deploy(909, "deploy-api").with_steps(vec![
                ("Checkout", "success"),
                ("Build bundle", "skipped"),
                ("Publish", "success"),
                ("Warm cache", "skipped"),
            ])],
        );

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);

    assert_eq!(
        set.targets[0].skipped,
        vec!["Build bundle", "Warm cache"],
        "skipped steps are named, because changes to them are not live"
    );
}

#[test]
fn a_newer_failed_run_makes_the_target_uncertain() {
    let world = World::new("concerns", PLAIN_WORKFLOW);
    let deployed = world.sandbox.head();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[
                StubRun::with(200, "2026-01-03T00:00:00Z", "completed", "failure"),
                StubRun::success(107, "2026-01-02T00:00:00Z", &deployed),
            ],
        )
        .jobs(107, &[StubJob::deploy(910, "deploy-api")]);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let mut set = build_targets(&context, &world.repo, &github, &runs);

    let labels: Vec<String> = set.targets.iter().map(|t| t.label.clone()).collect();
    set.targets[0].concerns = targets::concerns_for(&set.targets[0], &runs, &labels, &github);

    assert_eq!(set.targets[0].concerns.len(), 1);
    assert_eq!(set.targets[0].concerns[0].run_id, 200);
    assert_eq!(set.targets[0].concerns[0].state, "failed");
    assert!(set.targets[0].uncertain());
}

#[test]
fn an_older_failed_run_is_not_a_concern() {
    let world = World::new("older-failure", PLAIN_WORKFLOW);
    let deployed = world.sandbox.head();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[
                StubRun::success(108, "2026-01-02T00:00:00Z", &deployed),
                StubRun::with(50, "2026-01-01T00:00:00Z", "completed", "failure"),
            ],
        )
        .jobs(108, &[StubJob::deploy(911, "deploy-api")]);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);
    let labels: Vec<String> = set.targets.iter().map(|t| t.label.clone()).collect();

    let concerns = targets::concerns_for(&set.targets[0], &runs, &labels, &github);
    assert!(
        concerns.is_empty(),
        "what happened before the deploy says nothing about it"
    );
}

// ---------------------------------------------------------------------------------
// Verdicts
// ---------------------------------------------------------------------------------

fn merged_pr(number: u32, title: &str, sha: &str) -> String {
    format!(
        "{{\"number\":{number},\"title\":\"{title}\",\"state\":\"closed\",\
          \"merged_at\":\"2026-01-02T00:00:00Z\",\"merge_commit_sha\":\"{sha}\",\
          \"head\":{{\"ref\":\"feature/thing\"}}}}"
    )
}

/// A world whose deploy is at `deployed`, with one target covering services/api.
fn deployed_world(label: &str) -> (World, GitHub, Context, targets::TargetSet, String) {
    let world = World::new(label, PLAIN_WORKFLOW);
    let change = world.sandbox.commit(
        "services/api/feature.txt",
        "new",
        "feat: add a thing (#412)",
    );
    let deployed = world.sandbox.empty_commit("Merge pull request #413");

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(300, "2026-01-05T00:00:00Z", &deployed)],
        )
        .jobs(300, &[StubJob::deploy(950, "deploy-api")])
        .pull_request(412, &merged_pr(412, "feat: add a thing", &change));

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);
    (world, github, context, set, change)
}

#[test]
fn a_merged_change_inside_the_deployed_commit_is_live() {
    let (world, github, context, set, _) = deployed_world("live");
    let report = verdict::pull_request_report(&context, &world.repo, &github, &set, 412);

    assert_eq!(report.status, Status::Deplyd);
    assert!(!report.uncertain);
    assert_eq!(report.targets.len(), 1);
    assert_eq!(report.targets[0].status, TargetStatus::Deplyd);
    assert_eq!(verdict::exit_code(report.status, report.uncertain), 0);
}

#[test]
fn a_change_after_the_deploy_is_not_live() {
    let world = World::new("not-live", PLAIN_WORKFLOW);
    let deployed = world.sandbox.head();
    let later = world
        .sandbox
        .commit("services/api/later.txt", "later", "feat: later (#500)");

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(301, "2026-01-05T00:00:00Z", &deployed)],
        )
        .jobs(301, &[StubJob::deploy(951, "deploy-api")])
        .pull_request(500, &merged_pr(500, "feat: later", &later));

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let report = verdict::pull_request_report(&context, &world.repo, &github, &set, 500);
    assert_eq!(report.status, Status::NotDeplyd);
    assert_eq!(verdict::exit_code(report.status, report.uncertain), 2);
}

#[test]
fn a_change_that_shipped_and_was_undone_is_reverted_not_live() {
    let world = World::new("reverted", PLAIN_WORKFLOW);
    let change = world
        .sandbox
        .commit("services/api/thing.txt", "new", "feat: a thing (#600)");
    world.sandbox.revert(&change);
    let deployed = world.sandbox.head();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(302, "2026-01-05T00:00:00Z", &deployed)],
        )
        .jobs(302, &[StubJob::deploy(952, "deploy-api")])
        .pull_request(600, &merged_pr(600, "feat: a thing", &change));

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let report = verdict::pull_request_report(&context, &world.repo, &github, &set, 600);
    assert_eq!(
        report.status,
        Status::Reverted,
        "ancestry alone would have called this live"
    );
    assert!(
        !report.targets[0].reverts.is_empty(),
        "and it names the revert"
    );
    assert_eq!(verdict::exit_code(report.status, report.uncertain), 3);
}

#[test]
fn an_open_pull_request_is_not_merged() {
    let world = World::new("open", PLAIN_WORKFLOW);
    let deployed = world.sandbox.head();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(303, "2026-01-05T00:00:00Z", &deployed)],
        )
        .jobs(303, &[StubJob::deploy(953, "deploy-api")])
        .pull_request(
            700,
            "{\"number\":700,\"title\":\"wip\",\"state\":\"open\",\"merged_at\":null,\
              \"merge_commit_sha\":null,\"head\":{\"ref\":\"feature/wip\"}}",
        );

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let report = verdict::pull_request_report(&context, &world.repo, &github, &set, 700);
    assert_eq!(report.status, Status::NotMerged);
    assert_eq!(report.state, "OPEN");
    assert_eq!(verdict::exit_code(report.status, report.uncertain), 4);
}

#[test]
fn a_pull_request_nobody_can_see_is_not_found() {
    let (world, github, context, set, _) = deployed_world("missing");
    let report = verdict::pull_request_report(&context, &world.repo, &github, &set, 9999);

    assert_eq!(report.status, Status::NotFound);
    assert_eq!(verdict::exit_code(report.status, report.uncertain), 5);
}

#[test]
fn a_change_outside_every_scope_is_not_covered() {
    let world = World::new("not-covered", PLAIN_WORKFLOW);
    let elsewhere = world
        .sandbox
        .commit("docs/readme.md", "words", "docs: tidy (#800)");
    let deployed = world.sandbox.empty_commit("deploy");

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(304, "2026-01-05T00:00:00Z", &deployed)],
        )
        .jobs(304, &[StubJob::deploy(954, "deploy-api")])
        .pull_request(800, &merged_pr(800, "docs: tidy", &elsewhere));

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let report = verdict::pull_request_report(&context, &world.repo, &github, &set, 800);
    assert_eq!(
        report.status,
        Status::NotCovered,
        "the target covers services/api and this touched docs"
    );
    assert!(
        report.files.iter().any(|f| f.contains("docs/")),
        "and the files are kept so the report can show what was compared"
    );
    assert_eq!(verdict::exit_code(report.status, report.uncertain), 2);
}

#[test]
fn a_live_change_on_an_uncertain_target_exits_six_not_zero() {
    // The verdict has not changed; the evidence for it is weaker and says so.
    let (world, github, context, mut set, _) = deployed_world("uncertain");
    set.targets[0].concerns.push(deplyd_core::targets::Concern {
        run_id: 999,
        state: "failed".into(),
    });

    let report = verdict::pull_request_report(&context, &world.repo, &github, &set, 412);
    assert_eq!(report.status, Status::Deplyd);
    assert!(report.uncertain);
    assert_eq!(
        verdict::exit_code(report.status, report.uncertain),
        6,
        "a gate must never be told shipped on evidence deplyd has questioned"
    );
}

// ---------------------------------------------------------------------------------
// What is remembered between runs
// ---------------------------------------------------------------------------------

#[test]
fn a_remembered_commit_means_the_log_is_not_asked_for_again() {
    let world = World::new("cache-hit", INPUT_REF_WORKFLOW);
    let deployed = world.sandbox.head();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(400, "2026-01-02T00:00:00Z", &deployed)],
        )
        .jobs(400, &[StubJob::deploy(970, "deploy-api")])
        .log(970, &[&deployed]);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", true);

    let temp = TempCache::new("hit");

    // First time: nothing remembered, so the log is read and the answer kept.
    let mut cache = temp.open();
    let first = targets::build(&context, &world.repo, &github, &runs, &mut cache, |_| {});
    cache.save();
    assert_eq!(first.targets[0].sha, deployed);
    assert_eq!(cache.len(), 1, "the resolution should have been kept");

    // Second time, with a GitHub that would refuse to serve the log at all.
    let forgetful = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(400, "2026-01-02T00:00:00Z", &deployed)],
        )
        .jobs(400, &[StubJob::deploy(970, "deploy-api")]);

    let github = GitHub::new(Box::new(forgetful), "acme".into(), "widgets".into());
    let runs = runs_for(&github, "deploy-production.yml", true);
    let mut cache = temp.open();
    assert_eq!(cache.len(), 1, "it should have survived the write");

    let second = targets::build(&context, &world.repo, &github, &runs, &mut cache, |_| {});
    assert_eq!(
        second.targets[0].sha, deployed,
        "the remembered answer should stand in for the log"
    );
    assert_eq!(second.targets[0].sha_source, ShaSource::RunLog);
    assert!(second.targets[0].sha_is_exact);
}

#[test]
fn a_commit_no_longer_in_the_clone_is_not_trusted_from_the_cache() {
    // The cache says what the run did, not what this clone has. A remembered commit
    // that is missing locally has to be resolved again rather than reported.
    let world = World::new("cache-missing", INPUT_REF_WORKFLOW);
    let deployed = world.sandbox.head();

    let temp = TempCache::new("missing");

    let mut cache = temp.open();
    cache.remember_sha(971, &"b".repeat(40), "");
    cache.save();

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(401, "2026-01-02T00:00:00Z", &deployed)],
        )
        .jobs(401, &[StubJob::deploy(971, "deploy-api")])
        .log(971, &[&deployed]);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", true);
    let mut cache = temp.open();

    let set = targets::build(&context, &world.repo, &github, &runs, &mut cache, |_| {});
    assert_eq!(
        set.targets[0].sha, deployed,
        "it should fall back to reading the log"
    );
}

#[test]
fn a_run_still_in_flight_is_never_remembered() {
    let world = World::new("cache-in-flight", INPUT_REF_WORKFLOW);
    let deployed = world.sandbox.head();

    let mut run = StubRun::success(402, "2026-01-02T00:00:00Z", &deployed);
    run.status = "in_progress";

    let stub = StubGitHub::new()
        .runs("deploy-production.yml", &[run])
        .jobs(402, &[StubJob::deploy(972, "deploy-api")])
        .log(972, &[&deployed]);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", true);

    let temp = TempCache::new("inflight");
    let mut cache = temp.open();

    let _ = targets::build(&context, &world.repo, &github, &runs, &mut cache, |_| {});
    assert!(
        cache.is_empty(),
        "only a finished run is settled enough to remember"
    );
}

// ---------------------------------------------------------------------------------
// Asking about a commit rather than a pull request
// ---------------------------------------------------------------------------------

#[test]
fn a_commit_inside_the_deploy_is_deployd() {
    let (world, _github, context, set, change) = deployed_world("commit-live");
    let report = verdict::commit_report(&context, &world.repo, &set, &change);

    assert_eq!(report.status, Status::Deplyd);
    assert_eq!(
        report.change,
        deplyd_core::verdict::Change::Commit {
            sha: change.clone()
        },
        "a commit is reported as a commit, not as a pull request"
    );
    assert_eq!(
        report.number, 0,
        "there is no pull request number to invent"
    );
    assert_eq!(verdict::exit_code(report.status, report.uncertain), 0);
}

#[test]
fn a_commit_after_the_deploy_is_not_deployd() {
    let world = World::new("commit-not-live", PLAIN_WORKFLOW);
    let deployed = world.sandbox.head();
    let later = world
        .sandbox
        .commit("services/api/later.txt", "l", "later work");

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(600, "2026-01-05T00:00:00Z", &deployed)],
        )
        .jobs(600, &[StubJob::deploy(980, "deploy-api")]);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let report = verdict::commit_report(&context, &world.repo, &set, &later);
    assert_eq!(report.status, Status::NotDeplyd);
    assert_eq!(verdict::exit_code(report.status, report.uncertain), 2);
}

#[test]
fn a_commit_can_be_named_any_way_git_accepts() {
    let (world, _github, context, set, change) = deployed_world("commit-refs");
    let short: String = change.chars().take(8).collect();

    for reference in [change.as_str(), short.as_str()] {
        let report = verdict::commit_report(&context, &world.repo, &set, reference);
        assert_eq!(
            report.commit, change,
            "{reference} should resolve to the full sha"
        );
        assert_eq!(report.status, Status::Deplyd);
    }
}

#[test]
fn a_commit_this_clone_does_not_have_is_not_found() {
    let (world, _github, context, set, _) = deployed_world("commit-missing");
    let report = verdict::commit_report(&context, &world.repo, &set, &"e".repeat(40));

    assert_eq!(report.status, Status::NotFound);
    assert!(report.commit.is_empty());
}

#[test]
fn a_commit_outside_every_scope_is_not_covered() {
    let world = World::new("commit-not-covered", PLAIN_WORKFLOW);
    let elsewhere = world.sandbox.commit("docs/x.md", "words", "docs only");
    let deployed = world.sandbox.empty_commit("deploy");

    let stub = StubGitHub::new()
        .runs(
            "deploy-production.yml",
            &[StubRun::success(601, "2026-01-05T00:00:00Z", &deployed)],
        )
        .jobs(601, &[StubJob::deploy(981, "deploy-api")]);

    let github = GitHub::new(Box::new(stub), "acme".into(), "widgets".into());
    let context = world.context("production");
    let runs = runs_for(&github, "deploy-production.yml", false);
    let set = build_targets(&context, &world.repo, &github, &runs);

    let report = verdict::commit_report(&context, &world.repo, &set, &elsewhere);
    assert_eq!(report.status, Status::NotCovered);
}
