//! A repository and a GitHub that exist only for the test.
//!
//! The PowerShell suite shadows `git` with a local-only version so that fixtures can
//! be written to but nothing can reach a remote. This does the same for both halves:
//! the repository is real git in a temp directory with no remote configured, and
//! GitHub is a set of canned documents answering routes.
//!
//! Everything above the gateway then runs for real - detection, target building, sha
//! resolution, ancestry, verdicts - without a network or an account anywhere near it.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use deplyd_core::gateway::http::{HttpError, Route, Transport};

static COUNTER: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------------------------
// A repository
// ---------------------------------------------------------------------------------

pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    pub fn new(label: &str) -> Self {
        let unique = format!(
            "deplyd-core-{}-{}-{}",
            label.replace(|c: char| !c.is_ascii_alphanumeric(), "-"),
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
        sandbox.git(&["config", "commit.gpgsign", "false"]);
        sandbox
    }

    pub fn path(&self) -> PathBuf {
        self.root.join("repo")
    }

    /// Writes a file and commits it, returning the full sha.
    pub fn commit(&self, file: &str, contents: &str, message: &str) -> String {
        let target = self.path().join(file);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("directory");
        }
        std::fs::write(&target, contents).expect("write");
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", message]);
        self.head()
    }

    /// A commit touching nothing, for history that only needs to exist.
    pub fn empty_commit(&self, message: &str) -> String {
        self.git(&["commit", "-qm", message, "--allow-empty"]);
        self.head()
    }

    /// Reverts a commit the way git itself writes it, so the revert detection has
    /// something real to find rather than a message shaped like one.
    pub fn revert(&self, sha: &str) -> String {
        self.git(&["revert", "--no-edit", sha]);
        self.head()
    }

    pub fn head(&self) -> String {
        self.run_git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    pub fn short(&self, sha: &str) -> String {
        self.run_git(&["rev-parse", "--short", sha])
            .trim()
            .to_string()
    }

    /// Adds a workflow file so detection has something to read.
    pub fn workflow(&self, name: &str, contents: &str) {
        let directory = self.path().join(".github/workflows");
        std::fs::create_dir_all(&directory).expect("workflows");
        std::fs::write(directory.join(name), contents).expect("workflow");
    }

    /// An origin pointing at a plausible GitHub URL, so owner and repo can be read.
    /// Nothing is ever fetched from it: no such host exists, and the sandbox refuses
    /// every protocol but file in any case.
    pub fn set_origin(&self, slug: &str) {
        self.git(&[
            "remote",
            "add",
            "origin",
            &format!("https://github.invalid/{slug}.git"),
        ]);
    }

    fn git(&self, args: &[&str]) {
        self.run_git(args);
    }

    #[allow(clippy::disallowed_types)] // the sandbox is the tests' gateway
    fn run_git(&self, args: &[&str]) -> String {
        let home = self.root.join("home");
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(self.path())
            // git refuses ssh and https outright, so nothing here can reach a remote
            // even if something above it tried.
            .env("GIT_ALLOW_PROTOCOL", "file")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00+00:00")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00+00:00")
            .output()
            .expect("git should be installed to run the suite");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let temp = std::env::temp_dir();
        if self.root.starts_with(&temp)
            && self
                .root
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("deplyd-core-"))
        {
            #[allow(clippy::disallowed_methods)]
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

// ---------------------------------------------------------------------------------
// A GitHub
// ---------------------------------------------------------------------------------

/// Canned answers, keyed by the route asked for.
#[derive(Default)]
pub struct StubGitHub {
    answers: HashMap<String, String>,
    /// Every route asked for, in order, so a test can assert on what was *not* asked
    /// as well as what was.
    asked: std::sync::Mutex<Vec<String>>,
}

impl StubGitHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn runs(mut self, workflow: &str, runs: &[StubRun]) -> Self {
        let documents: Vec<String> = runs.iter().map(StubRun::to_json).collect();
        self.answers.insert(
            format!("runs:{workflow}"),
            format!("{{\"workflow_runs\":[{}]}}", documents.join(",")),
        );
        self
    }

    pub fn jobs(mut self, run_id: u64, jobs: &[StubJob]) -> Self {
        let documents: Vec<String> = jobs.iter().map(StubJob::to_json).collect();
        self.answers.insert(
            format!("jobs:{run_id}"),
            format!("{{\"jobs\":[{}]}}", documents.join(",")),
        );
        self
    }

    pub fn log(mut self, job_id: u64, lines: &[&str]) -> Self {
        // GitHub prefixes every log line with a timestamp, and the checkout sha is
        // recognised by sitting alone after it.
        let body: Vec<String> = lines
            .iter()
            .map(|line| format!("2026-01-01T00:00:00.0000000Z {line}"))
            .collect();
        self.answers
            .insert(format!("log:{job_id}"), body.join("\n"));
        self
    }

    pub fn deployments(mut self, environment: &str, deployments: &[(u64, &str)]) -> Self {
        let documents: Vec<String> = deployments
            .iter()
            .map(|(id, sha)| {
                format!("{{\"id\":{id},\"sha\":\"{sha}\",\"environment\":\"{environment}\"}}")
            })
            .collect();
        self.answers.insert(
            format!("deployments:{environment}"),
            format!("[{}]", documents.join(",")),
        );
        self
    }

    pub fn deployment_statuses(mut self, deployment_id: u64, run_id: u64) -> Self {
        self.answers.insert(
            format!("statuses:{deployment_id}"),
            format!("[{{\"log_url\":\"https://github.invalid/o/r/actions/runs/{run_id}\"}}]"),
        );
        self
    }

    pub fn pull_request(mut self, number: u32, document: &str) -> Self {
        self.answers
            .insert(format!("pr:{number}"), document.to_string());
        self
    }

    pub fn asked_for(&self) -> Vec<String> {
        self.asked.lock().map(|a| a.clone()).unwrap_or_default()
    }

    fn key(route: &Route) -> String {
        match route {
            Route::WorkflowRuns { workflow_file, .. } => format!("runs:{workflow_file}"),
            Route::RunJobs { run_id } => format!("jobs:{run_id}"),
            Route::JobLog { job_id } => format!("log:{job_id}"),
            Route::Deployments { environment, .. } => {
                format!("deployments:{}", environment.clone().unwrap_or_default())
            }
            Route::DeploymentStatuses { deployment_id, .. } => {
                format!("statuses:{deployment_id}")
            }
            Route::PullRequest { number } => format!("pr:{number}"),
            Route::LatestRelease => "latest-release".to_string(),
        }
    }
}

impl Transport for StubGitHub {
    fn get(&self, route: &Route, _owner: &str, _repo: &str) -> Result<String, HttpError> {
        let key = Self::key(route);
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(key.clone());
        }
        match self.answers.get(&key) {
            Some(document) => Ok(document.clone()),
            // A route nobody set up answers 404, which is what GitHub would say and
            // what the code under test has to cope with anyway.
            None => Err(HttpError::Status {
                code: 404,
                route: key,
            }),
        }
    }
}

pub struct StubRun {
    pub id: u64,
    pub created_at: &'static str,
    pub status: &'static str,
    pub conclusion: &'static str,
    pub head_sha: String,
}

impl StubRun {
    pub fn success(id: u64, created_at: &'static str, head_sha: &str) -> Self {
        Self {
            id,
            created_at,
            status: "completed",
            conclusion: "success",
            head_sha: head_sha.to_string(),
        }
    }

    pub fn with(
        id: u64,
        created_at: &'static str,
        status: &'static str,
        conclusion: &'static str,
    ) -> Self {
        Self {
            id,
            created_at,
            status,
            conclusion,
            head_sha: "0".repeat(40),
        }
    }

    fn to_json(&self) -> String {
        let conclusion = if self.conclusion.is_empty() {
            "null".to_string()
        } else {
            format!("\"{}\"", self.conclusion)
        };
        format!(
            "{{\"id\":{},\"created_at\":\"{}\",\"html_url\":\"https://github.invalid/o/r/actions/runs/{}\",\"status\":\"{}\",\"conclusion\":{conclusion},\"head_sha\":\"{}\"}}",
            self.id, self.created_at, self.id, self.status, self.head_sha
        )
    }
}

pub struct StubJob {
    pub id: u64,
    pub name: &'static str,
    pub conclusion: &'static str,
    pub started: bool,
    pub steps: Vec<(&'static str, &'static str)>,
}

impl StubJob {
    /// A job with three successful steps, which is the smallest thing deplyd will
    /// treat as a deploy.
    pub fn deploy(id: u64, name: &'static str) -> Self {
        Self {
            id,
            name,
            conclusion: "success",
            started: true,
            steps: vec![
                ("Checkout", "success"),
                ("Build", "success"),
                ("Ship", "success"),
            ],
        }
    }

    pub fn with_steps(mut self, steps: Vec<(&'static str, &'static str)>) -> Self {
        self.steps = steps;
        self
    }

    pub fn concluded(mut self, conclusion: &'static str) -> Self {
        self.conclusion = conclusion;
        self
    }

    fn to_json(&self) -> String {
        let steps: Vec<String> = self
            .steps
            .iter()
            .map(|(name, conclusion)| {
                format!("{{\"name\":\"{name}\",\"conclusion\":\"{conclusion}\"}}")
            })
            .collect();
        let started = if self.started {
            "\"2026-01-01T00:00:00Z\""
        } else {
            "null"
        };
        format!(
            "{{\"id\":{},\"name\":\"{}\",\"conclusion\":\"{}\",\"started_at\":{started},\"steps\":[{}]}}",
            self.id,
            self.name,
            self.conclusion,
            steps.join(",")
        )
    }
}

/// A workflow that deploys plainly: the run's own ref is the commit.
pub const PLAIN_WORKFLOW: &str = "\
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

/// A workflow whose checkout takes a branch input, so the log has to be read.
pub const INPUT_REF_WORKFLOW: &str = "\
name: Deploy production
on:
  workflow_dispatch:
    inputs:
      branch:
        type: string
jobs:
  deploy-api:
    runs-on: ubuntu-latest
    environment:
      name: production
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ inputs.branch }}
      - name: Build
        run: echo build
      - name: Ship
        run: echo ship
";

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate sits two levels under the workspace root")
        .to_path_buf()
}

/// A cache file that cleans itself up.
///
/// Deletion lives here because this module is the one place in the tests allowed to
/// remove anything, which is why the guard failed when the cases did it themselves.
pub struct TempCache {
    path: PathBuf,
}

impl TempCache {
    pub fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "deplyd-cache-{}-{}-{}.json",
            label,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let cache = Self { path };
        cache.clear();
        cache
    }

    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn open(&self) -> deplyd_core::cache::Cache {
        deplyd_core::cache::Cache::at(self.path.clone())
    }

    fn clear(&self) {
        if self.path.starts_with(std::env::temp_dir()) {
            #[allow(clippy::disallowed_methods)]
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Drop for TempCache {
    fn drop(&mut self) {
        self.clear();
    }
}
