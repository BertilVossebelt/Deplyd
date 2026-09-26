//! The README's claims, checked against the binary.
//!
//! Documentation drifts silently. These are the claims that would mislead someone
//! following the README, so they are asserted rather than trusted.

mod support;

use std::path::Path;

use support::Sandbox;

fn readme() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("README.md");
    std::fs::read_to_string(path).expect("README should be readable")
}

#[test]
fn every_command_the_readme_lists_exists() {
    let text = readme();
    let mut checked = 0;

    // Asked of the binary rather than of its source, so this checks what someone
    // following the README would actually be able to run.
    let sandbox = Sandbox::empty();
    let help = sandbox.deplyd_stdout(&["--help"]);
    let listed: Vec<String> = help
        .lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .collect();
    assert!(
        !listed.is_empty(),
        "could not read the command list:
{help}"
    );

    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("| `deplyd ") else {
            continue;
        };
        let Some(command) = rest.split(['`', ' ']).next() else {
            continue;
        };
        if command.is_empty() {
            continue;
        }

        assert!(
            listed.contains(&command.to_string()),
            "the README lists `deplyd {command}`, which the binary does not offer.
             it offers: {listed:?}"
        );
        checked += 1;
    }

    assert!(
        checked >= 8,
        "expected to check the command table, saw {checked}"
    );
}

#[test]
fn the_readme_does_not_still_use_the_old_vocabulary() {
    let text = readme();
    for stale in ["PUBLISHED", "NOT LIVE", "LIVE in", "-Json", "-RepoPath"] {
        assert!(
            !text.contains(stale),
            "the README still says {stale:?}, which the tool no longer prints"
        );
    }
}

#[test]
fn the_documented_exit_codes_are_the_ones_used() {
    use deplyd_core::verdict::{Status, exit_code};

    // The table in "In a script".
    assert_eq!(exit_code(Status::Deplyd, false), 0);
    assert_eq!(exit_code(Status::NotDeplyd, false), 2);
    assert_eq!(exit_code(Status::NotCovered, false), 2);
    assert_eq!(exit_code(Status::Reverted, false), 3);
    assert_eq!(exit_code(Status::NotMerged, false), 4);
    assert_eq!(exit_code(Status::NotFound, false), 5);
    assert_eq!(exit_code(Status::Deplyd, true), 6);

    let text = readme();
    for code in ['0', '2', '3', '4', '5', '6', '1'] {
        assert!(
            text.contains(&format!("| `{code}`")),
            "exit code {code} is not in the README table"
        );
    }
}

#[test]
fn the_check_output_in_the_readme_matches_what_it_prints() {
    let text = readme();
    for name in deplyd_core::gateway::selfcheck::run() {
        assert!(
            text.contains(name.name),
            "deplyd check prints a row called {:?} that the README does not show",
            name.name
        );
    }
}
