//! The git half of the gateway.
//!
//! The dangerous verbs do not exist, so there is nothing to refuse. What still needs
//! refusing are arguments: `config` writes when given two operands, `fetch` deletes
//! when given `--prune`.

use std::ffi::OsStr;
use std::path::Path;

use super::Denied;

/// Every git subcommand deplyd may run. Widening it takes a diff, not a string that
/// happened to pass a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    RevParse,
    RevList,
    Log,
    Show,
    MergeBase,
    Shortlog,
    CatFile,
    Diff,
    Status,
    Cherry,
    LsFiles,
    /// Reading only; the assigning forms are refused below.
    Config,
    /// The one subcommand that writes, and only to your own remote-tracking refs.
    Fetch,
}

impl Verb {
    /// Every variant, for the self-check. Kept honest by `position` below, by
    /// `VERB_COUNT`, and by the self-check's own frozen list - four places that
    /// disagree until a change is deliberate.
    pub const ALL: &'static [Verb] = &[
        Verb::RevParse,
        Verb::RevList,
        Verb::Log,
        Verb::Show,
        Verb::MergeBase,
        Verb::Shortlog,
        Verb::CatFile,
        Verb::Diff,
        Verb::Status,
        Verb::Cherry,
        Verb::LsFiles,
        Verb::Config,
        Verb::Fetch,
    ];

    /// Where a verb sits in [`Verb::ALL`]. Exists for its exhaustiveness: a new
    /// variant is a compile error here, which a hand-written array cannot manage.
    const fn position(self) -> usize {
        match self {
            Verb::RevParse => 0,
            Verb::RevList => 1,
            Verb::Log => 2,
            Verb::Show => 3,
            Verb::MergeBase => 4,
            Verb::Shortlog => 5,
            Verb::CatFile => 6,
            Verb::Diff => 7,
            Verb::Status => 8,
            Verb::Cherry => 9,
            Verb::LsFiles => 10,
            Verb::Config => 11,
            Verb::Fetch => 12,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Verb::RevParse => "rev-parse",
            Verb::RevList => "rev-list",
            Verb::Log => "log",
            Verb::Show => "show",
            Verb::MergeBase => "merge-base",
            Verb::Shortlog => "shortlog",
            Verb::CatFile => "cat-file",
            Verb::Diff => "diff",
            Verb::Status => "status",
            Verb::Cherry => "cherry",
            Verb::LsFiles => "ls-files",
            Verb::Config => "config",
            Verb::Fetch => "fetch",
        }
    }
}

/// Asserted against [`Verb::ALL`] below, so the array and `position` cannot drift.
const VERB_COUNT: usize = 13;

const _: () = {
    assert!(
        Verb::ALL.len() == VERB_COUNT,
        "Verb::ALL and VERB_COUNT disagree: a verb was added or removed without          updating both, and the self-check would compare a stale set."
    );
    // Each verb occupies its own slot; a clash is caught here, not by a reader.
    let mut index = 0;
    while index < VERB_COUNT {
        assert!(
            Verb::ALL[index].position() == index,
            "Verb::ALL is not in the order `position` declares."
        );
        index += 1;
    }
};

/// Options that change what git *is* rather than what it does. `-c alias.x=!sh`
/// reaches a shell; `--exec-path` moves where git finds its helpers. The verb says
/// nothing about them, so they are refused by name.
const FORBIDDEN_ANYWHERE: &[&str] = &[
    "-c",
    "--config-env",
    "--exec-path",
    "--upload-pack",
    "--receive-pack",
    "--work-tree",
    "--git-dir",
    "--namespace",
    "--output",
    "--ext-diff",
];

/// Argument forms that would write, checked per verb.
fn write_would_occur(verb: Verb, args: &[&str]) -> Option<String> {
    let positional: Vec<&str> = args
        .iter()
        .copied()
        .filter(|a| !a.starts_with('-'))
        .collect();

    match verb {
        Verb::Config => {
            // A second operand is the value being assigned.
            if positional.len() > 1 {
                return Some("git config with a value would write it".into());
            }
            const CONFIG_WRITES: &[&str] = &[
                "--add",
                "--unset",
                "--unset-all",
                "--replace-all",
                "--edit",
                "-e",
                "--rename-section",
                "--remove-section",
                "--set",
            ];
            for arg in args {
                if CONFIG_WRITES.contains(arg) {
                    return Some(format!("git config {arg} writes"));
                }
            }
            None
        }
        Verb::Fetch => {
            // Allowed by exact spelling: a refspec overwrites local branches and
            // --prune deletes them, and naming the three deplyd uses is easier than
            // enumerating every harmful form.
            const FETCH_ALLOWED: &[&str] = &["origin", "--quiet", "-q", "--no-tags"];
            for arg in args {
                if !FETCH_ALLOWED.contains(arg) {
                    return Some(format!(
                        "git fetch takes only {}; '{arg}' could rewrite local refs",
                        FETCH_ALLOWED.join(", ")
                    ));
                }
            }
            None
        }
        // No catch-all: a new verb does not compile until classified here.
        Verb::RevParse
        | Verb::RevList
        | Verb::Log
        | Verb::Show
        | Verb::MergeBase
        | Verb::Shortlog
        | Verb::CatFile
        | Verb::Diff
        | Verb::Status
        | Verb::Cherry
        | Verb::LsFiles => None,
    }
}

/// A git invocation proven read-only. The only way to run git, and fallible.
#[derive(Debug)]
pub struct ReadOnlyGit {
    verb: Verb,
    args: Vec<String>,
}

impl ReadOnlyGit {
    /// Refuses rather than panics, naming the argument and why.
    pub fn new(verb: Verb, args: &[&str]) -> Result<Self, Denied> {
        // A verb missing from Verb::ALL is invisible to the self-check, so it cannot
        // run at all. Nothing legitimate reaches this; omission fails closed.
        if !Verb::ALL.contains(&verb) {
            return Err(Denied::new(
                format!("git {}", verb.as_str()),
                "this verb is not in the audited set and cannot be run",
            ));
        }

        for arg in args {
            let name = arg.split('=').next().unwrap_or(arg);
            if FORBIDDEN_ANYWHERE.contains(&name) {
                return Err(Denied::new(
                    format!("git {} {}", verb.as_str(), args.join(" ")),
                    format!("'{name}' can make git run something this gateway cannot see"),
                ));
            }
        }

        if let Some(reason) = write_would_occur(verb, args) {
            return Err(Denied::new(
                format!("git {} {}", verb.as_str(), args.join(" ")),
                reason,
            ));
        }

        Ok(Self {
            verb,
            args: args.iter().map(|a| (*a).to_string()).collect(),
        })
    }

    pub fn verb(&self) -> Verb {
        self.verb
    }

    /// What would be run, for refusal messages and for tests that never spawn.
    pub fn rendered(&self) -> String {
        if self.args.is_empty() {
            format!("git {}", self.verb.as_str())
        } else {
            format!("git {} {}", self.verb.as_str(), self.args.join(" "))
        }
    }

    pub fn argv(&self) -> Vec<&OsStr> {
        let mut argv: Vec<&OsStr> = vec![OsStr::new(self.verb.as_str())];
        argv.extend(self.args.iter().map(OsStr::new));
        argv
    }

    /// The single place in deplyd that starts a git process.
    ///
    /// The environment is pinned: the pager and external-diff hooks both name a
    /// program to run and can be set by a repository's own config.
    #[allow(clippy::disallowed_types)] // the gateway is where spawning is allowed
    pub fn run(&self, cwd: &Path) -> std::io::Result<std::process::Output> {
        std::process::Command::new("git")
            .args(self.argv())
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("GIT_EXTERNAL_DIFF", "")
            .env("GIT_ASKPASS", "")
            .output()
    }
}
