//! An author's commits, grouped for the report. Nothing here decides anything.

use std::collections::HashMap;

use crate::gateway::git::Verb;
use crate::repo::Repo;
use crate::targets::Target;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub sha: String,
    pub author: String,
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
    pub author: String,
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

/// Commits reachable from a revision, by one author or by everyone. None is
/// everyone; an empty name is refused, because `--author=''` matches everyone
/// while the report still says the commits are yours.
pub fn records(
    repo: &Repo,
    revision_args: &[&str],
    scope: &[String],
    label: &str,
    author: Option<&str>,
) -> Result<Vec<Record>, NoAuthor> {
    if author.is_some_and(|name| name.trim().is_empty()) {
        return Err(NoAuthor);
    }

    let mut args: Vec<&str> = vec!["--no-merges"];
    if let Some(name) = author {
        // An author is a name, not a pattern. Without this, "Ada [Team]" is an
        // invalid regex and git fails, which deplyd read as "no changes" - a silent
        // wrong answer rather than an error.
        args.push("--fixed-strings");
        args.push("--author");
        args.push(name);
    }
    args.extend_from_slice(&[
        // The name last but one: a subject can hold anything, so it stays the
        // final field and takes whatever tabs are left.
        "--format=%h%x09%ct%x09%cd%x09%an%x09%s",
        "--date=format:%Y-%m-%d %H:%M",
    ]);
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
            let mut parts = line.splitn(5, '\t');
            let sha = parts.next()?.trim().to_string();
            let when = parts.next()?.trim().parse().ok()?;
            let date = parts.next()?.trim().to_string();
            let author = parts.next()?.trim().to_string();
            let subject = parts.next()?.trim().to_string();
            Some(Record {
                sha,
                author,
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
    // Grouped through an index rather than by scanning what is already grouped:
    // that scan was a comparison per record per record, which a repository with
    // real history notices.
    let mut first_seen: HashMap<&str, usize> = HashMap::new();
    let mut grouped: Vec<(String, Vec<&Record>)> = Vec::new();

    for record in records {
        match first_seen.get(record.sha.as_str()) {
            Some(&index) => grouped[index].1.push(record),
            None => {
                first_seen.insert(&record.sha, grouped.len());
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
                author: first.author.clone(),
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
    /// None when the report covers everyone.
    pub author: Option<String>,
    pub live: Vec<Entry>,
    pub page: Vec<Entry>,
    pub reverted: Vec<String>,
    pub skip: usize,
}

pub fn status(
    repo: &Repo,
    targets: &[Target],
    author: Option<&str>,
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
        author: author.map(str::to_string),
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
                author: "Ada".into(),
                when: 200,
                date: "2026-01-02 10:00".into(),
                subject: "shared change".into(),
                label: "API".into(),
            },
            Record {
                sha: "aaa".into(),
                author: "Ada".into(),
                when: 200,
                date: "2026-01-02 10:00".into(),
                subject: "shared change".into(),
                label: "WEB".into(),
            },
            Record {
                sha: "bbb".into(),
                author: "Grace".into(),
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
    fn grouping_holds_up_when_the_same_commit_is_far_apart() {
        // The index replaced a scan of everything grouped so far. Two records for
        // one commit with a thousand between them is what that scan was for.
        let mut records = Vec::new();
        records.push(record("same", 1_000, "API"));
        for index in 0..1_000 {
            records.push(record(&format!("c{index}"), 500 - index, "API"));
        }
        records.push(record("same", 1_000, "WEB"));

        let merged = merge_records(&records);
        assert_eq!(merged.len(), 1_001, "one row per commit");
        assert_eq!(merged[0].sha, "same", "newest first");
        assert_eq!(merged[0].label, "COMBINED", "both targets, one row");
    }

    fn record(sha: &str, when: i64, label: &str) -> Record {
        Record {
            sha: sha.into(),
            author: "Ada".into(),
            when,
            date: "2026-01-02 10:00".into(),
            subject: format!("change {sha}"),
            label: label.into(),
        }
    }

    #[test]
    fn an_empty_author_is_refused() {
        // git log --author='' matches everyone, reporting the whole team's work as
        // one person's. Asking for everyone outright is a different thing, and is
        // spelled None.
        let repo = crate::repo::Repo::discover(std::path::Path::new(".")).expect("a repo");
        assert!(records(&repo, &["HEAD"], &[], "API", Some("   ")).is_err());
        assert!(records(&repo, &["HEAD"], &[], "API", None).is_ok());
    }
}
