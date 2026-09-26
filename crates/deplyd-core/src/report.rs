//! An author's commits, grouped for the report. Nothing here decides anything.

use crate::gateway::git::Verb;
use crate::repo::Repo;
use crate::targets::Target;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub sha: String,
    pub when: i64,
    /// Already formatted by git, so deplyd never does calendar arithmetic.
    pub date: String,
    pub subject: String,
    pub label: String,
}

/// One row: a pull request where there is one, a commit where there is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub when: i64,
    pub date: String,
    pub label: String,
    pub sha: String,
    /// "PR #412", or the short sha for a commit pushed straight to a branch.
    pub id: String,
    /// The number behind that id, where there is one, for linking to it.
    pub pull_request: Option<u32>,
    pub title: String,
}

#[derive(Debug)]
pub struct NoAuthor;

impl std::fmt::Display for NoAuthor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "an empty author matches every commit, reporting it as yours"
        )
    }
}

impl std::error::Error for NoAuthor {}

/// An author's commits reachable from a revision. The author is required:
/// `--author=''` matches everyone, reporting the team's work as one person's.
pub fn records(
    repo: &Repo,
    revision_args: &[&str],
    scope: &[String],
    label: &str,
    author: &str,
) -> Result<Vec<Record>, NoAuthor> {
    if author.trim().is_empty() {
        return Err(NoAuthor);
    }

    let mut args: Vec<&str> = vec![
        // An author is a name, not a pattern. Without this, "Ada [Team]" is an
        // invalid regex and git fails, which deplyd read as "no changes" - a silent
        // wrong answer rather than an error.
        "--fixed-strings",
        "--author",
        author,
        "--no-merges",
        "--format=%h%x09%ct%x09%cd%x09%s",
        "--date=format:%Y-%m-%d %H:%M",
    ];
    args.extend_from_slice(revision_args);
    if !scope.is_empty() {
        args.push("--");
        for path in scope {
            args.push(path);
        }
    }

    let Ok(output) = repo.run(Verb::Log, &args) else {
        return Ok(Vec::new());
    };

    Ok(output
        .lines()
        .into_iter()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            let sha = parts.next()?.trim().to_string();
            let when = parts.next()?.trim().parse().ok()?;
            let date = parts.next()?.trim().to_string();
            let subject = parts.next()?.trim().to_string();
            Some(Record {
                sha,
                when,
                date,
                subject,
                label: label.to_string(),
            })
        })
        .collect())
}

/// Collapses records for the same commit across targets, newest first. A commit in
/// two scopes is one change labelled COMBINED, not two rows.
pub fn merge_records(records: &[Record]) -> Vec<Entry> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: Vec<(String, Vec<&Record>)> = Vec::new();

    for record in records {
        match order.iter().position(|sha| *sha == record.sha) {
            Some(index) => grouped[index].1.push(record),
            None => {
                order.push(record.sha.clone());
                grouped.push((record.sha.clone(), vec![record]));
            }
        }
    }

    let mut entries: Vec<Entry> = grouped
        .into_iter()
        .map(|(sha, group)| {
            let first = group[0];
            let mut labels: Vec<&str> = Vec::new();
            for record in &group {
                if !labels.contains(&record.label.as_str()) {
                    labels.push(&record.label);
                }
            }
            let label = if labels.len() > 1 {
                "COMBINED".to_string()
            } else {
                labels.first().map(|l| (*l).to_string()).unwrap_or_default()
            };

            let (id, title, pull_request) = format_entry(&sha, &first.subject);
            Entry {
                when: first.when,
                date: first.date.clone(),
                label,
                sha,
                id,
                pull_request,
                title,
            }
        })
        .collect();

    entries.sort_by_key(|entry| std::cmp::Reverse(entry.when));
    entries
}

/// Splits a subject into an identifier and a title, so the caller can line the
/// columns up: a short sha and a pull request number are different widths.
pub fn format_entry(sha: &str, subject: &str) -> (String, String, Option<u32>) {
    let trimmed = subject.trim_end();
    if trimmed.ends_with(')')
        && let Some(open) = trimmed.rfind("(#")
    {
        let inner = &trimmed[open + 2..trimmed.len() - 1];
        if let Ok(number) = inner.parse::<u32>() {
            return (
                format!("PR #{number}"),
                trimmed[..open].trim_end().to_string(),
                Some(number),
            );
        }
    }
    (sha.to_string(), trimmed.to_string(), None)
}

pub fn entry_width(entries: &[Entry]) -> usize {
    entries
        .iter()
        .map(|entry| entry.id.len())
        .max()
        .unwrap_or(0)
}

/// Everything the status report needs, worked out before anything prints.
pub struct Status {
    pub author: String,
    pub live: Vec<Entry>,
    pub page: Vec<Entry>,
    pub reverted: Vec<String>,
    pub skip: usize,
}

pub fn status(
    repo: &Repo,
    targets: &[Target],
    author: &str,
    take: usize,
    skip: usize,
) -> Result<Status, NoAuthor> {
    let mut all = Vec::new();
    let mut reverted: Vec<String> = Vec::new();

    for target in targets {
        all.extend(records(
            repo,
            &["-200", &target.sha],
            &target.scope,
            &target.label,
            author,
        )?);
        for sha in crate::history::reverted_commits(repo, &target.sha) {
            if !reverted.contains(&sha) {
                reverted.push(sha);
            }
        }
    }

    let live = merge_records(&all);
    let page: Vec<Entry> = live.iter().skip(skip).take(take).cloned().collect();

    Ok(Status {
        author: author.to_string(),
        live,
        page,
        reverted,
        skip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_squash_merged_commit_shows_its_pull_request() {
        let (id, title, number) = format_entry("a1b2c3d", "feat: add export (#412)");
        assert_eq!(id, "PR #412");
        assert_eq!(title, "feat: add export");
        assert_eq!(number, Some(412));
    }

    #[test]
    fn a_commit_with_no_pull_request_shows_its_sha() {
        let (id, title, number) = format_entry("a1b2c3d", "hotfix straight to main");
        assert_eq!(id, "a1b2c3d");
        assert_eq!(title, "hotfix straight to main");
        assert_eq!(
            number, None,
            "a plain commit has no pull request to link to"
        );
    }

    #[test]
    fn a_trailing_parenthesis_that_is_not_a_number_is_left_alone() {
        let (id, _, _) = format_entry("a1b2c3d", "tidy up (see #412)");
        assert_eq!(id, "a1b2c3d");
    }

    #[test]
    fn a_commit_in_two_targets_is_one_row_labelled_combined() {
        let records = vec![
            Record {
                sha: "aaa".into(),
                when: 200,
                date: "2026-01-02 10:00".into(),
                subject: "shared change".into(),
                label: "API".into(),
            },
            Record {
                sha: "aaa".into(),
                when: 200,
                date: "2026-01-02 10:00".into(),
                subject: "shared change".into(),
                label: "WEB".into(),
            },
            Record {
                sha: "bbb".into(),
                when: 100,
                date: "2026-01-01 09:00".into(),
                subject: "api only".into(),
                label: "API".into(),
            },
        ];

        let merged = merge_records(&records);
        assert_eq!(
            merged.len(),
            2,
            "the shared commit should collapse to one row"
        );
        assert_eq!(merged[0].label, "COMBINED");
        assert_eq!(merged[0].sha, "aaa");
        assert_eq!(merged[1].label, "API", "newest first");
    }

    #[test]
    fn an_empty_author_is_refused() {
        // git log --author='' matches everyone, reporting the whole team's work as
        // one person's.
        let error = records(
            &crate::repo::Repo::discover(std::path::Path::new(".")).expect("a repo"),
            &["HEAD"],
            &[],
            "API",
            "   ",
        );
        assert!(error.is_err());
    }
}
