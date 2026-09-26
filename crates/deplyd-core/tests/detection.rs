//! Detection, against the fixtures the PowerShell suite collected.
//!
//! Those tests drive the CLI and match on printed text. These assert the facts the
//! printing is derived from, so a failure names what was misread rather than which
//! line of output moved.

use deplyd_core::detect::{WorkflowFacts, facts_from_str, token};

fn facts(fixture: &str, file: &str) -> WorkflowFacts {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join("tests/fixtures")
        .join(fixture)
        .join(".github/workflows")
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    facts_from_str(file, &text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn tokenising_ignores_punctuation_and_case() {
    assert_eq!(token("Deploy-Production API"), "deployproductionapi");
    assert_eq!(token("deploy_production_api"), "deployproductionapi");
    assert_eq!(token("CD"), "cd");
}

#[test]
fn conventional_production_reads_its_environment_and_scope() {
    let facts = facts("conventional", "deploy-production-api.yml");

    assert_eq!(facts.name, "Deploy production API");
    assert_eq!(facts.declared_environments, vec!["production-api"]);
    assert_eq!(facts.jobs.len(), 1);

    let job = &facts.jobs[0];
    assert_eq!(job.key, "deploy-api");
    assert_eq!(job.working_directory.as_deref(), Some("services/api"));
    assert_eq!(job.step_count, 3);
}

#[test]
fn a_checkout_taking_an_input_ref_needs_the_log() {
    // The run's own ref says nothing about what was deployed when the checkout took
    // a branch from a dispatch input.
    let production = facts("conventional", "deploy-production-api.yml");
    assert!(production.uses_input_ref);
    assert!(production.needs_log());

    // The staging workflow checks out plainly, so it does not.
    let staging = facts("conventional", "deploy-staging-web.yml");
    assert!(!staging.uses_input_ref);
    assert!(!staging.needs_log());
    assert_eq!(
        staging.jobs[0].working_directory.as_deref(),
        Some("services/web")
    );
}

#[test]
fn a_job_calling_another_workflow_needs_the_log() {
    let facts = facts("external-reusable", "deploy-production.yml");

    assert!(facts.calls_workflow);
    assert!(facts.needs_log());
    assert_eq!(
        facts.jobs[0].calls_workflow.as_deref(),
        Some("deploy.yml"),
        "the called workflow should be reduced to its file name"
    );
}

#[test]
fn a_composite_checkout_is_read_as_the_runs_own_ref() {
    // A checkout hidden inside a composite action is assumed to take the ref the run
    // was triggered on, because the workflow file cannot show otherwise.
    let facts = facts("composite-checkout", "deploy-production.yml");
    assert!(!facts.needs_log());
    assert_eq!(facts.declared_environments, vec!["production"]);
}

#[test]
fn environments_are_found_when_the_filename_says_nothing() {
    // "wobble.yml" and "hoopdiepoopdieloop.yml" match no environment by name, so the
    // jobs' own declarations are the only source.
    let wobble = facts("unnamed", "wobble.yml");
    assert_eq!(wobble.declared_environments, vec!["staging"]);

    let other = facts("unnamed", "hoopdiepoopdieloop.yml");
    assert_eq!(other.declared_environments, vec!["production"]);
    assert_eq!(other.jobs[0].working_directory.as_deref(), Some("src/api"));
}

#[test]
fn an_environment_outside_the_alias_table_keeps_its_own_name() {
    let facts = facts("custom-env", "deploy-canary.yml");
    assert_eq!(facts.declared_environments, vec!["canary"]);
}

#[test]
fn one_workflow_deploying_to_two_environments_tells_the_jobs_apart() {
    for fixture in ["staged-pipeline", "four-space"] {
        let facts = facts(fixture, "deploy.yml");
        assert_eq!(
            facts.declared_environments,
            vec!["staging", "production"],
            "{fixture}: both environments should be found, in job order"
        );

        let staging = facts.job("deploy-staging").expect("staging job");
        let production = facts.job("deploy-production").expect("production job");
        assert_eq!(staging.environment.as_deref(), Some("staging"));
        assert_eq!(production.environment.as_deref(), Some("production"));
    }
}

#[test]
fn four_space_indentation_reads_identically_to_two() {
    let two = facts("staged-pipeline", "deploy.yml");
    let four = facts("four-space", "deploy.yml");

    assert_eq!(two.declared_environments, four.declared_environments);
    assert_eq!(
        two.jobs.iter().map(|j| &j.key).collect::<Vec<_>>(),
        four.jobs.iter().map(|j| &j.key).collect::<Vec<_>>()
    );
}

#[test]
fn a_choice_input_offers_its_options_as_environments() {
    let facts = facts("choice-input", "deploy-anywhere.yml");
    assert_eq!(facts.input_environments, vec!["demo", "sandbox"]);
    assert!(
        facts.declared_environments.is_empty(),
        "no job declares one; the options are the only source"
    );
}

#[test]
fn a_matrix_job_records_what_tells_its_legs_apart() {
    let facts = facts("matrix", "deploy-production.yml");
    let job = &facts.jobs[0];

    assert_eq!(job.matrix_dimensions, vec!["service"]);
    assert_eq!(job.working_directory.as_deref(), Some("services"));
    assert!(!facts.needs_log());
}

#[test]
fn a_workflow_with_no_environments_is_still_readable() {
    // A repo that deploys without naming environments is fine, and reports targets
    // without an environment name.
    let facts = facts("no-environments", "deploy.yml");
    assert!(facts.declared_environments.is_empty());
    assert!(facts.input_environments.is_empty());
    assert_eq!(facts.jobs.len(), 1);
    assert_eq!(facts.jobs[0].step_count, 3);
}

#[test]
fn cd_is_recognised_as_a_name() {
    let facts = facts("cd-named", "cd.yml");
    assert_eq!(facts.name, "CD");
    assert_eq!(facts.token, "cdcd", "file stem and name tokenise together");
}

#[test]
fn a_job_is_found_by_key_or_by_display_name() {
    let facts = facts("conventional", "deploy-production-api.yml");

    assert!(facts.job("deploy-api").is_some(), "by key");
    // Jobs from a called workflow arrive as "Deploy API / deploy-api".
    assert!(
        facts.job("Deploy production API / deploy-api").is_some(),
        "by the last segment of a called-workflow name"
    );
    assert!(facts.job("nothing-like-this").is_none());
}

#[test]
fn scope_is_per_job_not_per_workflow() {
    // The PowerShell version kept one list per workflow, so two jobs with different
    // working directories each got the union. The README always described it as a
    // per-job property; this is where the two agree.
    let text = concat!(
        "jobs:\n",
        "  deploy-api:\n",
        "    defaults:\n",
        "      run:\n",
        "        working-directory: services/api\n",
        "    steps: [a, b, c]\n",
        "  deploy-web:\n",
        "    defaults:\n",
        "      run:\n",
        "        working-directory: services/web\n",
        "    steps: [a, b, c]\n",
    );
    let facts = facts_from_str("deploy.yml", text).expect("parses");

    assert_eq!(
        facts
            .job("deploy-api")
            .unwrap()
            .working_directory
            .as_deref(),
        Some("services/api")
    );
    assert_eq!(
        facts
            .job("deploy-web")
            .unwrap()
            .working_directory
            .as_deref(),
        Some("services/web")
    );
    assert_eq!(
        facts.working_directories,
        vec!["services/api", "services/web"],
        "the workflow-wide view keeps both, for the report"
    );
}

#[test]
fn a_workflow_that_cannot_be_read_says_which_line() {
    let error = facts_from_str("deploy.yml", "jobs:\n  deploy:\n    environment: &shared\n")
        .expect_err("an anchor should stop detection");
    assert_eq!(error.line, 3);
    assert!(error.reason.contains("anchor"), "got: {error}");
}
