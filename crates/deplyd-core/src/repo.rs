//! The git questions deplyd asks, each one a read, each through the gateway.

use std::path::{Path, PathBuf};

use crate::gateway::Denied;
use crate::gateway::git::{ReadOnlyGit, Verb};

#[derive(Debug)]
pub enum RepoError {
    NotARepository(PathBuf),
    Refused(Denied),
    Io(String),
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoError::NotARepository(path) => {
                write!(f, "not a git repository: {}", path.display())
            }
            RepoError::Refused(denied) => write!(f, "{denied}"),
            RepoError::Io(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for RepoError {}

impl From<Denied> for RepoError {
    fn from(denied: Denied) -> Self {
        RepoError::Refused(denied)
    }
}

/// What one git call printed, and whether it succeeded. Status is kept rather than
/// turned into an error: most of deplyd's questions have "no" as a real answer.
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub ok: bool,
}

impl Output {
    pub fn lines(&self) -> Vec<&str> {
        self.stdout
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .collect()
    }

    pub fn first_line(&self) -> Option<&str> {
        self.lines().first().copied()
    }

    pub fn trimmed(&self) -> &str {
        self.stdout.trim()
    }
}

pub struct Repo {
    root: PathBuf,
    /// Costly on a large repository, so once per run.
    fetched: std::cell::Cell<bool>,
    default_branch: std::cell::RefCell<Option<String>>,
}

impl Repo {
    /// Finds the repository containing `path`, as `git rev-parse --show-toplevel`
    /// reports it.
    pub fn discover(path: &Path) -> Result<Self, RepoError> {
        if !path.exists() {
            return Err(RepoError::NotARepository(path.to_path_buf()));
        }

        let call = ReadOnlyGit::new(Verb::RevParse, &["--show-toplevel"])?;
        let output = call
            .run(path)
            .map_err(|error| RepoError::Io(error.to_string()))?;

        if !output.status.success() {
            return Err(RepoError::NotARepository(path.to_path_buf()));
        }

        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if root.is_empty() {
            return Err(RepoError::NotARepository(path.to_path_buf()));
        }

        Ok(Self {
            root: PathBuf::from(root),
            fetched: std::cell::Cell::new(false),
            default_branch: std::cell::RefCell::new(None),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn has_fetched(&self) -> bool {
        self.fetched.get()
    }

    pub fn run(&self, verb: Verb, args: &[&str]) -> Result<Output, RepoError> {
        let call = ReadOnlyGit::new(verb, args)?;
        let output = call
            .run(&self.root)
            .map_err(|error| RepoError::Io(error.to_string()))?;

        Ok(Output {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            ok: output.status.success(),
        })
    }

    /// `git config user.name`, which is who deplyd reports on unless told otherwise.
    pub fn config(&self, key: &str) -> Option<String> {
        let output = self.run(Verb::Config, &[key]).ok()?;
        if !output.ok {
            return None;
        }
        let value = output.trimmed().to_string();
        (!value.is_empty()).then_some(value)
    }

    /// Updates your own remote-tracking refs. Nothing is sent, and a fetch cannot
    /// change anything on the remote in any case. Once per run.
    pub fn fetch_once(&self) -> Result<bool, RepoError> {
        if self.fetched.get() {
            return Ok(false);
        }
        self.fetched.set(true);
        self.run(Verb::Fetch, &["origin", "--quiet"])?;
        Ok(true)
    }

    /// Whether a commit is in this clone, fetching once if it is not and we have not
    /// already tried.
    pub fn commit_exists(&self, sha: &str, allow_fetch: bool) -> bool {
        if sha.is_empty() {
            return false;
        }
        let spec = format!("{sha}^{{commit}}");
        let verify = |spec: &str| {
            self.run(Verb::RevParse, &["--verify", "--quiet", spec])
                .map(|output| output.ok)
                .unwrap_or(false)
        };

        if verify(&spec) {
            return true;
        }
        if !allow_fetch || self.fetched.get() {
            return false;
        }
        let _ = self.fetch_once();
        verify(&spec)
    }

    /// origin/HEAD is not always set: single-branch clones and many CI checkouts omit
    /// it, so the usual names are tried in turn.
    pub fn default_branch(&self) -> Option<String> {
        if let Some(found) = self.default_branch.borrow().as_ref() {
            return Some(found.clone());
        }

        for candidate in ["origin/HEAD", "origin/main", "origin/master"] {
            let spec = format!("{candidate}^{{commit}}");
            let found = self
                .run(Verb::RevParse, &["--verify", "--quiet", &spec])
                .map(|output| output.ok)
                .unwrap_or(false);
            if found {
                *self.default_branch.borrow_mut() = Some(candidate.to_string());
                return Some(candidate.to_string());
            }
        }
        None
    }

    /// Whether `ancestor` is contained in `descendant`.
    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        self.run(Verb::MergeBase, &["--is-ancestor", ancestor, descendant])
            .map(|output| output.ok)
            .unwrap_or(false)
    }

    /// The files a commit changed.
    ///
    /// `show` prints nothing for a clean merge, whose combined diff is empty, so a
    /// merge is diffed against its first parent instead.
    pub fn commit_files(&self, sha: &str) -> Vec<String> {
        let parents = self
            .run(Verb::RevList, &["--parents", "-n", "1", sha])
            .map(|output| {
                output
                    .trimmed()
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let output = if parents.len() > 2 {
            let first_parent = format!("{sha}^1");
            self.run(Verb::Diff, &["--name-only", &first_parent, sha])
        } else {
            self.run(Verb::Show, &["--name-only", "--format=", sha])
        };

        output
            .map(|output| output.lines().into_iter().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Resolves anything git accepts as a revision - a short sha, a branch, a tag,
    /// HEAD - to a full commit sha.
    pub fn resolve_commit(&self, reference: &str) -> Option<String> {
        if reference.trim().is_empty() {
            return None;
        }
        let spec = format!("{}^{{commit}}", reference.trim());
        let output = self
            .run(Verb::RevParse, &["--verify", "--quiet", &spec])
            .ok()?;
        if !output.ok {
            return None;
        }
        let sha = output.trimmed().to_string();
        (sha.len() == 40).then_some(sha)
    }

    /// The subject line of one commit.
    pub fn subject(&self, sha: &str) -> Option<String> {
        let output = self.run(Verb::Log, &["-1", "--format=%s", sha]).ok()?;
        output.ok.then(|| output.trimmed().to_string())
    }

    /// Authors on a branch, as `shortlog -sn` counts them.
    pub fn authors(&self, branch: &str) -> Vec<(u32, String)> {
        let Ok(output) = self.run(Verb::Shortlog, &["-sn", "--no-merges", branch]) else {
            return Vec::new();
        };

        output
            .lines()
            .into_iter()
            .filter_map(|line| {
                let trimmed = line.trim();
                let (count, name) = trimmed.split_once(char::is_whitespace)?;
                Some((count.trim().parse().ok()?, name.trim().to_string()))
            })
            .collect()
    }

    /// Whether git tracks a path. Asked rather than assumed: deplyd has no idea how a
    /// `.deplyd.json` came to be in a repository, and saying something is committed
    /// when it is not is the kind of claim the rest of the tool refuses to make.
    pub fn is_tracked(&self, path: &Path) -> bool {
        let Some(text) = path.to_str() else {
            return false;
        };
        self.run(Verb::LsFiles, &["--error-unmatch", "--", text])
            .map(|output| output.ok)
            .unwrap_or(false)
    }
}
