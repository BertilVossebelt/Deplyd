//! deplyd-core - deciding what deployed, and whether a change is in it.
//!
//! Reaches the outside world only through `deplyd-gateway`, and cannot print:
//! the decide/print split is a boundary the compiler checks, not a convention.

pub use deplyd_gateway as gateway;
pub mod cache;
pub mod context;
pub mod detect;
pub mod github;
pub mod history;
pub mod repo;
pub mod report;
pub mod settings;
pub mod startup;
pub mod targets;
pub mod verdict;
pub mod watch;
pub mod watchers;
pub mod when;
pub mod yaml;
