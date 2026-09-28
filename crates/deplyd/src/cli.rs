//! The command line. A verb says what to do, a flag says how.
//!
//! Options hang off the verbs that use them rather than off the root, so
//! `--help` for a verb lists what that verb will actually do something with.
//! `main` still wants them in one place, so [`Command::options`] gathers them
//! back up, filling in the default for anything the verb never offered.

use clap::builder::styling::Styles;
use clap::{Args, Parser, Subcommand};

use crate::term;

/// How many changes a listing shows when nothing says otherwise.
pub const DEFAULT_TAKE: u32 = 10;

/// The help wearing the same palette as the reports, rather than a second one
/// of its own. clap's default is bold and underline only, so without this the
/// help is the one colourless thing deplyd prints.
fn styles() -> Styles {
    Styles::styled()
        .header(term::ACCENT)
        .usage(term::ACCENT)
        .literal(term::OK)
        .placeholder(term::DIM)
        .error(term::BAD)
        .valid(term::OK)
        .invalid(term::WARN)
}

#[derive(Debug, Parser)]
#[command(
    name = "deplyd",
    about = "Which commit an environment was last deployed from",
    long_about = None,
    version,
    styles = styles(),
    infer_subcommands = true,
    disable_help_subcommand = true,
    subcommand_required = true,
    arg_required_else_help = false
)]
pub struct Cli {
    // Required, so a bare `deplyd` is the same missing-subcommand error as a
    // bare `deplyd list`, worded by the same code. One shape, not two.
    #[command(subcommand)]
    pub command: Command,
}

/// Which repo. Every verb that opens one takes it.
#[derive(Debug, Args, Clone, Default)]
pub struct RepoOption {
    /// Repo to inspect, default: the current directory
    #[arg(long = "repo-path", global = true, value_name = "PATH")]
    pub repo_path: Option<String>,
}

/// Where and whose: the two filters that narrow anything deplyd reports on.
#[derive(Debug, Args, Clone, Default)]
pub struct FilterOptions {
    #[command(flatten)]
    pub repo: RepoOption,

    /// Environment, or a prefix of one: -E prod, -E stag
    #[arg(short = 'E', long = "environment", global = true, value_name = "ENV")]
    pub environment: Option<String>,

    /// Author to filter on, default: git config user.name
    #[arg(short = 'A', long = "author", global = true, value_name = "NAME")]
    pub author: Option<String>,

    /// Every author, not just yours
    #[arg(long = "anyone", global = true, conflicts_with = "author")]
    pub anyone: bool,
}

/// The filters, plus how much history to read and how much of it to show.
#[derive(Debug, Args, Clone)]
pub struct ReportOptions {
    #[command(flatten)]
    pub filters: FilterOptions,

    /// How far back to read per target, default 200
    #[arg(short = 'D', long = "depth", global = true, value_name = "N")]
    pub depth: Option<u32>,

    /// How many changes to list
    #[arg(
        short = 'T',
        long = "take",
        global = true,
        default_value_t = DEFAULT_TAKE,
        value_parser = clap::value_parser!(u32).range(1..=1000),
        value_name = "N"
    )]
    pub take: u32,

    /// Skip this many, for paging
    #[arg(
        short = 'S',
        long = "skip",
        global = true,
        default_value_t = 0,
        value_name = "N"
    )]
    pub skip: u32,

    /// Machine-readable output
    #[arg(short = 'J', long = "json", global = true)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// The last deployed commit, and your changes in it
    #[command(infer_subcommands = true)]
    Status {
        /// Narrow it to one pull request or one commit
        #[command(subcommand)]
        change: Option<Change>,

        #[command(flatten)]
        options: ReportOptions,
    },

    /// Watch for deploys, and for changes going live
    #[command(infer_subcommands = true)]
    Watch {
        /// Stop once that pull request or commit is live
        #[command(subcommand)]
        change: Option<Change>,

        /// Stop after this long, e.g. 30m or 2h
        #[arg(long = "for", global = true, value_name = "DURATION")]
        duration: Option<String>,

        /// How often to look, default 60s
        #[arg(long = "every", global = true, value_name = "DURATION")]
        every: Option<String>,

        #[command(flatten)]
        options: ReportOptions,
    },

    /// Values that the -E and -A filters accept
    #[command(infer_subcommands = true, arg_required_else_help = false)]
    List {
        #[command(subcommand)]
        what: ListWhat,

        #[command(flatten)]
        repo: RepoOption,
    },

    /// What detection concluded about this repo
    #[command(infer_subcommands = true)]
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,

        #[command(flatten)]
        filters: FilterOptions,
    },

    /// Keep a default author, environment or repo
    Remember {
        /// author, environment or repo
        what: Option<String>,
        /// The value to remember
        value: Option<String>,
    },

    /// Prove it can only read
    Check,

    /// Whether a newer deplyd is out, and how to get it
    Update,

    /// How to remove deplyd from this machine
    Uninstall,

    /// Shell completion scripts
    Completions {
        /// bash, zsh, fish, powershell or elvish
        shell: clap_complete::Shell,
    },

    /// Bare names for the shell to complete against.
    ///
    /// Hidden: the shell calls it, people do not.
    #[command(hide = true)]
    Complete {
        /// environments or authors
        what: Option<String>,

        #[command(flatten)]
        repo: RepoOption,
    },
}

/// The one change being asked about. `status` reports on it and `watch` waits for
/// it, and either way it is a pull request or a commit, never both.
#[derive(Debug, Subcommand, Clone)]
pub enum Change {
    /// One pull request
    Pr {
        /// The pull request number
        number: Option<String>,
    },

    /// One commit: a sha, branch, tag or HEAD
    Commit {
        /// A sha, branch, tag or HEAD
        reference: Option<String>,
    },
}

#[derive(Debug, Subcommand, Clone)]
pub enum ListWhat {
    /// Names that -A accepts
    Authors,

    /// Environments that -E accepts
    Environments,
}

#[derive(Debug, Subcommand, Clone)]
pub enum ConfigAction {
    /// Write that conclusion to a settings file, to correct by hand
    Init {
        /// Rewrite a .deplyd.json that is already there
        #[arg(short = 'F', long = "force")]
        force: bool,
    },
}

/// Every option, gathered from whichever verb carried it. A verb that does not
/// offer one gets its default, which is what not being asked for means anyway.
#[derive(Debug, Clone)]
pub struct Options {
    pub environment: Option<String>,
    pub author: Option<String>,
    pub anyone: bool,
    pub depth: Option<u32>,
    pub take: u32,
    pub skip: u32,
    pub repo_path: Option<String>,
    pub json: bool,
    pub force: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            environment: None,
            author: None,
            anyone: false,
            depth: None,
            take: DEFAULT_TAKE,
            skip: 0,
            repo_path: None,
            json: false,
            force: false,
        }
    }
}

impl Options {
    fn from_filters(filters: &FilterOptions) -> Self {
        Self {
            environment: filters.environment.clone(),
            author: filters.author.clone(),
            anyone: filters.anyone,
            repo_path: filters.repo.repo_path.clone(),
            ..Self::default()
        }
    }

    fn from_report(report: &ReportOptions) -> Self {
        Self {
            depth: report.depth,
            take: report.take,
            skip: report.skip,
            json: report.json,
            ..Self::from_filters(&report.filters)
        }
    }
}

impl Command {
    pub fn options(&self) -> Options {
        match self {
            Command::Status { options, .. } | Command::Watch { options, .. } => {
                Options::from_report(options)
            }
            Command::Config { action, filters } => Options {
                force: matches!(action, Some(ConfigAction::Init { force: true })),
                ..Options::from_filters(filters)
            },
            Command::List { repo, .. } | Command::Complete { repo, .. } => Options {
                repo_path: repo.repo_path.clone(),
                ..Options::default()
            },
            _ => Options::default(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Command::Status { change, .. } => match change {
                None => "status",
                Some(Change::Pr { .. }) => "status pr",
                Some(Change::Commit { .. }) => "status commit",
            },
            Command::Watch { change, .. } => match change {
                None => "watch",
                Some(Change::Pr { .. }) => "watch pr",
                Some(Change::Commit { .. }) => "watch commit",
            },
            Command::List { what, .. } => match what {
                ListWhat::Authors => "list authors",
                ListWhat::Environments => "list environments",
            },
            Command::Config { action, .. } => match action {
                None => "config",
                Some(ConfigAction::Init { .. }) => "config init",
            },
            Command::Remember { .. } => "remember",
            Command::Check => "check",
            Command::Update => "update",
            Command::Uninstall => "uninstall",
            Command::Completions { .. } => "completions",
            Command::Complete { .. } => "complete",
        }
    }
}

/// Reads a pull request number, accepting the `#412` spelling people paste.
pub fn read_pull_request_number(value: Option<&String>) -> Result<u32, String> {
    let Some(text) = value.map(|v| v.trim()) else {
        return Err("Which pull request?".into());
    };
    if text.is_empty() {
        return Err("Which pull request?".into());
    }

    let digits = text.trim_start_matches('#');
    match digits.parse::<u32>() {
        Ok(number) if number > 0 => Ok(number),
        _ => Err(format!("Not a pull request number: {text}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn a_pull_request_number_may_be_pasted_with_its_hash() {
        assert_eq!(read_pull_request_number(Some(&"412".to_string())), Ok(412));
        assert_eq!(read_pull_request_number(Some(&"#412".to_string())), Ok(412));
    }

    #[test]
    fn anything_that_is_not_a_number_is_named_before_gh_is_looked_for() {
        assert!(read_pull_request_number(Some(&"abc".to_string())).is_err());
        assert!(read_pull_request_number(Some(&"0".to_string())).is_err());
        assert!(read_pull_request_number(Some(&"-3".to_string())).is_err());
        assert!(read_pull_request_number(None).is_err());
    }

    #[test]
    fn an_option_is_only_offered_by_the_verbs_that_act_on_it() {
        // --json reaches a verdict, so only the two verbs that reach one take it.
        assert!(Cli::try_parse_from(["deplyd", "status", "--json"]).is_ok());
        assert!(Cli::try_parse_from(["deplyd", "watch", "--json"]).is_ok());
        assert!(Cli::try_parse_from(["deplyd", "config", "--json"]).is_err());
        assert!(Cli::try_parse_from(["deplyd", "list", "authors", "--json"]).is_err());

        // --force only rewrites the file that config init writes.
        assert!(Cli::try_parse_from(["deplyd", "config", "init", "--force"]).is_ok());
        assert!(Cli::try_parse_from(["deplyd", "config", "--force"]).is_err());
        assert!(Cli::try_parse_from(["deplyd", "status", "--force"]).is_err());

        // Paging belongs to a listing, not to a question about one repo.
        assert!(Cli::try_parse_from(["deplyd", "status", "-T", "5"]).is_ok());
        assert!(Cli::try_parse_from(["deplyd", "config", "-T", "5"]).is_err());
    }

    #[test]
    fn the_gathered_options_carry_what_the_verb_was_given() {
        let parsed = Cli::try_parse_from(["deplyd", "status", "-E", "prod", "-T", "3", "-J"])
            .expect("documented shape");
        let options = parsed.command.options();
        assert_eq!(options.environment.as_deref(), Some("prod"));
        assert_eq!(options.take, 3);
        assert!(options.json);
        assert!(!options.force);

        // A verb that never offers an option still reports its default.
        let listing = Cli::try_parse_from(["deplyd", "list", "authors"]).expect("list authors");
        let options = listing.command.options();
        assert_eq!(options.take, DEFAULT_TAKE);
        assert!(!options.json);
    }

    #[test]
    fn the_command_line_parses_the_documented_shapes() {
        let parsed =
            Cli::try_parse_from(["deplyd", "status", "pr", "412", "-E", "production", "-J"])
                .expect("documented shape");
        assert_eq!(parsed.command.name(), "status pr");

        // watch takes its how-often flags on either side of the change.
        let watching = Cli::try_parse_from(["deplyd", "watch", "pr", "412", "--for", "2h"])
            .expect("documented shape");
        let Command::Watch {
            change: Some(Change::Pr { number }),
            duration,
            ..
        } = watching.command
        else {
            panic!("watch pr should parse");
        };
        assert_eq!(number.as_deref(), Some("412"));
        assert_eq!(duration.as_deref(), Some("2h"));

        // Commands shorten while they stay unambiguous, at both levels.
        let short = Cli::try_parse_from(["deplyd", "li", "env"]).expect("list env should infer");
        assert_eq!(short.command.name(), "list environments");

        // The old flat spellings are gone rather than quietly still working.
        assert!(Cli::try_parse_from(["deplyd", "pr", "412"]).is_err());
        assert!(Cli::try_parse_from(["deplyd", "init"]).is_err());

        // And a bare command line is the same missing-subcommand error as a
        // bare `list`, rather than a help page of its own.
        let bare = Cli::try_parse_from(["deplyd"]).expect_err("a verb is required");
        assert_eq!(bare.kind(), clap::error::ErrorKind::MissingSubcommand);
        assert_eq!(
            Cli::try_parse_from(["deplyd", "list"])
                .expect_err("list needs one")
                .kind(),
            bare.kind(),
            "both should be the same kind of mistake"
        );

        // And a range is enforced rather than accepted and ignored.
        assert!(Cli::try_parse_from(["deplyd", "status", "-T", "0"]).is_err());
        assert!(Cli::try_parse_from(["deplyd", "status", "-T", "2000"]).is_err());
    }
}
