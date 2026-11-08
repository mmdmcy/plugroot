//! External command and host-service protocols.

pub(crate) mod git;
pub(crate) mod process;
pub(crate) mod systemd;

pub(crate) use git::git_dirty_details;
pub(crate) use process::{
    command_text, ensure_success, print_cmd_output, run_command, run_command_with_env_values,
    run_prefixed_command, CmdOutput,
};
pub(crate) use systemd::{is_root, user_home, user_systemd_prefix};
