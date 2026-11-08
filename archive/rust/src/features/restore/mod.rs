//! Owns restore command parsing, plan review, and confirmed recovery execution.

mod execution;
mod plan;
mod types;

use crate::command_text;
use crate::is_root;
use crate::user_home;
use crate::Path;
use crate::PathBuf;
use std::env;
use std::io;
use std::io::Write;

#[cfg(test)]
pub(crate) use plan::{classify_restore_archive, find_restore_archive};
pub(crate) use types::{RestoreArchiveKind, RestorePlan};

use execution::{cmd_restore_finish, run_restore_plan};
use plan::{
    print_restore_plan, restore_archive_roots, restore_plan_for_source, validate_restore_targets,
};

pub(crate) fn cmd_restore(root: &Path, args: &[String]) -> io::Result<u8> {
    match args.first().map(String::as_str) {
        Some("check") => {
            let source = restore_source_arg(&args[1..])?;
            let plan = restore_plan_for_source(root, &source)?;
            print_restore_plan(&plan, true)?;
            Ok(0)
        }
        Some("finish") => cmd_restore_finish(root, &args[1..]),
        Some("-h") | Some("--help") => {
            print_restore_help();
            Ok(0)
        }
        Some(_) | None => {
            let source = restore_source_arg(args)?;
            let plan = restore_plan_for_source(root, &source)?;
            print_restore_plan(&plan, false)?;
            run_restore_plan(&plan)
        }
    }
}

fn restore_source_arg(args: &[String]) -> io::Result<PathBuf> {
    if args.len() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: plugroot restore [check] [directory-or-archive]",
        ));
    }
    if let Some(value) = args.first() {
        if value.starts_with('-') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "restore uses words, not flags: try `plugroot restore check /path/to/usb`",
            ));
        }
        return Ok(PathBuf::from(value));
    }
    Ok(default_restore_source_dir())
}

fn print_restore_help() {
    eprintln!(
        "usage: plugroot restore check [directory-or-archive]\n       plugroot restore [directory-or-archive]\n       plugroot restore finish"
    );
}

fn confirm_phrase(expected: &str, prompt: &str) -> io::Result<bool> {
    eprintln!("{prompt}");
    eprint!("> ");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim() == expected)
}

fn default_restore_source_dir() -> PathBuf {
    current_operator_home()
        .map(|home| PathBuf::from(home).join("Documents/plugroot-emergency-exports"))
        .or_else(|| {
            env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join("plugroot-emergency-exports"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp/plugroot-emergency-exports"))
}

fn current_operator_home() -> Option<String> {
    current_operator_user()
        .and_then(|user| user_home(&user))
        .or_else(|| env::var("HOME").ok())
        .filter(|home| !home.trim().is_empty())
}

fn current_operator_user() -> Option<String> {
    env::var("SUDO_USER")
        .ok()
        .filter(|user| user != "root" && !user.trim().is_empty())
        .or_else(|| {
            (!is_root())
                .then(|| command_text(&["id", "-un"]))
                .flatten()
                .filter(|user| user != "root" && !user.trim().is_empty())
        })
        .or_else(|| {
            env::var("USER")
                .ok()
                .filter(|user| user != "root" && !user.trim().is_empty())
        })
}
