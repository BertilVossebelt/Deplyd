//! The CLI, driven the way a person drives it.
//!
//! The cases are the PowerShell suite's own declarative table: each names a fixture,
//! the arguments, and the text that must appear. Keeping the expectations rather than
//! rewriting them is the point - they are what the tool has always promised, and a
//! port that quietly changed them would be a rewrite wearing a port's clothes.
//!
//! No case here reaches GitHub. Fixtures have no runs, and the sandbox git cannot
//! speak any protocol but file, so nothing can reach a remote even if something is
//! wrong.

mod support;

use support::Sandbox;

struct Case {
    fixture: &'static str,
    name: &'static str,
    args: &'static [&'static str],
    expect: &'static [&'static str],
    reject: &'static [&'static str],
}

const CASES: &[Case] = &[
    Case {
        fixture: "conventional",
        name: "environments and scopes from conventional filenames",
        args: &["environments"],
        expect: &[
            "production",
            "staging",
            "deploy-production-api.yml",
            "deploy-staging-web.yml",
        ],
        reject: &[],
    },
    Case {
        fixture: "conventional",
        name: "scope and log parsing for a branch-input workflow",
        args: &["config", "-E", "prod"],
        expect: &["Selected       production", "services/api", "the run log"],
        reject: &[],
    },
    Case {
        fixture: "conventional",
        name: "the run's own ref when nothing overrides the checkout",
        args: &["config", "-E", "stag"],
        expect: &[
            "Selected       staging",
            "services/web",
            "the run's own ref",
        ],
        reject: &[],
    },
    Case {
        fixture: "unnamed",
        name: "falls back to declared environments when names match nothing",
        args: &["environments"],
        expect: &[
            "production",
            "staging",
            "hoopdiepoopdieloop.yml",
            "wobble.yml",
        ],
        reject: &[],
    },
    Case {
        fixture: "choice-input",
        name: "reads environments off a workflow_dispatch choice input",
        args: &["environments"],
        expect: &["demo", "sandbox", "deploy-anywhere.yml"],
        reject: &[],
    },
    Case {
        fixture: "custom-env",
        name: "keeps an environment name that is not in the alias table",
        args: &["environments"],
        expect: &["canary", "deploy-canary.yml"],
        reject: &[],
    },
    Case {
        fixture: "external-reusable",
        name: "a job calling a workflow in another repo needs the log read",
        args: &["config"],
        expect: &["Selected       production", "the run log"],
        reject: &[],
    },
    Case {
        fixture: "matrix",
        name: "a matrix workflow keeps its environment and scope",
        args: &["config"],
        expect: &["Selected       production", "services", "the run's own ref"],
        reject: &[],
    },
    Case {
        fixture: "composite-checkout",
        name: "a composite action checkout uses the run ref",
        args: &["config"],
        expect: &["Selected       production", "the run's own ref"],
        reject: &[],
    },
    Case {
        fixture: "staged-pipeline",
        name: "one workflow deploying to staging then production",
        args: &["environments"],
        expect: &["production", "staging", "deploy.yml"],
        reject: &[],
    },
    Case {
        fixture: "scoped-override",
        name: "scopes can be set by hand when there is no working-directory",
        args: &["config"],
        expect: &[
            "Scope override API = services/api",
            "Scope override WEB = services/web",
        ],
        reject: &[],
    },
    Case {
        fixture: "cd-named",
        name: "a workflow called cd.yml is recognised as a deploy",
        args: &["config"],
        expect: &["Deploy workflows (1)", "cd.yml"],
        reject: &[],
    },
    Case {
        fixture: "four-space",
        name: "a four-space indented workflow reads the same as a two-space one",
        args: &["environments"],
        expect: &["production", "staging", "deploy.yml"],
        reject: &[],
    },
    Case {
        fixture: "no-environments",
        name: "a plain deploy workflow with no environment at all",
        args: &["environments"],
        expect: &["No named environments", "which is fine"],
        reject: &[],
    },
    Case {
        fixture: "no-environments",
        name: "still finds the workflow without an environment",
        args: &["config"],
        expect: &["Deploy workflows (1)", "deploy.yml", "the run's own ref"],
        reject: &[],
    },
    Case {
        fixture: "override",
        name: ".deplyd.json overrides the pattern and pins workflows",
        args: &["config"],
        expect: &["zonk.yml"],
        reject: &[],
    },
    Case {
        fixture: "prefix-env",
        name: "an exact environment name beats a longer one sharing its prefix",
        args: &["config", "-E", "canary"],
        expect: &["Selected       canary"],
        reject: &["Ambiguous"],
    },
];

#[test]
fn the_powershell_suites_detection_cases_all_pass() {
    let mut failures = Vec::new();

    for case in CASES {
        let sandbox = Sandbox::from_fixture(case.fixture);
        let output = sandbox.deplyd(case.args);

        for wanted in case.expect {
            if !output.contains(wanted) {
                failures.push(format!(
                    "{} / {}\n  expected to find: {wanted}\n  got:\n{}",
                    case.fixture,
                    case.name,
                    indent(&output)
                ));
            }
        }
        for unwanted in case.reject {
            if output.contains(unwanted) {
                failures.push(format!(
                    "{} / {}\n  should not contain: {unwanted}\n  got:\n{}",
                    case.fixture,
                    case.name,
                    indent(&output)
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n\n")
    );
}

#[test]
fn a_repository_that_deploys_nothing_says_so_rather_than_guessing() {
    let sandbox = Sandbox::from_fixture("no-deploys");
    let output = sandbox.deplyd(&["config"]);
    assert!(
        output.contains("No deploy workflows found"),
        "expected a clear refusal, got:\n{}",
        indent(&output)
    );
}

#[test]
fn an_ambiguous_environment_prefix_is_still_ambiguous() {
    let sandbox = Sandbox::from_fixture("prefix-env");
    let output = sandbox.deplyd(&["config", "-E", "can"]);
    assert!(
        output.contains("Ambiguous"),
        "a prefix matching two environments should stop, got:\n{}",
        indent(&output)
    );
}

#[test]
fn json_is_refused_for_a_command_that_reaches_no_verdict() {
    let sandbox = Sandbox::from_fixture("conventional");
    let output = sandbox.deplyd(&["config", "--json"]);
    assert!(
        output.contains("has nothing to say about"),
        "expected --json to be refused for config, got:\n{}",
        indent(&output)
    );
}

#[test]
fn a_bad_pull_request_number_is_named_before_a_credential_is_looked_for() {
    // The order matters: a typo is the user's to fix and is worth saying first.
    let sandbox = Sandbox::from_fixture("conventional");
    let output = sandbox.deplyd(&["pr", "not-a-number"]);
    assert!(
        output.contains("Not a pull request number"),
        "expected the argument to be checked first, got:\n{}",
        indent(&output)
    );
    assert!(
        !output.contains("gh auth login"),
        "it should not have reached the credential yet"
    );
}

#[test]
fn complete_prints_bare_names_for_the_shell() {
    let sandbox = Sandbox::from_fixture("conventional");
    let output = sandbox.deplyd(&["complete", "environments"]);
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        lines,
        vec!["production", "staging"],
        "the completer must print names and nothing else"
    );
}

#[test]
fn running_it_with_no_command_lists_the_commands() {
    let sandbox = Sandbox::from_fixture("conventional");
    let output = sandbox.deplyd(&[]);
    assert!(output.contains("status"), "should list the commands");
    assert!(
        output.contains("EXIT CODES"),
        "and what the exit codes mean"
    );
}

#[test]
fn the_self_check_needs_no_repository_at_all() {
    let sandbox = Sandbox::empty();
    let output = sandbox.deplyd(&["check"]);
    assert!(output.contains("read-only self-check"));
    assert!(output.contains("PASS"), "the guard should be intact");
    assert!(!output.contains("FAIL"));
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
