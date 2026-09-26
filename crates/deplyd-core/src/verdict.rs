//! What deplyd concluded, as data. Printed or serialised from the same object, so
//! a script and a person cannot be told different things.

use serde::Serialize;

use crate::context::Context;
use crate::github::{GitHub, PullRequest};
use crate::history::{self, CommitLine};
use crate::repo::Repo;
use crate::targets::{Target, TargetSet};

/// What is being asked about. Serialised as the kind alone; the number and the
/// commit are already fields of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    PullRequest { number: u32 },
    Commit { sha: String },
}

fn is_zero(number: &u32) -> bool {
    *number == 0
}

impl Serialize for Change {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Change::PullRequest { .. } => "pullRequest",
            Change::Commit { .. } => "commit",
        })
    }
}

impl Change {
    pub fn number(&self) -> u32 {
        match self {
            Change::PullRequest { number } => *number,
            Change::Commit { .. } => 0,
        }
    }

    pub fn is_commit(&self) -> bool {
        matches!(self, Change::Commit { .. })
    }
}

/// How a pull request's merge commit was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommitSource {
    /// From the API, which knows it whatever the merge style was.
    Api,
    /// Matched on a squash-merge subject, the API being unavailable.
    SubjectMatch,
}

/// The outcome. These strings are what `--json` emits, so they are stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Deplyd,
    NotDeplyd,
    Reverted,
    NotMerged,
    NotFound,
    NotCovered,
    CommitMissingLocally,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetStatus {
    Deplyd,
    DeplydAsCopy,
    NotDeplyd,
    Reverted,
}

#[derive(Debug, Clone, Serialize)]
pub struct CopyReport {
    pub commit: String,
    pub how: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitReport {
    pub commit: String,
    pub subject: String,
}

impl From<CommitLine> for CommitReport {
    fn from(line: CommitLine) -> Self {
        Self {
            commit: line.sha,
            subject: line.subject,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestTargetReport {
    pub label: String,
    pub deployed_commit: String,
    pub status: TargetStatus,
    pub uncertain: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy: Option<CopyReport>,
    pub reverts: Vec<CommitReport>,
    pub changed_after: Vec<CommitReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestReport {
    #[serde(rename = "kind")]
    pub change: Change,
    /// Zero for a commit, which has none.
    #[serde(skip_serializing_if = "is_zero")]
    pub number: u32,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub state: String,
    pub commit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_source: Option<CommitSource>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub branch: String,
    pub environment: String,
    pub status: Status,
    /// True when any covering target is shaky, so a gate is never told "shipped"
    /// on evidence deplyd has questioned.
    pub uncertain: bool,
    pub files: Vec<String>,
    pub targets: Vec<PullRequestTargetReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConcernReport {
    pub run_id: u64,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetReport {
    pub label: String,
    pub environment: String,
    pub commit: String,
    pub state: String,
    pub exact_commit: bool,
    pub commit_source: String,
    pub corroborated: bool,
    pub scope: Vec<String>,
    pub skipped: Vec<String>,
    pub run_id: u64,
    pub run_url: String,
    pub concerns: Vec<ConcernReport>,
}

pub fn target_reports(context: &Context, targets: &TargetSet) -> Vec<TargetReport> {
    targets
        .targets
        .iter()
        .map(|target| TargetReport {
            label: target.label.clone(),
            environment: context.environment.clone(),
            commit: target.sha.clone(),
            state: target.state().to_string(),
            exact_commit: target.sha_is_exact,
            commit_source: target.sha_source.describe().to_string(),
            corroborated: target.corroborated,
            scope: target.scope.clone(),
            skipped: target.skipped.clone(),
            run_id: target.run_id,
            run_url: target.run_url.clone(),
            concerns: target
                .concerns
                .iter()
                .map(|concern| ConcernReport {
                    run_id: concern.run_id,
                    state: concern.state.clone(),
                })
                .collect(),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeReport {
    pub label: String,
    pub id: String,
    pub title: String,
    pub commit: String,
    pub reverted: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusJson {
    pub author: String,
    pub environment: String,
    pub targets: Vec<TargetReport>,
    pub changes: Vec<ChangeReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestJson {
    pub change: PullRequestReport,
    pub targets: Vec<TargetReport>,
}

/// A verdict, not a failure. `0` only when the change shipped and nothing about the
/// reading is shaky, so a gate is never told "shipped" on doubted evidence.
pub fn exit_code(status: Status, uncertain: bool) -> u8 {
    match status {
        Status::Deplyd if uncertain => 6,
        Status::Deplyd => 0,
        Status::Reverted => 3,
        Status::NotMerged => 4,
        Status::NotFound => 5,
        _ => 2,
    }
}

/// Decides whether one commit is deployd.
///
/// The same reasoning as a pull request, minus finding it: a commit is already the
/// thing to look for, so there is no merge state and no number to report.
pub fn commit_report(
    context: &Context,
    repo: &Repo,
    targets: &TargetSet,
    reference: &str,
) -> PullRequestReport {
    let mut report = PullRequestReport {
        change: Change::Commit {
            sha: reference.to_string(),
        },
        number: 0,
        title: String::new(),
        state: String::new(),
        commit: String::new(),
        commit_source: None,
        branch: String::new(),
        environment: context.environment.clone(),
        status: Status::NotFound,
        uncertain: false,
        files: Vec::new(),
        targets: Vec::new(),
    };

    let Some(sha) = repo.resolve_commit(reference) else {
        return report;
    };

    report.change = Change::Commit { sha: sha.clone() };
    report.commit = sha.clone();
    report.title = repo.subject(&sha).unwrap_or_default();
    report.files = repo.commit_files(&sha);

    for target in &targets.targets {
        if !target.covers(&report.files) {
            continue;
        }
        report
            .targets
            .push(target_verdict(repo, target, &sha, &report.files));
    }

    if report.targets.is_empty() {
        report.status = Status::NotCovered;
        return report;
    }

    report.uncertain = report.targets.iter().any(|entry| entry.uncertain);

    if report
        .targets
        .iter()
        .any(|entry| entry.status == TargetStatus::Reverted)
    {
        report.status = Status::Reverted;
    } else if report.targets.iter().all(|entry| {
        matches!(
            entry.status,
            TargetStatus::Deplyd | TargetStatus::DeplydAsCopy
        )
    }) {
        report.status = Status::Deplyd;
    } else {
        report.status = Status::NotDeplyd;
    }

    report
}

/// Decides whether a pull request is live.
pub fn pull_request_report(
    context: &Context,
    repo: &Repo,
    github: &GitHub,
    targets: &TargetSet,
    number: u32,
) -> PullRequestReport {
    let mut report = PullRequestReport {
        change: Change::PullRequest { number },
        number,
        title: String::new(),
        state: String::new(),
        commit: String::new(),
        commit_source: None,
        branch: String::new(),
        environment: context.environment.clone(),
        status: Status::NotFound,
        uncertain: false,
        files: Vec::new(),
        targets: Vec::new(),
    };

    let Some(found) = find_pull_request(repo, github, number) else {
        return report;
    };

    report.title = found.title;
    report.state = found.state;
    report.commit = found.sha;
    report.commit_source = Some(found.source);
    report.branch = found.branch;

    if !found.merged || report.commit.is_empty() {
        report.status = Status::NotMerged;
        return report;
    }

    if !repo.commit_exists(&report.commit, true) {
        report.status = Status::CommitMissingLocally;
        return report;
    }

    report.files = repo.commit_files(&report.commit);

    for target in &targets.targets {
        if !target.covers(&report.files) {
            continue;
        }
        report
            .targets
            .push(target_verdict(repo, target, &report.commit, &report.files));
    }

    if report.targets.is_empty() {
        report.status = Status::NotCovered;
        return report;
    }

    report.uncertain = report.targets.iter().any(|entry| entry.uncertain);

    if report
        .targets
        .iter()
        .any(|entry| entry.status == TargetStatus::Reverted)
    {
        report.status = Status::Reverted;
    } else if report.targets.iter().all(|entry| {
        matches!(
            entry.status,
            TargetStatus::Deplyd | TargetStatus::DeplydAsCopy
        )
    }) {
        report.status = Status::Deplyd;
    } else {
        report.status = Status::NotDeplyd;
    }

    report
}

fn target_verdict(
    repo: &Repo,
    target: &Target,
    commit: &str,
    files: &[String],
) -> PullRequestTargetReport {
    let mut entry = PullRequestTargetReport {
        label: target.label.clone(),
        deployed_commit: target.sha.clone(),
        status: TargetStatus::NotDeplyd,
        uncertain: target.uncertain(),
        copy: None,
        reverts: Vec::new(),
        changed_after: Vec::new(),
    };

    if !repo.is_ancestor(commit, &target.sha) {
        // A cherry-pick or rebase carries the same change under a different sha.
        if let Some(copy) = history::find_copy(repo, commit, &target.sha) {
            entry.status = TargetStatus::DeplydAsCopy;
            entry.copy = Some(CopyReport {
                commit: copy.sha,
                how: copy.how.to_string(),
                subject: copy.subject,
            });
        }
        return entry;
    }

    // A revert is an ancestor too, so say which of the two happened.
    let reverts = history::find_revert(repo, commit, &target.sha);
    if !reverts.is_empty() {
        entry.status = TargetStatus::Reverted;
        entry.reverts = reverts.into_iter().map(CommitReport::from).collect();
        return entry;
    }

    entry.status = TargetStatus::Deplyd;

    // Information, not a caveat: later edits do not make the change any less
    // deployed. Only commits touching the same files, and only the first of them.
    entry.changed_after = history::later_commits_touching(repo, commit, &target.sha, files)
        .into_iter()
        .map(CommitReport::from)
        .collect();
    entry
}

struct FoundPullRequest {
    title: String,
    state: String,
    sha: String,
    branch: String,
    merged: bool,
    source: CommitSource,
}

/// The API knows the merge commit whatever the style was, so it is tried first. The
/// fallback only handles squash merges, which carry "(#412)" in the subject.
fn find_pull_request(repo: &Repo, github: &GitHub, number: u32) -> Option<FoundPullRequest> {
    if let Some(found) = github.pull_request(number) {
        return Some(from_api(found));
    }

    let branch = repo.default_branch()?;
    let needle = format!("(#{number})");
    let output = repo
        .run(
            crate::gateway::git::Verb::Log,
            &[
                &branch,
                "--no-merges",
                "--format=%H%x09%s",
                "--fixed-strings",
                &format!("--grep={needle}"),
                "-1",
            ],
        )
        .ok()?;

    let line = output.first_line()?;
    let (sha, subject) = line.split_once('\t')?;
    Some(FoundPullRequest {
        title: subject.to_string(),
        state: "MERGED".to_string(),
        sha: sha.to_string(),
        branch: String::new(),
        merged: true,
        source: CommitSource::SubjectMatch,
    })
}

fn from_api(found: PullRequest) -> FoundPullRequest {
    let merged = found.merged();
    FoundPullRequest {
        title: found.title.clone(),
        // The API says "closed" for a merged pull request; the report has always
        // distinguished the two, and a gate reading `state` should see the difference.
        state: if merged {
            "MERGED".to_string()
        } else {
            found.state.to_uppercase()
        },
        sha: found.merge_commit_sha.clone().unwrap_or_default(),
        branch: found.branch().to_string(),
        merged,
        source: CommitSource::Api,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_what_the_readme_promises() {
        assert_eq!(exit_code(Status::Deplyd, false), 0);
        assert_eq!(exit_code(Status::Deplyd, true), 6);
        assert_eq!(exit_code(Status::NotDeplyd, false), 2);
        assert_eq!(exit_code(Status::NotCovered, false), 2);
        assert_eq!(exit_code(Status::CommitMissingLocally, false), 2);
        assert_eq!(exit_code(Status::Reverted, false), 3);
        assert_eq!(exit_code(Status::NotMerged, false), 4);
        assert_eq!(exit_code(Status::NotFound, false), 5);
    }

    #[test]
    fn uncertainty_only_softens_a_live_verdict() {
        // A shaky reading turns "shipped" into "look first", and changes nothing
        // else: that would report something different, not less confidently.
        assert_eq!(exit_code(Status::NotDeplyd, true), 2);
        assert_eq!(exit_code(Status::Reverted, true), 3);
        assert_eq!(exit_code(Status::NotMerged, true), 4);
    }
}
