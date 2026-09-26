//! The command line. A verb says what to do, a flag says how.

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "deplyd",
    about = "Which commit an environment was last deployed from",
    long_about = None,
    version,
    infer_subcommands = true,
    disable_help_subcommand = true,
    subcommand_required = false,
    arg_required_else_help = false
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub options: Options,
}

#[derive(Debug, Args, Clone)]
pub struct Options {
    /// Environment, or a prefix of one: -E prod, -E stag
    #[arg(short = 'E', long = "environment", global = true, value_name = "ENV")]
    pub environment: Option<String>,

    /// Author to filter on, default: git config user.name
    #[arg(short = 'A', long = "author", global = true, value_name = "NAME")]
    pub author: Option<String>,

    /// How many changes to list
    #[arg(
        short = 'T',
        long = "take",
        global = true,
        default_value_t = 10,
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

    /// Repo to inspect, default: the current directory
    #[arg(long = "repo-path", global = true, value_name = "PATH")]
    pub repo_path: Option<String>,

    /// Machine-readable output, for status and pr
    #[arg(short = 'J', long = "json", global = true)]
    pub json: bool,

    /// Let init rewrite an existing .deplyd.json
    #[arg(short = 'F', long = "force", global = true)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// The last deployed commit, and your changes in it
    Status,

    /// Whether one pull request is live
    Pr {
        /// The pull request number
        number: Option<String>,
    },

    /// Whether one commit is deployd
    Commit {
        /// A sha, branch, tag or HEAD
        reference: Option<String>,
    },

    /// Names that -A accepts
    Authors,

    /// Environments that -E accepts
    Environments,

    /// What detection concluded about this repo
    Config,

    /// Write that conclusion to a settings file, to correct by hand
    Init,

    /// Keep a default author, environment or repo
    Remember {
        /// author, environment or repo
        what: Option<String>,
        /// The value to remember
        value: Option<String>,
    },

    /// Prove it can only read
    Check,

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
    },
}

impl Command {
    /// The commands that reach a verdict, which are the only ones `--json` suits.
    pub fn supports_json(&self) -> bool {
        matches!(
            self,
            Command::Status | Command::Pr { .. } | Command::Commit { .. }
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Command::Status => "status",
            Command::Pr { .. } => "pr",
            Command::Commit { .. } => "commit",
            Command::Authors => "authors",
            Command::Environments => "environments",
            Command::Config => "config",
            Command::Init => "init",
            Command::Remember { .. } => "remember",
            Command::Check => "check",
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
    fn json_is_offered_only_where_there_is_a_verdict() {
        assert!(Command::Status.supports_json());
        assert!(Command::Pr { number: None }.supports_json());
        assert!(Command::Commit { reference: None }.supports_json());
        assert!(!Command::Config.supports_json());
        assert!(!Command::Check.supports_json());
    }

    #[test]
    fn the_command_line_parses_the_documented_shapes() {
        use clap::Parser;

        let parsed = Cli::try_parse_from(["deplyd", "pr", "412", "-E", "production", "-J"])
            .expect("documented shape");
        assert!(parsed.options.json);
        assert_eq!(parsed.options.environment.as_deref(), Some("production"));

        // Commands shorten while they stay unambiguous.
        let short = Cli::try_parse_from(["deplyd", "env"]).expect("env should infer");
        assert_eq!(short.command.map(|c| c.name()), Some("environments"));

        // And a range is enforced rather than accepted and ignored.
        assert!(Cli::try_parse_from(["deplyd", "status", "-T", "0"]).is_err());
        assert!(Cli::try_parse_from(["deplyd", "status", "-T", "2000"]).is_err());
    }
}
