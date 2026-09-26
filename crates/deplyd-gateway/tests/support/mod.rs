//! A git repository that cannot reach a remote, however wrong the code under test is.
//!
//! Setting up a fixture needs `init`, `add` and `commit`, which are writes and so have
//! no place in the read-only gateway. That leaves the test harness able to run git
//! directly, which is exactly where a porting accident would come from. So the harness
//! removes the possibility rather than relying on care:
//!
//! * `GIT_ALLOW_PROTOCOL=file` makes git refuse ssh, https and the git protocol
//!   outright. Not a policy deplyd checks - git itself will not speak them.
//! * `HOME`, `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM` point into the sandbox, so
//!   the real user's config, credential helpers and stored tokens are unreachable.
//! * `GIT_TERMINAL_PROMPT=0` and an empty `GIT_ASKPASS` mean nothing can prompt for a
//!   credential that a sandboxed run should never have.
//!
//! Where a remote is needed, it is a bare repository in the same temp directory
//! reached over `file://`. There is no configuration of this sandbox that names a
//! host, so there is nothing to typo into a real one.

#![allow(dead_code)] // each test binary uses a different part of this

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Protocols git is permitted to speak inside the sandbox. Anything else is refused
/// by git, before any deplyd code is involved.
pub const ALLOWED_PROTOCOL: &str = "file";

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct Sandbox {
    root: PathBuf,
    repo: PathBuf,
}

impl Sandbox {
    /// Copies a fixture from the PowerShell suite and turns it into a repository.
    ///
    /// The fixtures are shared rather than duplicated: they describe workflow shapes,
    /// not PowerShell, and two copies would drift the first time one was corrected.
    pub fn from_fixture(name: &str) -> Self {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crate should sit two levels under the workspace root")
            .join("tests/fixtures");
        let source = fixtures.join(name);
        assert!(
            source.is_dir(),
            "no such fixture: {} (looked in {})",
            name,
            fixtures.display()
        );

        let sandbox = Self::empty(name);
        copy_tree(&source, &sandbox.repo);
        sandbox.commit_all("fixture");
        sandbox
    }

    /// A sandbox with an empty repository, for tests that build their own history.
    pub fn empty(label: &str) -> Self {
        let unique = format!(
            "deplyd-{}-{}-{}",
            label.replace(|c: char| !c.is_ascii_alphanumeric(), "-"),
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        let repo = root.join("repo");
        fs::create_dir_all(&repo).expect("sandbox directory should be creatable");
        fs::create_dir_all(root.join("home")).expect("sandbox home should be creatable");

        let sandbox = Self { root, repo };
        sandbox.init_repo();
        sandbox
    }

    pub fn path(&self) -> &Path {
        &self.repo
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn init_repo(&self) {
        self.git(&["init", "--initial-branch=main"]);
        self.git(&["config", "user.name", "Ada Lovelace"]);
        self.git(&["config", "user.email", "ada@example.invalid"]);
        self.git(&["config", "commit.gpgsign", "false"]);
        self.commit_all("initial");
    }

    /// Stages everything present and commits it. Empty commits are allowed: a
    /// sandbox with no fixture still needs a HEAD for most git questions to answer.
    pub fn commit_all(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", message, "--allow-empty"]);
    }

    /// Creates a bare repository inside the sandbox and points `origin` at it over
    /// `file://`, for the one code path that fetches.
    pub fn with_local_origin(self) -> Self {
        let bare = self.root.join("origin.git");
        run_git(&self.root, &self.root, &["init", "--bare", "origin.git"]);

        let url = format!("file://{}", bare.display().to_string().replace('\\', "/"));
        self.git(&["remote", "add", "origin", &url]);
        self.git(&["push", "origin", "main"]);
        self.git(&["fetch", "origin"]);
        self
    }

    /// Runs git inside the sandbox. Writing verbs are allowed here - this is fixture
    /// construction, not deplyd - but the environment makes a remote unreachable.
    pub fn git(&self, args: &[&str]) -> String {
        run_git(&self.root, &self.repo, args)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Only ever inside the system temp directory, and only a path this sandbox
        // made. Checked rather than assumed, because a wrong path here deletes
        // someone's work.
        let temp = std::env::temp_dir();
        if self.root.starts_with(&temp)
            && self
                .root
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("deplyd-"))
        {
            // The only deletion anywhere in deplyd, and it is the sandbox removing
            // what it itself created under the system temp directory, guarded by the
            // two checks above.
            #[allow(clippy::disallowed_methods)]
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

/// The single place the test harness starts git. Mirrors the gateway's role in `src/`:
/// `tests/guard_sandbox.rs` fails if any other test file spawns git directly.
#[allow(clippy::disallowed_types)] // the sandbox is the tests' gateway
pub fn run_git(sandbox_root: &Path, cwd: &Path, args: &[&str]) -> String {
    let home = sandbox_root.join("home");
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        // The whole point: git refuses every protocol but file, so no test can reach
        // a network remote even by accident.
        .env("GIT_ALLOW_PROTOCOL", ALLOWED_PROTOCOL)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
        .env("GIT_CONFIG_SYSTEM", home.join("gitconfig-system"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_PAGER", "cat")
        .env("GIT_AUTHOR_DATE", "2025-01-01T00:00:00+00:00")
        .env("GIT_COMMITTER_DATE", "2025-01-01T00:00:00+00:00")
        .output()
        .expect("git should be installed to run the test suite");

    if !output.status.success() {
        return String::from_utf8_lossy(&output.stderr).into_owned();
    }
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("destination should be creatable");
    for entry in fs::read_dir(from)
        .expect("fixture should be readable")
        .flatten()
    {
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target);
        } else {
            fs::copy(&source, &target).expect("fixture file should be copyable");
        }
    }
}

/// Strips comments and string literals from a line of Rust before auditing it.
///
/// The guard files describe the rules they enforce, in prose and in assertion
/// messages, so scanning raw text would fail on the explanation of the rule rather
/// than on a breach of it.
pub fn code_only(line: &str) -> String {
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
            previous = character;
            continue;
        }
        if !in_string {
            out.push(character);
        }
        previous = character;
    }
    out
}
