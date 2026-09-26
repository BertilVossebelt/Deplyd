//! Where the GitHub token comes from.
//!
//! `gh auth login` is a device flow against access you already have, so no personal
//! access token has to be created and no organisation has to approve one. This is the
//! only place deplyd runs anything but git, and it runs one command with two fixed
//! arguments.

use std::fmt;

/// How the token was obtained, so `deplyd check` can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `GITHUB_TOKEN`, which is what CI provides.
    Environment,
    /// `gh auth token`, which is what a person has.
    GitHubCli,
}

impl Source {
    pub fn describe(self) -> &'static str {
        match self {
            Source::Environment => "the GITHUB_TOKEN environment variable",
            Source::GitHubCli => "gh auth token",
        }
    }
}

#[derive(Debug)]
pub enum CredentialError {
    /// gh is not installed, or not on PATH.
    NoGitHubCli,
    /// gh is there but nobody has logged in.
    NotAuthenticated(String),
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CredentialError::NoGitHubCli => {
                write!(f, "the GitHub CLI (gh) is required and was not found")
            }
            CredentialError::NotAuthenticated(detail) => {
                write!(f, "gh is installed but not signed in: {detail}")
            }
        }
    }
}

impl std::error::Error for CredentialError {}

pub struct Credential {
    pub token: String,
    pub source: Source,
}

/// `GITHUB_TOKEN` first so that CI, where gh may not be installed, needs no setup;
/// then gh, which is where a person's credential lives.
pub fn find() -> Result<Credential, CredentialError> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN")
        && !token.trim().is_empty()
    {
        return Ok(Credential {
            token: token.trim().to_string(),
            source: Source::Environment,
        });
    }

    let token = gh_auth_token()?;
    Ok(Credential {
        token,
        source: Source::GitHubCli,
    })
}

/// Runs `gh auth token` and nothing else. The arguments are a literal, not a
/// parameter, so there is nothing to constrain.
/// Where gh is, which is not always on PATH.
///
/// A terminal keeps the PATH it was started with, so a gh installed since it opened
/// is invisible until it is reopened - and IDE terminals can lag further behind. That
/// made deplyd say gh was missing when it was sitting right there, so after PATH the
/// usual install locations are tried.
fn github_cli_path() -> std::path::PathBuf {
    let candidates: Vec<std::path::PathBuf> = if cfg!(windows) {
        ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|base| {
                let mut path = std::path::PathBuf::from(base);
                if path.ends_with("Local") {
                    path.push("Programs");
                }
                path.join("GitHub CLI").join("gh.exe")
            })
            .collect()
    } else {
        [
            "/opt/homebrew/bin/gh",
            "/usr/local/bin/gh",
            "/home/linuxbrew/.linuxbrew/bin/gh",
            "/usr/bin/gh",
        ]
        .iter()
        .map(std::path::PathBuf::from)
        .collect()
    };

    for candidate in candidates {
        if candidate.is_file() {
            return candidate;
        }
    }
    // Plain name, so PATH is still what answers in the ordinary case.
    std::path::PathBuf::from("gh")
}

#[allow(clippy::disallowed_types)] // the gateway is where spawning is allowed
fn gh_auth_token() -> Result<String, CredentialError> {
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .or_else(|_| {
            std::process::Command::new(github_cli_path())
                .args(["auth", "token"])
                .output()
        })
        .map_err(|_| CredentialError::NoGitHubCli)?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CredentialError::NotAuthenticated(detail));
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(CredentialError::NotAuthenticated(
            "it printed no token".into(),
        ));
    }
    Ok(token)
}

/// Whether gh is installed at all, for the message that says how to get it.
#[allow(clippy::disallowed_types)] // the gateway is where spawning is allowed
pub fn github_cli_present() -> bool {
    let ran = |program: std::path::PathBuf| {
        std::process::Command::new(program)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    };
    ran(std::path::PathBuf::from("gh")) || ran(github_cli_path())
}

/// The owner and repository a remote URL names, in any of the spellings GitHub
/// hands out: https, scp-style and ssh://.
/// Where a remote's repository lives on the web, for building links into it.
/// Keeps the host, so an enterprise instance links to itself rather than github.com.
pub fn web_base(url: &str) -> Option<String> {
    let (owner, repo) = parse_remote(url)?;
    let trimmed = url.trim().trim_end_matches('/');
    let without_git = trimmed.strip_suffix(".git").unwrap_or(trimmed);

    let host = if let Some((before, rest)) = without_git.split_once(':')
        && !rest.starts_with("//")
    {
        before.rsplit('@').next()?.to_string()
    } else {
        let after_scheme = without_git.rsplit("://").next()?;
        let host = after_scheme.split('/').next()?;
        host.rsplit('@').next()?.to_string()
    };

    (!host.is_empty()).then(|| format!("https://{host}/{owner}/{repo}"))
}

pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim().trim_end_matches('/');
    let without_git = trimmed.strip_suffix(".git").unwrap_or(trimmed);

    // scp-style: host:owner/repo
    let path = if let Some((_, rest)) = without_git.split_once(':')
        && !rest.starts_with("//")
    {
        rest.to_string()
    } else {
        // Anything with a scheme: take what follows the host.
        let after_scheme = without_git.rsplit("://").next().unwrap_or(without_git);
        let (_, rest) = after_scheme.split_once('/')?;
        rest.to_string()
    };

    let mut parts = path.trim_matches('/').split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}
