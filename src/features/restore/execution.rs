use crate::absolute_clean_path;
use crate::emergency_timestamp;
use crate::ensure_success;
use crate::is_root;
use crate::print_cmd_output;
use crate::program_available;
use crate::run_command;
use crate::Path;
use std::env;
use std::fs;
use std::io;

use super::{
    confirm_phrase, print_restore_help, restore_archive_roots, validate_restore_targets,
    RestoreArchiveKind, RestorePlan,
};

pub(super) fn run_restore_plan(plan: &RestorePlan) -> io::Result<u8> {
    if !is_root() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "restore requires root; the installed plugroot launcher will use sudo automatically",
        ));
    }
    validate_restore_targets(plan)?;
    validate_restore_tools(plan.kind)?;
    ensure_restore_targets_clear(plan)?;
    if !confirm_phrase(
        "RESTORE",
        "Type RESTORE to extract Plugroot roots onto this host",
    )? {
        eprintln!("restore cancelled");
        return Ok(2);
    }

    let mut temp_tar = None;
    let tar_path = match plan.kind {
        RestoreArchiveKind::Encrypted => {
            let path = env::temp_dir().join(format!(
                "plugroot-restore-{}.tar.zst",
                emergency_timestamp()
            ));
            decrypt_restore_archive(&plan.archive, &path)?;
            temp_tar = Some(path);
            temp_tar.as_deref().unwrap()
        }
        RestoreArchiveKind::Plain => plan.archive.as_path(),
    };

    ensure_restore_archive_has_roots(tar_path, plan)?;
    extract_restore_roots(tar_path, plan)?;
    if let Some(path) = temp_tar {
        let _ = fs::remove_file(path);
    }
    create_restore_compat_symlink(&plan.state_root)?;
    println!(
        "restored {} and {}",
        plan.code_root.display(),
        plan.state_root.display()
    );
    println!("next: install any missing host prerequisites, then run `plugroot restore finish`");
    Ok(0)
}

fn validate_restore_tools(kind: RestoreArchiveKind) -> io::Result<()> {
    for program in ["tar", "zstd"] {
        if !program_available(program) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("required program not found in PATH: {program}"),
            ));
        }
    }
    if kind == RestoreArchiveKind::Encrypted && !program_available("gpg") {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "required program not found in PATH: gpg",
        ));
    }
    Ok(())
}

fn ensure_restore_targets_clear(plan: &RestorePlan) -> io::Result<()> {
    for path in [
        &plan.code_root,
        &plan.state_root,
        Path::new("/var/lib/plugroot"),
    ] {
        if path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "restore refuses to overwrite existing path: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

fn decrypt_restore_archive(archive: &Path, tar_path: &Path) -> io::Result<()> {
    let out = run_command(
        "gpg",
        &[
            "--decrypt".into(),
            "--output".into(),
            tar_path.display().to_string(),
            archive.display().to_string(),
        ],
        None,
        &[],
    );
    if out.code == 0 {
        return Ok(());
    }
    let _ = fs::remove_file(tar_path);
    Err(io::Error::other(format!(
        "gpg decrypt failed: {}",
        out.text
    )))
}

fn ensure_restore_archive_has_roots(tar_path: &Path, plan: &RestorePlan) -> io::Result<()> {
    for root in restore_archive_roots(plan)? {
        let out = run_command(
            "tar",
            &[
                "--zstd".into(),
                "-tf".into(),
                tar_path.display().to_string(),
                root.clone(),
            ],
            None,
            &[],
        );
        if out.code != 0 {
            return Err(io::Error::other(format!(
                "archive is missing {root}: {}",
                out.text
            )));
        }
    }
    Ok(())
}

fn extract_restore_roots(tar_path: &Path, plan: &RestorePlan) -> io::Result<()> {
    let roots = restore_archive_roots(plan)?;
    let out = run_command(
        "tar",
        &[
            "--zstd".into(),
            "-xpf".into(),
            tar_path.display().to_string(),
            "-C".into(),
            "/".into(),
            roots[0].clone(),
            roots[1].clone(),
        ],
        None,
        &[],
    );
    if out.code == 0 {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "tar extract failed: {}",
        out.text
    )))
}

fn create_restore_compat_symlink(state_root: &Path) -> io::Result<()> {
    let link = Path::new("/var/lib/plugroot");
    if link.exists() {
        return Ok(());
    }
    std::os::unix::fs::symlink(state_root, link)
}

pub(super) fn cmd_restore_finish(root: &Path, args: &[String]) -> io::Result<u8> {
    if !args.is_empty() {
        print_restore_help();
        return Ok(2);
    }
    if !is_root() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "restore finish requires root; the installed plugroot launcher will use sudo automatically",
        ));
    }
    let root = absolute_clean_path(root)?;
    let bin = root.join("bin/plugroot");
    if !bin.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("plugroot binary not found: {}", bin.display()),
        ));
    }
    println!("checking restored Plugroot roots");
    ensure_success(run_command(
        &bin.display().to_string(),
        &[
            "--root".into(),
            root.display().to_string(),
            "boundary".into(),
            "--strict".into(),
        ],
        None,
        &[],
    ))?;
    println!("previewing apply");
    print_cmd_output(&run_command(
        &bin.display().to_string(),
        &[
            "--root".into(),
            root.display().to_string(),
            "apply".into(),
            "--dry-run".into(),
        ],
        None,
        &[],
    ));
    if !confirm_phrase("APPLY", "Type APPLY to install units and start services")? {
        eprintln!("restore finish cancelled");
        return Ok(2);
    }
    ensure_success(run_command(
        &bin.display().to_string(),
        &["--root".into(), root.display().to_string(), "apply".into()],
        None,
        &[],
    ))?;
    ensure_success(run_command(
        &bin.display().to_string(),
        &[
            "--root".into(),
            root.display().to_string(),
            "up".into(),
            "all".into(),
        ],
        None,
        &[],
    ))?;
    let doctor = run_command(
        &bin.display().to_string(),
        &["--root".into(), root.display().to_string(), "doctor".into()],
        None,
        &[],
    );
    print_cmd_output(&doctor);
    Ok(doctor.code.min(255) as u8)
}
