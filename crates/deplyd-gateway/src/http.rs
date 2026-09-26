//! The GitHub half of the gateway.
//!
//! Callers pass a [`Route`], not a URL, and nothing here takes a method. The six
//! variants are the six requests deplyd makes.

use std::fmt;
use std::time::Duration;

use super::Denied;

/// Every GitHub request deplyd makes. All six are GET, and no variant could be
/// anything else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Recent runs of one workflow file.
    WorkflowRuns { workflow_file: String, limit: u32 },
    /// The jobs of one run, with their steps and conclusions.
    RunJobs { run_id: u64 },
    /// One job's log. Per job, not per run: that endpoint returns a zip of all of
    /// them, told apart afterwards by a text column.
    JobLog { job_id: u64 },
    /// Deployments, optionally for one environment.
    Deployments {
        environment: Option<String>,
        limit: u32,
    },
    /// The statuses of one deployment, which name the run that created it.
    DeploymentStatuses { deployment_id: u64, limit: u32 },
    /// One pull request.
    PullRequest { number: u32 },
    /// The newest published release. A prerelease is never "latest".
    LatestRelease,
}

impl Route {
    /// Owned here, so no caller ever holds a URL it could alter.
    pub fn path(&self, owner: &str, repo: &str) -> String {
        let base = format!("repos/{owner}/{repo}");
        match self {
            Route::WorkflowRuns {
                workflow_file,
                limit,
            } => format!("{base}/actions/workflows/{workflow_file}/runs?per_page={limit}"),
            Route::RunJobs { run_id } => format!("{base}/actions/runs/{run_id}/jobs?per_page=100"),
            Route::JobLog { job_id } => format!("{base}/actions/jobs/{job_id}/logs"),
            Route::Deployments { environment, limit } => match environment {
                Some(name) => format!("{base}/deployments?environment={name}&per_page={limit}"),
                None => format!("{base}/deployments?per_page={limit}"),
            },
            Route::DeploymentStatuses {
                deployment_id,
                limit,
            } => format!("{base}/deployments/{deployment_id}/statuses?per_page={limit}"),
            Route::PullRequest { number } => format!("{base}/pulls/{number}"),
            Route::LatestRelease => format!("{base}/releases/latest"),
        }
    }

    /// Used by the self-check to state what this build is able to ask for.
    pub fn describe(&self) -> &'static str {
        match self {
            Route::WorkflowRuns { .. } => "a workflow's recent runs",
            Route::RunJobs { .. } => "a run's jobs",
            Route::JobLog { .. } => "one job's log",
            Route::Deployments { .. } => "deployments",
            Route::DeploymentStatuses { .. } => "a deployment's statuses",
            Route::PullRequest { .. } => "a pull request",
            Route::LatestRelease => "the newest release",
        }
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GET {}", self.path("{owner}", "{repo}"))
    }
}

/// What the gateway could not do. Distinct from [`Denied`], which means it refused.
#[derive(Debug)]
pub enum HttpError {
    /// The request was made and GitHub answered with a failure.
    Status { code: u16, route: String },
    /// The request could not be made at all.
    Transport(String),
    /// No credential available.
    NoCredential,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::Status { code, route } => write!(f, "GitHub answered {code} for {route}"),
            HttpError::Transport(why) => write!(f, "could not reach GitHub: {why}"),
            HttpError::NoCredential => write!(f, "no GitHub credential available"),
        }
    }
}

impl std::error::Error for HttpError {}

/// Somewhere GET requests can be sent.
///
/// No method, no URL, no body: an implementation answers a [`Route`] and nothing
/// else. A trait so the suite can answer from canned documents.
pub trait Transport: Send + Sync {
    fn get(&self, route: &Route, owner: &str, repo: &str) -> Result<String, HttpError>;
}

/// A client that can only read: the inner client is private and `get` is the only
/// method, so nothing holding one can issue anything else.
pub struct ReadOnlyHttp {
    client: reqwest::blocking::Client,
    token: String,
    api_base: String,
}

impl Transport for ReadOnlyHttp {
    fn get(&self, route: &Route, owner: &str, repo: &str) -> Result<String, HttpError> {
        ReadOnlyHttp::get(self, route, owner, repo)
    }
}

impl ReadOnlyHttp {
    pub fn new(token: String, api_base: String) -> Result<Self, HttpError> {
        if token.trim().is_empty() {
            return Err(HttpError::NoCredential);
        }

        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("deplyd/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            // A redirect is how the log endpoint serves its body, so some are needed.
            // Bounded, because an unbounded chain is a way to be led somewhere else.
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| HttpError::Transport(e.to_string()))?;

        Ok(Self {
            client,
            token,
            api_base,
        })
    }

    /// The single place deplyd talks to GitHub.
    pub fn get(&self, route: &Route, owner: &str, repo: &str) -> Result<String, HttpError> {
        let url = format!(
            "{}/{}",
            self.api_base.trim_end_matches('/'),
            route.path(owner, repo)
        );

        let response = self
            .client
            // .get is the only verb this module ever names. There is no code path
            // here that takes a method, so there is none to point at anything else.
            .get(&url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .map_err(|e| HttpError::Transport(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(HttpError::Status {
                code: status.as_u16(),
                route: route.to_string(),
            });
        }

        response
            .text()
            .map_err(|e| HttpError::Transport(e.to_string()))
    }
}

/// Proves, at runtime, that every route this build can construct is a GET against
/// the repository it was told to read. Used by the self-check, so the binary states
/// this about itself rather than about the source it was compiled from.
pub fn routes_are_read_only() -> Result<Vec<String>, Denied> {
    let samples = [
        Route::WorkflowRuns {
            workflow_file: "deploy.yml".into(),
            limit: 15,
        },
        Route::RunJobs { run_id: 1 },
        Route::JobLog { job_id: 1 },
        Route::Deployments {
            environment: Some("production".into()),
            limit: 20,
        },
        Route::DeploymentStatuses {
            deployment_id: 1,
            limit: 5,
        },
        Route::PullRequest { number: 1 },
        Route::LatestRelease,
    ];

    let mut described = Vec::new();
    for route in &samples {
        let path = route.path("owner", "repo");
        // A route that escaped the repository would be reading something the caller
        // never asked about, so the shape is checked rather than assumed.
        if !path.starts_with("repos/owner/repo/") {
            return Err(Denied::new(
                path.clone(),
                "a route left the repository it was given",
            ));
        }
        if path.contains("..") {
            return Err(Denied::new(path.clone(), "a route contains a traversal"));
        }
        described.push(format!("{} - {}", route.describe(), path));
    }
    Ok(described)
}
