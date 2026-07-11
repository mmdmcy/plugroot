use crate::absolute_clean_path;
use crate::clean_path;
use crate::default_state_root;
use crate::features::backup::tar_relative_path;
use crate::program_available;
use crate::run_command;
use crate::same_or_inside;
use crate::EmergencyExportManifest;
use crate::Path;
use crate::PathBuf;
use crate::UNIX_EPOCH;
use std::fs;
use std::io;

use super::{RestoreArchiveKind, RestorePlan};

pub(super) fn restore_plan_for_source(root: &Path, source: &Path) -> io::Result<RestorePlan> {
    let source = absolute_clean_path(source)?;
    let archive = find_restore_archive(&source)?;
    let kind = classify_restore_archive(&archive).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a Plugroot backup archive: {}", archive.display()),
        )
    })?;
    let manifest = companion_restore_manifest(&archive, kind);
    let (code_root, state_root) = restore_targets_from_manifest(root, manifest.as_deref())?;
    let plan = RestorePlan {
        archive,
        kind,
        manifest,
        code_root,
        state_root,
    };
    validate_restore_targets(&plan)?;
    Ok(plan)
}

pub(crate) fn find_restore_archive(source: &Path) -> io::Result<PathBuf> {
    if source.is_file() {
        if classify_restore_archive(source).is_some() {
            return Ok(source.to_path_buf());
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "not a supported Plugroot backup archive: {}",
                source.display()
            ),
        ));
    }
    if !source.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("restore source does not exist: {}", source.display()),
        ));
    }
    if !source.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "restore source is not a directory or archive: {}",
                source.display()
            ),
        ));
    }

    let mut candidates = Vec::new();
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let Some(kind) = classify_restore_archive(&path) else {
            continue;
        };
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        candidates.push((restore_archive_priority(kind), modified, path));
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    candidates
        .into_iter()
        .map(|(_, _, path)| path)
        .next()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no Plugroot backup archive found in {}", source.display()),
            )
        })
}

fn restore_archive_priority(kind: RestoreArchiveKind) -> u8 {
    match kind {
        RestoreArchiveKind::Encrypted => 1,
        RestoreArchiveKind::Plain => 0,
    }
}

pub(crate) fn classify_restore_archive(path: &Path) -> Option<RestoreArchiveKind> {
    let name = path.file_name()?.to_string_lossy();
    if name.ends_with(".tar.zst.gpg") {
        Some(RestoreArchiveKind::Encrypted)
    } else if name.ends_with(".tar.zst") {
        Some(RestoreArchiveKind::Plain)
    } else {
        None
    }
}

fn companion_restore_manifest(archive: &Path, kind: RestoreArchiveKind) -> Option<PathBuf> {
    let name = archive.file_name()?.to_string_lossy();
    let suffix = match kind {
        RestoreArchiveKind::Encrypted => ".tar.zst.gpg",
        RestoreArchiveKind::Plain => ".tar.zst",
    };
    let stem = name.strip_suffix(suffix)?;
    let candidate = archive.with_file_name(format!("{stem}.manifest.json"));
    if candidate.exists() {
        return Some(candidate);
    }
    let candidate = archive.with_file_name(format!("{stem}.manifest.txt"));
    candidate.exists().then_some(candidate)
}

fn restore_targets_from_manifest(
    root: &Path,
    manifest: Option<&Path>,
) -> io::Result<(PathBuf, PathBuf)> {
    let code_root = absolute_clean_path(root)?;
    let default = (code_root.clone(), default_state_root(&code_root));
    let Some(manifest) = manifest else {
        return Ok(default);
    };
    if manifest.extension().and_then(|ext| ext.to_str()) != Some("json") {
        return Ok(default);
    }
    let raw = fs::read_to_string(manifest)?;
    let parsed: EmergencyExportManifest = serde_json::from_str(&raw).map_err(io::Error::other)?;
    Ok((
        clean_path(Path::new(&parsed.code_root)),
        clean_path(Path::new(&parsed.state_root)),
    ))
}

pub(super) fn print_restore_plan(plan: &RestorePlan, check_only: bool) -> io::Result<()> {
    println!("restore {}", if check_only { "check" } else { "plan" });
    println!("backup: {}", plan.archive.display());
    println!("type: {}", plan.kind.label());
    if let Some(manifest) = &plan.manifest {
        println!("manifest: {}", manifest.display());
    } else {
        println!("manifest: missing; using default roots");
    }
    println!("target code root: {}", plan.code_root.display());
    println!("target state root: {}", plan.state_root.display());
    println!(
        "compatibility link: /var/lib/plugroot -> {}",
        plan.state_root.display()
    );
    if plan.kind == RestoreArchiveKind::Plain {
        println!("warning: this archive is not encrypted");
    }
    validate_restore_targets(plan)?;
    print_restore_target_status(plan);
    print_restore_tool_status(plan.kind);
    println!("flow:");
    println!("  1. install host prerequisites for your distro");
    println!("  2. plugroot restore {}", plan.archive.display());
    println!("  3. plugroot restore finish");
    println!("  4. plugroot doctor");
    Ok(())
}

fn print_restore_target_status(plan: &RestorePlan) {
    for path in [
        &plan.code_root,
        &plan.state_root,
        Path::new("/var/lib/plugroot"),
    ] {
        println!(
            "target {}: {}",
            path.display(),
            if path.exists() { "exists" } else { "clear" }
        );
    }
}

fn print_restore_tool_status(kind: RestoreArchiveKind) {
    let mut tools = vec!["tar", "zstd", "systemctl"];
    if kind == RestoreArchiveKind::Encrypted {
        tools.push("gpg");
    }
    tools.extend(["git", "docker"]);
    println!("tools:");
    for tool in tools {
        println!(
            "  - {tool}: {}",
            if program_available(tool) {
                "found"
            } else {
                "missing"
            }
        );
    }
    let compose = run_command("docker", &["compose".into(), "version".into()], None, &[]);
    println!(
        "  - docker compose: {}",
        if compose.code == 0 {
            "found"
        } else {
            "missing"
        }
    );
}

pub(super) fn validate_restore_targets(plan: &RestorePlan) -> io::Result<()> {
    let code_root = clean_path(&plan.code_root);
    let state_root = clean_path(&plan.state_root);
    if !code_root.is_absolute() || !state_root.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "restore targets must be absolute paths",
        ));
    }
    if code_root == Path::new("/") || state_root == Path::new("/") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "restore refuses to use the filesystem root as a target",
        ));
    }
    if same_or_inside(&code_root, &state_root) || same_or_inside(&state_root, &code_root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "code and state restore targets must be separate directories",
        ));
    }
    Ok(())
}

pub(super) fn restore_archive_roots(plan: &RestorePlan) -> io::Result<[String; 2]> {
    let code_root = tar_relative_path(&plan.code_root).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "code restore target cannot be the filesystem root",
        )
    })?;
    let state_root = tar_relative_path(&plan.state_root).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "state restore target cannot be the filesystem root",
        )
    })?;
    Ok([code_root, state_root])
}

#[cfg(test)]
mod tests {
    use super::{
        restore_archive_roots, restore_targets_from_manifest, validate_restore_targets,
        RestoreArchiveKind, RestorePlan,
    };
    use std::path::{Path, PathBuf};

    fn plan(code_root: &str, state_root: &str) -> RestorePlan {
        RestorePlan {
            archive: PathBuf::from("/tmp/example.tar.zst"),
            kind: RestoreArchiveKind::Plain,
            manifest: None,
            code_root: PathBuf::from(code_root),
            state_root: PathBuf::from(state_root),
        }
    }

    #[test]
    fn default_restore_targets_follow_the_selected_code_root() {
        let (code_root, state_root) =
            restore_targets_from_manifest(Path::new("/srv/example"), None).unwrap();

        assert_eq!(code_root, PathBuf::from("/srv/example"));
        assert_eq!(state_root, PathBuf::from("/srv/example-state"));
    }

    #[test]
    fn restore_targets_must_be_distinct_absolute_roots() {
        assert!(validate_restore_targets(&plan("/srv/example", "/srv/example-state")).is_ok());
        assert!(validate_restore_targets(&plan("example", "/srv/example-state")).is_err());
        assert!(validate_restore_targets(&plan("/srv/example", "/srv/example/data")).is_err());
    }

    #[test]
    fn archive_members_follow_the_restore_targets() {
        assert_eq!(
            restore_archive_roots(&plan("/srv/example", "/srv/example-state")).unwrap(),
            ["srv/example", "srv/example-state"]
        );
    }
}
