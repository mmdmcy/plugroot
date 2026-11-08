use crate::clean_path;
use crate::command_text;
use crate::ensure_success;
use crate::is_root;
use crate::run_command;
use crate::same_or_inside;
use crate::user_home;
use crate::Context;
use crate::Path;
use crate::PathBuf;
use std::env;
use std::fs;
use std::io;

mod types;

pub(crate) use types::{
    EmergencyCommandSpec, EmergencyExportManifest, EmergencyExportOptions, EmergencySource,
};

pub(crate) fn cmd_backup(ctx: &Context, args: &[String]) -> io::Result<u8> {
    let default_output_dir = default_emergency_output_dir(ctx);
    let Some((output_dir, dry_run)) = parse_simple_backup_args(args, &default_output_dir)? else {
        return Ok(2);
    };
    if !dry_run {
        println!("encrypted backup: GPG will ask for a passphrase");
        println!("keep that passphrase outside this laptop and outside Vaultwarden");
    }
    run_emergency_export(
        ctx,
        EmergencyExportOptions {
            output_dir,
            yes: true,
            dry_run,
            passphrase_file: None,
            create_output_dir: true,
            owner_user: emergency_output_owner_user(ctx),
        },
    )
}

pub(crate) fn cmd_emergency(ctx: &Context, args: &[String]) -> io::Result<u8> {
    match args.first().map(String::as_str) {
        Some("save") => cmd_backup(ctx, &args[1..]),
        Some("check") => {
            let mut backup_args = vec!["check".to_string()];
            backup_args.extend(args[1..].iter().cloned());
            cmd_backup(ctx, &backup_args)
        }
        Some("export") => cmd_emergency_export(ctx, &args[1..]),
        _ => {
            eprintln!(
                "usage: plugroot backup [check] [directory]\n       plugroot emergency save [directory]\n       plugroot emergency export --output <directory> --yes [--dry-run] [--passphrase-file <path>]"
            );
            Ok(2)
        }
    }
}

pub(crate) fn cmd_emergency_export(ctx: &Context, args: &[String]) -> io::Result<u8> {
    let options = match parse_emergency_export_options(args)? {
        Some(options) => options,
        None => return Ok(2),
    };

    run_emergency_export(ctx, options)
}

pub(crate) fn run_emergency_export(
    ctx: &Context,
    options: EmergencyExportOptions,
) -> io::Result<u8> {
    if !options.yes && !options.dry_run {
        eprintln!("refusing to export without --yes; run --dry-run first to inspect the plan");
        return Ok(2);
    }

    let output_dir = absolute_clean_path(&options.output_dir)?;
    validate_emergency_output_boundary(ctx, &output_dir)?;
    if options.create_output_dir {
        fs::create_dir_all(&output_dir)?;
        chown_path_to_user(&output_dir, options.owner_user.as_deref())?;
    }
    validate_emergency_output_dir(&output_dir)?;

    let timestamp = emergency_timestamp();
    let host = emergency_host_name(ctx);
    let archive_stem = format!(
        "plugroot-emergency-{}-{}",
        sanitize_filename_component(&host),
        timestamp
    );
    let tar_path = output_dir.join(format!("{archive_stem}.tar.zst"));
    let encrypted_path = output_dir.join(format!("{archive_stem}.tar.zst.gpg"));
    let companion_manifest_path = output_dir.join(format!("{archive_stem}.manifest.json"));
    let metadata_dir = env::temp_dir().join(format!("{archive_stem}-metadata"));

    let (included_sources, skipped_sources) = emergency_sources(ctx);
    let included_paths = included_sources
        .iter()
        .map(|source| source.path.display().to_string())
        .collect::<Vec<_>>();
    let skipped_paths = skipped_sources
        .iter()
        .map(|source| format!("{} ({})", source.path.display(), source.label))
        .collect::<Vec<_>>();
    let metadata_command_lines = emergency_metadata_commands()
        .iter()
        .map(command_line_for_spec)
        .collect::<Vec<_>>();
    let manifest = EmergencyExportManifest {
        version: 1,
        created_utc: timestamp.clone(),
        code_root: clean_path(&ctx.root).display().to_string(),
        state_root: clean_path(&ctx.state_root()).display().to_string(),
        output_dir: output_dir.display().to_string(),
        encrypted_archive: encrypted_path.display().to_string(),
        included_paths,
        skipped_paths,
        metadata_commands: metadata_command_lines,
    };

    print_emergency_export_plan(
        &manifest,
        &encrypted_path,
        options.dry_run,
        options.passphrase_file.as_ref(),
    );

    if options.dry_run {
        return Ok(0);
    }

    validate_emergency_tools(options.passphrase_file.as_ref())?;

    if tar_path.exists() || encrypted_path.exists() || companion_manifest_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "refusing to overwrite an existing emergency export file",
        ));
    }

    fs::create_dir(&metadata_dir)?;
    let export_result = (|| {
        write_emergency_metadata(&metadata_dir, &manifest)?;
        write_json_file(&companion_manifest_path, &manifest)?;
        create_emergency_tar(ctx, &included_sources, &metadata_dir, &tar_path)?;
        encrypt_emergency_tar(&tar_path, &encrypted_path, options.passphrase_file.as_ref())?;
        chown_path_to_user(&encrypted_path, options.owner_user.as_deref())?;
        chown_path_to_user(&companion_manifest_path, options.owner_user.as_deref())?;
        println!("wrote encrypted export: {}", encrypted_path.display());
        println!(
            "wrote companion manifest: {}",
            companion_manifest_path.display()
        );
        Ok(())
    })();

    let _ = fs::remove_file(&tar_path);
    let _ = fs::remove_dir_all(&metadata_dir);

    export_result.map(|()| 0)
}

pub(crate) fn parse_simple_backup_args(
    args: &[String],
    default_output_dir: &Path,
) -> io::Result<Option<(PathBuf, bool)>> {
    let mut dry_run = false;
    let mut rest = args;
    if let Some(first) = rest.first().map(String::as_str) {
        if matches!(first, "check" | "preview" | "dry-run") {
            dry_run = true;
            rest = &rest[1..];
        }
    }
    if rest.iter().any(|arg| arg == "-h" || arg == "--help") {
        eprintln!("usage: plugroot backup [check] [directory]");
        return Ok(None);
    }
    if rest.len() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: plugroot backup [check] [directory]",
        ));
    }
    if let Some(value) = rest.first() {
        if value.starts_with('-') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "backup uses words, not flags: try `plugroot backup check` or `plugroot backup /path/to/usb`",
            ));
        }
    }
    let output_dir = rest
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_output_dir.to_path_buf());
    Ok(Some((output_dir, dry_run)))
}

pub(crate) fn parse_emergency_export_options(
    args: &[String],
) -> io::Result<Option<EmergencyExportOptions>> {
    let mut output_dir = None;
    let mut yes = false;
    let mut dry_run = false;
    let mut passphrase_file = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--output requires a directory",
                    ));
                };
                output_dir = Some(PathBuf::from(value));
                index += 2;
            }
            value if value.starts_with("--output=") => {
                output_dir = Some(PathBuf::from(value.trim_start_matches("--output=")));
                index += 1;
            }
            "--yes" => {
                yes = true;
                index += 1;
            }
            "--dry-run" => {
                dry_run = true;
                index += 1;
            }
            "--passphrase-file" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--passphrase-file requires a path",
                    ));
                };
                passphrase_file = Some(PathBuf::from(value));
                index += 2;
            }
            value if value.starts_with("--passphrase-file=") => {
                passphrase_file = Some(PathBuf::from(
                    value.trim_start_matches("--passphrase-file="),
                ));
                index += 1;
            }
            "-h" | "--help" => {
                eprintln!(
                    "usage: plugroot emergency export --output <directory> --yes [--dry-run] [--passphrase-file <path>]"
                );
                return Ok(None);
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown emergency export option: {other}"),
                ));
            }
        }
    }

    let Some(output_dir) = output_dir else {
        eprintln!(
            "usage: plugroot emergency export --output <directory> --yes [--dry-run] [--passphrase-file <path>]"
        );
        return Ok(None);
    };

    Ok(Some(EmergencyExportOptions {
        output_dir,
        yes,
        dry_run,
        passphrase_file,
        create_output_dir: false,
        owner_user: None,
    }))
}

pub(crate) fn validate_emergency_output_boundary(
    ctx: &Context,
    output_dir: &Path,
) -> io::Result<()> {
    let code_root = clean_path(&ctx.root);
    let state_root = clean_path(&ctx.state_root());
    if same_or_inside(&code_root, output_dir) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "output directory must not be inside the Plugroot code checkout",
        ));
    }
    if same_or_inside(&state_root, output_dir) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "output directory must not be inside the Plugroot state root",
        ));
    }
    Ok(())
}

pub(crate) fn validate_emergency_output_dir(output_dir: &Path) -> io::Result<()> {
    if !output_dir.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("output directory does not exist: {}", output_dir.display()),
        ));
    }
    if !output_dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("output path is not a directory: {}", output_dir.display()),
        ));
    }
    Ok(())
}

pub(crate) fn validate_emergency_tools(passphrase_file: Option<&PathBuf>) -> io::Result<()> {
    for program in ["tar", "gpg", "zstd"] {
        if !program_available(program) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("required program not found in PATH: {program}"),
            ));
        }
    }
    if let Some(passphrase_file) = passphrase_file {
        if !passphrase_file.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("passphrase file not found: {}", passphrase_file.display()),
            ));
        }
    }
    Ok(())
}

pub(crate) fn default_emergency_output_dir(ctx: &Context) -> PathBuf {
    emergency_operator_home(ctx)
        .map(|home| PathBuf::from(home).join("Documents/plugroot-emergency-exports"))
        .or_else(|| {
            env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join("plugroot-emergency-exports"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp/plugroot-emergency-exports"))
}

pub(crate) fn emergency_operator_home(ctx: &Context) -> Option<String> {
    emergency_output_owner_user(ctx)
        .and_then(|user| user_home(&user))
        .or_else(|| env::var("HOME").ok())
        .filter(|home| !home.trim().is_empty())
}

pub(crate) fn emergency_output_owner_user(ctx: &Context) -> Option<String> {
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
            ctx.manifest
                .host
                .as_ref()
                .and_then(|host| host.user.clone())
                .filter(|user| user != "root" && !user.trim().is_empty())
        })
        .or_else(|| {
            env::var("USER")
                .ok()
                .filter(|user| user != "root" && !user.trim().is_empty())
        })
}

pub(crate) fn chown_path_to_user(path: &Path, user: Option<&str>) -> io::Result<()> {
    if !is_root() {
        return Ok(());
    }
    let Some(user) = user.filter(|user| !user.trim().is_empty()) else {
        return Ok(());
    };
    let group = command_text(&["id", "-gn", user]).unwrap_or_else(|| user.to_string());
    ensure_success(run_command(
        "chown",
        &[format!("{user}:{group}"), path.display().to_string()],
        None,
        &[],
    ))
}

mod archive;
mod sources;

#[cfg(test)]
pub(crate) use archive::emergency_tar_excludes;
pub(crate) use archive::{
    absolute_clean_path, command_line_for_spec, create_emergency_tar, emergency_host_name,
    emergency_timestamp, encrypt_emergency_tar, print_emergency_export_plan, program_available,
    sanitize_filename_component, tar_relative_path, write_emergency_metadata, write_json_file,
};
pub(crate) use sources::{emergency_metadata_commands, emergency_sources};
