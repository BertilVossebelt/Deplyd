//! The descendant of `Test-SourceIsReadOnly`.
//!
//! The PowerShell version scanned its own files before loading them, which it could
//! do because the source was the program. Here the equivalent runs at build time:
//! the crate fails its tests if anything outside `src/gateway/` can start a process,
//! or if the gateway's narrow opt-out from the lint is copied somewhere else.
//!
//! This is a backstop. The primary guarantee is that dangerous operations are
//! unrepresentable - there is no `Verb::Push` to call. This catches the case where
//! someone reaches past the gateway rather than widening it.

use std::fs;
use std::path::{Path, PathBuf};

/// Everything the gateway is allowed to do that nothing else is.
const GATEWAY_ONLY: &[(&str, &str)] = &[
    ("std::process::Command", "spawning a process"),
    ("process::Command", "spawning a process"),
    ("Command::new", "spawning a process"),
    ("clippy::disallowed_types", "opting out of the spawn lint"),
    ("clippy::disallowed_methods", "opting out of the write lint"),
];

/// Constructs that would reach a process or the filesystem without naming either,
/// refused everywhere including the gateway.
const FORBIDDEN_EVERYWHERE: &[(&str, &str)] = &[
    (
        "unsafe ",
        "unsafe code could do anything this audit cannot see",
    ),
    (
        "std::env::set_var",
        "changing the environment changes what a child process is",
    ),
    ("fs::remove_dir_all", "deplyd never deletes"),
    ("fs::remove_file", "deplyd never deletes"),
];

/// The workspace root, two levels above this crate. The guards audit every crate,
/// not just the one they happen to live in: a spawn added to the CLI crate must fail
/// here too.
fn workspace_root() -> PathBuf {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .and_then(Path::parent)
        .expect("crate should sit two levels under the workspace root")
        .to_path_buf()
}

fn source_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => panic!("could not read {}: {error}", directory.display()),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Each crate's tests/ builds fixtures and may spawn; guard_sandbox
                // audits that directory under its own, stricter rule.
                if path
                    .file_name()
                    .is_some_and(|n| n == "tests" || n == "target")
                {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Comments describe the gateway at length, so scanning them would fail on the prose
/// that explains the rule. Strips line comments and string literals, the same
/// reasoning as the PowerShell audit's stripping step.
fn code_only(line: &str) -> String {
    let without_comment = match line.find("//") {
        Some(at) => &line[..at],
        None => line,
    };
    let mut out = String::with_capacity(without_comment.len());
    let mut in_string = false;
    let mut previous = '\0';
    for character in without_comment.chars() {
        if character == '"' && previous != '\\' {
            in_string = !in_string;
            continue;
        }
        if !in_string {
            out.push(character);
        }
        previous = character;
    }
    out
}

/// The gateway is a whole crate now, so the exemption is the crate directory rather
/// than a module name. Matched exactly: a crate called "deplyd-gateway-helpers"
/// added later must not inherit the exemption by looking similar.
fn is_gateway(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "deplyd-gateway")
}

#[test]
fn nothing_outside_the_gateway_can_start_a_process() {
    let root = workspace_root().join("crates");
    let files = source_files(&root);
    assert!(
        files.len() >= 3,
        "expected to audit the crate, found {} file(s) under {}",
        files.len(),
        root.display()
    );

    let mut violations = Vec::new();
    for file in &files {
        if is_gateway(file) {
            continue;
        }
        let text = fs::read_to_string(file).expect("source file should be readable");
        for (number, line) in text.lines().enumerate() {
            let code = code_only(line);
            for (needle, why) in GATEWAY_ONLY {
                if code.contains(needle) {
                    violations.push(format!(
                        "{}:{}: {why} belongs in src/gateway/ - {}",
                        file.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "deplyd reaches git or GitHub outside the read-only gateway:\n{}",
        violations.join("\n")
    );
}

#[test]
fn indirection_is_refused_everywhere() {
    let root = workspace_root().join("crates");

    let mut violations = Vec::new();
    for file in source_files(&root) {
        let text = fs::read_to_string(file.clone()).expect("source file should be readable");
        for (number, line) in text.lines().enumerate() {
            let code = code_only(line);
            for (needle, why) in FORBIDDEN_EVERYWHERE {
                if code.contains(needle) {
                    violations.push(format!(
                        "{}:{}: {why} - {}",
                        file.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "deplyd contains indirection this audit cannot see through:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_gateway_itself_is_still_where_we_think_it_is() {
    // The scan above proves a negative, and a negative is also what you get if the
    // paths are wrong and it scanned nothing. This fails loudly in that case.
    let gateway = workspace_root().join("crates/deplyd-gateway/src/git.rs");
    let text = fs::read_to_string(&gateway).expect("the gateway should exist");
    assert!(
        text.contains("Command::new"),
        "{} no longer spawns git; has the gateway moved?",
        gateway.display()
    );
}
