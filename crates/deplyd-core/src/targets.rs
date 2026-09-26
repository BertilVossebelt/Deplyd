//! Turning deploy runs into targets: one per deploy job, each with the commit it
//! checked out, what it skipped, and what it covers.

use std::collections::BTreeMap;

use crate::cache::Cache;
use crate::context::{Context, resolve_alias};
use crate::detect::token;
use crate::github::{GitHub, Job, Run};
use crate::repo::Repo;

/// Stop after this many runs in a row reveal no new target.
pub const BARREN_RUNS_BEFORE_STOPPING: usize = 3;

/// Carry no information. Stripped whole: as substrings they leave "build-and".
const LABEL_NOISE: &[&str] = &[
    "deploy",
    "deploys",
    "deployment",
    "deploying",
    "quick",
    "to",
    "and",
    "the",
    "job",
];

/// Where a target's commit came from, which decides how far to trust it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaSource {
    /// The run's own ref, nothing having overridden the checkout.
    RunRef,
    /// `actions/checkout` printed it in the job's log.
    RunLog,
    /// The log was gone or unusable, so this is what the deployment names.
    DeploymentRecord,
}

impl ShaSource {
    pub fn describe(self) -> &'static str {
        match self {
            ShaSource::RunRef => "the run's own ref",
            ShaSource::RunLog => "the run log",
            ShaSource::DeploymentRecord => "the deployment record",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Concern {
    pub run_id: u64,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub label: String,
    pub job: String,
    pub run_id: u64,
    pub run_url: String,
    pub run_created_at: String,
    pub run_workflow_file: String,
    pub sha: String,
    /// False for a guess, or when two records disagree.
    pub sha_is_exact: bool,
    pub sha_source: ShaSource,
    pub sha_warning: String,
    /// The deployment record independently names the same commit.
    pub corroborated: bool,
    pub skipped: Vec<String>,
    pub scope: Vec<String>,
    pub concerns: Vec<Concern>,
}

impl Target {
    /// Whether this target's reading is sound enough to gate on.
    pub fn uncertain(&self) -> bool {
        !self.concerns.is_empty() || !self.sha_is_exact
    }

    pub fn state(&self) -> &'static str {
        if self.uncertain() {
            "uncertain"
        } else {
            "deplyd"
        }
    }

    /// Whether any file is inside what this covers. No scope covers everything.
    pub fn covers(&self, files: &[String]) -> bool {
        if self.scope.is_empty() {
            return true;
        }
        files.iter().any(|file| {
            self.scope
                .iter()
                .any(|scope| file.starts_with(scope.as_str()))
        })
    }
}

/// Targets in job order, plus what was passed over and why, so "nothing found"
/// is never a dead end.
pub struct TargetSet {
    pub targets: Vec<Target>,
    pub ignored_jobs: BTreeMap<String, String>,
    pub runs_examined: usize,
    pub runs_available: usize,
    pub stopped_early: bool,
}

impl TargetSet {
    pub fn get(&self, label: &str) -> Option<&Target> {
        self.targets.iter().find(|target| target.label == label)
    }

    /// False when the column would say the same thing on every row.
    pub fn labels_are_informative(&self) -> bool {
        self.targets.len() >= 2 && self.targets.iter().any(|target| !target.scope.is_empty())
    }

    pub fn label_width(&self) -> usize {
        self.targets
            .iter()
            .map(|target| target.label.len())
            .max()
            .unwrap_or(0)
            .max(8)
    }
}

/// A job's name becomes its label, so `deploy-api` reports as `API`.
pub fn target_label(job_name: &str) -> String {
    let last = job_name.rsplit('/').next().unwrap_or(job_name).trim();

    // A matrix leg arrives as "deploy (api, eu-west-1)", and those values are what
    // tell the legs apart, so they belong in the label.
    let (name, values) = match (last.find('('), last.ends_with(')')) {
        (Some(open), true) => (&last[..open], &last[open + 1..last.len() - 1]),
        _ => (last, ""),
    };

    let parts: Vec<String> = [clean_label_part(name), clean_label_part(values)]
        .into_iter()
        .filter(|piece| !piece.is_empty())
        .collect();

    if parts.is_empty() {
        // A job with no name at all would otherwise get an empty label, and an empty
        // label matches every workflow when concerns are worked out.
        if last.trim().is_empty() {
            return "DEPLOY".to_string();
        }
        return last.to_uppercase();
    }
    parts.join("-").to_uppercase()
}

fn clean_label_part(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c == '_' || c == ',' || c == '-')
        .filter(|segment| !segment.is_empty())
        .filter(|segment| !LABEL_NOISE.contains(&segment.to_lowercase().as_str()))
        .collect::<Vec<_>>()
        .join("-")
}

/// Ignore words that matched a job name, compared against whole words: matching
/// anywhere made "deploy-latest" contain "test". A word still matches the start of a
/// longer one, so "prod" catches "production".
pub fn matched_ignore_words(name: &str, ignore_jobs: &[String]) -> Vec<String> {
    let words = crate::detect::segments(name);
    ignore_jobs
        .iter()
        .filter(|entry| {
            let needle = token(entry);
            !needle.is_empty() && words.iter().any(|word| word.starts_with(&needle))
        })
        .cloned()
        .collect()
}

/// The commit a job checked out, read from its own log.
///
/// `actions/checkout` prints it alone on a line after GitHub's timestamp. That shape
/// is the only trustworthy source; anything looser is reported as a guess.
pub struct ResolvedSha {
    pub sha: String,
    pub exact: bool,
    pub warning: String,
}

/// The one question resolving a commit asks of a repository. A trait so the logic
/// above can be tested without building real history.
pub trait CommitLookup {
    fn commit_exists(&self, sha: &str, allow_fetch: bool) -> bool;
}

impl CommitLookup for Repo {
    fn commit_exists(&self, sha: &str, allow_fetch: bool) -> bool {
        Repo::commit_exists(self, sha, allow_fetch)
    }
}

pub fn resolve_checkout_sha(log: &str, repo: &dyn CommitLookup) -> Option<ResolvedSha> {
    let mut bare: Vec<String> = Vec::new();

    for line in log.lines() {
        if line.contains("Download action repository") {
            continue;
        }
        let Some(candidate) = bare_sha_on_line(line) else {
            continue;
        };
        if repo.commit_exists(&candidate, true) && !bare.contains(&candidate) {
            bare.push(candidate);
        }
    }

    if let Some(first) = bare.first() {
        return Some(ResolvedSha {
            sha: first.clone(),
            exact: true,
            warning: if bare.len() > 1 {
                format!(
                    "the job checked out {} different commits; using the first",
                    bare.len()
                )
            } else {
                String::new()
            },
        });
    }

    // Nothing bare, so fall back to any commit named anywhere in the log. Without
    // fetching: pinned action SHAs and cache keys are forty hex characters too, and
    // fetching on the first that misses would pay the whole cost to answer a guess.
    for line in log.lines() {
        if line.contains("Download action repository") {
            continue;
        }
        for candidate in hex_runs(line) {
            if repo.commit_exists(&candidate, false) {
                return Some(ResolvedSha {
                    sha: candidate,
                    exact: false,
                    warning:
                        "no checkout line found for this job; this commit was taken from elsewhere \
                         in the run log"
                            .into(),
                });
            }
        }
    }

    None
}

/// A line whose only content after the timestamp is a forty-character hex string.
fn bare_sha_on_line(line: &str) -> Option<String> {
    // GitHub prefixes "2026-09-25T10:11:12.1234567Z ". Anything before the Z is the
    // timestamp; what follows must be the sha alone.
    let after = match line.find('Z') {
        Some(at) => &line[at + 1..],
        None => line,
    };
    let candidate = after.trim().trim_matches('\'').trim_matches('"');
    is_sha(candidate).then(|| candidate.to_lowercase())
}

fn hex_runs(line: &str) -> Vec<String> {
    let characters: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    let mut start = 0usize;

    while start < characters.len() {
        if !characters[start].is_ascii_hexdigit() {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < characters.len() && characters[end].is_ascii_hexdigit() {
            end += 1;
        }
        if end - start == 40 {
            found.push(
                characters[start..end]
                    .iter()
                    .collect::<String>()
                    .to_lowercase(),
            );
        }
        start = end;
    }
    found
}

fn is_sha(text: &str) -> bool {
    text.len() == 40 && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// Builds the targets for a set of runs.
pub fn build(
    context: &Context,
    repo: &Repo,
    github: &GitHub,
    runs: &[Run],
    cache: &mut Cache,
    mut note: impl FnMut(&str),
) -> TargetSet {
    let successful: Vec<&Run> = runs.iter().filter(|run| run.succeeded()).collect();

    let mut targets: Vec<Target> = Vec::new();
    let mut ignored_jobs: BTreeMap<String, String> = BTreeMap::new();
    let mut barren = 0usize;
    let mut examined = 0usize;
    let mut stopped_early = false;

    // Jobs lists are independent, so they go together - but only for runs the walk
    // will reach. Asking for all of them trades waiting in turn for fetching far more
    // than is needed: forty successful runs, and it stops after five.
    prefetch_wave(github, &successful, 0);

    // Then the logs, which are the large requests. A run reveals a new target by its
    // job names, so which logs are needed is known before any is asked for.
    let wanted = plan_log_fetches(context, github, &successful, cache);
    if !wanted.is_empty() {
        note(&format!("  reading {} job log(s)...", wanted.len()));
        github.prefetch_logs(&wanted);
    }

    let mut prefetched = crate::github::MAX_IN_FLIGHT.min(successful.len());

    for (index, run) in successful.iter().enumerate() {
        if index >= prefetched {
            prefetched = prefetch_wave(github, &successful, prefetched);
        }
        if !targets.is_empty() && barren >= BARREN_RUNS_BEFORE_STOPPING {
            stopped_early = true;
            note(&format!(
                "  stopped after {examined} of {} successful run(s): the last {BARREN_RUNS_BEFORE_STOPPING} revealed no new target",
                successful.len()
            ));
            break;
        }
        examined += 1;
        let before = targets.len();

        for job in github.jobs(run.id) {
            match target_from_job(context, repo, github, run, &job, &targets, cache) {
                Ok(Some(target)) => targets.push(target),
                Ok(None) => {}
                Err((name, why)) => {
                    ignored_jobs.insert(name, why);
                }
            }
        }

        if targets.len() == before {
            barren += 1;
        } else {
            barren = 0;
        }
    }

    TargetSet {
        targets,
        ignored_jobs,
        runs_examined: examined,
        runs_available: successful.len(),
        stopped_early,
    }
}

/// Asks for one wave of jobs lists, starting at `from`.
///
/// A wave rather than everything: the walk stops as soon as a few runs in a row
/// reveal nothing new, so fetching the whole history would be work thrown away.
fn prefetch_wave(github: &GitHub, successful: &[&Run], from: usize) -> usize {
    let end = (from + crate::github::MAX_IN_FLIGHT).min(successful.len());
    if from >= end {
        return from;
    }
    let ids: Vec<u64> = successful[from..end].iter().map(|run| run.id).collect();
    github.prefetch_jobs(&ids);
    end
}

/// The job logs the walk below will ask for. A dry run of the same loop: a run
/// reveals a new target by its job names, so no logs are needed to plan them.
fn plan_log_fetches(
    context: &Context,
    github: &GitHub,
    successful: &[&Run],
    cache: &Cache,
) -> Vec<u64> {
    let mut wanted = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut barren = 0usize;
    let mut prefetched = crate::github::MAX_IN_FLIGHT.min(successful.len());

    for (index, run) in successful.iter().enumerate() {
        if !labels.is_empty() && barren >= BARREN_RUNS_BEFORE_STOPPING {
            break;
        }
        // Past the wave already in hand, so ask for the next one rather than letting
        // each run fall back to a request of its own.
        if index >= prefetched {
            prefetched = prefetch_wave(github, successful, prefetched);
        }
        let before = labels.len();

        for job in github.jobs(run.id) {
            if !job.succeeded() || job.steps.len() < 3 {
                continue;
            }
            if !matched_ignore_words(&job.name, &context.ignore_jobs).is_empty() {
                continue;
            }

            let facts = context
                .facts
                .iter()
                .find(|fact| fact.file == run.workflow_file);
            if !context.environment.is_empty()
                && let Some(declared) = facts
                    .and_then(|fact| fact.job(&job.name))
                    .and_then(|job_facts| job_facts.environment.as_deref())
                && resolve_alias(declared) != context.environment
            {
                continue;
            }

            let label = target_label(&job.name);
            if labels.contains(&label) {
                continue;
            }
            labels.push(label);

            // A completed run's log is settled, so a remembered answer needs no
            // request at all.
            if run.needs_log && cache.sha_for_job(job.id).is_none() {
                wanted.push(job.id);
            }
        }

        if labels.len() == before {
            barren += 1;
        } else {
            barren = 0;
        }
    }

    wanted
}

type JobSkipped = (String, String);

fn target_from_job(
    context: &Context,
    repo: &Repo,
    github: &GitHub,
    run: &Run,
    job: &Job,
    known: &[Target],
    cache: &mut Cache,
) -> Result<Option<Target>, JobSkipped> {
    if !job.succeeded() {
        return Ok(None);
    }

    let facts = context
        .facts
        .iter()
        .find(|fact| fact.file == run.workflow_file);
    let job_facts = facts.and_then(|fact| fact.job(&job.name));

    // One workflow can deploy to several environments in the same run.
    if !context.environment.is_empty()
        && let Some(declared) = job_facts.and_then(|facts| facts.environment.as_deref())
        && resolve_alias(declared) != context.environment
    {
        return Err((
            job.name.clone(),
            format!("deploys to '{declared}', not {}", context.environment),
        ));
    }

    let matched = matched_ignore_words(&job.name, &context.ignore_jobs);
    if let Some(word) = matched.first() {
        return Err((job.name.clone(), format!("name contains '{word}'")));
    }
    if job.steps.len() < 3 {
        return Err((job.name.clone(), "fewer than three steps".into()));
    }

    let label = target_label(&job.name);
    if known.iter().any(|target| target.label == label) {
        return Ok(None);
    }

    // Scope, from whichever source says it most directly.
    let mut scope: Vec<String> = job_facts
        .and_then(|facts| facts.working_directory.clone())
        .into_iter()
        .collect();

    // A job whose every step works in one directory covers it just as surely.
    if scope.is_empty()
        && let Some(facts) = job_facts
    {
        scope = facts.step_working_directories.clone();
    }

    // A calling job does its work over there, so the scope is described in that file.
    if scope.is_empty()
        && let Some(called) = job_facts.and_then(|facts| facts.calls_workflow.as_deref())
        && let Some(other) = context.facts.iter().find(|fact| fact.file == called)
    {
        scope = other.working_directories.clone();
        if scope.is_empty() {
            scope = other.trigger_paths.clone();
        }
    }

    // Last, the trigger filter. Only for a single-job workflow, since a filter
    // describes the file rather than one job.
    if scope.is_empty()
        && let Some(facts) = facts
        && !facts.trigger_paths.is_empty()
        && facts.jobs.len() == 1
    {
        scope = facts.trigger_paths.clone();
    }

    if let Some(from_override) = context.scope_override(&label) {
        scope = from_override;
    }

    let mut sha_warning = String::new();
    let mut sha_is_exact = true;
    let mut sha_source = ShaSource::RunRef;
    let sha;

    if run.needs_log {
        // A completed run is frozen, so a remembered answer stands in for the log.
        // Still checked against this clone, which is a separate question.
        let remembered = cache
            .sha_for_job(job.id)
            .filter(|found| repo.commit_exists(&found.sha, false))
            .cloned();

        let resolved = match remembered {
            Some(found) => Some(ResolvedSha {
                sha: found.sha,
                exact: true,
                warning: found.warning,
            }),
            None => github
                .job_log(job.id)
                .as_deref()
                .and_then(|text| resolve_checkout_sha(text, repo)),
        };

        match resolved {
            Some(found) => {
                // Only an exact reading of a finished run is worth keeping: a guess
                // could read differently once the clone has more history.
                if found.exact && run.completed() {
                    cache.remember_sha(job.id, &found.sha, &found.warning);
                }
                sha = found.sha;
                sha_is_exact = found.exact;
                sha_warning = found.warning;
                sha_source = ShaSource::RunLog;
            }
            None => {
                // The log is gone, or named nothing this clone has. GitHub keeps the
                // deployment record after it deletes the log.
                let fallback = github.deployment_sha(run.id, &context.environment, true);
                match fallback {
                    Some(found) if repo.commit_exists(&found, true) => {
                        sha = found;
                        sha_is_exact = false;
                        sha_source = ShaSource::DeploymentRecord;
                        sha_warning = "the run log could not name the commit, so this is what the \
                             deployment was created for"
                            .into();
                    }
                    _ => {
                        let why = if github.job_log(job.id).is_none() {
                            format!(
                                "run {} has no readable log, and no deployment record names a \
                                 commit this clone has",
                                run.id
                            )
                        } else {
                            "no commit in the run log or the deployment record that exists in \
                             this clone; try fetching"
                                .into()
                        };
                        return Err((job.name.clone(), why));
                    }
                }
            }
        }
    } else {
        if !repo.commit_exists(&run.head_sha, true) {
            let short: String = run.head_sha.chars().take(9).collect();
            return Err((
                job.name.clone(),
                format!("commit {short} is not in this clone; try fetching"),
            ));
        }
        sha = run.head_sha.clone();
    }

    // A second opinion: free once the deployment walk has happened, worth paying for
    // when the commit is already a guess, never paid for just to agree.
    let mut corroborated = false;
    let worth_fetching = !sha_is_exact || sha_source == ShaSource::DeploymentRecord;
    if sha_source != ShaSource::DeploymentRecord
        && let Some(recorded) = github.deployment_sha(run.id, &context.environment, worth_fetching)
    {
        if recorded == sha {
            corroborated = true;
        } else {
            // Two records disagreeing. Usually the branch moved mid-deploy, so
            // neither is wrong and the reader should decide rather than deplyd.
            sha_is_exact = false;
            let short: String = recorded.chars().take(9).collect();
            sha_warning = format!(
                "the deployment record names {short} instead; the branch may have moved mid-deploy"
            );
        }
    }

    Ok(Some(Target {
        label,
        job: job.name.clone(),
        run_id: run.id,
        run_url: run.html_url.clone(),
        run_created_at: run.created_at.clone(),
        run_workflow_file: run.workflow_file.clone(),
        sha,
        sha_is_exact,
        sha_source,
        sha_warning,
        corroborated,
        skipped: job.skipped_steps(),
        scope,
        concerns: Vec::new(),
    }))
}

/// Newer runs that did not complete, which may have changed part of the environment.
pub fn concerns_for(
    target: &Target,
    runs: &[Run],
    all_labels: &[String],
    github: &GitHub,
) -> Vec<Concern> {
    let target_token = token(&target.label);
    let other_tokens: Vec<String> = all_labels
        .iter()
        .filter(|label| *label != &target.label)
        .map(|label| token(label))
        .collect();

    let mut concerns: Vec<Concern> = Vec::new();

    for run in runs {
        if run.created_at <= target.run_created_at {
            continue;
        }

        let relevant = run.workflow_token.contains(&target_token)
            || run.workflow_file == target.run_workflow_file;
        if !relevant {
            // A workflow naming a different target is that target's problem. One
            // naming neither - a combined deploy - counts for both.
            let mentions_other = other_tokens
                .iter()
                .any(|other| !other.is_empty() && run.workflow_token.contains(other));
            if mentions_other {
                continue;
            }
        }

        if !run.completed() {
            // The API reports snake_case states; do not put those in front of a reader.
            concerns.push(Concern {
                run_id: run.id,
                state: run.status.replace('_', " "),
            });
        } else if run.conclusion.as_deref() == Some("failure") {
            concerns.push(Concern {
                run_id: run.id,
                state: "failed".into(),
            });
        } else if run.conclusion.as_deref() == Some("cancelled") && run_did_anything(run, github) {
            concerns.push(Concern {
                run_id: run.id,
                state: "cancelled part-way".into(),
            });
        }
    }

    concerns.sort_by_key(|concern| std::cmp::Reverse(concern.run_id));
    concerns.dedup_by_key(|concern| concern.run_id);
    concerns
}

/// A run cancelled in the queue deployed nothing. One whose jobs had started may have
/// published part of a deploy, which is worth flagging.
fn run_did_anything(run: &Run, github: &GitHub) -> bool {
    github
        .jobs(run.id)
        .iter()
        .any(|job| job.conclusion.as_deref() != Some("skipped") && job.started_at.is_some())
}
