//! Printing. The only place that writes to a terminal; nothing here decides.

use std::io::Write;

use anstream::{eprintln, println};

use deplyd_core::context::Context;
use deplyd_core::report::{Status as StatusReport, entry_width};
use deplyd_core::targets::{ShaSource, Target, TargetSet};
use deplyd_core::verdict::{
    Change, PullRequestReport, PullRequestTargetReport, Status, TargetStatus,
};

use crate::term::{self, ACCENT, BAD, BOLD, DIM, OK, WARN, WebBase};

/// "2026-09-24 10:58".
const DATE_WIDTH: usize = 16;

pub struct Output {
    pub json: bool,
}

impl Output {
    /// Progress chatter, on stderr so it never lands in a redirected document.
    pub fn note(&self, text: &str) {
        if self.json {
            return;
        }
        eprintln!("{DIM}{text}{DIM:#}");
    }
}

/// Always exit 1: deplyd could not run, which is never a verdict.
pub fn stop(message: &str, hints: &[String]) -> ! {
    refuse(message, hints);
    std::process::exit(1);
}

/// The same, but it returns, so a caller that owes the shell a different exit
/// code can choose one.
pub fn refuse(message: &str, hints: &[String]) {
    eprintln!();
    eprintln!("{BAD}{message}{BAD:#}");
    if !hints.is_empty() {
        eprintln!();
    }
    for hint in hints {
        eprintln!("{DIM}  {hint}{DIM:#}");
    }
    eprintln!();
    let _ = std::io::stderr().flush();
}

/// What each target is running, and whether that can be trusted.
/// Names vary wildly, and a column grown to the longest one would push titles
/// off the screen.
const AUTHOR_WIDTH: usize = 16;

/// The width to give the author column, or None when the report is one author's
/// and saying so on every row would be noise.
fn author_width_for<'a>(
    author: Option<&str>,
    entries: impl Iterator<Item = &'a deplyd_core::report::Entry>,
) -> Option<usize> {
    if author.is_some() {
        return None;
    }
    let widest = entries
        .map(|entry| entry.author.chars().count())
        .max()
        .unwrap_or(0);
    Some(widest.min(AUTHOR_WIDTH))
}

/// The author, padded to the column, or nothing when there is no column.
fn author_cell(entry: &deplyd_core::report::Entry, width: Option<usize>) -> String {
    match width {
        Some(width) => format!("{:<width$}  ", term::truncate(&entry.author, width)),
        None => String::new(),
    }
}

/// How much of a list to show. `-T` and `-S` page every list, not just one.
#[derive(Clone, Copy)]
pub struct Page {
    pub take: usize,
    pub skip: usize,
}

pub fn target_summary(
    context: &Context,
    targets: &TargetSet,
    repo: &deplyd_core::repo::Repo,
    web: &WebBase,
    pending: Option<Page>,
) {
    let mark = term::glyphs();

    println!();
    if !context.environment.is_empty() {
        let count = targets.targets.len();
        let plural = if count == 1 { "target" } else { "targets" };
        println!(
            "{DIM}{}  {}  {count} {plural}{DIM:#}",
            context.environment, mark.dot
        );
        println!();
    }

    for target in &targets.targets {
        let scope = if target.scope.is_empty() {
            "everything".to_string()
        } else {
            target.scope.join(", ")
        };
        println!("  {BOLD}{}{BOLD:#}  {DIM}{scope}{DIM:#}", target.label);

        verdict_line(target, repo, web, &mark);

        // Why it is uncertain comes first: it is the reason the line above said so.
        for (index, concern) in target.concerns.iter().enumerate() {
            let label = if index == 0 { "because" } else { "" };
            field_styled(
                label,
                &format!("run {} {}", concern.run_id, concern.state),
                WARN,
            );
        }
        if !target.sha_warning.is_empty() {
            field_styled("because", &target.sha_warning, WARN);
        }
        if target.sha_source == ShaSource::DeploymentRecord {
            field_styled(
                "because",
                "the run log was unavailable, so this is what the deployment names",
                WARN,
            );
        }

        field(
            "run",
            &term::link(&target.run_id.to_string(), &target.run_url),
        );

        // One per line: a workflow can skip a dozen, and a run-on line hides which.
        for (index, step) in target.skipped.iter().enumerate() {
            let label = if index == 0 { "skipped" } else { "" };
            field_styled(label, &step_text(step), WARN);
        }

        if let Some(page) = pending {
            pending_for_target(context, repo, web, target, page);
        }

        println!();
    }
}

fn step_text(step: &str) -> String {
    term::fit(step, 4 + term::FIELD + 1)
}

/// A labelled row. The value column lines up with the commit on the verdict line.
fn field(label: &str, value: &str) {
    field_styled(label, value, DIM);
}

fn field_styled(label: &str, value: &str, style: anstyle::Style) {
    let width = term::FIELD;
    println!("    {DIM}{label:<width$}{DIM:#} {style}{value}{style:#}");
}

/// The verdict, the commit it is about, and what that commit was.
fn verdict_line(
    target: &Target,
    repo: &deplyd_core::repo::Repo,
    web: &WebBase,
    mark: &term::Glyphs,
) {
    let (glyph, word, style) = if !target.concerns.is_empty() || !target.sha_is_exact {
        (mark.warn, "UNCERTAIN", WARN)
    } else {
        (mark.ok, "DEPLYD", OK)
    };

    let (short, when, subject) = commit_parts(repo, &target.sha);
    let linked = term::link(&short, &web.commit(&target.sha));
    let width = term::FIELD;

    // 2 indent + glyph + space + word column + space, then sha, date and subject.
    let used = 2 + 2 + width + 1 + short.len() + 2 + when.len() + 2;
    println!(
        "  {style}{glyph} {word:<width$}{style:#} {linked}  {DIM}{when}{DIM:#}  {}",
        term::fit(&subject, used)
    );
}

/// The commit as git describes it: short sha, minute-precision date, subject.
fn commit_parts(repo: &deplyd_core::repo::Repo, sha: &str) -> (String, String, String) {
    let short: String = sha.chars().take(7).collect();
    let Ok(output) = repo.run(
        deplyd_core::gateway::git::Verb::Log,
        &[
            "-1",
            "--format=%h%x09%cd%x09%s",
            "--date=format:%Y-%m-%d %H:%M",
            sha,
        ],
    ) else {
        return (short, String::new(), String::new());
    };
    let Some(line) = output.ok.then(|| output.first_line()).flatten() else {
        return (short, String::new(), String::new());
    };
    let mut parts = line.splitn(3, '\t');
    (
        parts.next().unwrap_or(&short).to_string(),
        parts.next().unwrap_or_default().to_string(),
        parts.next().unwrap_or_default().to_string(),
    )
}

fn pending_for_target(
    context: &Context,
    repo: &deplyd_core::repo::Repo,
    web: &WebBase,
    target: &Target,
    page: Page,
) {
    let Some(branch) = repo.default_branch() else {
        field_styled(
            "pending",
            "cannot tell, no default branch to compare against",
            WARN,
        );
        return;
    };

    let range = format!("{}..{}", target.sha, branch);
    let all = deplyd_core::report::records(
        repo,
        &[&range],
        &target.scope,
        &target.label,
        context.author.as_deref(),
    )
    .map(|found| deplyd_core::report::merge_records(&found))
    .unwrap_or_default();

    if all.is_empty() {
        return;
    }

    let pending: Vec<_> = all.iter().skip(page.skip).take(page.take).collect();
    if pending.is_empty() {
        field_styled(
            "pending",
            &format!(
                "{} in total, nothing left after skipping {}",
                all.len(),
                page.skip
            ),
            DIM,
        );
        return;
    }

    let id_width = pending.iter().map(|e| e.id.len()).max().unwrap_or(0);
    let names = author_width_for(context.author.as_deref(), pending.iter().copied());
    for (index, entry) in pending.iter().enumerate() {
        let label = if index == 0 { "pending" } else { "" };
        let linked = link_for(entry, web);
        let pad = id_width.saturating_sub(entry.id.len());
        let who = author_cell(entry, names);
        let used = 4 + term::FIELD + 1 + id_width + 2 + who.chars().count();
        field_styled(
            label,
            &format!(
                "{linked}{:<pad$}  {who}{}",
                "",
                term::fit(&entry.title, used)
            ),
            WARN,
        );
    }

    let last = page.skip + pending.len();
    if last < all.len() {
        let mark = term::glyphs();
        field_styled(
            "",
            &format!("{} more  {}  --skip {last}", all.len() - last, mark.dot),
            DIM,
        );
    }
}

fn link_for(entry: &deplyd_core::report::Entry, web: &WebBase) -> String {
    match entry.pull_request {
        Some(number) => term::link(&entry.id, &web.pull_request(number)),
        None => term::link(&entry.id, &web.commit(&entry.sha)),
    }
}

/// The default report: an author's changes, deployd and pending.
pub fn status(
    context: &Context,
    targets: &TargetSet,
    report: &StatusReport,
    web: &WebBase,
    depth: usize,
) {
    let mark = term::glyphs();
    // Not "PRs": commits pushed straight to a branch appear here too.
    let whose = context.author.as_deref().unwrap_or("everyone");
    let heading = format!("deplyd changes  {}  {whose}", mark.dot);

    // A capped read stopped where it was told to, so the count is a floor. The
    // plus sign is the difference between "355" and "at least 355".
    let total = format!(
        "{}{}",
        report.live.len(),
        if report.capped { "+" } else { "" }
    );

    if report.live.is_empty() {
        println!("{ACCENT}{heading}{ACCENT:#}  {DIM}none{DIM:#}");
        return;
    }
    if report.page.is_empty() {
        println!(
            "{ACCENT}{heading}{ACCENT:#}  {DIM}{total} in total, nothing left after skipping {}{DIM:#}",
            report.skip
        );
        return;
    }

    let first = report.skip + 1;
    let last = report.skip + report.page.len();
    println!("{ACCENT}{heading}{ACCENT:#}  {DIM}{first}-{last} of {total}{DIM:#}");
    println!();

    let label_width = targets.label_width();
    let id_width = entry_width(&report.page);
    let show_labels = targets.labels_are_informative();
    let names = author_width_for(context.author.as_deref(), report.page.iter());

    for entry in &report.page {
        let reverted = deplyd_core::history::was_reverted(&entry.sha, &report.reverted);
        let suffix = if reverted {
            format!("  {} reverted", mark.bad)
        } else {
            String::new()
        };

        let who = author_cell(entry, names);
        let used = 2
            + if show_labels { label_width + 2 } else { 0 }
            + DATE_WIDTH
            + 2
            + id_width
            + 2
            + who.chars().count()
            + suffix.chars().count();
        let title = term::fit(&entry.title, used);
        let linked = link_for(entry, web);
        let pad = id_width.saturating_sub(entry.id.len());

        let label = if show_labels {
            format!("{BOLD}{:<label_width$}{BOLD:#}  ", entry.label)
        } else {
            String::new()
        };

        if reverted {
            println!(
                "  {label}{DIM}{:<DATE_WIDTH$}{DIM:#}  {linked}{:<pad$}  {DIM}{who}{DIM:#}{BAD}{title}{suffix}{BAD:#}",
                entry.date, ""
            );
        } else {
            println!(
                "  {label}{DIM}{:<DATE_WIDTH$}{DIM:#}  {linked}{:<pad$}  {DIM}{who}{DIM:#}{title}",
                entry.date, ""
            );
        }
    }

    if last < report.live.len() {
        println!(
            "  {DIM}{}{} older  {}  --skip {last}{DIM:#}",
            report.live.len() - last,
            if report.capped { "+" } else { "" },
            mark.dot
        );
    }

    if report.capped {
        println!();
        println!(
            "{DIM}  Stopped after {depth} commits per target. Read further with --depth.{DIM:#}"
        );
    }

    if targets.targets.iter().any(|t| !t.concerns.is_empty()) {
        println!();
        println!(
            "{DIM}  Listed against the last completed deploy; newer ones did not complete.{DIM:#}"
        );
    }

    println!();
}

/// `pr` and `commit`: is this change deployd, and if not, why not.
pub fn change(targets: &TargetSet, report: &PullRequestReport, web: &WebBase) {
    if report.status == Status::NotFound {
        match &report.change {
            Change::Commit { sha } => {
                println!("{WARN}commit {sha} : not found.{WARN:#}");
                println!("{DIM}No such commit in this clone.{DIM:#}");
            }
            Change::PullRequest { number } => {
                println!("{WARN}PR #{number} : not found.{WARN:#}");
                println!("{DIM}No such pull request, or no access to it.{DIM:#}");
            }
        }
        println!();
        return;
    }

    // A commit is not a pull request and must not be labelled as one: it has no
    // number, no merge state and nothing to link to but itself.
    let (label, url) = match &report.change {
        Change::Commit { sha } => (
            format!("commit {}", sha.chars().take(9).collect::<String>()),
            web.commit(sha),
        ),
        Change::PullRequest { number } => (format!("PR #{number}"), web.pull_request(*number)),
    };

    let heading = term::link(&label, &url);
    println!(
        "{ACCENT}{heading}  {}{ACCENT:#}",
        term::fit(&report.title, label.len() + 2)
    );

    if report.status == Status::NotMerged {
        unmerged(report);
        return;
    }

    let short: String = report.commit.chars().take(7).collect();
    let linked = term::link(&short, &web.commit(&report.commit));
    let width = term::FIELD;
    match report.commit_source {
        Some(deplyd_core::verdict::CommitSource::Api) => {
            println!("  {DIM}{:<width$} {linked}{DIM:#}", "merge");
        }
        Some(_) => {
            println!(
                "  {DIM}{:<width$} {linked}  matched on the subject, the API being unavailable{DIM:#}",
                "merge"
            )
        }
        // A commit was named outright, so saying where it came from would be noise.
        None => {}
    }
    println!();

    if report.status == Status::CommitMissingLocally {
        println!();
        println!("{WARN}  The merge commit {short} is not in your local clone.{WARN:#}");
        println!("{DIM}  Fetch, then run this again.{DIM:#}");
        println!();
        return;
    }

    if report.status == Status::NotCovered {
        not_covered(targets, report);
        return;
    }

    let width = targets.label_width();
    for entry in &report.targets {
        pull_request_target(entry, width, web);
    }
    println!();
}

fn unmerged(report: &PullRequestReport) {
    println!();
    match report.state.as_str() {
        "OPEN" => {
            println!("{WARN}  NOT MERGED - still open, so it cannot be deployed{WARN:#}");
            if !report.branch.is_empty() {
                println!("{DIM}  branch {}{DIM:#}", report.branch);
            }
        }
        "CLOSED" => println!("{WARN}  NOT MERGED - closed without merging{WARN:#}"),
        other => {
            println!("{WARN}  NOT MERGED - state is {other}, with no merge commit{WARN:#}")
        }
    }
    println!();
}

/// Either the change really is outside everything that deploys, or a scope is wrong.
/// Those look identical from one line, so show the comparison that was made.
fn not_covered(targets: &TargetSet, report: &PullRequestReport) {
    println!();
    println!("{WARN}  NOT COVERED - it changed nothing inside any deployed target{WARN:#}");
    println!();
    println!("{DIM}  Targets considered:{DIM:#}");

    let width = targets.label_width();
    let mut unscoped = 0;
    for target in &targets.targets {
        if target.scope.is_empty() {
            unscoped += 1;
            println!(
                "{DIM}    {:<width$}  covers everything, yet matched nothing{DIM:#}",
                target.label
            );
        } else {
            println!(
                "{DIM}    {:<width$}  covers {}{DIM:#}",
                target.label,
                target.scope.join(", ")
            );
        }
    }

    println!();
    println!("{DIM}  Files it changed:{DIM:#}");
    for file in &report.files {
        println!("{DIM}    {}{DIM:#}", term::fit(file, 4));
    }

    println!();
    if unscoped > 0 {
        println!(
            "{WARN}  A target above covers everything and still matched nothing, so this is{WARN:#}"
        );
        println!("{WARN}  a bug in deplyd rather than a scope to fix.{WARN:#}");
    } else {
        println!(
            "{DIM}  If a path above should belong to a target, its scope is wrong. Set it:{DIM:#}"
        );
        println!();
        println!("    deplyd config init");
        println!(
            "{DIM}    then edit scopes so the right target lists the path, for example:{DIM:#}"
        );

        let example_label = targets
            .targets
            .first()
            .map(|t| t.label.clone())
            .unwrap_or_else(|| "API".into());
        // The first file with a directory above it: a root-level file would print an
        // empty scope.
        let example_path = report
            .files
            .iter()
            .find_map(|file| {
                let parent = std::path::Path::new(file).parent()?;
                let text = parent.to_string_lossy().replace('\\', "/");
                (!text.is_empty()).then_some(text)
            })
            .or_else(|| report.files.first().cloned())
            .unwrap_or_else(|| "src/whatever".into());

        println!("      \"scopes\": {{ \"{example_label}\": [\"{example_path}\"] }}");
    }
    println!();
}

fn pull_request_target(entry: &PullRequestTargetReport, width: usize, web: &WebBase) {
    let mark = term::glyphs();
    let deployed: String = entry.deployed_commit.chars().take(7).collect();
    let linked = term::link(&deployed, &web.commit(&entry.deployed_commit));
    let verdict = 12;

    match entry.status {
        TargetStatus::NotDeplyd => {
            println!(
                "  {BOLD}{:<width$}{BOLD:#}  {BAD}{} {:<verdict$}{BAD:#} {DIM}deployed {linked}{DIM:#}",
                entry.label, mark.bad, "NOT DEPLYD"
            );
        }
        TargetStatus::DeplydAsCopy => {
            let Some(copy) = &entry.copy else { return };
            println!(
                "  {BOLD}{:<width$}{BOLD:#}  {OK}{} {:<verdict$}{OK:#} in {linked} {DIM}as a copy{DIM:#}",
                entry.label, mark.ok, "DEPLYD"
            );
            let detail = if copy.commit.is_empty() {
                format!("{}: {}", copy.how, copy.subject)
            } else {
                format!("{} {}: {}", copy.how, copy.commit, copy.subject)
            };
            let used = 2 + width + 2 + 2 + verdict + 1;
            println!(
                "  {:<width$}  {:<gap$} {DIM}{}{DIM:#}",
                "",
                "",
                term::fit(&detail, used),
                gap = verdict + 2
            );
        }
        TargetStatus::Reverted => {
            println!(
                "  {BOLD}{:<width$}{BOLD:#}  {BAD}{} {:<verdict$}{BAD:#} {DIM}undone before {linked}{DIM:#}",
                entry.label, mark.bad, "REVERTED"
            );
            for revert in &entry.reverts {
                let used = 2 + width + 2 + verdict + 6 + revert.commit.len() + 2;
                println!(
                    "  {:<width$}  {:<gap$} {BAD}by {}  {}{BAD:#}",
                    "",
                    "",
                    revert.commit,
                    term::fit(&revert.subject, used),
                    gap = verdict + 2
                );
            }
        }
        TargetStatus::Deplyd => {
            // The caveat is yellow though the verdict is green: uncertainty never
            // takes the colour reserved for a bad answer.
            if entry.uncertain {
                println!(
                    "  {BOLD}{:<width$}{BOLD:#}  {OK}{} {:<verdict$}{OK:#} in {linked}{WARN}  {} see UNCERTAIN above{WARN:#}",
                    entry.label, mark.ok, "DEPLYD", mark.warn
                );
            } else {
                println!(
                    "  {BOLD}{:<width$}{BOLD:#}  {OK}{} {:<verdict$}{OK:#} in {linked}",
                    entry.label, mark.ok, "DEPLYD"
                );
            }

            for (index, later) in entry.changed_after.iter().enumerate() {
                let label = if index == 0 { "changed after" } else { "" };
                let used = 2 + width + 2 + verdict + 16 + later.commit.len() + 2;
                println!(
                    "  {:<width$}  {DIM}{label:<gap$} {}  {}{DIM:#}",
                    "",
                    later.commit,
                    term::fit(&later.subject, used),
                    gap = verdict + 2
                );
            }
        }
    }
}

/// `environments`: what `-E` accepts, and where each name came from.
pub fn environments(context: &Context) {
    println!();
    if context.environments.is_empty() {
        println!("{ACCENT}No named environments.{ACCENT:#}");
        println!("{DIM}This repo deploys without naming environments, which is fine.{DIM:#}");
        println!(
            "{DIM}Run deplyd with no -E. Name them in .deplyd.json if you want them split.{DIM:#}"
        );
        println!();
        return;
    }

    println!("{ACCENT}Environments{ACCENT:#}");
    let width = context
        .environments
        .iter()
        .map(String::len)
        .max()
        .unwrap_or(0);

    for name in &context.environments {
        let matched = context.workflows_for_environment(name);
        println!("  {name:<width$}  {} workflow(s)", matched.len());
        for index in matched {
            println!(
                "{DIM}  {:<width$}    {}{DIM:#}",
                "", context.facts[index].file
            );
        }
    }

    println!();
    println!("{DIM}Use any of these, or a prefix: deplyd -E <name>{DIM:#}");
    println!();
}

/// `config`: what detection concluded, so it can be checked before being trusted.
pub fn config(context: &Context) {
    let workflows: Vec<_> = context.environment_facts().collect();
    let needing_log = workflows.iter().filter(|fact| fact.needs_log()).count();

    println!();
    println!("Repo           {}", context.repo_root.display());
    println!("Environments   {}", context.environments.join(", "));
    println!("Selected       {}", context.environment);
    println!(
        "Author         {}",
        context.author.as_deref().unwrap_or("everyone")
    );
    println!();
    println!("{ACCENT}Deploy workflows ({}){ACCENT:#}", workflows.len());

    let mut groups: Vec<(String, Vec<&str>)> = Vec::new();
    for fact in &workflows {
        let scope = if fact.working_directories.is_empty() {
            "(no scope)".to_string()
        } else {
            fact.working_directories.join(", ")
        };
        match groups.iter_mut().find(|(name, _)| *name == scope) {
            Some((_, files)) => files.push(&fact.file),
            None => groups.push((scope, vec![&fact.file])),
        }
    }
    groups.sort_by(|a, b| b.0.cmp(&a.0));

    for (scope, files) in groups {
        for (index, file) in files.iter().enumerate() {
            let shown = if index == 0 { scope.as_str() } else { "" };
            println!("  {shown:<16} {file}");
        }
    }

    println!();
    if needing_log == 0 {
        println!("{DIM}Commit source  the run's own ref{DIM:#}");
    } else if needing_log == workflows.len() {
        println!(
            "{DIM}Commit source  the run log - these workflows deploy a branch input or call other workflows{DIM:#}"
        );
    } else {
        println!(
            "{DIM}Commit source  the run log for {needing_log} of {}, the run's own ref for the rest{DIM:#}",
            workflows.len()
        );
    }
    println!(
        "{DIM}Ignored jobs   names containing {}{DIM:#}",
        context.ignore_jobs.join(", ")
    );

    if let Some(overrides) = &context.overrides {
        for (label, paths) in &overrides.scopes {
            println!("{DIM}Scope override {label} = {}{DIM:#}", paths.join(", "));
        }
        let where_it_is = if context.override_location.shared {
            "committed in the repo"
        } else {
            "kept beside deplyd"
        };
        println!(
            "{DIM}Overrides      {} ({where_it_is}){DIM:#}",
            context.override_location.path.display()
        );
    } else {
        println!(
            "{DIM}No overrides   deplyd config init writes the above to a file you can correct{DIM:#}"
        );
    }

    // A workflow that could not be read must not silently vanish: a missing deploy
    // workflow looks exactly like a repository that does not deploy.
    if !context.unreadable.is_empty() {
        println!();
        println!(
            "{WARN}Unreadable     {} workflow(s) could not be parsed and were left out:{WARN:#}",
            context.unreadable.len()
        );
        for (file, error) in &context.unreadable {
            println!("{WARN}               {file}: {error}{WARN:#}");
        }
        println!(
            "{DIM}               Set scopes and environments in .deplyd.json to cover them.{DIM:#}"
        );
    }
    println!();
}

pub fn authors(repo: &deplyd_core::repo::Repo) {
    // Local HEAD is whatever this clone has checked out, usually behind.
    let Some(branch) = repo.default_branch() else {
        println!();
        println!("{WARN}Cannot list authors: no default branch ref to count over.{WARN:#}");
        println!("{DIM}origin/HEAD, origin/main and origin/master all failed to resolve.{DIM:#}");
        println!();
        return;
    };

    println!();
    println!("{ACCENT}Authors (commit count, name) on {branch}{ACCENT:#}");
    for (count, name) in repo.authors(&branch) {
        println!("  {count:>6}  {name}");
    }
    println!();
    println!(
        "{DIM}Use any of these with -A. Partial matches work, so a first name is enough.{DIM:#}"
    );
    println!("{DIM}One person can appear under several names; git counts them separately.{DIM:#}");
    println!();
}

// --- watching ---------------------------------------------------------------

/// The line a watcher opens with, so it is obvious what it is waiting for and
/// that nothing being printed means nothing is happening.
pub fn watch_opening(
    context: &Context,
    until: Option<&crate::Asked>,
    every: std::time::Duration,
    has_deadline: bool,
) {
    let mark = term::glyphs();
    let whose = context.author.as_deref().unwrap_or("everyone");
    let waiting = match until {
        None => "until you stop it".to_string(),
        Some(crate::Asked::PullRequest(number)) => format!("until PR #{number} is live"),
        Some(crate::Asked::Commit(reference)) => format!("until {reference} is live"),
    };
    let deadline = if has_deadline {
        ", or --for runs out"
    } else {
        ""
    };

    println!();
    println!(
        "{ACCENT}watching  {}  {whose}{ACCENT:#}  {DIM}every {}, {waiting}{deadline}{DIM:#}",
        mark.dot,
        spoken(every)
    );
    println!();
}

/// One event. Human output is a line you can watch scroll past; --json is one
/// object per line, so something else can read it as it arrives.
pub fn watch_event(event: &deplyd_core::watch::Event, json: bool, web: &WebBase) {
    if json {
        if let Ok(line) = serde_json::to_string(event) {
            println!("{line}");
            // A watcher's reader is waiting on this line, not on the process
            // ending, so it cannot sit in a buffer until then.
            let _ = std::io::stdout().flush();
        }
        return;
    }

    let mark = term::glyphs();
    let (glyph, colour) = match event.kind {
        deplyd_core::watch::Kind::DeployStarted => (mark.dot, DIM),
        deplyd_core::watch::Kind::DeploySucceeded => (mark.ok, OK),
        deplyd_core::watch::Kind::DeployFailed => (mark.bad, BAD),
        deplyd_core::watch::Kind::ChangeLive => (mark.ok, OK),
    };

    let linked = if event.url.is_empty() {
        event.id.clone()
    } else {
        term::link(&event.id, &event.url)
    };
    let who = if event.author.is_empty() {
        String::new()
    } else {
        format!("  {DIM}{}{DIM:#}", event.author)
    };
    let _ = web;

    println!(
        "  {DIM}{}{DIM:#}  {colour}{glyph} {:<8}{colour:#}  {BOLD}{}{BOLD:#}  {linked}  {}{who}",
        now_hms(),
        event.kind.as_str().split('.').next_back().unwrap_or(""),
        event.label,
        event.title
    );
    let _ = std::io::stdout().flush();
}

/// GitHub asked for a pause. Said out loud, because a watcher that went quiet
/// for twenty minutes would otherwise look broken.
pub fn watch_paused(wait: std::time::Duration) {
    println!(
        "  {DIM}{}  GitHub asked deplyd to wait {}s before looking again{DIM:#}",
        now_hms(),
        wait.as_secs()
    );
    let _ = std::io::stdout().flush();
}

pub fn watch_closing(reason: &str) {
    println!();
    println!("{DIM}Stopped: {reason}.{DIM:#}");
    println!();
}

/// A length of time said the way it was typed: 300 seconds is "5m".
fn spoken(length: std::time::Duration) -> String {
    let seconds = length.as_secs();
    match seconds {
        0 => "no time".to_string(),
        s if s % 3600 == 0 => format!("{}h", s / 3600),
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

/// Wall-clock time of day, for a log someone reads as it happens. Only the clock
/// is asked for: deplyd does no calendar arithmetic of its own.
fn now_hms() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let day = seconds % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3600, (day % 3600) / 60, day % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(author: &str) -> deplyd_core::report::Entry {
        deplyd_core::report::Entry {
            when: 0,
            author: author.to_string(),
            date: String::new(),
            label: String::new(),
            sha: String::new(),
            id: String::new(),
            pull_request: None,
            title: String::new(),
        }
    }

    #[test]
    fn a_long_name_is_cut_to_the_column() {
        let long = entry("Wolfeschlegelsteinhausenbergerdorff");
        let cell = author_cell(&long, Some(AUTHOR_WIDTH));
        assert_eq!(
            cell.chars().count(),
            AUTHOR_WIDTH + 2,
            "padded to the column"
        );
        assert!(
            cell.trim_end().ends_with('…'),
            "cut, not squeezed: {cell:?}"
        );
    }

    #[test]
    fn the_column_is_only_as_wide_as_the_longest_name() {
        let entries = [entry("Ada"), entry("Grace")];
        assert_eq!(author_width_for(None, entries.iter()), Some(5));
    }

    #[test]
    fn one_authors_report_has_no_column() {
        let entries = [entry("Ada")];
        assert_eq!(author_width_for(Some("Ada"), entries.iter()), None);
        assert_eq!(author_cell(&entries[0], None), "");
    }
}
