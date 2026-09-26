//! The only place in deplyd that starts a process or opens a connection.
//!
//! Four layers: dangerous operations are unrepresentable ([`git::Verb`] has no
//! `Push`, the HTTP client takes no method), writing arguments are refused before
//! anything runs, `clippy.toml` keeps `Command` inside this crate, and `reqwest` is
//! listed here and nowhere else.
//!
//! [`selfcheck`] runs the refusal paths compiled into the binary, since a binary
//! cannot audit the source it came from.

pub mod credential;
pub mod git;
pub mod http;
pub mod selfcheck;

use std::fmt;

/// A call the gateway refused, carrying enough to explain itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    /// What was attempted, rendered for a person.
    pub attempted: String,
    /// Why it was refused.
    pub reason: String,
}

impl Denied {
    pub fn new(attempted: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            attempted: attempted.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for Denied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Refused: deplyd only performs read operations.\n  Blocked: {}\n  {}",
            self.attempted, self.reason
        )
    }
}

impl std::error::Error for Denied {}
