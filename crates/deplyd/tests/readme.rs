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

/// The verbs the binary offers, in the order `--help` prints them. Asked of the
/// binary, so this is what a README reader could actually run.
fn verbs_the_binary_offers() -> Vec<String> {
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
        "could not read the command list:\n{help}"
    );
    listed
}

/// Every command the README puts forward, from both places it lists them. Prose
/// is deliberately not read: a sentence mentioning a verb is not a promise the
/// way a listing is.
fn commands_the_readme_lists(text: &str) -> Vec<(String, Option<String>)> {
    let mut found = Vec::new();

    // The block starts on the line spelling out the first verb in full.
    let block = text
        .lines()
        .skip_while(|line| !line.starts_with("deplyd status"))
        .take_while(|line| !line.starts_with("```"));
    for line in block {
        // The block is two columns, held apart by a run of spaces: the command
        // on the left, what it does on the right. Only the left half is a claim.
        let Some(command) = line.split("  ").find(|chunk| !chunk.trim().is_empty()) else {
            continue;
        };
        let words: Vec<&str> = command
            .trim()
            .trim_start_matches("deplyd")
            .split_whitespace()
            .collect();
        match words.as_slice() {
            // A third word is the argument, e.g. the 412 in `status pr 412`.
            [verb, second, ..] => found.push((verb.to_string(), Some(second.to_string()))),
            [verb] => found.push((verb.to_string(), None)),
            [] => {}
        }
    }

    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("| `deplyd ") else {
            continue;
        };
        if let Some(verb) = rest.split(['`', ' ']).next().filter(|v| !v.is_empty()) {
            found.push((verb.to_string(), None));
        }
    }

    found
}

#[test]
fn every_command_the_readme_lists_exists() {
    let text = readme();
    let listed = verbs_the_binary_offers();
    let claimed = commands_the_readme_lists(&text);

    let sandbox = Sandbox::empty();
    for (verb, second) in &claimed {
        assert!(
            listed.contains(verb),
            "the README lists `deplyd {verb}`, which the binary does not offer.
             it offers: {listed:?}"
        );

        // A second word is a subcommand, and only the binary knows whether it
        // is one. A verb with no subcommands takes values instead, and then
        // the word has to be one its help names: `remember author` is fine
        // because `remember --help` says author.
        let Some(second) = second else { continue };
        let help = sandbox.deplyd_stdout(&[verb, "--help"]);
        let children: Vec<&str> = help
            .lines()
            .skip_while(|line| !line.starts_with("Commands:"))
            .skip(1)
            .take_while(|line| line.starts_with("  "))
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        if children.is_empty() {
            let named = help
                .split(|c: char| !c.is_alphanumeric())
                .any(|word| word == second);
            assert!(
                named,
                "the README lists `deplyd {verb} {second}`, and {verb}'s help never mentions {second}:
{help}"
            );
            continue;
        }
        assert!(
            children.contains(&second.as_str()),
            "the README lists `deplyd {verb} {second}`, which {verb} does not take.
             it takes: {children:?}"
        );
    }

    assert!(
        claimed.len() >= 12,
        "expected to check both listings, saw {}",
        claimed.len()
    );
}

#[test]
fn every_command_the_binary_offers_is_in_the_readme() {
    // The other direction: a verb added without a line about it is a verb nobody
    // will find.
    let text = readme();
    let listed = verbs_the_binary_offers();
    let claimed = commands_the_readme_lists(&text);

    for verb in &listed {
        assert!(
            claimed.iter().any(|(named, _)| named == verb),
            "the binary offers `deplyd {verb}`, which the README does not list"
        );
    }
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
