//! One module per external cue source.
//!
//! These were a single 1,800-line `sources/external.rs` covering Notion
//! pagination and block-tree rendering, three SonarQube endpoints, Slack,
//! Trello, Asana, Sentry and GitHub — with no tests at all, so every provider
//! change was a blind edit that could only be validated by hand against a live
//! service. Splitting them apart makes each provider's parsing small enough to
//! test directly against captured fixtures (see each module's `tests`).
//!
//! Shared HTTP, token and shell-quoting helpers stay in
//! [`super::external`], which every provider imports.

pub(super) mod asana;
pub(super) mod github;
pub(super) mod notion;
pub(super) mod sentry;
pub(super) mod slack;
pub(super) mod sonarqube;
pub(super) mod trello;
