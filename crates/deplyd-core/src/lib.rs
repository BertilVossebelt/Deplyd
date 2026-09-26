//! deplyd-core - deciding what deployed, and whether a change is in it.
//!
//! This crate reaches the outside world only through `deplyd-gateway`, and cannot
//! print: it has no terminal dependency, so the decide/print split that `lib/` and
//! `commands/` kept by convention is a boundary the compiler checks.

pub use deplyd_gateway as gateway;
pub mod cache;
pub mod context;
pub mod detect;
pub mod github;
pub mod history;
pub mod repo;
pub mod report;
pub mod settings;
pub mod targets;
pub mod verdict;
pub mod yaml;
