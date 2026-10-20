use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{self, Stdout};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod app;
mod features;
mod integrations;
mod interfaces;
mod security;
mod shared;

use app::{
    load_env_file, load_manifest_file, merge_manifest, Context, ManagedDir, ManagedFile,
    ManagedSymlink, Manifest, Repo, Service,
};
use features::{
    absolute_clean_path, broad_ufw_allow_rules, cmd_action, cmd_apply, cmd_backup, cmd_doctor,
    cmd_emergency, cmd_list, cmd_release, cmd_release_check, cmd_repos, cmd_restore, cmd_status,
    emergency_metadata_commands, emergency_timestamp, expected_container_names,
    expected_manifest_tcp_ports, has_control, limit_lines, logs_for, meminfo_kb,
    print_status_table, program_available, run_fixed_action, run_read_command, status_rows,
    tailscale_text_has_public_funnel, unexpected_wildcard_tcp_listeners, DoctorFinding,
    EmergencyCommandSpec, EmergencyExportManifest, EmergencySource, StatusRow,
};
use integrations::{
    command_text, ensure_success, git_dirty_details, is_root, print_cmd_output, run_command,
    run_command_with_env_values, run_prefixed_command, user_home, user_systemd_prefix, CmdOutput,
};
use interfaces::{cmd_tui, cmd_web, print_help, trim_output, tui_content_width};
use security::{
    audit_public, boundary_findings, cmd_audit_public, cmd_boundary, AuditFinding, BoundaryFinding,
    BoundarySeverity,
};
use shared::{
    clean_path, default_state_root, nearest_git_root, resolve_path, same_or_inside,
    state_root_for_manifest,
};

pub fn run() -> io::Result<u8> {
    let (root, args) = parse_global_args(env::args().skip(1).collect())?;
    let command = args.first().map(String::as_str).unwrap_or("help");

    if matches!(command, "help" | "-h" | "--help") {
        print_help();
        return Ok(0);
    }

    if command == "audit-public" {
        return cmd_audit_public(&root, &args[1..]);
    }
    if command == "restore" {
        return cmd_restore(&root, &args[1..]);
    }

    let ctx = Context::load(root)?;
    match command {
        "boundary" => cmd_boundary(&ctx, &args[1..]),
        "status" => cmd_status(&ctx, &args[1..]),
        "apply" => cmd_apply(&ctx, &args[1..]),
        "repos" => cmd_repos(&ctx, &args[1..]),
        "doctor" => cmd_doctor(&ctx, &args[1..]),
        "release-check" => cmd_release_check(&ctx),
        "release" => cmd_release(&ctx, &args[1..]),
        "backup" => cmd_backup(&ctx, &args[1..]),
        "emergency" => cmd_emergency(&ctx, &args[1..]),
        "up" => cmd_action(&ctx, "start", &args[1..]),
        "down" => cmd_action(&ctx, "stop", &args[1..]),
        "restart" => cmd_action(&ctx, "restart", &args[1..]),
        "logs" => cmd_action(&ctx, "logs", &args[1..]),
        "tui" => cmd_tui(ctx, &args[1..]),
        "web" => cmd_web(ctx, &args[1..]),
        "list" => cmd_list(&ctx),
        _ => {
            eprintln!("unknown command: {command}");
            print_help();
            Ok(2)
        }
    }
}

fn parse_global_args(args: Vec<String>) -> io::Result<(PathBuf, Vec<String>)> {
    let mut root = None;
    let mut rest = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--root" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--root requires a path",
                    ));
                };
                root = Some(PathBuf::from(value));
                index += 2;
            }
            value if value.starts_with("--root=") => {
                root = Some(PathBuf::from(value.trim_start_matches("--root=")));
                index += 1;
            }
            _ => {
                rest.push(args[index].clone());
                index += 1;
            }
        }
    }

    let root = root.unwrap_or(env::current_dir()?);
    Ok((root, rest))
}

#[cfg(test)]
mod tests;
