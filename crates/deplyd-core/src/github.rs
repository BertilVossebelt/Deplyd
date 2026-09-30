//! What GitHub says about runs, jobs, deployments and pull requests. Nothing here
//! builds a URL; every request names a gateway `Route`.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;

use crate::gateway::http::{HttpError, Route, Transport};

/// How far back to look. Said in the output when it bites, since silence reads
/// as "no such target".
pub const RUNS_PER_WORKFLOW: u32 = 15;
/// How many deployments to walk when matching runs to an environment.
pub const DEPLOYMENTS_PER_ENVIRONMENT: u32 = 20;
/// How many requests may be in flight. Modest on purpose: secondary rate limits
/// punish bursts, and eight is most of the win.
pub const MAX_IN_FLIGHT: usize = 8;

#[derive(Debug, Clone, Deserialize)]
struct Release {
    tag_name: String,
}

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
    pub completed_at: Option<String>,
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
    /// The longest pause GitHub has asked for and nobody has acted on yet.
    /// Kept apart from the caches, which `forget` empties.
    paused: Mutex<Option<std::time::Duration>>,
    /// Set when a listing failed and was handed on as an empty one. Kept apart
    /// from the caches for the same reason.
    missed: Mutex<bool>,
}

impl GitHub {
    pub fn new(http: Box<dyn Transport>, owner: String, repo: String) -> Self {
        Self {
            http,
            owner,
            repo,
            caches: Mutex::new(Caches::default()),
            paused: Mutex::new(None),
            missed: Mutex::new(false),
        }
    }

    /// How long GitHub asked deplyd to wait, clearing it as it answers. Callers
    /// read a failed request as an empty answer, which is right for one run and
    /// wrong for a loop.
    pub fn rate_limited(&self) -> Option<std::time::Duration> {
        self.paused.lock().ok().and_then(|mut held| held.take())
    }

    /// Whether a listing failed since this was last asked, clearing as it
    /// answers. `rate_limited` covers the refusal GitHub announces; this covers
    /// the rest, which arrive as an empty list and look like a quiet repository.
    pub fn missed_a_read(&self) -> bool {
        self.missed
            .lock()
            .map(|mut held| std::mem::replace(&mut *held, false))
            .unwrap_or(false)
    }

    fn note_missed_read(&self) {
        if let Ok(mut held) = self.missed.lock() {
            *held = true;
        }
    }

    /// Forgets what it has been told. A watcher asks the same questions on
    /// purpose, and memoised answers would make it blind.
    pub fn forget(&self) {
        if let Ok(mut caches) = self.caches.lock() {
            *caches = Caches::default();
        }
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    fn get(&self, route: &Route) -> Result<String, HttpError> {
        let answer = self.http.get(route, &self.owner, &self.repo);
        // The one place every request passes through, and the callers above
        // swallow errors into empty answers.
        if let Err(HttpError::RateLimited { wait, .. }) = &answer
            && let Ok(mut held) = self.paused.lock()
        {
            let longest = held.map_or(*wait, |already| already.max(*wait));
            *held = Some(longest);
        }
        answer
    }

    /// The tag of the newest published release, for the update check.
    pub fn latest_release(&self) -> Option<String> {
        let body = self.get(&Route::LatestRelease).ok()?;
        let parsed: Release = serde_json::from_str(&body).ok()?;
        Some(parsed.tag_name)
    }

    /// What is left of the hourly allowance, and when it refills. From the
    /// headers of a real request, since `/rate_limit` answers about a window of
    /// its own - 5000 of 5000 while the headers count down properly.
    pub fn quota(&self) -> Result<Quota, HttpError> {
        if self.http.allowance().is_none() {
            // One from each window: asking only one reported thousands left
            // while the other was a hundred and fifty requests further on.
            let _ = self.get(&Route::LatestRelease);
            let _ = self.get(&Route::Deployments {
                environment: None,
                limit: 1,
            });
        }
        let stated = self.http.allowance().ok_or_else(|| {
            HttpError::Transport("GitHub said nothing about the allowance".into())
        })?;

        Ok(Quota {
            limit: stated.limit,
            remaining: stated.remaining,
            reset: stated.reset,
        })
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

    /// Empty when the request failed, and the failure remembered: callers want a
    /// list to get on with, a watcher wants to know this was not an answer.
    fn runs_or_none(&self, workflow_file: &str) -> Vec<Run> {
        match self.runs_for_workflow(workflow_file) {
            Ok(found) => found,
            Err(_) => {
                self.note_missed_read();
                Vec::new()
            }
        }
    }

    /// Recent runs of several workflows at once: they have nothing to do with
    /// each other, so waiting for each in turn was six round trips.
    pub fn runs_for_workflows(&self, workflows: &[WorkflowRequest]) -> Vec<(usize, Vec<Run>)> {
        let mut collected: Vec<(usize, Vec<Run>)> = Vec::new();

        for chunk in workflows.chunks(MAX_IN_FLIGHT) {
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|request| {
                        scope.spawn(move || (request.index, self.runs_or_none(&request.file)))
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

    /// Several job logs at once: these are the large, slow requests.
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

    /// Several runs' jobs at once: the call is small, the waiting is latency.
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

    /// One job's log as plain text. Per job, not per run: the run endpoint sends
    /// a zip told apart by a text column, which could hand one matrix leg
    /// another's commit.
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

    /// Walks an environment's deployments for the run ids that created them,
    /// recording each commit. One walk per environment, statuses together.
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

        let deployments: Vec<Deployment> = match self.get(&route) {
            Ok(body) => serde_json::from_str(&body).unwrap_or_default(),
            Err(_) => {
                self.note_missed_read();
                Vec::new()
            }
        };

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
        // With an environment named, stop rather than borrow another one's
        // commit: a wrong comparison reads as a real disagreement.
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

/// The account's hourly allowance for ordinary API reads.
#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct Quota {
    pub limit: u32,
    pub remaining: u32,
    /// Unix time at which the allowance refills.
    pub reset: i64,
}

impl Quota {
    pub fn used(&self) -> u32 {
        self.limit.saturating_sub(self.remaining)
    }

    /// 0.0 to 1.0. A limit of zero is not a real answer, so it reads as spent.
    pub fn spent(&self) -> f64 {
        if self.limit == 0 {
            return 1.0;
        }
        f64::from(self.used()) / f64::from(self.limit)
    }

    /// How long until it refills, or None once that moment has passed.
    pub fn refills_in(&self) -> Option<std::time::Duration> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;
        let left = self.reset - now;
        (left > 0).then(|| std::time::Duration::from_secs(left as u64))
    }
}

#[cfg(test)]
mod quota_tests {
    use super::Quota;

    fn at(limit: u32, remaining: u32) -> Quota {
        Quota {
            limit,
            remaining,
            reset: 0,
        }
    }

    #[test]
    fn what_is_spent_is_what_is_gone() {
        let quota = at(5000, 4000);
        assert_eq!(quota.used(), 1000);
        assert!((quota.spent() - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn a_limit_of_nothing_reads_as_spent_rather_than_dividing_by_zero() {
        let quota = at(0, 0);
        assert_eq!(quota.used(), 0);
        assert!((quota.spent() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn more_remaining_than_the_limit_does_not_wrap() {
        // Not a shape GitHub sends; saturating_sub keeps it from reading as
        // four billion used.
        let quota = at(10, 99);
        assert_eq!(quota.used(), 0);
    }

    #[test]
    fn a_reset_already_past_is_no_wait_at_all() {
        let quota = Quota {
            limit: 5000,
            remaining: 0,
            reset: 1,
        };
        assert_eq!(quota.refills_in(), None);
    }
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
