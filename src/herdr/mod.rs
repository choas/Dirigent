//! Herdr backend: Dirigent runs native, interactive Claude Code inside a pane
//! of herdr (a terminal multiplexer for coding agents), where the user can
//! watch and type into it, and collects the result — diff, transcript and
//! metrics — once Claude's Stop hook fires.
//!
//! See `docs/How_to_Herdr.md` for setup and limitations, and
//! `docs/Dirigent_Herdr_Integration_Concept.md` for the wider direction.

mod client;
mod command;
mod pane;
mod run;
mod transcript;

pub(crate) use client::check_server;
pub(crate) use run::run_herdr_cli;
