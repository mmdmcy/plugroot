//! Binary releases for systemd-backed services.
//!
//! A service that runs a single binary keeps its builds under
//! `<state root>/releases/<service>/<stamp>/<service>` with a `current`
//! symlink that the unit's ExecStart points at. Installing a build copies it
//! into a fresh stamped directory and flips `current` with one rename, so the
//! previous build stays on disk and `rollback` is the same flip the other way.

use std::fs;
use std::io;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::{Context, Service};
use crate::features::services::{has_control, run_fixed_action};
use crate::integrations::print_cmd_output;

const CURRENT: &str = "current";

/// What `current` pointed at after a change, and what it pointed at before.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReleaseChange {
    pub(crate) current: String,
    pub(crate) previous: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReleaseListing {
    pub(crate) current: Option<String>,
    pub(crate) releases: Vec<String>,
}

pub(crate) fn cmd_release(ctx: &Context, args: &[String]) -> io::Result<u8> {
    let mut restart = false;
    let mut words = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--restart" => restart = true,
            value if value.starts_with("--") => {
                eprintln!("unknown option: {value}");
                return Ok(2);
            }
            value => words.push(value.to_string()),
        }
    }
    let Some(target) = words.first() else {
        eprintln!("usage: plugroot release <service> [<binary>|rollback] [--restart]");
        return Ok(2);
    };
    let Some(service) = ctx.service(target) else {
        eprintln!("unknown service: {target}");
        return Ok(2);
    };
    if !matches!(service.kind.as_str(), "systemd" | "user-systemd") {
        eprintln!(
            "{} is a {} service; releases are for systemd units that run releases/{}/current",
            service.id, service.kind, service.id
        );
        return Ok(2);
    }

    let dir = release_dir(ctx, &service);
    let rolling_back = words.get(1).map(String::as_str) == Some("rollback");
    let change = match words.get(1).map(String::as_str) {
        None => {
            let listing = list_releases(&dir)?;
            println!("{}: {}", service.id, dir.display());
            if listing.releases.is_empty() {
                println!("  no releases installed");
            }
            for release in &listing.releases {
                let marker = if listing.current.as_deref() == Some(release) {
                    " <- current"
                } else {
                    ""
                };
                println!("  {release}{marker}");
            }
            return Ok(0);
        }
        Some("rollback") => rollback_release(&dir)?,
        Some(binary) => {
            let stamp = unique_stamp(&dir, &release_stamp());
            install_release(&dir, &service.id, Path::new(binary), &stamp)?
        }
    };

    println!("{}: current -> {}", service.id, change.current);
    match (&change.previous, rolling_back) {
        (Some(previous), false) => println!(
            "kept {previous}; `plugroot release {} rollback` goes back to it",
            service.id
        ),
        (Some(previous), true) => println!("left {previous} on disk"),
        (None, _) => println!("no earlier build to roll back to"),
    }

    if !restart {
        println!(
            "the running service still uses the old build; `plugroot restart {}` switches",
            service.id
        );
        return Ok(0);
    }
    if !has_control(&service, "restart") {
        eprintln!("{} does not allow `restart`", service.id);
        return Ok(2);
    }
    println!("==> {} restart", service.id);
    let out = run_fixed_action(ctx, &service, "restart");
    print_cmd_output(&out);
    Ok(out.code.min(255) as u8)
}

/// `stamp`, or `stamp-2`, `stamp-3`, ... when a build landed in the same second.
pub(crate) fn unique_stamp(dir: &Path, stamp: &str) -> String {
    if !dir.join(stamp).exists() {
        return stamp.to_string();
    }
    (2..)
        .map(|n| format!("{stamp}-{n}"))
        .find(|candidate| !dir.join(candidate).exists())
        .unwrap_or_else(|| stamp.to_string())
}

pub(crate) fn release_dir(ctx: &Context, service: &Service) -> PathBuf {
    ctx.state_root().join("releases").join(&service.id)
}

/// Copies `binary` to `<dir>/<stamp>/<name>` (mode 0755) and points
/// `<dir>/current` at the new directory. Nothing already on disk is removed.
pub(crate) fn install_release(
    dir: &Path,
    name: &str,
    binary: &Path,
    stamp: &str,
) -> io::Result<ReleaseChange> {
    let metadata = fs::metadata(binary).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("cannot read {}: {err}", binary.display()),
        )
    })?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a file", binary.display()),
        ));
    }
    if stamp.is_empty() || stamp.starts_with('.') || stamp == CURRENT || stamp.contains('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid release name {stamp:?}"),
        ));
    }

    fs::create_dir_all(dir).map_err(|err| writable_hint(dir, err))?;
    let stage = dir.join(stamp);
    if stage.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists; try again in a second", stage.display()),
        ));
    }
    fs::create_dir(&stage).map_err(|err| writable_hint(dir, err))?;
    let installed = stage.join(name);
    fs::copy(binary, &installed)?;
    fs::set_permissions(&installed, fs::Permissions::from_mode(0o755))?;

    let previous = current_release(dir);
    point_current(dir, stamp)?;
    Ok(ReleaseChange {
        current: stamp.to_string(),
        previous,
    })
}

/// Points `current` at the newest release older than the current one.
pub(crate) fn rollback_release(dir: &Path) -> io::Result<ReleaseChange> {
    let listing = list_releases(dir)?;
    let Some(current) = listing.current.clone() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} has no current release", dir.display()),
        ));
    };
    let Some(target) = listing
        .releases
        .iter()
        .rfind(|release| release.as_str() < current.as_str())
        .cloned()
    else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no release older than {current} in {}", dir.display()),
        ));
    };
    point_current(dir, &target)?;
    Ok(ReleaseChange {
        current: target,
        previous: Some(current),
    })
}

pub(crate) fn list_releases(dir: &Path) -> io::Result<ReleaseListing> {
    let mut releases = Vec::new();
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == CURRENT || name.starts_with('.') {
                    continue;
                }
                if entry.file_type()?.is_dir() {
                    releases.push(name);
                }
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    releases.sort();
    Ok(ReleaseListing {
        current: current_release(dir),
        releases,
    })
}

fn current_release(dir: &Path) -> Option<String> {
    fs::read_link(dir.join(CURRENT))
        .ok()
        .and_then(|target| target.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// Replaces `current` atomically: a temporary symlink is renamed over it.
fn point_current(dir: &Path, release: &str) -> io::Result<()> {
    let staging = dir.join(format!(".{CURRENT}.{release}.{}", std::process::id()));
    let _ = fs::remove_file(&staging);
    symlink(release, &staging).map_err(|err| writable_hint(dir, err))?;
    fs::rename(&staging, dir.join(CURRENT)).map_err(|err| {
        let _ = fs::remove_file(&staging);
        writable_hint(dir, err)
    })
}

fn writable_hint(dir: &Path, err: io::Error) -> io::Error {
    if err.kind() == io::ErrorKind::PermissionDenied {
        io::Error::new(
            err.kind(),
            format!(
                "{} is not writable by this user: run plugroot with sudo, or give the directory to the service user",
                dir.display()
            ),
        )
    } else {
        err
    }
}

/// UTC `YYYYMMDDHHMMSS`, the naming the releases directory already uses.
pub(crate) fn release_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    stamp_from_unix(secs)
}

pub(crate) fn stamp_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant), proleptic Gregorian.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}
