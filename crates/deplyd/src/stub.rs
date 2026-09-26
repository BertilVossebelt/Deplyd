//! A GitHub made of files on disk, for testing the report without a network.
//!
//! `DEPLYD_STUB_DIR` answers the same six routes from files. It changes where answers
//! come from, not what may be asked.

use std::path::PathBuf;

use deplyd_core::gateway::http::{HttpError, Route, Transport};

pub const STUB_VARIABLE: &str = "DEPLYD_STUB_DIR";

pub struct FileTransport {
    directory: PathBuf,
}

impl FileTransport {
    /// Some only when the variable is set to a directory that exists.
    pub fn from_environment() -> Option<Self> {
        let directory = PathBuf::from(std::env::var_os(STUB_VARIABLE)?);
        directory.is_dir().then_some(Self { directory })
    }

    fn file_for(route: &Route) -> String {
        match route {
            Route::WorkflowRuns { workflow_file, .. } => format!("runs-{workflow_file}.json"),
            Route::RunJobs { run_id } => format!("jobs-{run_id}.json"),
            Route::JobLog { job_id } => format!("log-{job_id}.txt"),
            Route::Deployments { environment, .. } => format!(
                "deployments-{}.json",
                environment.clone().unwrap_or_else(|| "all".into())
            ),
            Route::DeploymentStatuses { deployment_id, .. } => {
                format!("statuses-{deployment_id}.json")
            }
            Route::PullRequest { number } => format!("pr-{number}.json"),
            Route::LatestRelease => "latest-release.json".to_string(),
        }
    }
}

impl Transport for FileTransport {
    fn get(&self, route: &Route, _owner: &str, _repo: &str) -> Result<String, HttpError> {
        let name = Self::file_for(route);
        std::fs::read_to_string(self.directory.join(&name)).map_err(|_| HttpError::Status {
            code: 404,
            route: name,
        })
    }
}
