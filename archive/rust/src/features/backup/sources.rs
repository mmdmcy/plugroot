use crate::clean_path;
use crate::user_home;
use crate::Context;
use crate::EmergencyCommandSpec;
use crate::EmergencySource;
use crate::HashSet;
use crate::PathBuf;
use std::env;
use std::fs;

pub(crate) fn emergency_sources(ctx: &Context) -> (Vec<EmergencySource>, Vec<EmergencySource>) {
    let mut candidates = Vec::new();
    push_source(&mut candidates, "code root", clean_path(&ctx.root));
    push_source(&mut candidates, "state root", clean_path(&ctx.state_root()));
    push_source(
        &mut candidates,
        "rustdesk system config",
        PathBuf::from("/etc/rustdesk"),
    );
    push_source(&mut candidates, "ufw config", PathBuf::from("/etc/ufw"));
    push_source(
        &mut candidates,
        "tailscale config",
        PathBuf::from("/etc/tailscale"),
    );
    push_source(
        &mut candidates,
        "docker config",
        PathBuf::from("/etc/docker"),
    );
    for path in [
        "/etc/hostname",
        "/etc/hosts",
        "/etc/fstab",
        "/etc/os-release",
        "/etc/systemd/resolved.conf",
    ] {
        push_source(&mut candidates, "system edge config", PathBuf::from(path));
    }
    for path in emergency_systemd_unit_paths(ctx) {
        push_source(&mut candidates, "systemd unit", path);
    }
    if let Some(path) = emergency_user_rustdesk_config(ctx) {
        push_source(&mut candidates, "rustdesk user config", path);
    }

    let mut seen = HashSet::new();
    let mut included = Vec::new();
    let mut skipped = Vec::new();
    for source in candidates {
        let key = clean_path(&source.path).display().to_string();
        if !seen.insert(key) {
            continue;
        }
        if source.path.exists() {
            included.push(source);
        } else {
            skipped.push(source);
        }
    }
    (included, skipped)
}

pub(crate) fn push_source(
    sources: &mut Vec<EmergencySource>,
    label: impl Into<String>,
    path: PathBuf,
) {
    sources.push(EmergencySource {
        label: label.into(),
        path,
    });
}

pub(crate) fn emergency_systemd_unit_paths(ctx: &Context) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for service in &ctx.manifest.service {
        let Some(unit) = service.unit.as_deref() else {
            continue;
        };
        match service.kind.as_str() {
            "user-systemd" => {
                if let Some(user) = service.user.as_deref() {
                    if let Some(home) = user_home(user) {
                        paths.push(PathBuf::from(home).join(".config/systemd/user").join(unit));
                    }
                }
            }
            _ => paths.push(PathBuf::from("/etc/systemd/system").join(unit)),
        }
    }
    if let Ok(entries) = fs::read_dir("/etc/systemd/system") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with("plugroot-")
                && (name.ends_with(".service") || name.ends_with(".timer"))
            {
                paths.push(entry.path());
            }
        }
    }
    paths
}

pub(crate) fn emergency_user_rustdesk_config(ctx: &Context) -> Option<PathBuf> {
    let user = ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.user.clone())
        .or_else(|| env::var("SUDO_USER").ok().filter(|user| user != "root"))
        .or_else(|| env::var("USER").ok().filter(|user| user != "root"))?;
    user_home(&user).map(|home| PathBuf::from(home).join(".config/rustdesk"))
}

pub(crate) fn emergency_metadata_commands() -> Vec<EmergencyCommandSpec> {
    vec![
        EmergencyCommandSpec {
            file_name: "uname.txt",
            program: "uname",
            args: &["-a"],
        },
        EmergencyCommandSpec {
            file_name: "df.txt",
            program: "df",
            args: &["-hT"],
        },
        EmergencyCommandSpec {
            file_name: "lsblk.txt",
            program: "lsblk",
            args: &["-f"],
        },
        EmergencyCommandSpec {
            file_name: "systemd-failed.txt",
            program: "systemctl",
            args: &["list-units", "--failed", "--no-pager"],
        },
        EmergencyCommandSpec {
            file_name: "systemd-enabled.txt",
            program: "systemctl",
            args: &["list-unit-files", "--state=enabled", "--no-pager"],
        },
        EmergencyCommandSpec {
            file_name: "docker-ps.txt",
            program: "docker",
            args: &["ps", "-a"],
        },
        EmergencyCommandSpec {
            file_name: "docker-compose-ls.txt",
            program: "docker",
            args: &["compose", "ls"],
        },
        EmergencyCommandSpec {
            file_name: "docker-volumes.txt",
            program: "docker",
            args: &["volume", "ls"],
        },
        EmergencyCommandSpec {
            file_name: "tailscale-status.txt",
            program: "tailscale",
            args: &["status"],
        },
        EmergencyCommandSpec {
            file_name: "tailscale-serve.txt",
            program: "tailscale",
            args: &["serve", "status"],
        },
        EmergencyCommandSpec {
            file_name: "ufw-status.txt",
            program: "ufw",
            args: &["status", "verbose"],
        },
        EmergencyCommandSpec {
            file_name: "dpkg-packages.txt",
            program: "dpkg-query",
            args: &["-W", "-f=${binary:Package}\t${Version}\n"],
        },
        EmergencyCommandSpec {
            file_name: "flatpak-apps.txt",
            program: "flatpak",
            args: &[
                "list",
                "--app",
                "--columns=application,version,branch,origin",
            ],
        },
    ]
}
