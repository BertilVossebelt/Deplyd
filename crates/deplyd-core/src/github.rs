//! What GitHub says about runs, jobs, deployments and pull requests. Nothing here
//! builds a URL; every request names a gateway `Route`.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;

use crate::gateway::http::{HttpError, Route, Transport};

/// How far back to look. Said in the output when it bites, since silence would
/// otherwise read as "no such target".
pub const RUNS_PER_WORKFLOW: u32 = 15;
/// How many deployments to walk when matching runs to an environment.
pub const DEPLOYMENTS_PER_ENVIRONMENT: u32 = 20;
/// How many requests may be in flight. Modest on purpose: secondary rate limits
/// punish bursts, and eight is most of the win.
pub const MAX_IN_FLIGHT: usize = 8;

#[derive(Debug, Clone, Deserialize)]
pub struct Run {
    pub id: u64,
    pub created_at: String,
    pub html_url: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub head_sha: String,

    /// Filled in by deplyd, not by GitHub: which workflow file this run came from.
    #[serde(skip)]
    pub workflow_file: String,
    #[serde(skip)]
    pub workflow_token: String,
    #[serde(skip)]
    pub needs_log: bool,
}

impl Run {
    pub fn succeeded(&self) -> bool {
        self.conclusion.as_deref() == Some("success")
    }

    pub fn completed(&self) -> bool {
        self.status == "completed"
    }
}

/// One workflow to ask about, with the caller's position so answers can be put
/// back in order.
#[derive(Debug, Clone)]
pub struct WorkflowRequest {
    pub index: usize,
    pub file: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RunsResponse {
    #[serde(default)]
    workflow_runs: Vec<Run>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub name: String,
    pub conclusion: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub conclusion: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

impl Job {
    pub fn succeeded(&self) -> bool {
        self.conclusion.as_deref() == Some("success")
    }

    pub fn skipped_steps(&self) -> Vec<String> {
        self.steps
            .iter()
            .filter(|step| step.conclusion.as_deref() == Some("skipped"))
            .map(|step| step.name.clone())
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct JobsResponse {
    #[serde(default)]
    jobs: Vec<Job>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Deployment {
    pub id: u64,
    pub sha: String,
    #[serde(default)]
    pub environment: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeploymentStatus {
    #[serde(default)]
    pub log_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestHead {
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub number: u32,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub merged_at: Option<String>,
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
    #[serde(default)]
    pub head: Option<PullRequestHead>,
}

impl PullRequest {
    pub fn merged(&self) -> bool {
        self.merged_at.is_some()
    }

    pub fn branch(&self) -> &str {
        self.head.as_ref().map_or("", |head| &head.reference)
    }
}

/// One jobs list and one log per run, however many targets ask for them.
#[derive(Default)]
struct Caches {
    jobs: HashMap<u64, Vec<Job>>,
    logs: HashMap<u64, Option<String>>,
    /// Keyed by run and environment: one run can deploy to several from different
    /// checkouts.
    deployment_shas: HashMap<String, String>,
    /// Which environments have already been walked.
    deployments_asked: HashMap<String, Vec<u64>>,
}

pub struct GitHub {
    http: Box<dyn Transport>,
    owner: String,
    repo: String,
    caches: Mutex<Caches>,
}

impl GitHub {
    pub fn new(http: Box<dyn Transport>, owner: String, repo: String) -> Self {
        Self {
            http,
            owner,
            repo,
            caches: Mutex::new(Caches::default()),
        }
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    fn get(&self, route: &Route) -> Result<String, HttpError> {
        self.http.get(route, &self.owner, &self.repo)
    }

    /// Recent runs of one workflow.
    pub fn runs_for_workflow(&self, workflow_file: &str) -> Result<Vec<Run>, HttpError> {
        let body = self.get(&Route::WorkflowRuns {
            workflow_file: workflow_file.to_string(),
            limit: RUNS_PER_WORKFLOW,
        })?;
        let parsed: RunsResponse = serde_json::from_str(&body)
            .map_err(|error| HttpError::Transport(format!("unreadable runs: {error}")))?;
        Ok(parsed.workflow_runs)
    }

    /// Recent runs of several workflows at once. They have nothing to do with each
    /// other, so waiting for each in turn was six round trips for no reason.
    pub fn runs_for_workflows(&self, workflows: &[WorkflowRequest]) -> Vec<(usize, Vec<Run>)> {
        let mut collected: Vec<(usize, Vec<Run>)> = Vec::new();

        for chunk in workflows.chunks(MAX_IN_FLIGHT) {
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|request| {
                        scope.spawn(move || {
                            (
                                request.index,
                                self.runs_for_workflow(&request.file).unwrap_or_default(),
                            )
                        })
                    })
                    .collect();

                for handle in handles {
                    if let Ok(found) = handle.join() {
                        collected.push(found);
                    }
                }
            });
        }

        collected.sort_by_key(|(index, _)| *index);
        collected
    }

    /// Fetches several job logs at once. These are the large, slow requests, and the
    /// loop that builds targets used to read them one at a time.
    pub fn prefetch_logs(&self, job_ids: &[u64]) {
        let pending: Vec<u64> = {
            let Ok(caches) = self.caches.lock() else {
                return;
            };
            job_ids
                .iter()
                .copied()
                .filter(|id| !caches.logs.contains_key(id))
                .collect()
        };

        for chunk in pending.chunks(MAX_IN_FLIGHT) {
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|job_id| {
                        let id = *job_id;
                        scope.spawn(move || (id, self.get(&Route::JobLog { job_id: id }).ok()))
                    })
                    .collect();

                for handle in handles {
                    if let Ok((job_id, log)) = handle.join()
                        && let Ok(mut caches) = self.caches.lock()
                    {
                        caches.logs.insert(job_id, log);
                    }
                }
            });
        }
    }

    /// The jobs of a run, cached: several targets come from one run.
    pub fn jobs(&self, run_id: u64) -> Vec<Job> {
        if let Some(found) = self
            .caches
            .lock()
            .ok()
            .and_then(|caches| caches.jobs.get(&run_id).cloned())
        {
            return found;
        }

        let jobs = self
            .get(&Route::RunJobs { run_id })
            .ok()
            .and_then(|body| serde_json::from_str::<JobsResponse>(&body).ok())
            .map(|parsed| parsed.jobs)
            .unwrap_or_default();

        if let Ok(mut caches) = self.caches.lock() {
            caches.jobs.insert(run_id, jobs.clone());
        }
        jobs
    }

    /// Fetches several runs' jobs at once: the call is small and the waiting is all
    /// latency.
    pub fn prefetch_jobs(&self, run_ids: &[u64]) {
        let pending: Vec<u64> = {
            let Ok(caches) = self.caches.lock() else {
                return;
            };
            run_ids
                .iter()
                .copied()
                .filter(|id| !caches.jobs.contains_key(id))
                .collect()
        };

        for chunk in pending.chunks(MAX_IN_FLIGHT) {
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|run_id| {
                        let id = *run_id;
                        scope.spawn(move || (id, self.fetch_jobs_uncached(id)))
                    })
                    .collect();

                for handle in handles {
                    if let Ok((run_id, jobs)) = handle.join()
                        && let Ok(mut caches) = self.caches.lock()
                    {
                        caches.jobs.insert(run_id, jobs);
                    }
                }
            });
        }
    }

    fn fetch_jobs_uncached(&self, run_id: u64) -> Vec<Job> {
        self.get(&Route::RunJobs { run_id })
            .ok()
            .and_then(|body| serde_json::from_str::<JobsResponse>(&body).ok())
            .map(|parsed| parsed.jobs)
            .unwrap_or_default()
    }

    /// One job's log as plain text. Per job, not per run: the run endpoint returns a
    /// zip of every job's log, told apart afterwards by a text column - the step that
    /// could hand one matrix leg another's commit.
    pub fn job_log(&self, job_id: u64) -> Option<String> {
        if let Ok(caches) = self.caches.lock()
            && let Some(found) = caches.logs.get(&job_id)
        {
            return found.clone();
        }

        let log = self.get(&Route::JobLog { job_id }).ok();
        if let Ok(mut caches) = self.caches.lock() {
            caches.logs.insert(job_id, log.clone());
        }
        log
    }

    /// Walks an environment's deployments, returning the run ids that created them
    /// and recording each commit. One walk per environment; the per-deployment
    /// statuses calls are independent, so they go together.
    pub fn deployments(&self, environment: &str) -> Vec<u64> {
        if let Ok(caches) = self.caches.lock()
            && let Some(found) = caches.deployments_asked.get(environment)
        {
            return found.clone();
        }

        let route = Route::Deployments {
            environment: (!environment.is_empty()).then(|| environment.to_string()),
            limit: DEPLOYMENTS_PER_ENVIRONMENT,
        };

        let deployments: Vec<Deployment> = self
            .get(&route)
            .ok()
            .and_then(|body| serde_json::from_str(&body).ok())
            .unwrap_or_default();

        let mut run_ids: Vec<u64> = Vec::new();
        let mut shas: Vec<(u64, String, String)> = Vec::new();

        for chunk in deployments.chunks(MAX_IN_FLIGHT) {
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|deployment| {
                        scope.spawn(move || {
                            let statuses: Vec<DeploymentStatus> = self
                                .get(&Route::DeploymentStatuses {
                                    deployment_id: deployment.id,
                                    limit: 5,
                                })
                                .ok()
                                .and_then(|body| serde_json::from_str(&body).ok())
                                .unwrap_or_default();
                            (deployment, statuses)
                        })
                    })
                    .collect();

                for handle in handles {
                    let Ok((deployment, statuses)) = handle.join() else {
                        continue;
                    };
                    for status in statuses {
                        let Some(run_id) = status.log_url.as_deref().and_then(run_id_from_log_url)
                        else {
                            continue;
                        };
                        if !run_ids.contains(&run_id) {
                            run_ids.push(run_id);
                        }
                        shas.push((
                            run_id,
                            deployment.environment.clone(),
                            deployment.sha.clone(),
                        ));
                    }
                }
            });
        }

        if let Ok(mut caches) = self.caches.lock() {
            for (run_id, environment, sha) in shas {
                if sha.is_empty() {
                    continue;
                }
                caches
                    .deployment_shas
                    .entry(deployment_key(run_id, &environment))
                    .or_insert_with(|| sha.clone());
                // A run-wide entry too, used only when the environment is unknown.
                caches
                    .deployment_shas
                    .entry(deployment_key(run_id, ""))
                    .or_insert(sha);
            }
            caches
                .deployments_asked
                .insert(environment.to_string(), run_ids.clone());
        }

        run_ids
    }

    /// The commit a deployment record names for a run, if one has been seen.
    pub fn deployment_sha(
        &self,
        run_id: u64,
        environment: &str,
        allow_fetch: bool,
    ) -> Option<String> {
        if let Some(found) = self.find_deployment_sha(run_id, environment) {
            return Some(found);
        }
        if !allow_fetch {
            return None;
        }
        let _ = self.deployments(environment);
        self.find_deployment_sha(run_id, environment)
    }

    fn find_deployment_sha(&self, run_id: u64, environment: &str) -> Option<String> {
        let caches = self.caches.lock().ok()?;
        if let Some(found) = caches
            .deployment_shas
            .get(&deployment_key(run_id, environment))
        {
            return Some(found.clone());
        }
        // With an environment named, stop rather than borrowing another one's commit:
        // a wrong comparison reads as a real disagreement.
        if !environment.is_empty() {
            return None;
        }
        caches
            .deployment_shas
            .get(&deployment_key(run_id, ""))
            .cloned()
    }

    pub fn pull_request(&self, number: u32) -> Option<PullRequest> {
        let body = self.get(&Route::PullRequest { number }).ok()?;
        serde_json::from_str(&body).ok()
    }
}

fn deployment_key(run_id: u64, environment: &str) -> String {
    format!("{run_id}|{}", environment.to_lowercase())
}

/// A deployment status links back to the run that created it through its log URL.
fn run_id_from_log_url(url: &str) -> Option<u64> {
    let (_, tail) = url.split_once("/actions/runs/")?;
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_run_id_out_of_a_deployment_log_url() {
        assert_eq!(
            run_id_from_log_url("https://github.com/acme/widgets/actions/runs/10234567890"),
            Some(10_234_567_890)
        );
        assert_eq!(
            run_id_from_log_url("https://github.com/acme/widgets/actions/runs/123/job/456"),
            Some(123)
        );
        assert_eq!(run_id_from_log_url("https://example.com/nothing"), None);
    }

    #[test]
    fn deployment_keys_separate_environments() {
        assert_ne!(
            deployment_key(1, "production"),
            deployment_key(1, "staging")
        );
        assert_eq!(
            deployment_key(1, "Production"),
            deployment_key(1, "production")
        );
    }
}
