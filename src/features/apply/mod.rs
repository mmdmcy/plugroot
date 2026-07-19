use crate::ensure_success;
use crate::is_root;
use crate::nearest_git_root;
use crate::print_cmd_output;
use crate::resolve_path;
use crate::run_command;
use crate::run_prefixed_command;
use crate::same_or_inside;
use crate::user_home;
use crate::user_systemd_prefix;
use crate::Context;
use crate::ManagedDir;
use crate::ManagedFile;
use crate::ManagedSymlink;
use crate::Path;
use crate::PathBuf;
use crate::Repo;
use crate::Service;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::process::Command;

mod types;

pub(crate) use types::ApplyOptions;

pub(crate) fn parse_apply_options(args: &[String]) -> Result<ApplyOptions, String> {
    let mut options = ApplyOptions::default();
    for arg in args {
        match arg.as_str() {
            "--dry-run" => options.dry_run = true,
            "--sync-repos" => options.sync_repos = true,
            "-h" | "--help" => {
                return Err("usage: plugroot apply [--dry-run] [--sync-repos]".into())
            }
            unknown => return Err(format!("unknown apply option: {unknown}")),
        }
    }
    Ok(options)
}

pub(crate) fn cmd_apply(ctx: &Context, args: &[String]) -> io::Result<u8> {
    let options = match parse_apply_options(args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            eprintln!("usage: plugroot apply [--dry-run] [--sync-repos]");
            return Ok(2);
        }
    };
    let dry_run = options.dry_run;
    let state_root = ctx.state_root();
    let generated_root = state_root.join(".plugroot/generated");
    println!("code root: {}", ctx.root.display());
    println!("state root: {}", state_root.display());
    if let Some(repo_dir) = ctx.repo_dir() {
        println!("repo dir: {}", repo_dir.display());
    }
    if dry_run {
        println!("dry run: no files or services will be changed");
    }
    if options.sync_repos {
        println!("repo sync: enabled");
    } else {
        println!("repo sync: skipped; use `plugroot repos sync` or `plugroot apply --sync-repos`");
    }
    if !dry_run {
        fs::create_dir_all(&generated_root)?;
    }

    for directory in &ctx.manifest.directory {
        println!(
            "{} directory {}",
            if dry_run { "would create" } else { "creating" },
            directory.path
        );
        if !dry_run {
            apply_directory(directory)?;
        }
    }

    for file in &ctx.manifest.file {
        println!(
            "{} file {} -> {}",
            if dry_run { "would copy" } else { "copying" },
            file.source,
            file.target
        );
        if !dry_run {
            apply_file(ctx, file)?;
        }
    }

    for repo in &ctx.manifest.repo {
        if options.sync_repos {
            println!(
                "{} repo {} -> {}",
                if dry_run { "would sync" } else { "syncing" },
                repo.id,
                repo.path
            );
            if !dry_run {
                sync_repo(repo)?;
            }
        } else {
            println!("skipping repo sync {} -> {}", repo.id, repo.path);
        }
    }

    for managed_symlink in &ctx.manifest.symlink {
        println!(
            "{} symlink {} -> {}",
            if dry_run { "would create" } else { "creating" },
            managed_symlink.target,
            managed_symlink.source
        );
        if !dry_run {
            apply_symlink(ctx, managed_symlink)?;
        }
    }

    for service in &ctx.manifest.service {
        if let Some(unit) = generate_unit(ctx, service)? {
            let scope = if service.kind == "user-systemd" {
                "user"
            } else {
                "system"
            };
            let unit_name = service.unit.as_deref().unwrap_or("missing.service");
            let generated_path = generated_root.join("systemd").join(scope).join(unit_name);
            println!(
                "{} {} unit {}",
                if dry_run { "would write" } else { "writing" },
                scope,
                unit_name
            );
            if !dry_run {
                if let Some(parent) = generated_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&generated_path, unit)?;
                install_unit(service, &generated_path)?;
            }
        }
    }
    Ok(0)
}

pub(crate) fn cmd_repos(ctx: &Context, args: &[String]) -> io::Result<u8> {
    match args.first().map(String::as_str) {
        Some("sync") => {
            for repo in &ctx.manifest.repo {
                println!("syncing {} -> {}", repo.id, repo.path);
                sync_repo(repo)?;
            }
            Ok(0)
        }
        _ => {
            eprintln!("usage: plugroot repos sync");
            Ok(2)
        }
    }
}

pub(crate) fn sync_repo(repo: &Repo) -> io::Result<()> {
    let path = PathBuf::from(&repo.path);
    if path.join(".git").exists() {
        let out = run_command(
            "git",
            &[
                "-C".into(),
                repo.path.clone(),
                "fetch".into(),
                "--all".into(),
                "--prune".into(),
            ],
            None,
            &[],
        );
        print_cmd_output(&out);
        if out.code != 0 {
            return Err(io::Error::other("git fetch failed"));
        }
        if let Some(git_ref) = &repo.git_ref {
            let out = run_command(
                "git",
                &[
                    "-C".into(),
                    repo.path.clone(),
                    "checkout".into(),
                    git_ref.clone(),
                ],
                None,
                &[],
            );
            print_cmd_output(&out);
            if out.code != 0 {
                return Err(io::Error::other("git checkout failed"));
            }
            let out = run_command(
                "git",
                &[
                    "-C".into(),
                    repo.path.clone(),
                    "pull".into(),
                    "--ff-only".into(),
                ],
                None,
                &[],
            );
            print_cmd_output(&out);
            if out.code != 0 {
                return Err(io::Error::other("git pull failed"));
            }
        }
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let out = run_command(
        "git",
        &["clone".into(), repo.url.clone(), repo.path.clone()],
        None,
        &[],
    );
    print_cmd_output(&out);
    if out.code != 0 {
        return Err(io::Error::other("git clone failed"));
    }
    if let Some(git_ref) = &repo.git_ref {
        let out = run_command(
            "git",
            &[
                "-C".into(),
                repo.path.clone(),
                "checkout".into(),
                git_ref.clone(),
            ],
            None,
            &[],
        );
        print_cmd_output(&out);
        if out.code != 0 {
            return Err(io::Error::other("git checkout failed"));
        }
    }
    Ok(())
}

pub(crate) fn apply_directory(directory: &ManagedDir) -> io::Result<()> {
    fs::create_dir_all(&directory.path)?;
    apply_owner_mode(
        Path::new(&directory.path),
        directory.owner.as_deref(),
        directory.mode.as_deref(),
    )
}

pub(crate) fn apply_file(ctx: &Context, file: &ManagedFile) -> io::Result<()> {
    let source = resolve_path(&ctx.root, &file.source);
    let target = PathBuf::from(&file.target);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&source, &target)?;
    apply_owner_mode(&target, file.owner.as_deref(), file.mode.as_deref())
}

pub(crate) fn apply_symlink(ctx: &Context, managed_symlink: &ManagedSymlink) -> io::Result<()> {
    let state_root = fs::canonicalize(ctx.state_root()).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!(
                "canonicalize state root {}: {err}",
                ctx.state_root().display()
            ),
        )
    })?;
    let requested_source = resolve_path(&ctx.root, &managed_symlink.source);
    let source = fs::canonicalize(&requested_source).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!(
                "canonicalize symlink source {}: {err}",
                requested_source.display()
            ),
        )
    })?;
    if !same_or_inside(&state_root, &source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "symlink source {} must be inside state root {}",
                source.display(),
                state_root.display()
            ),
        ));
    }

    let target = resolve_path(&ctx.root, &managed_symlink.target);
    let parent = target.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("symlink target {} has no parent", target.display()),
        )
    })?;
    if !parent.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "symlink target parent {} does not exist; create or sync it first",
                parent.display()
            ),
        ));
    }

    if let Some(git_root) = nearest_git_root(parent) {
        let relative_target = target.strip_prefix(&git_root).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "symlink target {} could not be resolved inside Git checkout {}",
                    target.display(),
                    git_root.display()
                ),
            )
        })?;
        let ignored = Command::new("git")
            .arg("-C")
            .arg(&git_root)
            .args(["check-ignore", "--quiet", "--"])
            .arg(relative_target)
            .status()?;
        if !ignored.success() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "symlink target {} is inside a Git checkout but is not ignored",
                    target.display()
                ),
            ));
        }
    }

    match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let existing = fs::read_link(&target)?;
            let existing = if existing.is_absolute() {
                existing
            } else {
                parent.join(existing)
            };
            if fs::canonicalize(existing).ok().as_ref() == Some(&source) {
                return Ok(());
            }
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "refusing to replace conflicting symlink {}",
                    target.display()
                ),
            ))
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("refusing to replace existing path {}", target.display()),
        )),
        Err(err) if err.kind() == io::ErrorKind::NotFound => symlink(source, target),
        Err(err) => Err(err),
    }
}

pub(crate) fn apply_owner_mode(
    path: &Path,
    owner: Option<&str>,
    mode: Option<&str>,
) -> io::Result<()> {
    if let Some(mode) = mode {
        ensure_success(run_command(
            "chmod",
            &[mode.into(), path.display().to_string()],
            None,
            &[],
        ))?;
    }
    if let Some(owner) = owner {
        ensure_success(run_command(
            "chown",
            &[owner.into(), path.display().to_string()],
            None,
            &[],
        ))?;
    }
    Ok(())
}

pub(crate) fn generate_unit(ctx: &Context, service: &Service) -> io::Result<Option<String>> {
    let Some(unit) = service.unit.as_ref() else {
        return Ok(None);
    };
    if let Some(source) = &service.unit_source {
        return fs::read_to_string(resolve_path(&ctx.root, source)).map(Some);
    }
    let Some(command) = service.command.as_ref() else {
        return Ok(None);
    };
    let working_dir = service
        .working_dir
        .as_deref()
        .or(service.path.as_deref())
        .unwrap_or(".");
    let env_lines = service.env.clone().unwrap_or_default();
    let mut text = String::new();
    text.push_str("[Unit]\n");
    text.push_str(&format!("Description=Plugroot - {}\n", service.name));
    text.push_str("\n[Service]\n");
    text.push_str("Type=simple\n");
    text.push_str(&format!(
        "WorkingDirectory={}\n",
        systemd_quote(working_dir)
    ));
    for item in env_lines {
        text.push_str(&format!("Environment={}\n", systemd_quote(&item)));
    }
    text.push_str(&format!(
        "ExecStart={}\n",
        command
            .iter()
            .map(|arg| systemd_quote(arg))
            .collect::<Vec<_>>()
            .join(" ")
    ));
    text.push_str("Restart=on-failure\n");
    text.push_str("RestartSec=2\n");
    text.push_str("\n[Install]\n");
    text.push_str(if service.kind == "user-systemd" {
        "WantedBy=default.target\n"
    } else {
        "WantedBy=multi-user.target\n"
    });
    if unit.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

pub(crate) fn systemd_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "/:._-=@".contains(ch))
    {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn install_unit(service: &Service, generated_path: &Path) -> io::Result<()> {
    let unit = service
        .unit
        .as_ref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing unit"))?;
    match service.kind.as_str() {
        "user-systemd" => {
            let user = service.user.as_deref().unwrap_or("");
            let home = user_home(user).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("user {user} not found"),
                )
            })?;
            let target = PathBuf::from(home).join(".config/systemd/user").join(unit);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(generated_path, target)?;
            let prefix = user_systemd_prefix(user).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("user {user} not found"),
                )
            })?;
            ensure_success(run_prefixed_command(
                &prefix,
                "systemctl",
                &["--user".into(), "daemon-reload".into()],
            ))?;
            ensure_success(run_prefixed_command(
                &prefix,
                "systemctl",
                &["--user".into(), "enable".into(), unit.into()],
            ))?;
        }
        "systemd" => {
            if !is_root() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "system units require root",
                ));
            }
            fs::copy(
                generated_path,
                PathBuf::from("/etc/systemd/system").join(unit),
            )?;
            ensure_success(run_command(
                "systemctl",
                &["daemon-reload".into()],
                None,
                &[],
            ))?;
            ensure_success(run_command(
                "systemctl",
                &["enable".into(), unit.into()],
                None,
                &[],
            ))?;
        }
        _ => {}
    }
    Ok(())
}
