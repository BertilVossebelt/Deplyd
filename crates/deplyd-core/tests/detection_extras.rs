//! Detection beyond what the PowerShell version could see.

mod support;

use deplyd_core::detect::{facts_from_str, segments};
use deplyd_core::targets::matched_ignore_words;

fn default_ignores() -> Vec<String> {
    deplyd_core::detect::DEFAULT_IGNORE_JOBS
        .iter()
        .map(|w| (*w).to_string())
        .collect()
}

#[test]
fn a_word_inside_another_word_is_not_a_match() {
    // The PowerShell version compared against the name with punctuation stripped, so
    // "deploy-latest" contained "test" and the job was silently dropped.
    let ignore = default_ignores();
    for name in [
        "deploy-latest",
        "deploy-attestation",
        "deploy-fastest",
        "deploy-contest",
        "publish-manifest",
    ] {
        assert!(
            matched_ignore_words(name, &ignore).is_empty(),
            "{name} should not be ignored, got {:?}",
            matched_ignore_words(name, &ignore)
        );
    }
}

#[test]
fn the_words_that_should_match_still_do() {
    let ignore = default_ignores();
    assert_eq!(
        matched_ignore_words("deploy-test-api", &ignore),
        vec!["test"]
    );
    assert_eq!(
        matched_ignore_words("notify-slack", &ignore),
        vec!["notify"]
    );
    assert_eq!(matched_ignore_words("Merge Queue", &ignore), vec!["merge"]);
    // A word still matches the start of a longer one.
    assert_eq!(
        matched_ignore_words("deploy-testing", &ignore),
        vec!["test"]
    );
    assert!(matched_ignore_words("deploy-api", &ignore).is_empty());
}

#[test]
fn camel_case_names_split_into_words() {
    assert_eq!(
        segments("deployApiToProd"),
        vec!["deploy", "api", "to", "prod"]
    );
    assert_eq!(segments("deploy-api"), vec!["deploy", "api"]);
    assert_eq!(
        segments("deploy (api, eu-west-1)"),
        vec!["deploy", "api", "eu", "west", "1"]
    );
}

#[test]
fn a_trigger_path_filter_is_read_as_scope() {
    let text = concat!(
        "name: Ship API\n",
        "on:\n",
        "  push:\n",
        "    branches: [main]\n",
        "    paths:\n",
        "      - services/api/**\n",
        "      - shared/contracts/**\n",
        "jobs:\n",
        "  ship:\n",
        "    steps: [a, b, c]\n",
    );
    let facts = facts_from_str("ship.yml", text).expect("parses");
    assert_eq!(
        facts.trigger_paths,
        vec!["services/api", "shared/contracts"],
        "a workflow that only runs for these paths is saying what it covers"
    );
}

#[test]
fn exclusions_and_bare_wildcards_are_skipped_rather_than_guessed_at() {
    let text = concat!(
        "on:\n",
        "  push:\n",
        "    paths:\n",
        "      - '!services/api/docs/**'\n",
        "      - '**/*.md'\n",
        "      - 'services/*/deploy.sh'\n",
        "      - services/web/**\n",
        "jobs:\n",
        "  ship:\n",
        "    steps: [a]\n",
    );
    let facts = facts_from_str("ship.yml", text).expect("parses");
    assert_eq!(
        facts.trigger_paths,
        vec!["services/web"],
        "only patterns naming one directory are usable"
    );
}

#[test]
fn steps_agreeing_on_a_directory_give_the_job_its_scope() {
    let text = concat!(
        "jobs:\n",
        "  ship:\n",
        "    steps:\n",
        "      - uses: actions/checkout@v4\n",
        "      - name: Build\n",
        "        working-directory: services/api\n",
        "        run: echo build\n",
        "      - name: Ship\n",
        "        working-directory: services/api\n",
        "        run: echo ship\n",
    );
    let facts = facts_from_str("deploy.yml", text).expect("parses");
    assert_eq!(facts.jobs[0].step_working_directories, vec!["services/api"]);
}

#[test]
fn steps_disagreeing_give_no_scope_rather_than_the_wrong_one() {
    let text = concat!(
        "jobs:\n",
        "  ship:\n",
        "    steps:\n",
        "      - name: API\n",
        "        working-directory: services/api\n",
        "        run: echo one\n",
        "      - name: Web\n",
        "        working-directory: services/web\n",
        "        run: echo two\n",
    );
    let facts = facts_from_str("deploy.yml", text).expect("parses");
    assert!(
        facts.jobs[0].step_working_directories.is_empty(),
        "covering both is not the same as covering one, and guessing would be worse"
    );
}

#[test]
fn a_deploy_action_is_recognised_when_the_name_says_nothing() {
    let text = concat!(
        "name: Widgets\n",
        "on:\n",
        "  push:\n",
        "    branches: [main]\n",
        "jobs:\n",
        "  go:\n",
        "    steps:\n",
        "      - uses: actions/checkout@v4\n",
        "      - uses: azure/webapps-deploy@v3\n",
        "      - name: Done\n",
        "        run: echo done\n",
    );
    let facts = facts_from_str("widgets.yml", text).expect("parses");
    assert!(
        facts.deploys_by_action,
        "nothing in the name or the jobs says deploy, but the action does"
    );
}

#[test]
fn a_deploy_command_is_recognised_too() {
    let text = concat!(
        "name: Widgets\n",
        "jobs:\n",
        "  go:\n",
        "    steps:\n",
        "      - name: Apply\n",
        "        run: |\n",
        "          kubectl apply -f manifests/\n",
        "          kubectl rollout status deploy/api\n",
    );
    let facts = facts_from_str("widgets.yml", text).expect("parses");
    assert!(facts.deploys_by_action);
}

#[test]
fn planning_is_not_deploying() {
    let text = concat!(
        "name: Checks\n",
        "jobs:\n",
        "  go:\n",
        "    steps:\n",
        "      - name: Plan\n",
        "        run: terraform plan -out plan.tfplan\n",
        "      - name: Diff\n",
        "        run: kubectl diff -f manifests/\n",
    );
    let facts = facts_from_str("checks.yml", text).expect("parses");
    assert!(
        !facts.deploys_by_action,
        "a plan is the opposite of a deploy and must not be mistaken for one"
    );
}

#[test]
fn a_pinned_action_sha_does_not_hide_it() {
    let text = concat!(
        "jobs:\n",
        "  go:\n",
        "    steps:\n",
        "      - uses: docker/build-push-action@0565240e2d4ab88bba5387d719585280857ece09\n",
    );
    let facts = facts_from_str("x.yml", text).expect("parses");
    assert!(facts.deploys_by_action);
}

#[test]
fn an_author_name_is_a_name_not_a_pattern() {
    // "Ada [Team]" is an invalid regex, and git fails on it outright. Without
    // --fixed-strings deplyd read that failure as "no changes", which is a silent
    // wrong answer about someone's work.
    let sandbox = support::Sandbox::new("author-brackets");
    sandbox.commit("a.txt", "one", "a change");

    let repo = deplyd_core::repo::Repo::discover(&sandbox.path()).expect("repo");
    let found = deplyd_core::report::records(&repo, &["HEAD"], &[], "API", "Ada Lovelace")
        .expect("an author was given");
    assert_eq!(found.len(), 1, "the ordinary case still works");

    // The awkward names must not error, and must not match either.
    for name in ["Ada [Team]", "Ada (Work)", "Ada*", "Ada+B", "a|b"] {
        let found = deplyd_core::report::records(&repo, &["HEAD"], &[], "API", name)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            found.is_empty(),
            "{name} should match nobody, not error and not match everybody"
        );
    }
}

#[test]
fn a_literal_name_still_matches_as_a_substring() {
    let sandbox = support::Sandbox::new("author-partial");
    sandbox.commit("a.txt", "one", "a change");
    let repo = deplyd_core::repo::Repo::discover(&sandbox.path()).expect("repo");

    // The README promises a first name is enough.
    let found = deplyd_core::report::records(&repo, &["HEAD"], &[], "API", "Ada").expect("ok");
    assert_eq!(found.len(), 1);
}

#[test]
fn a_settings_file_that_will_not_parse_stops_rather_than_being_ignored() {
    let sandbox = support::Sandbox::new("broken-override");
    sandbox.workflow("deploy.yml", PLAIN_WORKFLOW_FOR_OVERRIDE);
    sandbox.commit("services/api/a.txt", "one", "first");
    std::fs::write(sandbox.path().join(".deplyd.json"), "{ not json at all").expect("write");

    let repo = deplyd_core::repo::Repo::discover(&sandbox.path()).expect("repo");
    let outcome = deplyd_core::context::Context::build(
        repo.root(),
        "Ada".into(),
        deplyd_core::settings::Settings::default(),
    );
    let Err(error) = outcome else {
        panic!("a broken override should stop deplyd, not be ignored");
    };

    assert!(
        matches!(
            error,
            deplyd_core::context::ContextError::UnreadableOverride { .. }
        ),
        "got {error}"
    );
}

const PLAIN_WORKFLOW_FOR_OVERRIDE: &str = "\
name: Deploy production
on:
  push:
    branches: [main]
jobs:
  deploy-api:
    runs-on: ubuntu-latest
    environment:
      name: production
    steps:
      - uses: actions/checkout@v4
      - name: Build
        run: echo build
      - name: Ship
        run: echo ship
";
