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
        args: &["list", "environments"],
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
        args: &["list", "environments"],
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
        args: &["list", "environments"],
        expect: &["demo", "sandbox", "deploy-anywhere.yml"],
        reject: &[],
    },
    Case {
        fixture: "custom-env",
        name: "keeps an environment name that is not in the alias table",
        args: &["list", "environments"],
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
        args: &["list", "environments"],
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
        args: &["list", "environments"],
        expect: &["production", "staging", "deploy.yml"],
        reject: &[],
    },
    Case {
        fixture: "no-environments",
        name: "a plain deploy workflow with no environment at all",
        args: &["list", "environments"],
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
    // Not a check at run time: config never offers --json, so the parser stops it.
    let sandbox = Sandbox::from_fixture("conventional");
    let output = sandbox.deplyd(&["config", "--json"]);
    assert!(
        output.contains("'--json' is not one of this command's options"),
        "expected --json to be refused for config, got:\n{}",
        indent(&output)
    );
    assert!(
        !output.contains("Deploy workflows"),
        "and it should not have run config anyway"
    );
}

#[test]
fn a_command_that_was_not_understood_points_at_help_instead_of_printing_it() {
    let sandbox = Sandbox::from_fixture("conventional");

    let output = sandbox.deplyd(&["confg"]);
    assert!(
        output.contains("deplyd has no 'confg' command"),
        "expected it to name what it did not understand, got:\n{}",
        indent(&output)
    );
    assert!(
        output.contains("Did you mean config?"),
        "a near miss is worth naming, got:\n{}",
        indent(&output)
    );
    assert!(
        output.contains("--help"),
        "and it should say where the rest is, got:\n{}",
        indent(&output)
    );
    assert!(
        !output.contains("Usage:"),
        "but not print the help page itself, got:\n{}",
        indent(&output)
    );
}

#[test]
fn a_bad_pull_request_number_is_named_before_a_credential_is_looked_for() {
    // The order matters: a typo is the user's to fix and is worth saying first.
    let sandbox = Sandbox::from_fixture("conventional");
    let output = sandbox.deplyd(&["status", "pr", "not-a-number"]);
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
fn the_grouped_verbs_are_the_only_spelling_there_is() {
    let sandbox = Sandbox::from_fixture("conventional");

    // The flat forms are gone rather than quietly still working.
    for old in [
        &["environments"][..],
        &["authors"],
        &["init"],
        &["pr", "412"],
        &["commit", "HEAD"],
    ] {
        let output = sandbox.deplyd(old);
        assert!(
            output.contains("deplyd has no"),
            "`deplyd {}` should no longer parse, got:\n{}",
            old.join(" "),
            indent(&output)
        );
    }

    // And a group with nothing after it names the choices without dumping help.
    let output = sandbox.deplyd(&["list"]);
    assert!(
        output.contains("deplyd list needs to know which one")
            && output.contains("authors, environments"),
        "bare `list` should name what it lists, got:\n{}",
        indent(&output)
    );
    assert!(
        !output.contains("Usage:"),
        "and not print the help page, got:\n{}",
        indent(&output)
    );
}

#[test]
fn a_bare_command_line_reads_like_any_other_verb_missing_its_second_word() {
    let sandbox = Sandbox::from_fixture("conventional");
    let bare = sandbox.deplyd(&[]);
    let listing = sandbox.deplyd(&["list"]);

    for (what, output) in [("deplyd", &bare), ("deplyd list", &listing)] {
        assert!(
            output.contains("needs to know") && output.contains("--help"),
            "`{what}` should say what is missing and where to look, got:
{}",
            indent(output)
        );
        assert!(
            !output.contains("Usage:") && !output.contains("Options:"),
            "`{what}` should not print the help page, got:
{}",
            indent(output)
        );
        // Four lines with something on them: the sentence, the choices, the
        // pointer. It cannot grow back into a page without someone noticing.
        let lines = output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        assert!(
            lines <= 4,
            "`{what}` should stay short, got {lines} lines:
{}",
            indent(output)
        );
    }

    // The verbs are named, but not the one the shell calls and people do not.
    assert!(
        bare.contains("status, watch, list"),
        "got:
{}",
        indent(&bare)
    );
    assert!(
        !bare.contains("complete,") && !bare.ends_with("complete"),
        "the hidden completer should not be offered, got:
{}",
        indent(&bare)
    );
}

#[test]
fn the_reading_key_is_under_the_verbs_that_print_those_words() {
    let sandbox = Sandbox::from_fixture("conventional");

    // Where the output uses them, the words are explained next to it.
    for verb in ["status", "watch"] {
        let output = sandbox.deplyd(&[verb, "--help"]);
        for word in ["Per target", "UNCERTAIN", "NOT COVERED", "Exit codes"] {
            assert!(
                output.contains(word),
                "`deplyd {verb} --help` should explain {word}, got:
{}",
                indent(&output)
            );
        }
    }

    // Everywhere else it would be a wall nobody asked for.
    for args in [&["--help"][..], &["list", "--help"], &["config", "--help"]] {
        let output = sandbox.deplyd(args);
        assert!(
            output.contains("Options:") || output.contains("Commands:"),
            "`deplyd {}` should still print help, got:
{}",
            args.join(" "),
            indent(&output)
        );
        assert!(
            !output.contains("Per target") && !output.contains("Exit codes"),
            "`deplyd {}` should not carry the reading key, got:
{}",
            args.join(" "),
            indent(&output)
        );
    }
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
