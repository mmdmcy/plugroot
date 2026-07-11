use crate::clean_path;
use crate::command_text;
use crate::emergency_metadata_commands;
use crate::run_command;
use crate::Command;
use crate::Context;
use crate::EmergencyCommandSpec;
use crate::EmergencyExportManifest;
use crate::EmergencySource;
use crate::Path;
use crate::PathBuf;
use crate::SystemTime;
use crate::UNIX_EPOCH;
use serde::Serialize;
use std::env;
use std::fs;
use std::io;

pub(crate) fn write_emergency_metadata(
    dir: &Path,
    manifest: &EmergencyExportManifest,
) -> io::Result<()> {
    write_json_file(&dir.join("manifest.json"), manifest)?;
    fs::write(
        dir.join("README.txt"),
        "Plugroot emergency export metadata. The companion .manifest.json next to the encrypted archive is intentionally non-secret; this directory is inside the encrypted archive and may contain private host inventory.\n",
    )?;
    for spec in emergency_metadata_commands() {
        let output = run_command(
            spec.program,
            &spec
                .args
                .iter()
                .map(|arg| arg.to_string())
                .collect::<Vec<_>>(),
            None,
            &[],
        );
        let text = format!(
            "$ {}\nexit: {}\n\n{}\n",
            command_line_for_spec(&spec),
            output.code,
            output.text
        );
        fs::write(dir.join(spec.file_name), text)?;
    }
    Ok(())
}

pub(crate) fn write_json_file(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let json = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    fs::write(path, format!("{json}\n"))
}

pub(crate) fn create_emergency_tar(
    ctx: &Context,
    sources: &[EmergencySource],
    metadata_dir: &Path,
    tar_path: &Path,
) -> io::Result<()> {
    let mut args = vec![
        "--zstd".into(),
        "-cf".into(),
        tar_path.display().to_string(),
    ];
    for exclude in emergency_tar_excludes(ctx) {
        args.push(format!("--exclude={exclude}"));
    }
    args.extend(["-C".into(), "/".into()]);
    for source in sources {
        if let Some(path) = tar_relative_path(&source.path) {
            args.push(path);
        }
    }
    let metadata_parent = metadata_dir.parent().unwrap_or_else(|| Path::new("/"));
    let metadata_name = metadata_dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid metadata path"))?;
    args.extend([
        "-C".into(),
        metadata_parent.display().to_string(),
        metadata_name,
    ]);

    let out = run_command("tar", &args, None, &[]);
    if out.code == 0 {
        return Ok(());
    }
    Err(io::Error::other(format!("tar failed: {}", out.text)))
}

pub(crate) fn encrypt_emergency_tar(
    tar_path: &Path,
    encrypted_path: &Path,
    passphrase_file: Option<&PathBuf>,
) -> io::Result<()> {
    let mut args = Vec::new();
    if let Some(passphrase_file) = passphrase_file {
        args.extend([
            "--batch".into(),
            "--yes".into(),
            "--pinentry-mode".into(),
            "loopback".into(),
            "--passphrase-file".into(),
            passphrase_file.display().to_string(),
        ]);
    }
    args.extend([
        "--symmetric".into(),
        "--cipher-algo".into(),
        "AES256".into(),
        "--output".into(),
        encrypted_path.display().to_string(),
        tar_path.display().to_string(),
    ]);
    let out = run_command("gpg", &args, None, &[]);
    if out.code == 0 {
        return Ok(());
    }
    let _ = fs::remove_file(encrypted_path);
    Err(io::Error::other(format!("gpg failed: {}", out.text)))
}

pub(crate) fn print_emergency_export_plan(
    manifest: &EmergencyExportManifest,
    encrypted_path: &Path,
    dry_run: bool,
    passphrase_file: Option<&PathBuf>,
) {
    println!(
        "emergency export {}",
        if dry_run { "dry run" } else { "plan" }
    );
    println!("code root: {}", manifest.code_root);
    println!("state root: {}", manifest.state_root);
    println!("output: {}", encrypted_path.display());
    println!(
        "encryption: gpg symmetric AES256 ({})",
        if passphrase_file.is_some() {
            "passphrase file"
        } else {
            "interactive prompt"
        }
    );
    println!("included paths:");
    for path in &manifest.included_paths {
        println!("  - {path}");
    }
    if !manifest.skipped_paths.is_empty() {
        println!("skipped missing paths:");
        for path in &manifest.skipped_paths {
            println!("  - {path}");
        }
    }
}

pub(crate) fn emergency_tar_excludes(ctx: &Context) -> Vec<String> {
    let code_root = clean_path(&ctx.root);
    let state_root = clean_path(&ctx.state_root());
    let mut excludes = Vec::new();
    for name in ["target", ".cache", "node_modules"] {
        if let Some(relative) = tar_relative_path(&code_root.join(name)) {
            excludes.push(relative);
        }
    }
    if let Some(state_relative) = tar_relative_path(&state_root) {
        for pattern in ["repos/*/target", "repos/*/.cache", "repos/*/node_modules"] {
            excludes.push(format!("{state_relative}/{pattern}"));
        }
    }
    excludes
}

pub(crate) fn tar_relative_path(path: &Path) -> Option<String> {
    let clean = clean_path(path);
    if clean.is_absolute() {
        let text = clean.strip_prefix("/").ok()?.display().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    } else {
        Some(clean.display().to_string())
    }
}

pub(crate) fn absolute_clean_path(path: &Path) -> io::Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    Ok(clean_path(&path))
}

pub(crate) fn emergency_timestamp() -> String {
    command_text(&["date", "-u", "+%Y%m%dT%H%M%SZ"]).unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs().to_string())
            .unwrap_or_else(|_| "unknown-time".into())
    })
}

pub(crate) fn emergency_host_name(ctx: &Context) -> String {
    ctx.manifest
        .host
        .as_ref()
        .and_then(|host| host.name.clone())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| command_text(&["hostname"]))
        .unwrap_or_else(|| "plugroot-host".into())
}

pub(crate) fn sanitize_filename_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "plugroot-host".into()
    } else {
        sanitized
    }
}

pub(crate) fn command_line_for_spec(spec: &EmergencyCommandSpec) -> String {
    let mut parts = vec![spec.program.to_string()];
    parts.extend(spec.args.iter().map(|arg| arg.to_string()));
    parts.join(" ")
}

pub(crate) fn program_available(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}
