//! The sandbox the CLI tests run in: a fixture turned into a repository that cannot
//! reach a remote.
//!
//! The one place in this crate's tests allowed to start a process, mirroring the
//! gateway's role in `src/`. `guard_sandbox.rs` fails if any other test file spawns
//! anything, which is how this module came to exist: the first version of `cli.rs`
//! spawned directly and the guard caught it.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fixture turned into a repository, and a deplyd run against it.
///
/// `GIT_ALLOW_PROTOCOL=file` means git refuses ssh and https outright, so no test
/// here can reach a remote even if the code under test tried.
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    pub fn empty() -> Self {
        let unique = format!(
            "deplyd-cli-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(root.join("repo")).expect("sandbox");
        std::fs::create_dir_all(root.join("home")).expect("sandbox home");
        let sandbox = Self { root };
        sandbox.git(&["init", "-q", "--initial-branch=main"]);
        sandbox.git(&["config", "user.name", "Ada Lovelace"]);
        sandbox.git(&["config", "user.email", "ada@example.invalid"]);
        sandbox
    }

    pub fn from_fixture(name: &str) -> Self {
        let source = workspace_root().join("tests/fixtures").join(name);
        assert!(source.is_dir(), "no such fixture: {name}");

        let sandbox = Self::empty();
        copy_tree(&source, &sandbox.repo());
        sandbox.git(&["add", "-A"]);
        sandbox.git(&["commit", "-qm", "fixture", "--allow-empty"]);
        sandbox
    }

    pub fn repo(&self) -> PathBuf {
        self.root.join("repo")
    }

    /// Runs the built binary and returns stdout and stderr together, which is what a
    /// person sees.
    pub fn deplyd(&self, args: &[&str]) -> String {
        let mut all: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        all.push("--repo-path".into());
        all.push(self.repo().to_string_lossy().into_owned());

        let output = self.spawn(env!("CARGO_BIN_EXE_deplyd"), &all);
        format!("{}{}", output.0, output.1)
    }

    fn git(&self, args: &[&str]) {
        let all: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        self.spawn("git", &all);
    }

    #[allow(clippy::disallowed_types)] // the sandbox is the tests' gateway
    fn spawn(&self, program: &str, args: &[String]) -> (String, String) {
        let home = self.root.join("home");
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(self.repo())
            .env("GIT_ALLOW_PROTOCOL", "file")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("NO_COLOR", "1")
            .output()
            .unwrap_or_else(|e| panic!("could not run {program}: {e}"));

        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let temp = std::env::temp_dir();
        if self.root.starts_with(&temp)
            && self
                .root
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("deplyd-cli-"))
        {
            // The only deletion in the test suite, guarded by the two checks above:
            // inside the system temp directory, and a name this sandbox made.
            #[allow(clippy::disallowed_methods)]
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate sits two levels under the workspace root")
        .to_path_buf()
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("destination");
    for entry in std::fs::read_dir(from).expect("fixture readable").flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target);
        } else {
            std::fs::copy(&source, &target).expect("copy");
        }
    }
}

// ---------------------------------------------------------------------------------
// A repository built commit by commit, with a GitHub made of files
// ---------------------------------------------------------------------------------

impl Sandbox {
    /// Writes a workflow so detection has something to read.
    pub fn workflow(&self, name: &str, contents: &str) {
        let directory = self.repo().join(".github/workflows");
        std::fs::create_dir_all(&directory).expect("workflows");
        std::fs::write(directory.join(name), contents).expect("workflow");
    }

    pub fn write(&self, file: &str, contents: &str) {
        let target = self.repo().join(file);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("directory");
        }
        std::fs::write(target, contents).expect("write");
    }

    /// Commits everything present and returns the full sha.
    pub fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", message, "--allow-empty"]);
        self.spawn("git", &["rev-parse".into(), "HEAD".into()])
            .0
            .trim()
            .to_string()
    }

    pub fn set_origin(&self, slug: &str) {
        self.git(&[
            "remote",
            "add",
            "origin",
            &format!("https://github.invalid/{slug}.git"),
        ]);
    }

    fn stub_dir(&self) -> PathBuf {
        let directory = self.root.join("stub");
        std::fs::create_dir_all(&directory).expect("stub");
        directory
    }

    /// A canned answer for one route, named the way the file transport looks for it.
    pub fn stub(&self, name: &str, contents: &str) {
        std::fs::write(self.stub_dir().join(name), contents).expect("stub file");
    }

    /// Runs deplyd against the stubbed GitHub, returning output and exit code.
    pub fn deplyd_stubbed(&self, args: &[&str]) -> (String, i32) {
        let mut all: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        all.push("--repo-path".into());
        all.push(self.repo().to_string_lossy().into_owned());
        self.spawn_with_stub(env!("CARGO_BIN_EXE_deplyd"), &all)
    }

    #[allow(clippy::disallowed_types)] // the sandbox is the tests' gateway
    fn spawn_with_stub(&self, program: &str, args: &[String]) -> (String, i32) {
        let home = self.root.join("home");
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(self.repo())
            .env("DEPLYD_STUB_DIR", self.stub_dir())
            .env("GIT_ALLOW_PROTOCOL", "file")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("NO_COLOR", "1")
            .env("COLUMNS", "200")
            .output()
            .unwrap_or_else(|e| panic!("could not run {program}: {e}"));

        (
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            output.status.code().unwrap_or(-1),
        )
    }
}

/// A workflow that deploys the run's own commit, scoped to services/api.
pub const API_WORKFLOW: &str = "\
name: Deploy production
on:
  push:
    branches: [main]
jobs:
  deploy-api:
    runs-on: ubuntu-latest
    environment:
      name: production
    defaults:
      run:
        working-directory: services/api
    steps:
      - uses: actions/checkout@v4
      - name: Build
        run: echo build
      - name: Ship
        run: echo ship
";

pub fn runs_json(run_id: u64, head_sha: &str) -> String {
    format!(
        "{{\"workflow_runs\":[{{\"id\":{run_id},\"created_at\":\"2026-09-24T10:00:00Z\",\
          \"html_url\":\"https://github.invalid/acme/widgets/actions/runs/{run_id}\",\
          \"status\":\"completed\",\"conclusion\":\"success\",\"head_sha\":\"{head_sha}\"}}]}}"
    )
}

pub fn jobs_json(job_id: u64, name: &str, skipped: &[&str]) -> String {
    let mut steps = vec![
        "{\"name\":\"Checkout\",\"conclusion\":\"success\"}".to_string(),
        "{\"name\":\"Ship\",\"conclusion\":\"success\"}".to_string(),
        "{\"name\":\"Verify\",\"conclusion\":\"success\"}".to_string(),
    ];
    for step in skipped {
        steps.push(format!(
            "{{\"name\":\"{step}\",\"conclusion\":\"skipped\"}}"
        ));
    }
    format!(
        "{{\"jobs\":[{{\"id\":{job_id},\"name\":\"{name}\",\"conclusion\":\"success\",\
          \"started_at\":\"2026-09-24T10:00:00Z\",\"steps\":[{}]}}]}}",
        steps.join(",")
    )
}

pub fn merged_pr_json(number: u32, title: &str, sha: &str) -> String {
    format!(
        "{{\"number\":{number},\"title\":\"{title}\",\"state\":\"closed\",\
          \"merged_at\":\"2026-09-24T09:00:00Z\",\"merge_commit_sha\":\"{sha}\",\
          \"head\":{{\"ref\":\"feature/thing\"}}}}"
    )
}

impl Sandbox {
    /// Only stdout, for checking that a document is a document.
    #[allow(clippy::disallowed_types)] // the sandbox is the tests' gateway
    pub fn deplyd_stdout(&self, args: &[&str]) -> String {
        let home = self.root.join("home");
        let mut all: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        all.push("--repo-path".into());
        all.push(self.repo().to_string_lossy().into_owned());

        let output = std::process::Command::new(env!("CARGO_BIN_EXE_deplyd"))
            .args(&all)
            .current_dir(self.repo())
            .env("DEPLYD_STUB_DIR", self.stub_dir())
            .env("GIT_ALLOW_PROTOCOL", "file")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("NO_COLOR", "1")
            .output()
            .expect("deplyd should run");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}
