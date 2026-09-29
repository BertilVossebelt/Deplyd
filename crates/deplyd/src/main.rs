//! Checks the guard, parses arguments, dispatches. The work lives in
//! `deplyd-core`, which cannot print.

mod cli;
mod completions;
mod init;
mod render;
mod stub;
mod term;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

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

use cli::{Change, Cli, Command, ConfigAction, HookAction, ListWhat, StartupAction, WatchAction};
use render::Output;
use term::WebBase;

const GITHUB_API: &str = "https://api.github.com";
/// Where deplyd itself is published, for the update check and the two signposts.
const DEPLYD_OWNER: &str = "BertilVossebelt";
const DEPLYD_NAME: &str = "Deplyd";
const DEPLYD_RAW: &str = "https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main";

fn main() -> ExitCode {
    // A weakened build must not reach a repository at all. Microseconds.
    if let Some(code) = refuse_if_guard_is_broken() {
        return code;
    }

    let parsed = match Cli::try_parse() {
        Ok(parsed) => parsed,
        Err(error) => return misread(error),
    };

    let command = parsed.command;
    let options = command.options();

    // The copy doing the watching, before it can stop for any reason. A record
    // that says running after its process has left is worse than no record:
    // `list watchers` shows it, `watch stop` waits on it, and nothing comes.
    if let Command::Watch {
        watcher_id: Some(id),
        ..
    } = &command
    {
        let id = id.clone();
        render::before_leaving(move || {
            // Read again rather than kept: what is on disk by now is what
            // other processes have been told, and it is that copy being closed.
            if let Ok(mut record) = deplyd_core::watchers::find(&id) {
                record.mark_stopped();
            }
        });
    }
    let output = Output { json: options.json };

    match &command {
        Command::Check => {
            show_self_check();
            return ExitCode::SUCCESS;
        }
        Command::Quota => {
            return show_quota();
        }
        // What the machine calls at login, before anything else: there is no
        // terminal to talk to and no repository to open.
        Command::Watch {
            action:
                Some(WatchAction::Startup {
                    action: Some(StartupAction::Run { id }),
                }),
            ..
        } => {
            return run_at_startup(id.as_deref());
        }
        Command::Uninstall => {
            show_uninstall();
            return ExitCode::SUCCESS;
        }
        Command::Update => {
            show_update();
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

    if let Command::Watch {
        action: Some(WatchAction::Startup { action }),
        ..
    } = &command
        && !matches!(action, Some(StartupAction::Run { .. }))
    {
        startup(action.as_ref());
        return ExitCode::SUCCESS;
    }

    if let Command::Watch {
        action: Some(action @ (WatchAction::Stop { .. } | WatchAction::Log { .. })),
        ..
    } = &command
    {
        watchers(action);
        return ExitCode::SUCCESS;
    }

    if let Command::Hooks { action } = &command {
        hooks(&mut settings, action);
        return ExitCode::SUCCESS;
    }

    if let Command::Remember { what, value } = &command {
        remember(&mut settings, what.as_deref(), value.as_deref());
        return ExitCode::SUCCESS;
    }

    if let Command::List { what, .. } = &command {
        match what {
            ListWhat::Watchers => {
                render::watchers(&deplyd_core::watchers::all());
                return ExitCode::SUCCESS;
            }
            ListWhat::Hooks => {
                render::hooks(&settings.hooks);
                return ExitCode::SUCCESS;
            }
            _ => {}
        }
    }

    let repo = open_repo(&options.repo_path, &settings);

    if matches!(
        command,
        Command::List {
            what: ListWhat::Authors,
            ..
        }
    ) {
        render::authors(&repo);
        return ExitCode::SUCCESS;
    }

    // Before the author is resolved: a tab press must not error on a missing name.
    if let Command::Complete { what, .. } = &command {
        complete(&repo, &settings, what.as_deref());
        return ExitCode::SUCCESS;
    }

    let author = if options.anyone {
        None
    } else {
        Some(resolve_author(&options.author, &settings, &repo))
    };

    // The flag beats the kept default beats the built-in.
    let depth = options
        .depth
        .or(settings.depth)
        .map(|value| value as usize)
        .unwrap_or(deplyd_core::report::DEFAULT_DEPTH);

    let mut context = match Context::build(repo.root(), author, settings) {
        Ok(context) => context,
        Err(error) => stop_for_context(&error, &repo),
    };

    if matches!(
        command,
        Command::List {
            what: ListWhat::Environments,
            ..
        }
    ) {
        render::environments(&context);
        return ExitCode::SUCCESS;
    }

    // Before an environment is chosen, which such a repo may well reject.
    if matches!(
        command,
        Command::Config {
            action: Some(ConfigAction::Init { .. }),
            ..
        }
    ) {
        init::run(&context, &repo, options.force);
        return ExitCode::SUCCESS;
    }

    if let Err(error) = context.select_environment(options.environment.as_deref().unwrap_or("")) {
        stop_for_context(&error, &repo);
    }

    if matches!(command, Command::Config { action: None, .. }) {
        render::config(&context);
        return ExitCode::SUCCESS;
    }

    // --- needs GitHub ---------------------------------------------------------------

    // Read the argument first: a typo in it is worth saying before a missing
    // credential is.
    let verb = command.name();
    let asked = match &command {
        Command::Status { change, .. } => change.as_ref().map(|change| Asked::read(change, verb)),
        Command::Watch { action, .. } => action
            .as_ref()
            .and_then(WatchAction::change)
            .map(|change| Asked::read(&change, verb)),
        _ => None,
    };

    // Read here, with the other arguments and before any request: a typo in a
    // duration is the user's to fix, and finding out after a round trip is worse.
    let watch_plan = match &command {
        Command::Watch {
            duration, every, ..
        } => Some(WatchPlan::read(
            asked.clone(),
            duration.as_deref(),
            every.as_deref(),
            context.settings.watch_every,
        )),
        _ => None,
    };

    // Before GitHub is opened at all. The child does the looking, so a parent
    // that took the first look would spend the allowance twice over for it.
    // watcher_id is set only on the copy that was started for this purpose, so
    // it is what tells parent from child. Without it the child sees the same
    // flags the parent did, backgrounds itself again, and deplyd spawns until
    // something else stops it.
    if let (true, Command::Watch { at_startup, .. }, Some(plan)) =
        (cli::should_detach(&command), &command, &watch_plan)
    {
        // Registered first. A watch that cannot be written into the startup
        // folder should say so before one is left running that will not come
        // back, which is the failure nobody notices until the next reboot.
        if *at_startup && let Err(why) = register_at_startup(&repo) {
            render::stop(&why, &["Nothing was started.".into()]);
        }
        return start_in_background(&context, &repo, plan);
    }

    let (github, slug, web) = open_github(&repo);
    let mut cache = deplyd_core::cache::Cache::open(&slug.0, &slug.1);

    output.note(&format!(
        "Inspecting {}deploys...",
        context.environment_phrase()
    ));

    // The first look, retried rather than abandoned when it is a watcher doing
    // the looking.
    //
    // A refused request answers empty, so every question after it would be
    // answered from nothing: no targets found, no changes live. Both read as
    // facts about the repository and neither is one, which is why this sits
    // before the empty check and before anything is reported.
    //
    // Once watching, being refused is something to wait out - the loop already
    // does exactly that further down. Exiting here instead would kill a watcher
    // that started at boot into a spent allowance, and nothing would bring it
    // back until the machine restarted.
    // Both started before the first look, because the first look can itself be
    // refused and waited out. A deadline created afterwards would not count that
    // wait, and a record loaded afterwards would leave it unstamped.
    let running_as = match &command {
        Command::Watch { watcher_id, .. } => watcher_id.clone(),
        _ => None,
    };
    let mut record = running_as
        .as_deref()
        .and_then(|id| deplyd_core::watchers::find(id).ok());
    let deadline = watch_plan
        .as_ref()
        .and_then(|plan| plan.length)
        .map(|length| Instant::now() + length);

    let (runs, mut targets) = loop {
        let runs = collect_runs(&context, &github, &output);
        let targets = targets::build(&context, &repo, &github, &runs, &mut cache, |line| {
            output.note(line)
        });
        cache.save();

        let Some(wait) = github.rate_limited() else {
            break (runs, targets);
        };

        if watch_plan.is_none() {
            stop_for_spent_allowance(wait);
        }

        // The same floor the loop uses. Without it a reset already almost due
        // gives a one second wait, and this turns into a tight retry against
        // the very thing that just refused us.
        let wait = wait.max(Duration::from_secs(60));
        if !options.json {
            render::watch_paused(wait);
        }
        // Measured against the deadline the whole watch shares, so waiting out
        // a refusal spends the time it was given rather than adding to it.
        if let Some(reason) = wait_without_going_quiet(wait, &mut record, deadline) {
            return finished(record.as_mut(), options.json, reason);
        }
        github.forget();
    };

    if targets.targets.is_empty() {
        stop_for_no_targets(&context, &targets, runs.len());
    }

    // Worked out before anything prints, because the JSON path needs it too.
    let labels: Vec<String> = targets.targets.iter().map(|t| t.label.clone()).collect();
    for index in 0..targets.targets.len() {
        let concerns = targets::concerns_for(&targets.targets[index], &runs, &labels, &github);
        targets.targets[index].concerns = concerns;
    }

    if let Some(plan) = watch_plan {
        return watch_loop(
            &context,
            &repo,
            &github,
            &mut cache,
            &web,
            &options,
            depth,
            (runs, targets),
            plan,
            record,
            deadline,
        );
    }

    match asked {
        Some(change) => {
            let report = match &change {
                Asked::Commit(reference) => {
                    let report = verdict::commit_report(&context, &repo, &targets, reference);
                    if report.status == verdict::Status::NotFound {
                        render::stop(
                            &format!("No such commit in this clone: {reference}"),
                            &[
                                "deplyd reads history from your own clone, so it has to be there."
                                    .into(),
                                "Fetch, then run this again.".into(),
                            ],
                        );
                    }
                    report
                }
                Asked::PullRequest(number) => {
                    verdict::pull_request_report(&context, &repo, &github, &targets, *number)
                }
            };

            if options.json {
                let document = verdict::PullRequestJson {
                    change: report.clone(),
                    targets: verdict::target_reports(&context, &targets),
                };
                print_json(&document);
            } else {
                render::target_summary(&context, &targets, &repo, &web, None);
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
                context.author.as_deref(),
                options.take as usize,
                options.skip as usize,
                depth,
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
                            author: entry.author.clone(),
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
                let page = render::Page {
                    take: options.take as usize,
                    skip: options.skip as usize,
                };
                render::target_summary(&context, &targets, &repo, &web, Some(page));
                render::status(&context, &targets, &report, &web, depth);
            }

            ExitCode::SUCCESS
        }
    }
}

/// A command line that could not be read.
///
/// clap answers a typo with a usage block, and a missing subcommand with the
/// whole help page. Neither is what someone who mistyped one word needs, so
/// this says what was not understood and leaves finding the rest to `--help`.
fn misread(error: clap::Error) -> ExitCode {
    use clap::error::{ContextKind, ContextValue, ErrorKind};

    // --help and --version are not mistakes: they are the output.
    if matches!(
        error.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        let _ = error.print();
        if error.kind() == ErrorKind::DisplayHelp {
            show_reading_key(help_subject().as_deref());
        }
        return ExitCode::SUCCESS;
    }

    let one = |kind| match error.get(kind) {
        Some(ContextValue::String(value)) => Some(value.clone()),
        _ => None,
    };
    let many = |kind| match error.get(kind) {
        Some(ContextValue::Strings(values)) => values.clone(),
        _ => Vec::new(),
    };

    let mut hints: Vec<String> = Vec::new();
    // Which verb the mistake was under, so --help can point at that one.
    let mut verb = "deplyd".to_string();

    let said = match error.kind() {
        ErrorKind::InvalidSubcommand => {
            let what = one(ContextKind::InvalidSubcommand).unwrap_or_else(|| "that".into());
            for near in many(ContextKind::SuggestedSubcommand) {
                hints.push(format!("Did you mean {}?", near.trim_matches('\'')));
            }
            format!("deplyd has no '{what}' command.")
        }
        ErrorKind::UnknownArgument => {
            let what = one(ContextKind::InvalidArg).unwrap_or_else(|| "that".into());
            for near in many(ContextKind::SuggestedArg) {
                hints.push(format!("Did you mean {}?", near.trim_matches('\'')));
            }
            format!("'{what}' is not one of this command's options.")
        }
        ErrorKind::MissingSubcommand => {
            // clap names the path as it was invoked, e.g. "deplyd.exe list".
            let path = one(ContextKind::InvalidSubcommand).unwrap_or_default();
            let mut words = path.split_whitespace().skip(1).peekable();
            let nested = words.peek().is_some();

            // Asked of the tree rather than taken from the error: clap's list of
            // valid subcommands counts `complete`, which is hidden because the
            // shell calls it and people do not.
            let choices = visible_children(words);
            if !choices.is_empty() {
                hints.push(choices.join(", "));
            }

            if nested {
                verb = path.replace(".exe", "");
                format!("{verb} needs to know which one.")
            } else {
                "deplyd needs to know what to do.".to_string()
            }
        }
        // Everything else is about a value, and clap's own first line already
        // names it better than a guess from the kind would.
        _ => summary(&error),
    };

    hints.push(format!("Run {verb} --help to see what it takes."));
    render::refuse(&said, &hints);
    // 2 is what a shell expects of a usage error, and is not one of the verdicts.
    ExitCode::from(2)
}

/// clap's own first line, without its "error: " prefix or the usage block under it.
fn summary(error: &clap::Error) -> String {
    let rendered = error.render().to_string();
    let first = rendered
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    first
        .trim()
        .strip_prefix("error: ")
        .unwrap_or(first.trim())
        .to_string()
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

/// The one change a command was pointed at. `status` reports on it, `watch`
/// waits for it; being a subcommand, it cannot be both at once.
#[derive(Clone)]
enum Asked {
    PullRequest(u32),
    Commit(String),
}

impl Asked {
    /// `verb` spells the example back the way it was typed: `status pr`, `watch pr`.
    fn read(change: &Change, verb: &str) -> Self {
        match change {
            Change::Pr { number } => match cli::read_pull_request_number(number.as_ref()) {
                Ok(number) => Asked::PullRequest(number),
                Err(message) => render::stop(&message, &[format!("deplyd {verb} 412")]),
            },
            Change::Commit { reference } => match reference.as_deref().map(str::trim) {
                Some(text) if !text.is_empty() => Asked::Commit(text.to_string()),
                _ => render::stop(
                    "Which commit?",
                    &[
                        format!("deplyd {verb} a1b2c3d"),
                        "Anything git accepts works: a sha, a branch, a tag, HEAD.".into(),
                    ],
                ),
            },
        }
    }
}

/// Looking oftener than this spends an hourly allowance fast for very little:
/// a look costs a request per deploy workflow, plus a few for what it finds.
const FASTEST_LOOK: Duration = Duration::from_secs(10);

/// Everything `watch` was asked for, checked before anything is fetched.
struct WatchPlan {
    until: Option<Asked>,
    every: Duration,
    length: Option<Duration>,
}

impl WatchPlan {
    fn read(
        until: Option<Asked>,
        duration: Option<&str>,
        every: Option<&str>,
        kept: Option<u32>,
    ) -> Self {
        let interval = read_interval(every, kept.unwrap_or(60) as u64, "--every");
        if interval < FASTEST_LOOK {
            render::stop(
                &format!(
                    "Looking every {}s is faster than deplyd will go.",
                    interval.as_secs()
                ),
                &[
                    format!(
                        "{}s is the floor: a look costs several requests, out of an hourly allowance.",
                        FASTEST_LOOK.as_secs()
                    ),
                    "--every 5m is plenty for watching a deploy you are waiting on.".into(),
                ],
            );
        }

        Self {
            until,
            every: interval,
            length: duration.map(|given| read_interval(Some(given), 0, "--for")),
        }
    }
}

fn read_interval(text: Option<&str>, fallback: u64, flag: &str) -> Duration {
    match text {
        None => Duration::from_secs(fallback),
        Some(given) => match deplyd_core::watch::parse_duration(given) {
            Some(duration) => duration,
            None => render::stop(
                &format!("'{given}' is not a length of time."),
                &[format!("{flag} 30s, {flag} 5m and {flag} 2h all work.")],
            ),
        },
    }
}

/// Polls, says what changed, and stops when it was told to.
///
/// The first look is the baseline. Announcing everything already true would be
/// a wall of news about things that happened before anyone was watching.
#[allow(clippy::too_many_arguments)]
fn watch_loop(
    context: &Context,
    repo: &Repo,
    github: &GitHub,
    cache: &mut deplyd_core::cache::Cache,
    web: &WebBase,
    options: &cli::Options,
    depth: usize,
    first: (Vec<deplyd_core::github::Run>, TargetSet),
    plan: WatchPlan,
    // Set when this is the detached copy, so it can stamp its own record and
    // notice when it has been asked to stop.
    mut record: Option<deplyd_core::watchers::Watcher>,
    // Started before the first look, so time spent waiting out a refusal before
    // the loop began is already counted against it.
    deadline: Option<Instant>,
) -> ExitCode {
    let WatchPlan {
        until,
        every,
        length,
    } = plan;
    let _ = length;

    // Progress chatter belongs to the first look only; repeating it every minute
    // would bury the events underneath it.
    let quiet = Output { json: true };
    let mut previous: Option<deplyd_core::watch::Snapshot> = None;
    let (mut runs, mut targets) = first;

    if !options.json {
        render::watch_opening(context, until.as_ref(), every, deadline.is_some());
    }

    loop {
        let report = match deplyd_core::report::status(
            repo,
            &targets.targets,
            context.author.as_deref(),
            usize::MAX,
            0,
            depth,
        ) {
            Ok(report) => report,
            Err(error) => render::stop(&error.to_string(), &[]),
        };

        // Asked before the events are worked out: a look that was refused saw an
        // empty GitHub, and treating that as the truth would report everything as
        // gone and then, next time, as new.
        if let Some(wait) = github.rate_limited() {
            let wait = wait.max(Duration::from_secs(60));
            if !options.json {
                render::watch_paused(wait);
            }
            if let Some(reason) = wait_without_going_quiet(wait, &mut record, deadline) {
                return finished(record.as_mut(), options.json, reason);
            }
            github.forget();
            let _ = repo.fetch_again();
            runs = collect_runs(context, github, &quiet);
            targets = targets::build(context, repo, github, &runs, cache, |_| {});
            cache.save();
            continue;
        }

        let snapshot = snapshot_of(&runs, &targets, &report);
        match &previous {
            None => {}
            Some(before) => {
                for event in deplyd_core::watch::changes(before, &snapshot) {
                    render::watch_event(&event, options.json, web);

                    // After the event is printed, so what deplyd saw is on
                    // screen whatever the hooks then do with it. A hook that
                    // fails is said and the watch carries on: it is a
                    // notification, not a step the deploy depends on.
                    if !context.settings.hooks.is_empty() {
                        let payload = serde_json::to_string(&event).unwrap_or_default();
                        let mut failed = Vec::new();

                        for path in &context.settings.hooks {
                            let (path, outcome) = run_one_hook(path, &payload);
                            if outcome.is_err() {
                                failed.push((path, outcome));
                            }
                            // Between hooks, not after all of them. Each may
                            // take its full timeout, and a watcher that has not
                            // stamped for long enough reads as lost - at which
                            // point `watchers stop` refuses it while it is still
                            // very much running.
                            if let Some(watcher) = record.as_mut() {
                                watcher.beat();
                            }
                        }

                        if !failed.is_empty() && !options.json {
                            render::hook_results(&failed, false);
                        }
                    }
                }
            }
        }
        previous = Some(snapshot);

        // Stamped every look, which is what tells `watchers` this one is still
        // going - and the same look picks up whatever another process wrote to
        // the record, which is how being asked to stop arrives.
        if let Some(watcher) = record.as_mut()
            && watcher.beat()
        {
            return finished(record.as_mut(), options.json, "asked to stop");
        }

        // Asked after the events, so the run that carried a change is reported
        // before the watcher exits on it.
        if let Some((code, reason)) = watch_reached(context, repo, github, &targets, until.as_ref())
        {
            if !options.json {
                render::watch_closing(reason);
            }
            if let Some(watcher) = record.as_mut() {
                watcher.mark_stopped();
            }
            return code;
        }
        let out_of_time = || deadline.is_some_and(|end| Instant::now() >= end);
        if out_of_time() {
            return finished(record.as_mut(), options.json, "time is up");
        }

        // Never sleep past the deadline: --for 30s with --every 5m should stop at
        // thirty seconds, not five minutes.
        let pause = match deadline {
            Some(end) => every.min(end.saturating_duration_since(Instant::now())),
            None => every,
        };
        std::thread::sleep(pause);

        // Checked again rather than falling into a look nobody will read.
        if out_of_time() {
            return finished(record.as_mut(), options.json, "time is up");
        }

        // Everything memoised is from the last look, and a watcher that trusted
        // it would report nothing for as long as it ran.
        github.forget();
        let _ = repo.fetch_again();
        runs = collect_runs(context, github, &quiet);
        targets = targets::build(context, repo, github, &runs, cache, |_| {});
        cache.save();
    }
}

/// Waits, without going quiet.
///
/// A rate limit can pause a watcher for the best part of an hour. Sleeping that
/// off in one go stops the heartbeat, and a record that has not been stamped for
/// three intervals reads as `Lost` - at which point `watchers stop` refuses it
/// as "not running" while the process is very much alive and unstoppable.
///
/// So the wait is taken in slices: stamped each time, and still listening for a
/// stop or a deadline it was given.
fn wait_without_going_quiet(
    wait: Duration,
    record: &mut Option<deplyd_core::watchers::Watcher>,
    deadline: Option<Instant>,
) -> Option<&'static str> {
    const SLICE: Duration = Duration::from_secs(5);

    let until = Instant::now() + wait;
    loop {
        if deadline.is_some_and(|end| Instant::now() >= end) {
            return Some("time is up");
        }
        if let Some(watcher) = record.as_mut()
            && watcher.beat()
        {
            return Some("asked to stop");
        }

        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        std::thread::sleep(SLICE.min(left));
    }
}

/// The one way out of the loop, so a record cannot be left saying "running"
/// for a watcher that finished perfectly well.
///
/// Without this, ending on `--for` leaves the record untouched, and three
/// intervals later `watchers` calls it "went quiet" - which is what it says for
/// a crash. A clean end should not look like a failure.
fn finished(
    record: Option<&mut deplyd_core::watchers::Watcher>,
    json: bool,
    reason: &str,
) -> ExitCode {
    if !json {
        render::watch_closing(reason);
    }
    if let Some(watcher) = record {
        watcher.mark_stopped();
    }
    ExitCode::SUCCESS
}

/// Whether the thing being waited for has happened, and what to say about it.
/// None means keep watching. Saying it is the caller's job, because a JSON
/// stream has no room for a sentence.
fn watch_reached(
    context: &Context,
    repo: &Repo,
    github: &GitHub,
    targets: &TargetSet,
    until: Option<&Asked>,
) -> Option<(ExitCode, &'static str)> {
    let report = match until? {
        Asked::PullRequest(number) => {
            verdict::pull_request_report(context, repo, github, targets, *number)
        }
        Asked::Commit(reference) => verdict::commit_report(context, repo, targets, reference),
    };

    match report.status {
        verdict::Status::Deplyd => Some((ExitCode::SUCCESS, "what you were waiting for is live")),
        // A pull request that cannot ever go live is not something to wait out.
        verdict::Status::NotFound | verdict::Status::NotCovered => Some((
            ExitCode::from(verdict::exit_code(report.status, false)),
            "there is nothing here to wait for",
        )),
        _ => None,
    }
}

/// One look, in the shape the differ compares.
fn snapshot_of(
    runs: &[deplyd_core::github::Run],
    targets: &TargetSet,
    report: &deplyd_core::report::Status,
) -> deplyd_core::watch::Snapshot {
    let label_for = |run: &deplyd_core::github::Run| {
        targets
            .targets
            .iter()
            .find(|target| target.run_id == run.id)
            .map(|target| target.label.clone())
            .unwrap_or_else(|| run.workflow_file.clone())
    };

    deplyd_core::watch::Snapshot {
        runs: runs
            .iter()
            .map(|run| {
                (
                    run.id,
                    deplyd_core::watch::RunState {
                        label: label_for(run),
                        status: run.status.clone(),
                        conclusion: run.conclusion.clone(),
                        url: run.html_url.clone(),
                    },
                )
            })
            .collect(),
        live: report
            .live
            .iter()
            .map(|entry| {
                (
                    entry.sha.clone(),
                    deplyd_core::watch::LiveChange {
                        id: entry.id.clone(),
                        title: entry.title.clone(),
                        author: entry.author.clone(),
                        label: entry.label.clone(),
                    },
                )
            })
            .collect(),
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
                "Run deplyd list environments to see where each one came from.".into(),
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
        "deplyd remember every 5m".to_string(),
        "deplyd remember depth 500".to_string(),
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
        "every" => match deplyd_core::watch::parse_duration(value) {
            // Refused here as well as at watch time, so a default that could
            // never be used is not quietly written down.
            Some(every) if every >= FASTEST_LOOK => {
                settings.watch_every = Some(every.as_secs() as u32)
            }
            Some(_) => render::stop(
                &format!(
                    "Looking every {value} is faster than deplyd will go, {}s is the floor.",
                    FASTEST_LOOK.as_secs()
                ),
                &["deplyd remember every 5m".into()],
            ),
            None => render::stop(
                &format!("'{value}' is not a length of time."),
                &["deplyd remember every 5m".into()],
            ),
        },
        "depth" => match value.trim().parse::<u32>() {
            Ok(depth) if depth > 0 => settings.depth = Some(depth),
            _ => render::stop(
                &format!("A depth is a whole number of commits, not '{value}'."),
                &["deplyd remember depth 500".into()],
            ),
        },
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
            println!("{}Saved to {}{:#}", term::OK, path.display(), term::OK);
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
            // No author: completion must work before one is configured.
            let Ok(context) = Context::build(repo.root(), None, settings.clone()) else {
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

/// The subcommands a verb offers, hidden ones left out. `path` is the words
/// after the binary name, so an empty one asks the root.
fn visible_children<'a>(path: impl Iterator<Item = &'a str>) -> Vec<String> {
    let built = <Cli as clap::CommandFactory>::command();
    let mut here = &built;
    for word in path {
        match here.get_subcommands().find(|sub| sub.get_name() == word) {
            Some(found) => here = found,
            None => return Vec::new(),
        }
    }
    here.get_subcommands()
        .filter(|sub| !sub.is_hide_set())
        .map(|sub| sub.get_name().to_string())
        .collect()
}

/// Which verb's `--help` was asked for, resolved the way clap resolves it so
/// that `deplyd st --help` counts as `status`.
fn help_subject() -> Option<String> {
    let built = <Cli as clap::CommandFactory>::command();
    let word = std::env::args().skip(1).find(|a| !a.starts_with('-'))?;

    if let Some(exact) = built.get_subcommands().find(|s| s.get_name() == word) {
        return Some(exact.get_name().to_string());
    }
    // A prefix counts only while it still names one verb, as at parse time.
    let mut matching = built
        .get_subcommands()
        .filter(|s| s.get_name().starts_with(&word));
    let first = matching.next()?;
    matching
        .next()
        .is_none()
        .then(|| first.get_name().to_string())
}

/// The words the reports use, under the help of the verbs that print them.
///
/// On the root help this was a wall nobody asked for. Here it sits beside the
/// thing it explains, and `list` or `check` never shows it at all.
fn show_reading_key(verb: Option<&str>) {
    let (status, watch) = (Some("status") == verb, Some("watch") == verb);
    if !status && !watch {
        return;
    }

    println!();
    println!("{}Per target{:#}", term::ACCENT, term::ACCENT);
    key(
        term::OK,
        "DEPLYD",
        "built and released; no newer deploy failing or in flight",
    );
    key(
        term::WARN,
        "UNCERTAIN",
        "a newer deploy did not complete, or the commit read badly",
    );
    key(
        term::WARN,
        "skipped",
        "steps the run skipped - changes to those are not live",
    );

    println!();
    println!("{}Per change{:#}", term::ACCENT, term::ACCENT);
    key(
        term::OK,
        "DEPLYD",
        "the commit, or an equivalent cherry-pick, is in the deploy",
    );
    key(
        term::BAD,
        "REVERTED",
        "it shipped, then was undone before the deployed commit",
    );
    key(
        term::BAD,
        "NOT DEPLYD",
        "neither the commit nor an equivalent change is there",
    );
    key(
        term::WARN,
        "NOT MERGED",
        "still open, or closed without merging",
    );
    key(
        term::WARN,
        "NOT COVERED",
        "it changed no path any target covers",
    );

    println!();
    println!("{}Exit codes{:#}", term::ACCENT, term::ACCENT);
    if status {
        println!(
            "  {}status pr{:#} and {}status commit{:#} answer with one:",
            term::OK,
            term::OK,
            term::OK,
            term::OK
        );
        println!("  0 deplyd       2 not deplyd   3 reverted");
        println!("  4 not merged   5 no such PR   6 deplyd, but see UNCERTAIN");
        println!("  1 deplyd could not run");
    } else {
        println!("  0  what it waited for went live, or --for ran out");
        // A change that can never go live is not something to wait out, so the
        // loop stops on the verdict rather than running to the deadline.
        println!("  2  it covers no target, so it can never go live");
        println!("  5  no such pull request");
        println!("  1  deplyd could not run");
    }
    println!();
}

/// One row of the key: the word in the colour the reports print it, then what
/// it means.
fn key(style: anstyle::Style, word: &str, meaning: &str) {
    println!(
        "  {style}{word:<12}{style:#}{}{meaning}{:#}",
        term::DIM,
        term::DIM
    );
}

/// Writes the settings, or says why it could not.
///
/// Swallowed, this loses a hook silently: the list on screen would be the one
/// that was wanted and the one on disk the one that will actually run.
fn save_settings(settings: &Settings) {
    if let Err(error) = settings.save() {
        render::stop(
            &format!("Could not write the settings: {error}"),
            &["Nothing was changed.".into()],
        );
    }
}

/// `startup`: the watches that come back when the machine does.
fn startup(action: Option<&StartupAction>) {
    use deplyd_core::startup;

    let named = |id: &Option<String>| -> String {
        match id.as_deref().map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => id.to_string(),
            None => render::stop("Which one?", &["deplyd startup lists them.".into()]),
        }
    };

    match action {
        None => render::startup_entries(&startup::all(), startup::os_location().ok()),
        Some(StartupAction::Disable { id }) => match startup::set_enabled(&named(id), false) {
            Ok(entry) => render::startup_changed(&entry),
            Err(why) => render::stop(&why, &["deplyd startup lists them.".into()]),
        },
        Some(StartupAction::Enable { id }) => match startup::set_enabled(&named(id), true) {
            Ok(entry) => render::startup_changed(&entry),
            Err(why) => render::stop(&why, &["deplyd startup lists them.".into()]),
        },
        Some(StartupAction::Run { .. }) => unreachable!("handled before the repo is opened"),
    }
}

/// Writes this watch into wherever the machine looks at login.
///
/// The arguments are this run's own, minus the flag that asked for it, so what
/// comes back after a reboot is the watch that was asked for.
fn register_at_startup(repo: &Repo) -> Result<(), String> {
    use deplyd_core::startup::{self, Entry};

    let exe = std::env::current_exe()
        .map_err(|error| format!("Could not find deplyd itself: {error}"))?
        .display()
        .to_string();

    let given: Vec<String> = std::env::args().skip(1).collect();
    let root = repo.root().display().to_string();
    let args = cli::args_for_startup(&given, &root);

    // Asking twice should not mean two of them at every boot.
    if let Some(mut existing) = startup::already_registered(&root, &args) {
        if !existing.enabled {
            existing = startup::set_enabled(&existing.id, true)?;
        }
        render::startup_already(&existing);
        return Ok(());
    }

    let id = startup::new_id(std::process::id());
    let written = startup::install(&id, &exe)?;
    let entry = Entry {
        id: id.clone(),
        args,
        repo: repo.root().display().to_string(),
        enabled: true,
        created_at: deplyd_core::watchers::now(),
        os_file: written.display().to_string(),
    };
    entry
        .save()
        .map_err(|error| format!("Wrote {}, but not its record: {error}", written.display()))?;

    render::startup_registered(&entry);
    Ok(())
}

/// What the machine calls at login. Starts the watch, or does nothing if it has
/// since been turned off - the file it was told to run stays either way.
fn run_at_startup(id: Option<&str>) -> ExitCode {
    use deplyd_core::startup;

    let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) else {
        return ExitCode::from(2);
    };
    let Ok(entry) = startup::find(id) else {
        return ExitCode::from(2);
    };
    if !entry.enabled {
        return ExitCode::SUCCESS;
    }

    // entry.id, not the argument: `find` accepts a prefix, so `startup run abc`
    // for entry abc123 would otherwise write into abc.log while every other
    // command talks about abc123.
    let log = startup::directory().join(format!("{}.log", entry.id));
    match deplyd_core::gateway::background::respawn(&entry.args, &log) {
        Ok(_) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// `watchers`: what is running in the background, and how to stop it.
fn watchers(action: &WatchAction) {
    use deplyd_core::watchers;

    match action {
        WatchAction::Stop { id } => {
            let Some(id) = id.as_deref().filter(|id| !id.trim().is_empty()) else {
                render::stop(
                    "Which watcher?",
                    &["deplyd list watchers lists them.".into()],
                );
            };
            match watchers::request_stop(id) {
                Ok(watcher) => render::watcher_stopping(&watcher),
                Err(why) => render::stop(&why, &["deplyd list watchers lists them.".into()]),
            }
        }
        WatchAction::Log { id } => {
            let Some(id) = id.as_deref().filter(|id| !id.trim().is_empty()) else {
                render::stop(
                    "Which watcher?",
                    &["deplyd list watchers lists them.".into()],
                );
            };
            match watchers::find(id) {
                Ok(watcher) => render::watcher_log(&watcher),
                Err(why) => render::stop(&why, &["deplyd list watchers lists them.".into()]),
            }
        }
        // Everything else `watch` can be asked is handled before this point.
        WatchAction::Pr { .. } | WatchAction::Commit { .. } | WatchAction::Startup { .. } => {
            unreachable!("handled earlier")
        }
    }
}

/// Hands the whole watch over to a detached copy of deplyd and returns.
///
/// The child is given the same command line with the background flag dropped
/// and its id added, so what runs in the background is the watch that was asked
/// for rather than a reconstruction of it.
fn start_in_background(context: &Context, repo: &Repo, plan: &WatchPlan) -> ExitCode {
    use deplyd_core::watchers::{self, Watcher};

    let id = watchers::new_id(std::process::id());
    let log = watchers::directory().join(format!("{id}.log"));

    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Both flags dropped: the child is the watcher, not another asker. Leaving
    // --at-startup on would have it register itself all over again.
    args.retain(|arg| !matches!(arg.as_str(), "--background" | "-B" | "--at-startup"));
    args.push("--watcher-id".into());
    args.push(id.clone());
    // Pinned, so the child is not at the mercy of whatever directory it inherits.
    if !args.iter().any(|arg| arg == "--repo-path") {
        args.push("--repo-path".into());
        args.push(repo.root().display().to_string());
    }

    // Written before the child exists, not after. The child looks its own record
    // up on the way in, and a record that arrives late is a watcher that never
    // heartbeats, never hears a stop, and cannot be listed - while the parent
    // says it started fine.
    let mut watcher = Watcher {
        id: id.clone(),
        pid: 0,
        repo: repo.root().display().to_string(),
        environment: context.environment.clone(),
        author: context.author.clone().unwrap_or_default(),
        every_secs: plan.every.as_secs(),
        started_at: watchers::now(),
        last_seen: watchers::now(),
        stop_requested: false,
        stopped_at: None,
        log: log.display().to_string(),
    };
    if let Err(error) = watcher.save() {
        render::stop(
            &format!("Could not write its record: {error}"),
            &[
                "Nothing was started: a watcher nothing can list or stop is worse".into(),
                "than no watcher at all.".into(),
            ],
        );
    }

    match deplyd_core::gateway::background::respawn(&args, &log) {
        Ok(pid) => {
            watcher.pid = pid;
            let _ = watcher.save();
        }
        Err(error) => {
            // The record exists but nothing is running, so it is closed off
            // rather than left looking like something that went quiet.
            watcher.mark_stopped();
            render::stop(&error.to_string(), &[]);
        }
    }

    render::watcher_started(&watcher);
    ExitCode::SUCCESS
}

/// `hooks`: the scripts a watcher kicks, and the three things you do to them.
fn hooks(settings: &mut Settings, action: &HookAction) {
    match action {
        HookAction::Add { path } => {
            let full = absolute(path);
            let shown = full.display().to_string();

            if !full.is_file() {
                render::stop(
                    &format!("No such file: {shown}"),
                    &["A hook is a script deplyd starts, so it has to be there first.".into()],
                );
            }
            if settings.hooks.iter().any(|held| held == &shown) {
                render::stop(
                    &format!("Already a hook: {shown}"),
                    &["deplyd hooks lists them.".into()],
                );
            }

            settings.hooks.push(shown.clone());
            save_settings(settings);
            render::stop_free(&format!("Added {shown}"), &settings.hooks);
        }
        HookAction::Remove { path } => {
            let shown = absolute(path).display().to_string();
            let before = settings.hooks.len();
            // Matched on the full path, or on what the user typed, because the
            // list prints full paths and people paste what they typed.
            settings.hooks.retain(|held| held != &shown && held != path);

            if settings.hooks.len() == before {
                render::stop(
                    &format!("Not a hook: {path}"),
                    &["deplyd hooks lists them.".into()],
                );
            }
            save_settings(settings);
            render::stop_free(&format!("Removed {shown}"), &settings.hooks);
        }
        HookAction::Test => {
            if settings.hooks.is_empty() {
                render::stop(
                    "No hooks registered, so there is nothing to test.",
                    &["deplyd hooks add <script>".into()],
                );
            }
            let sample = deplyd_core::watch::sample_event_json();
            render::hook_results(&run_hooks(&settings.hooks, &sample), true);
        }
    }
}

/// A path as the user typed it, made absolute so the list means one thing from
/// whatever directory a watcher happens to run in.
fn absolute(path: &str) -> PathBuf {
    let given = PathBuf::from(path);
    let full = if given.is_absolute() {
        given
    } else {
        std::env::current_dir()
            .map(|here| here.join(&given))
            .unwrap_or(given)
    };
    // Rebuilt from its parts, so a path typed with forward slashes is not
    // written down half one way and half the other.
    full.components().collect()
}

/// Kicks every hook with one event, and says how each went. A hook that fails is
/// reported and the rest still run: they are notifications, not steps in a chain.
fn run_hooks(hooks: &[String], payload: &str) -> Vec<(String, Result<String, String>)> {
    hooks
        .iter()
        .map(|path| run_one_hook(path, payload))
        .collect()
}

/// One hook, kicked and reported on.
fn run_one_hook(path: &str, payload: &str) -> (String, Result<String, String>) {
    {
        {
            let outcome = deplyd_core::gateway::hook::run(
                Path::new(path),
                payload,
                deplyd_core::gateway::hook::DEFAULT_TIMEOUT,
            );
            let told = match outcome {
                Ok(done) if done.ok() => Ok("ok".to_string()),
                Ok(done) => {
                    let code = done
                        .code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "signal".into());
                    Err(if done.stderr.is_empty() {
                        format!("exit {code}")
                    } else {
                        format!("exit {code}: {}", done.stderr.lines().next().unwrap_or(""))
                    })
                }
                Err(error) => Err(error.to_string()),
            };
            (path.to_string(), told)
        }
    }
}

/// Nothing was read, because there was nothing left to read with.
///
/// Said rather than worked around: retrying is what spent it, and a watcher or
/// a shell loop that cannot tell "no" from "nothing" will keep going.
fn stop_for_spent_allowance(wait: std::time::Duration) -> ! {
    render::stop(
        "GitHub's hourly allowance is spent, so nothing was read.",
        &[
            format!("It refills in {}.", render::spell_duration(wait)),
            "deplyd quota shows what is left, and costs nothing to ask.".into(),
        ],
    )
}

/// `quota`: how much of the hourly allowance is left.
///
/// Worth its own verb because every other verb spends it, and a watcher spends
/// it steadily. Asking costs nothing: GitHub does not count this route.
fn show_quota() -> ExitCode {
    // No repository asked for. The allowance belongs to the account, not to a
    // repo, and needing to stand in one to ask how much is left would be a
    // strange thing to insist on - not least when the reason you are asking is
    // that something else already refused.
    let github = match stub::FileTransport::from_environment() {
        Some(files) => GitHub::new(Box::new(files), DEPLYD_OWNER.into(), DEPLYD_NAME.into()),
        None => {
            let found = match credential::find() {
                Ok(found) => found,
                Err(error) => render::stop(
                    &error.to_string(),
                    &[gh_install_hint().into(), "gh auth login".into()],
                ),
            };
            match ReadOnlyHttp::new(found.token, GITHUB_API.to_string()) {
                Ok(http) => GitHub::new(Box::new(http), DEPLYD_OWNER.into(), DEPLYD_NAME.into()),
                Err(error) => render::stop(&error.to_string(), &[]),
            }
        }
    };

    match github.quota() {
        Ok(quota) => {
            render::quota(&quota);
            ExitCode::SUCCESS
        }
        Err(error) => render::stop(
            &format!("Could not read the allowance: {error}"),
            &["GitHub states it on every answer, including a refusal.".into()],
        ),
    }
}

/// `check`: what the gateway allows, and whether this binary still obeys it.
/// A signpost, not a deed. deplyd never deletes - `check` says so and the build
/// guard enforces it - so removing it stays the installer's job, and this prints
/// the line that does it.
/// The line that installs the newest release. Installing over an existing copy is
/// the update, so there is nothing separate to print.
fn install_line() -> String {
    if cfg!(windows) {
        format!("irm {DEPLYD_RAW}/install.ps1 | iex")
    } else {
        format!("curl -fsSL {DEPLYD_RAW}/install.sh | sh")
    }
}

/// Asks which release is newest and says whether this is it. Another signpost:
/// replacing the binary is the installer's job, because deplyd does not write
/// outside its own config.
fn show_update() {
    let current = env!("CARGO_PKG_VERSION");

    let asking = |transport| {
        GitHub::new(transport, DEPLYD_OWNER.into(), DEPLYD_NAME.into()).latest_release()
    };
    let latest = match stub::FileTransport::from_environment() {
        Some(files) => asking(Box::new(files)),
        None => credential::find()
            .ok()
            .and_then(|found| ReadOnlyHttp::new(found.token, GITHUB_API.to_string()).ok())
            .and_then(|http| asking(Box::new(http))),
    };

    println!();
    println!("{}deplyd {current}{:#}", term::ACCENT, term::ACCENT);
    println!();

    match latest.as_deref() {
        Some(tag) if tag.trim_start_matches('v') == current => {
            println!("  Up to date: {tag} is the newest release.");
            println!();
            return;
        }
        Some(tag) => println!("  {tag} is out. To update, run:"),
        // Not being able to ask is not the same as being current, so it says which.
        None => println!("  Could not ask which release is newest. To update, run:"),
    }

    println!();
    println!("  {}", install_line());
    println!();
    println!("  That replaces the binary where it already is. Uninstalling first is");
    println!("  not needed, and deplyd cannot do it itself: see deplyd check.");
    println!();
}

fn show_uninstall() {
    println!();
    println!("{}Removing deplyd{:#}", term::ACCENT, term::ACCENT);
    println!();
    println!("  The installer takes back what it put there. Run:");
    println!();

    let repo = DEPLYD_RAW;
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
    println!(
        "{}deplyd read-only self-check{:#}",
        term::ACCENT,
        term::ACCENT
    );
    println!();

    for result in selfcheck::run() {
        let (mark, style) = if result.passed {
            ("PASS", term::OK)
        } else {
            ("FAIL", term::BAD)
        };
        println!(
            "  {:<20}{style}{mark}{style:#}  {}",
            result.name, result.detail
        );
    }

    println!();
    println!(
        "  writes on disk      inside {}",
        deplyd_core::settings::config_directory().display()
    );
    println!(
        "                      defaults, watcher records and logs, and what config init writes"
    );

    // Named separately because it is the one write that lands outside deplyd's
    // own directory. A check that says "only inside" while a file sits in the
    // startup folder would be telling a comfortable lie.
    let booted = deplyd_core::startup::all();
    match deplyd_core::startup::os_location() {
        Ok(where_) if !booted.is_empty() => {
            println!(
                "  and at startup      {} entry in {}",
                booted.len(),
                where_.display()
            );
            println!("                      written when you asked with --at-startup");
        }
        Ok(where_) => {
            println!("  and at startup      nothing. --at-startup would write one into");
            println!("                      {}", where_.display());
        }
        Err(_) => {}
    }
    println!("  the one git write   fetch, which updates your own remote-tracking refs");
    println!("                      nothing is sent, and a fetch cannot change a remote");

    // Deplyd still only reads. But it will start these, and what they do is not
    // deplyd's to promise, so a check that reports what it does has to say so.
    let hooks = Settings::load().hooks;
    if hooks.is_empty() {
        println!("  hooks               none registered, so nothing else is ever started");
    } else {
        println!(
            "  hooks               {} registered, started on what a watcher sees",
            hooks.len()
        );
        for path in &hooks {
            println!("                      {path}");
        }
        println!("                      deplyd starts these; what they do is yours");
    }
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
