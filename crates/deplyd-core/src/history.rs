//! What became of a commit after it merged. Ancestry says it was included; it does
//! not say the change survived, because a revert is an ancestor too.

use crate::gateway::git::Verb;
use crate::repo::Repo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitLine {
    pub sha: String,
    pub subject: String,
}

fn parse_lines(output: &crate::repo::Output) -> Vec<CommitLine> {
    output
        .lines()
        .into_iter()
        .filter_map(|line| {
            let (sha, subject) = line.split_once('\t')?;
            Some(CommitLine {
                sha: sha.trim().to_string(),
                subject: subject.trim().to_string(),
            })
        })
        .collect()
}

/// Commits between the two that undo it. Both git and GitHub write "This reverts
/// commit <sha>."; a hand-written one may only say so in the subject.
pub fn find_revert(repo: &Repo, commit: &str, deployed: &str) -> Vec<CommitLine> {
    let range = format!("{commit}..{deployed}");
    let needle = format!("This reverts commit {commit}");

    if let Ok(output) = repo.run(
        Verb::Log,
        &[
            &range,
            "--format=%h%x09%s",
            "--fixed-strings",
            &format!("--grep={needle}"),
        ],
    ) {
        let found = parse_lines(&output);
        if !found.is_empty() {
            return found;
        }
    }

    let Some(subject) = repo.subject(commit) else {
        return Vec::new();
    };
    // "feat: add export (#412)" and its revert share the words, not the PR number.
    let quoted = strip_pr_suffix(&subject);
    if quoted.is_empty() {
        return Vec::new();
    }

    repo.run(
        Verb::Log,
        &[
            &range,
            "--format=%h%x09%s",
            "--fixed-strings",
            "--grep=Revert",
            &format!("--grep={quoted}"),
            "--all-match",
        ],
    )
    .map(|output| parse_lines(&output))
    .unwrap_or_default()
}

fn strip_pr_suffix(subject: &str) -> String {
    let trimmed = subject.trim_end();
    let Some(open) = trimmed.rfind(" (#") else {
        return trimmed.to_string();
    };
    if !trimmed.ends_with(')') {
        return trimmed.to_string();
    }
    let inner = &trimmed[open + 3..trimmed.len() - 1];
    if inner.chars().all(|c| c.is_ascii_digit()) && !inner.is_empty() {
        return trimmed[..open].trim_end().to_string();
    }
    trimmed.to_string()
}

/// The first commit after it that touched the same files. File level on purpose:
/// `git log -L` resolves against the end of the range while the line numbers come
/// from the start.
pub fn later_commits_touching(
    repo: &Repo,
    commit: &str,
    deployed: &str,
    paths: &[String],
) -> Vec<CommitLine> {
    let range = format!("{commit}..{deployed}");
    let mut args: Vec<&str> = vec![&range, "--no-merges", "--format=%h%x09%s"];
    if !paths.is_empty() {
        args.push("--");
        for path in paths {
            args.push(path);
        }
    }

    let Ok(output) = repo.run(Verb::Log, &args) else {
        return Vec::new();
    };
    let commits = parse_lines(&output);
    // log is newest first, so the earliest is last.
    commits.last().cloned().into_iter().collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copy {
    pub sha: String,
    pub subject: String,
    /// How the copy was identified, which is worth showing: a recorded cherry-pick is
    /// exact, an equivalent patch is inference.
    pub how: &'static str,
}

/// Whether the same change reached `deployed` under a different sha.
pub fn find_copy(repo: &Repo, commit: &str, deployed: &str) -> Option<Copy> {
    // A recorded cherry-pick names its origin in the message. Exact, so first.
    let trailer = format!("(cherry picked from commit {commit}");
    if let Ok(output) = repo.run(
        Verb::Log,
        &[
            deployed,
            "--format=%h%x09%s",
            "--fixed-strings",
            &format!("--grep={trailer}"),
        ],
    ) && let Some(found) = parse_lines(&output).first()
    {
        return Some(Copy {
            sha: found.sha.clone(),
            subject: found.subject.clone(),
            how: "recorded cherry-pick",
        });
    }

    // git cherry compares by patch id, so a copy with a different sha still matches.
    // A leading "-" means the deployed commit already contains an equivalent change.
    let marks = repo.run(Verb::Cherry, &[deployed, commit]).ok()?;
    let equivalent = marks.lines().into_iter().any(|line| {
        let mut parts = line.split_whitespace();
        parts.next() == Some("-") && parts.next().is_some_and(|sha| sha == commit)
    });
    if !equivalent {
        return None;
    }

    // Name the copy where possible: a cherry-pick keeps the original subject.
    let subject = repo.subject(commit).unwrap_or_default();
    if !subject.is_empty()
        && let Ok(output) = repo.run(
            Verb::Log,
            &[
                deployed,
                "--max-count=5",
                "--format=%h%x09%s",
                "--fixed-strings",
                &format!("--grep={subject}"),
            ],
        )
        && let Some(found) = parse_lines(&output).first()
    {
        return Some(Copy {
            sha: found.sha.clone(),
            subject: found.subject.clone(),
            how: "identical change",
        });
    }

    Some(Copy {
        sha: String::new(),
        subject,
        how: "identical change",
    })
}

/// Every commit reverted before `deployed`. One pass collects them all, because every
/// revert names what it undid.
pub fn reverted_commits(repo: &Repo, deployed: &str) -> Vec<String> {
    let Ok(output) = repo.run(
        Verb::Log,
        &[
            deployed,
            "--max-count=500",
            "--format=%b",
            "--fixed-strings",
            "--grep=This reverts commit",
        ],
    ) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for line in output.stdout.lines() {
        let Some((_, tail)) = line.split_once("This reverts commit ") else {
            continue;
        };
        let sha: String = tail.chars().take_while(char::is_ascii_hexdigit).collect();
        if sha.len() >= 7 && !found.contains(&sha) {
            found.push(sha);
        }
    }
    found
}

/// Whether a short sha names one of the reverted commits.
pub fn was_reverted(short_sha: &str, reverted: &[String]) -> bool {
    !short_sha.is_empty() && reverted.iter().any(|full| full.starts_with(short_sha))
}

#[cfg(test)]
mod tests {
    use super::strip_pr_suffix;

    #[test]
    fn a_squash_merge_subject_loses_its_pull_request_number() {
        assert_eq!(
            strip_pr_suffix("feat(billing): add invoice export (#412)"),
            "feat(billing): add invoice export"
        );
    }

    #[test]
    fn a_subject_without_one_is_left_alone() {
        assert_eq!(strip_pr_suffix("plain subject"), "plain subject");
        assert_eq!(
            strip_pr_suffix("mentions (#notanumber)"),
            "mentions (#notanumber)"
        );
    }
}
