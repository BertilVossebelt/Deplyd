//! Checks the guard, parses arguments, dispatches. The work lives in
//! `deplyd-core`, which cannot print.

mod cli;
mod completions;
mod init;
mod render;
mod stub;
mod term;

use std::path::PathBuf;
use std::process::ExitCode;

use anstream::println;
use clap::Parser;

use deplyd_core::context::{Context, ContextError};
use deplyd_core::gateway::credential;
use deplyd_core::gateway::http::ReadOnlyHttp;
use deplyd_core::gateway::selfcheck;
use deplyd_core::github::GitHub;
use deplyd_core::repo::Repo;
use deplyd_core::settings::Settings;
use deplyd_core::targets::{self, TargetSet};
use deplyd_core::verdict;

use cli::{Cli, Command};
use render::Output;
use term::WebBase;

const GITHUB_API: &str = "https://api.github.com";

fn main() -> ExitCode {
    // A weakened build must not reach a repository at all. Microseconds.
    if let Some(code) = refuse_if_guard_is_broken() {
        return code;
    }

    let parsed = Cli::parse();
    let options = parsed.options.clone();
    let output = Output { json: options.json };

    let Some(command) = parsed.command else {
        // A half-typed command line says what the verbs are and stops.
        show_help();
        return ExitCode::SUCCESS;
    };

    if options.json && !command.supports_json() {
        render::stop(
            &format!("--json has nothing to say about '{}'", command.name()),
            &[
                "It is available for the two commands that reach a verdict:".into(),
                "  deplyd status --json".into(),
                "  deplyd pr 412 --json".into(),
            ],
        );
    }

    match &command {
        Command::Check => {
            show_self_check();
            return ExitCode::SUCCESS;
        }
        Command::Uninstall => {
            show_uninstall();
            return ExitCode::SUCCESS;
        }
        Command::Completions { shell } => {
            let mut built = <Cli as clap::CommandFactory>::command();
            completions::emit(*shell, &mut built);
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    let mut settings = Settings::load();

    if let Command::Remember { what, value } = &command {
        remember(&mut settings, what.as_deref(), value.as_deref());
        return ExitCode::SUCCESS;
    }

    let repo = open_repo(&options.repo_path, &settings);

    if matches!(command, Command::Authors) {
        render::authors(&repo);
        return ExitCode::SUCCESS;
    }

    // Before the author is resolved: a tab press must not error on a missing name.
    if let Command::Complete { what } = &command {
        complete(&repo, &settings, what.as_deref());
        return ExitCode::SUCCESS;
    }

    let author = resolve_author(&options.author, &settings, &repo);

    let mut context = match Context::build(repo.root(), author, settings) {
        Ok(context) => context,
        Err(error) => stop_for_context(&error, &repo),
    };

    if matches!(command, Command::Environments) {
        render::environments(&context);
        return ExitCode::SUCCESS;
    }

    // Before an environment is chosen, which such a repo may well reject.
    if matches!(command, Command::Init) {
        init::run(&context, &repo, options.force);
        return ExitCode::SUCCESS;
    }

    if let Err(error) = context.select_environment(options.environment.as_deref().unwrap_or("")) {
        stop_for_context(&error, &repo);
    }

    if matches!(command, Command::Config) {
        render::config(&context);
        return ExitCode::SUCCESS;
    }

    // --- needs GitHub ---------------------------------------------------------------

    // Read the argument first: a typo in it is worth saying before a missing
    // credential is.
    let commit_reference = match &command {
        Command::Commit { reference } => match reference.as_deref().map(str::trim) {
            Some(text) if !text.is_empty() => Some(text.to_string()),
            _ => render::stop(
                "Which commit?",
                &[
                    "deplyd commit a1b2c3d".into(),
                    "Anything git accepts works: a sha, a branch, a tag, HEAD.".into(),
                ],
            ),
        },
        _ => None,
    };

    let pull_request_number = match &command {
        Command::Pr { number } => match cli::read_pull_request_number(number.as_ref()) {
            Ok(number) => Some(number),
            Err(message) => render::stop(&message, &["deplyd pr 412".into()]),
        },
        _ => None,
    };

    let (github, slug, web) = open_github(&repo);
    let mut cache = deplyd_core::cache::Cache::open(&slug.0, &slug.1);

    output.note(&format!(
        "Inspecting {}deploys...",
        context.environment_phrase()
    ));

    let runs = collect_runs(&context, &github, &output);
    let mut targets = targets::build(&context, &repo, &github, &runs, &mut cache, |line| {
        output.note(line)
    });
    cache.save();

    if targets.targets.is_empty() {
        stop_for_no_targets(&context, &targets, runs.len());
    }

    // Worked out before anything prints, because the JSON path needs it too.
    let labels: Vec<String> = targets.targets.iter().map(|t| t.label.clone()).collect();
    for index in 0..targets.targets.len() {
        let concerns = targets::concerns_for(&targets.targets[index], &runs, &labels, &github);
        targets.targets[index].concerns = concerns;
    }

    if let Some(reference) = commit_reference {
        let report = verdict::commit_report(&context, &repo, &targets, &reference);

        if report.status == verdict::Status::NotFound {
            render::stop(
                &format!("No such commit in this clone: {reference}"),
                &[
                    "deplyd reads history from your own clone, so it has to be there.".into(),
                    "Fetch, then run this again.".into(),
                ],
            );
        }

        if options.json {
            print_json(&verdict::PullRequestJson {
                change: report.clone(),
                targets: verdict::target_reports(&context, &targets),
            });
        } else {
            render::target_summary(&context, &targets, &repo, &web, false);
            render::change(&targets, &report, &web);
        }

        return ExitCode::from(verdict::exit_code(report.status, report.uncertain));
    }

    match pull_request_number {
        Some(number) => {
            let report = verdict::pull_request_report(&context, &repo, &github, &targets, number);

            if options.json {
                let document = verdict::PullRequestJson {
                    change: report.clone(),
                    targets: verdict::target_reports(&context, &targets),
                };
                print_json(&document);
            } else {
                render::target_summary(&context, &targets, &repo, &web, false);
                render::change(&targets, &report, &web);
            }

            ExitCode::from(verdict::exit_code(report.status, report.uncertain))
        }
        None => {
            // The pending list compares against the default branch, so it needs a
            // fetch. Only say so when one actually happened.
            if matches!(repo.fetch_once(), Ok(true)) {
                output.note("  fetching: so the pending list is current...");
            }

            let report = match deplyd_core::report::status(
                &repo,
                &targets.targets,
                &context.author,
                options.take as usize,
                options.skip as usize,
            ) {
                Ok(report) => report,
                Err(error) => render::stop(&error.to_string(), &[]),
            };

            if options.json {
                let document = verdict::StatusJson {
                    author: report.author.clone(),
                    environment: context.environment.clone(),
                    targets: verdict::target_reports(&context, &targets),
                    changes: report
                        .live
                        .iter()
                        .map(|entry| verdict::ChangeReport {
                            label: entry.label.clone(),
                            id: entry.id.clone(),
                            title: entry.title.clone(),
                            commit: entry.sha.clone(),
                            reverted: deplyd_core::history::was_reverted(
                                &entry.sha,
                                &report.reverted,
                            ),
                        })
                        .collect(),
                };
                print_json(&document);
            } else {
                render::target_summary(&context, &targets, &repo, &web, true);
                render::status(&context, &targets, &report, &web);
            }

            ExitCode::SUCCESS
        }
    }
}

fn print_json<T: serde::Serialize>(document: &T) {
    match serde_json::to_string_pretty(document) {
        Ok(text) => println!("{text}"),
        Err(error) => render::stop(&format!("could not write JSON: {error}"), &[]),
    }
}

fn refuse_if_guard_is_broken() -> Option<ExitCode> {
    let failures = selfcheck::failures();
    if failures.is_empty() {
        return None;
    }

    anstream::eprintln!();
    anstream::eprintln!("Refused: deplyd's read-only guard is not intact in this build.");
    anstream::eprintln!();
    for failure in &failures {
        anstream::eprintln!("  {}", failure.name);
        for line in failure.detail.lines() {
            anstream::eprintln!("    {line}");
        }
    }
    anstream::eprintln!();
    anstream::eprintln!("  This build would not be safe to point at a repository.");
    anstream::eprintln!("  Nothing was read and nothing was run.");
    anstream::eprintln!();
    Some(ExitCode::from(1))
}

fn open_repo(requested: &Option<String>, settings: &Settings) -> Repo {
    let path: PathBuf = requested
        .clone()
        .or_else(|| settings.repo_path.clone())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match Repo::discover(&path) {
        Ok(repo) => repo,
        Err(_) => render::stop(
            &format!("Not a git repository: {}", path.display()),
            &[
                "Either cd into a repo first, or point at one:".into(),
                "deplyd --repo-path <path to a repo>".into(),
                "deplyd remember repo <path to a repo>".into(),
            ],
        ),
    }
}

fn resolve_author(requested: &Option<String>, settings: &Settings, repo: &Repo) -> String {
    if let Some(author) = requested.clone().filter(|a| !a.trim().is_empty()) {
        return author;
    }
    if let Some(author) = settings.author.clone().filter(|a| !a.trim().is_empty()) {
        return author;
    }
    if let Some(author) = repo.config("user.name") {
        return author;
    }

    // git log --author='' matches everyone, reporting their work as yours.
    render::stop(
        "No author to filter on.",
        &[
            "Pass one with -A <name>, or keep one with: deplyd remember author <name>".into(),
            "It normally comes from git config user.name, which is not set here.".into(),
        ],
    )
}

fn open_github(repo: &Repo) -> (GitHub, (String, String), WebBase) {
    let Some(url) = repo.config("remote.origin.url") else {
        render::stop(
            "This repository has no origin remote, so there is nothing on GitHub to read.",
            &["deplyd reads runs and deployments for the repo origin points at.".into()],
        )
    };

    let web = WebBase(credential::web_base(&url).unwrap_or_default());
    let Some((owner, name)) = credential::parse_remote(&url) else {
        render::stop(
            &format!("Could not read an owner and repository out of: {url}"),
            &["deplyd expects an origin pointing at a GitHub repository.".into()],
        )
    };

    // A stubbed GitHub, when one is configured. No credential is needed for it and
    // none is looked for: there is nothing to authenticate against.
    if let Some(files) = stub::FileTransport::from_environment() {
        return (
            GitHub::new(Box::new(files), owner.clone(), name.clone()),
            (owner, name),
            web,
        );
    }

    let found = match credential::find() {
        Ok(found) => found,
        Err(error) => render::stop(
            &error.to_string(),
            &[gh_install_hint().into(), "gh auth login".into()],
        ),
    };

    match ReadOnlyHttp::new(found.token, GITHUB_API.to_string()) {
        Ok(http) => (
            GitHub::new(Box::new(http), owner.clone(), name.clone()),
            (owner, name),
            web,
        ),
        Err(error) => render::stop(&error.to_string(), &[]),
    }
}

fn collect_runs(
    context: &Context,
    github: &GitHub,
    output: &Output,
) -> Vec<deplyd_core::github::Run> {
    let facts: Vec<_> = context.environment_facts().collect();

    // One request per workflow, all at once. They have nothing to do with each other,
    // so waiting for each in turn was latency spent for no reason.
    let requests: Vec<deplyd_core::github::WorkflowRequest> = facts
        .iter()
        .enumerate()
        .map(|(index, fact)| deplyd_core::github::WorkflowRequest {
            index,
            file: fact.file.clone(),
        })
        .collect();

    let mut runs = Vec::new();
    for (index, found) in github.runs_for_workflows(&requests) {
        let fact = facts[index];
        if found.is_empty() {
            output.note(&format!("  no runs read for {}", fact.file));
        }
        for mut run in found {
            run.workflow_file = fact.file.clone();
            run.workflow_token = fact.token.clone();
            run.needs_log = fact.needs_log();
            runs.push(run);
        }
    }

    runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    runs.dedup_by_key(|run| run.id);

    // The API does not expose dispatch inputs, but it does record a deployment per
    // environment linking back to the run that created it.
    if !context.environment.is_empty() && !context.narrowed_by_name {
        let matching = github.deployments(&context.environment);
        if matching.is_empty() {
            output.note(&format!(
                "  no deployment records for '{}'; showing runs from every environment",
                context.environment
            ));
        } else {
            let filtered: Vec<_> = runs
                .iter()
                .filter(|run| matching.contains(&run.id))
                .cloned()
                .collect();
            if filtered.is_empty() {
                output.note(&format!(
                    "  no runs matched the deployment records for '{}'; showing runs from every environment",
                    context.environment
                ));
            } else {
                output.note(&format!(
                    "  matched {} run(s) to '{}' via GitHub deployments",
                    filtered.len(),
                    context.environment
                ));
                return filtered;
            }
        }
    }

    runs
}

/// "Nothing found" is a dead end. An ignore word matching by accident is the usual
/// cause, so name the jobs that were passed over.
fn stop_for_no_targets(context: &Context, targets: &TargetSet, runs: usize) -> ! {
    // Nothing succeeded is a different answer from nothing was recognised, and
    // saying the second when the first is true sends people looking at job names.
    if targets.runs_available == 0 {
        let phrase = context.environment_phrase();
        if runs == 0 {
            render::stop(
                &format!("No {phrase}deploy runs found."),
                &[
                    "The workflows deplyd found have never run, or the runs are older than it looks back."
                        .into(),
                    "Run deplyd config to see which workflows it is looking at.".into(),
                ],
            );
        }
        render::stop(
            &format!("No successful {phrase}deploy runs found."),
            &[
                format!("{runs} run(s) were found, and none of them succeeded."),
                "deplyd reports what was deployed, so it has nothing to report yet.".into(),
            ],
        );
    }

    let mut hints = vec![
        format!(
            "Looked at the newest {} runs of each workflow, stopping after {} in a row revealed no new target.",
            deplyd_core::github::RUNS_PER_WORKFLOW,
            targets::BARREN_RUNS_BEFORE_STOPPING
        ),
        "A target deployed less recently than that will not be found.".into(),
    ];

    if !targets.ignored_jobs.is_empty() {
        hints.push("These jobs were skipped:".into());
        for (name, why) in &targets.ignored_jobs {
            hints.push(format!("  {name} - {why}"));
        }
        hints.push("Set ignoreJobs in .deplyd.json to change that list.".into());
    }

    render::stop(
        &format!(
            "No deploy jobs recognised in the recent {}runs.",
            context.environment_phrase()
        ),
        &hints,
    )
}

fn stop_for_context(error: &ContextError, repo: &Repo) -> ! {
    match error {
        ContextError::NoWorkflowDirectory(path) => render::stop(
            &format!("No .github/workflows found in {}", path.display()),
            &["There is nothing for deplyd to inspect in this repository.".into()],
        ),
        ContextError::NoDeployWorkflows { pattern, .. } => render::stop(
            "No deploy workflows found.",
            &[
                format!(
                    "Workflow names were searched for /{pattern}/, and no job declares an environment."
                ),
                "Add .deplyd.json at the repo root with a deployPattern or an explicit environments map.".into(),
                "See the README section \"Adapting it to your repo\".".into(),
            ],
        ),
        ContextError::UnreadableOverride { path, reason } => render::stop(
            "Your deplyd settings file could not be read.",
            &[
                path.display().to_string(),
                reason.clone(),
                "Ignoring it would drop the corrections it exists to hold, so deplyd stops.".into(),
                "Fix the JSON, or delete the file to go back to what detection finds.".into(),
            ],
        ),
        ContextError::BadDeployPattern { pattern, reason } => render::stop(
            &format!("deployPattern /{pattern}/ is not a valid regex."),
            &[reason.clone(), "Correct it in .deplyd.json.".into()],
        ),
        ContextError::UnreadableWorkflow { file, error } => render::stop(
            &format!("Could not read {file}"),
            &[error.to_string()],
        ),
        ContextError::AmbiguousEnvironment { requested, matched } => render::stop(
            &format!("Ambiguous environment '{requested}'"),
            &[
                format!("It matches: {}", matched.join(", ")),
                "Spell out more of the name.".into(),
            ],
        ),
        ContextError::UnknownEnvironment {
            requested,
            detected,
            from_settings,
        } => {
            let mut hints = vec![
                format!("Detected: {}", detected.join(", ")),
                "Run deplyd environments to see where each one came from.".into(),
            ];
            if *from_settings {
                // Nobody typed this, so say where it came from before they go looking.
                hints.push(
                    "This came from a remembered default. Change it with: deplyd remember environment <name>"
                        .into(),
                );
            }
            let _ = repo;
            render::stop(&format!("Unknown environment '{requested}'"), &hints)
        }
    }
}

fn remember(settings: &mut Settings, what: Option<&str>, value: Option<&str>) {
    let usage = [
        "deplyd remember author \"Ada\"".to_string(),
        "deplyd remember environment staging".to_string(),
        "deplyd remember repo <path>".to_string(),
    ];

    let Some(key) = what.map(str::to_lowercase) else {
        render::stop("What should be remembered?", &usage);
    };
    let Some(value) = value.filter(|v| !v.trim().is_empty()) else {
        render::stop(
            &format!("Remember {key} as what?"),
            &[format!("deplyd remember {key} <value>")],
        );
    };

    match key.as_str() {
        "author" => settings.author = Some(value.to_string()),
        "environment" => settings.environment = Some(value.to_string()),
        "repo" => {
            let path = PathBuf::from(value);
            if !path.is_dir() {
                render::stop(
                    &format!("No such directory: {value}"),
                    &["deplyd remember repo <path to a repo>".into()],
                );
            }
            let full = path.canonicalize().unwrap_or(path);
            if !full.join(".git").exists() {
                // Remembering a path that cannot work leaves every later run failing
                // here.
                render::stop(
                    &format!("Not a git repository: {}", full.display()),
                    &["Point at the root of a clone, the directory holding .git".into()],
                );
            }
            settings.repo_path = Some(full.to_string_lossy().into_owned());
        }
        _ => render::stop("What should be remembered?", &usage),
    }

    match settings.save() {
        Ok(path) => {
            println!(
                "{}Saved to {}{:#}",
                term::GREEN,
                path.display(),
                term::GREEN
            );
            if let Some(author) = &settings.author {
                println!("  author = {author}");
            }
            if let Some(environment) = &settings.environment {
                println!("  environment = {environment}");
            }
            if let Some(repo) = &settings.repo_path {
                println!("  repoPath = {repo}");
            }
        }
        Err(error) => render::stop(&format!("Could not save settings: {error}"), &[]),
    }
}

/// Bare names, one per line, for the shell to complete against.
fn complete(repo: &Repo, settings: &Settings, what: Option<&str>) {
    match what {
        Some("environments") => {
            // An empty author: completion must work before one is configured.
            let Ok(context) = Context::build(repo.root(), String::new(), settings.clone()) else {
                return;
            };
            for name in &context.environments {
                println!("{name}");
            }
        }
        Some("authors") => {
            let Some(branch) = repo.default_branch() else {
                return;
            };
            for (_, name) in repo.authors(&branch) {
                println!("{name}");
            }
        }
        _ => {}
    }
}

fn gh_install_hint() -> &'static str {
    if cfg!(windows) {
        "winget install GitHub.cli"
    } else if cfg!(target_os = "macos") {
        "brew install gh"
    } else {
        "see https://github.com/cli/cli#installation"
    }
}

fn show_help() {
    let mut built = <Cli as clap::CommandFactory>::command();
    let _ = built.print_help();
    println!();
    println!("{}PER TARGET{:#}", term::CYAN, term::CYAN);
    println!("  DEPLYD      built and released; no newer deploy failing or in flight");
    println!("  UNCERTAIN   a newer deploy for that target did not complete");
    println!("  skipped     steps the run skipped - changes to those are not live");
    println!();
    println!("{}PER PULL REQUEST{:#}", term::CYAN, term::CYAN);
    println!("  DEPLYD      the commit, or an equivalent cherry-pick, is in the deployed commit");
    println!("  REVERTED    it shipped, then was undone before the deployed commit");
    println!("  NOT DEPLYD  neither the commit nor an equivalent change is there");
    println!("  NOT MERGED  still open, or closed without merging");
    println!("  NOT COVERED it changed no path any target covers; names them so you can check");
    println!();
    println!("{}EXIT CODES for deplyd pr{:#}", term::CYAN, term::CYAN);
    println!("  0  deplyd          3  reverted        5  no such pull request");
    println!("  2  not deplyd      4  not merged      6  deplyd, but see UNCERTAIN");
    println!("  1  deplyd could not run");
    println!();
    println!("Everything is detected from .github/workflows. Run \"deplyd config\" to see");
    println!("what it found, and \"deplyd init\" to write it somewhere you can correct it.");
    println!();
    println!(
        "{}Requires the GitHub CLI: {}, then gh auth login.{:#}",
        term::DIM,
        gh_install_hint(),
        term::DIM
    );
    println!();
}

/// `check`: what the gateway allows, and whether this binary still obeys it.
/// A signpost, not a deed. deplyd never deletes - `check` says so and the build
/// guard enforces it - so removing it stays the installer's job, and this prints
/// the line that does it.
fn show_uninstall() {
    println!();
    println!("{}Removing deplyd{:#}", term::CYAN, term::CYAN);
    println!();
    println!("  The installer takes back what it put there. Run:");
    println!();

    let repo = "https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main";
    if cfg!(windows) {
        println!("  & ([scriptblock]::Create((irm {repo}/install.ps1))) -Uninstall");
        println!();
        println!("  Add -Purge to take the remembered defaults and the cache too.");
    } else {
        println!("  curl -fsSL {repo}/install.sh | sh -s -- --uninstall");
        println!();
        println!("  Add --purge to take the remembered defaults and the cache too.");
    }

    println!();
    println!(
        "  Settings live in {}",
        deplyd_core::settings::config_directory().display()
    );
    println!("  deplyd does not delete, so it cannot do this itself. See: deplyd check");
    println!();
}

fn show_self_check() {
    println!();
    println!("{}deplyd read-only self-check{:#}", term::CYAN, term::CYAN);
    println!();

    for result in selfcheck::run() {
        let (mark, style) = if result.passed {
            ("PASS", term::GREEN)
        } else {
            ("FAIL", term::RED)
        };
        println!(
            "  {:<20}{style}{mark}{style:#}  {}",
            result.name, result.detail
        );
    }

    println!();
    println!(
        "  writes on disk      only inside {}",
        deplyd_core::settings::config_directory().display()
    );
    println!("                      remembered defaults, and what deplyd init scaffolds");
    println!("  the one git write   fetch, which updates your own remote-tracking refs");
    println!("                      nothing is sent, and a fetch cannot change a remote");
    println!();
    println!(
        "{}  These ran just now, against the code compiled into this binary,{:#}",
        term::DIM,
        term::DIM
    );
    println!(
        "{}  not against source sitting beside it.{:#}",
        term::DIM,
        term::DIM
    );
    println!();
}
