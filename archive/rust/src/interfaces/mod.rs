//! User-facing CLI, TUI, and web interfaces.

pub(crate) mod cli;
pub(crate) mod tui;
pub(crate) mod web;

pub(crate) use cli::print_help;
pub(crate) use tui::{cmd_tui, trim_output, tui_content_width};
pub(crate) use web::cmd_web;
