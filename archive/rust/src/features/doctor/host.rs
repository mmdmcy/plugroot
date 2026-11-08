use crate::is_root;
use crate::run_command;
use crate::CmdOutput;
use crate::Context;
use crate::DoctorFinding;
use crate::HashSet;

pub(crate) fn doctor_tmux(findings: &mut Vec<DoctorFinding>) {
    let out = run_command(
        "tmux",
        &[
            "list-sessions".into(),
            "-F".into(),
            "#{session_name}\t#{session_attached}\t#{t:session_activity}".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        if out.text.to_ascii_lowercase().contains("no server running") {
            findings.push(DoctorFinding::ok("tmux", "no tmux server is running"));
        } else {
            findings.push(DoctorFinding::info(
                "tmux",
                "could not inspect tmux sessions",
                out.text,
            ));
        }
        return;
    }

    let retired = retired_tmux_sessions(&out.text);
    if retired.is_empty() {
        findings.push(DoctorFinding::ok(
            "tmux",
            "no retired codex/openclaw tmux sessions detected",
        ));
    } else {
        findings.push(DoctorFinding::warn(
            "tmux",
            format!("{} retired tmux session(s) still exist", retired.len()),
            limit_lines(&retired, 12),
        ));
    }
}

pub(crate) fn doctor_fuse_mounts(findings: &mut Vec<DoctorFinding>) {
    let out = run_command(
        "findmnt",
        &["-rn".into(), "-o".into(), "TARGET,FSTYPE,SOURCE".into()],
        None,
        &[],
    );
    if out.code != 0 {
        findings.push(DoctorFinding::info(
            "fuse",
            "could not inspect mounted filesystems",
            out.text,
        ));
        return;
    }

    let mut stale = Vec::new();
    for line in out.text.lines() {
        let columns: Vec<&str> = line.split_whitespace().collect();
        let Some(target) = columns.first().copied() else {
            continue;
        };
        let fstype = columns.get(1).copied().unwrap_or("");
        if !fstype.starts_with("fuse") {
            continue;
        }
        let stat = run_command(
            "timeout",
            &[
                "2".into(),
                "stat".into(),
                "-c".into(),
                "%F".into(),
                target.into(),
            ],
            None,
            &[],
        );
        if stat.code != 0 {
            stale.push(format!("{target}: {}", stat.text));
        }
    }

    if stale.is_empty() {
        findings.push(DoctorFinding::ok("fuse", "no stale FUSE mounts detected"));
    } else {
        findings.push(DoctorFinding::warn(
            "fuse",
            format!("{} stale FUSE mount(s) detected", stale.len()),
            limit_lines(&stale, 8),
        ));
    }
}

pub(crate) fn meminfo_kb(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (name, rest) = line.split_once(':')?;
        if name != key {
            return None;
        }
        rest.split_whitespace().next()?.parse::<u64>().ok()
    })
}

pub(crate) fn broad_ufw_allow_rules(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            line.contains("ALLOW IN")
                && line.contains("Anywhere")
                && !line.contains(" on tailscale0")
                && !line.contains("41641/udp")
        })
        .map(str::to_string)
        .collect()
}

pub(crate) fn tailscale_text_has_public_funnel(text: &str) -> bool {
    text.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains("funnel")
            && !lower.contains("funnel off")
            && !lower.contains("funnel is off")
            && !lower.contains("disabled")
    })
}

pub(crate) fn expected_manifest_tcp_ports(ctx: &Context) -> HashSet<u16> {
    let mut ports = HashSet::new();
    for service in &ctx.manifest.service {
        if matches!(service.kind.as_str(), "port" | "manual") {
            if let Some(port) = service.port {
                ports.insert(port);
            }
        }
        for descriptor in service.ports.as_deref().unwrap_or(&[]) {
            add_tcp_ports_from_descriptor(descriptor, &mut ports);
        }
    }
    ports
}

pub(crate) fn add_tcp_ports_from_descriptor(descriptor: &str, ports: &mut HashSet<u16>) {
    for token in descriptor.split(|ch: char| ch.is_whitespace() || ch == ',') {
        let Some((range, protocol)) = token.split_once('/') else {
            continue;
        };
        if !protocol.to_ascii_lowercase().starts_with("tcp") {
            continue;
        }
        add_port_range(range, ports);
    }
}

pub(crate) fn add_port_range(range: &str, ports: &mut HashSet<u16>) {
    if let Some((start, end)) = range.split_once('-') {
        let Some(start) = start.parse::<u16>().ok() else {
            return;
        };
        let Some(end) = end.parse::<u16>().ok() else {
            return;
        };
        if start > end {
            return;
        }
        for port in start..=end {
            ports.insert(port);
        }
        return;
    }
    if let Ok(port) = range.parse::<u16>() {
        ports.insert(port);
    }
}

pub(crate) fn unexpected_wildcard_tcp_listeners(
    text: &str,
    expected_tcp_ports: &HashSet<u16>,
) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let columns: Vec<&str> = line.split_whitespace().collect();
            if columns.first().copied() != Some("tcp") {
                return None;
            }
            let local = columns.get(4).copied()?;
            if !is_wildcard_listener(local) {
                return None;
            }
            let port = listener_port(local)?;
            if expected_tcp_ports.contains(&port) {
                return None;
            }
            Some(line.trim().to_string())
        })
        .collect()
}

pub(crate) fn is_wildcard_listener(local_address: &str) -> bool {
    local_address.starts_with("0.0.0.0:")
        || local_address.starts_with("[::]:")
        || local_address.starts_with("*:")
}

pub(crate) fn listener_port(local_address: &str) -> Option<u16> {
    local_address
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
}

pub(crate) fn expected_container_names(ctx: &Context) -> HashSet<String> {
    ctx.manifest
        .service
        .iter()
        .flat_map(|service| service.containers.as_deref().unwrap_or(&[]))
        .cloned()
        .collect()
}

pub(crate) fn retired_tmux_sessions(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut columns = line.split('\t');
            let name = columns.next().unwrap_or("");
            let attached = columns.next().unwrap_or("");
            let activity = columns.next().unwrap_or("");
            if attached == "0" && (name.starts_with("codex-") || name.starts_with("openclaw")) {
                Some(format!("{name}: detached, last active {activity}"))
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn limit_lines(lines: &[String], limit: usize) -> String {
    let mut selected = lines.iter().take(limit).cloned().collect::<Vec<_>>();
    if lines.len() > limit {
        selected.push(format!("... {} more", lines.len() - limit));
    }
    selected.join("\n")
}

pub(crate) fn run_read_command(program: &str, args: &[String]) -> CmdOutput {
    if is_root() {
        return run_command(program, args, None, &[]);
    }

    let mut sudo_args = vec!["-n".into(), program.into()];
    sudo_args.extend(args.iter().cloned());
    let sudo = run_command("sudo", &sudo_args, None, &[]);
    if sudo.code == 0 {
        sudo
    } else {
        run_command(program, args, None, &[])
    }
}
