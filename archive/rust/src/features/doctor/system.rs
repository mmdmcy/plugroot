use crate::broad_ufw_allow_rules;
use crate::expected_manifest_tcp_ports;
use crate::limit_lines;
use crate::meminfo_kb;
use crate::run_command;
use crate::run_read_command;
use crate::tailscale_text_has_public_funnel;
use crate::unexpected_wildcard_tcp_listeners;
use crate::Context;
use crate::DoctorFinding;
use std::fs;

pub(crate) fn doctor_load(findings: &mut Vec<DoctorFinding>) {
    let Ok(loadavg) = fs::read_to_string("/proc/loadavg") else {
        findings.push(DoctorFinding::info(
            "load",
            "could not read system load",
            "/proc/loadavg unavailable",
        ));
        return;
    };
    let Some(load1) = loadavg
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
    else {
        findings.push(DoctorFinding::info(
            "load",
            "could not parse system load",
            loadavg,
        ));
        return;
    };
    let cpus = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1) as f64;
    if load1 > cpus * 1.5 {
        findings.push(DoctorFinding::warn(
            "load",
            "one-minute load is high",
            format!("load1={load1:.2}, cpu_count={cpus:.0}"),
        ));
    } else {
        findings.push(DoctorFinding::ok(
            "load",
            format!("one-minute load is {load1:.2} across {cpus:.0} CPU(s)"),
        ));
    }
}

pub(crate) fn doctor_memory(findings: &mut Vec<DoctorFinding>) {
    let Ok(meminfo) = fs::read_to_string("/proc/meminfo") else {
        findings.push(DoctorFinding::info(
            "memory",
            "could not read memory pressure",
            "/proc/meminfo unavailable",
        ));
        return;
    };
    let Some(total) = meminfo_kb(&meminfo, "MemTotal") else {
        findings.push(DoctorFinding::info(
            "memory",
            "could not parse memory total",
            "/proc/meminfo missing MemTotal",
        ));
        return;
    };
    let Some(available) = meminfo_kb(&meminfo, "MemAvailable") else {
        findings.push(DoctorFinding::info(
            "memory",
            "could not parse available memory",
            "/proc/meminfo missing MemAvailable",
        ));
        return;
    };
    let percent = available as f64 * 100.0 / total as f64;
    let detail = format!(
        "{:.1}% available ({} MiB of {} MiB)",
        percent,
        available / 1024,
        total / 1024
    );
    if percent < 10.0 {
        findings.push(DoctorFinding::warn(
            "memory",
            "available memory is low",
            detail,
        ));
    } else {
        findings.push(DoctorFinding::ok("memory", detail));
    }
}

pub(crate) fn doctor_root_disk(findings: &mut Vec<DoctorFinding>) {
    let out = run_command("df", &["-P".into(), "/".into()], None, &[]);
    if out.code != 0 {
        findings.push(DoctorFinding::warn(
            "disk",
            "could not inspect root filesystem usage",
            out.text,
        ));
        return;
    }
    let Some(line) = out.text.lines().nth(1) else {
        findings.push(DoctorFinding::info(
            "disk",
            "could not parse root filesystem usage",
            out.text,
        ));
        return;
    };
    let columns: Vec<&str> = line.split_whitespace().collect();
    let Some(use_percent) = columns
        .get(4)
        .and_then(|value| value.trim_end_matches('%').parse::<u8>().ok())
    else {
        findings.push(DoctorFinding::info(
            "disk",
            "could not parse root filesystem percentage",
            line,
        ));
        return;
    };
    if use_percent >= 90 {
        findings.push(DoctorFinding::warn(
            "disk",
            "root filesystem is close to full",
            line,
        ));
    } else {
        findings.push(DoctorFinding::ok(
            "disk",
            format!("root filesystem is {use_percent}% used"),
        ));
    }
}

pub(crate) fn doctor_failed_units(findings: &mut Vec<DoctorFinding>) {
    let system = run_command(
        "systemctl",
        &["--failed".into(), "--no-legend".into(), "--no-pager".into()],
        None,
        &[],
    );
    if system.code == 0 {
        let text = system.text.trim();
        if text.is_empty() {
            findings.push(DoctorFinding::ok("systemd", "no failed system units"));
        } else {
            findings.push(DoctorFinding::warn(
                "systemd",
                "failed system units are present",
                text,
            ));
        }
    } else {
        findings.push(DoctorFinding::info(
            "systemd",
            "could not inspect failed system units",
            system.text,
        ));
    }

    let user = run_command(
        "systemctl",
        &[
            "--user".into(),
            "--failed".into(),
            "--no-legend".into(),
            "--no-pager".into(),
        ],
        None,
        &[],
    );
    if user.code == 0 {
        let text = user.text.trim();
        if text.is_empty() {
            findings.push(DoctorFinding::ok("user-systemd", "no failed user units"));
        } else {
            findings.push(DoctorFinding::warn(
                "user-systemd",
                "failed user units are present",
                text,
            ));
        }
    } else {
        findings.push(DoctorFinding::info(
            "user-systemd",
            "could not inspect failed user units",
            user.text,
        ));
    }
}

pub(crate) fn doctor_ufw(findings: &mut Vec<DoctorFinding>) {
    let out = run_read_command("ufw", &["status".into(), "verbose".into()]);
    if out.code != 0 {
        findings.push(DoctorFinding::warn(
            "ufw",
            "could not inspect firewall status",
            out.text,
        ));
        return;
    }

    let lower = out.text.to_ascii_lowercase();
    if lower.contains("status: inactive") {
        findings.push(DoctorFinding::error(
            "ufw",
            "firewall is inactive",
            out.text,
        ));
        return;
    }
    if !lower.contains("status: active") {
        findings.push(DoctorFinding::info(
            "ufw",
            "firewall status was not recognized",
            out.text,
        ));
        return;
    }

    let broad = broad_ufw_allow_rules(&out.text);
    if broad.is_empty() {
        findings.push(DoctorFinding::ok(
            "ufw",
            "firewall is active with no broad inbound allow rules detected",
        ));
    } else {
        findings.push(DoctorFinding::warn(
            "ufw",
            format!("{} broad inbound allow rule(s) detected", broad.len()),
            limit_lines(&broad, 12),
        ));
    }
}

pub(crate) fn doctor_tailscale(findings: &mut Vec<DoctorFinding>) {
    let serve = run_command("tailscale", &["serve".into(), "status".into()], None, &[]);
    let funnel = run_command("tailscale", &["funnel".into(), "status".into()], None, &[]);
    if serve.code == 127 && funnel.code == 127 {
        findings.push(DoctorFinding::info(
            "tailscale",
            "tailscale command is unavailable",
            serve.text,
        ));
        return;
    }

    let mut details = Vec::new();
    if serve.code == 0 && !serve.text.is_empty() {
        details.push(serve.text.clone());
    }
    if funnel.code == 0 && !funnel.text.is_empty() && funnel.text != serve.text {
        details.push(funnel.text.clone());
    }
    let combined = details.join("\n");
    if tailscale_text_has_public_funnel(&combined) {
        findings.push(DoctorFinding::error(
            "tailscale",
            "Tailscale Funnel appears to be enabled",
            combined,
        ));
    } else if serve.code == 0 || funnel.code == 0 {
        findings.push(DoctorFinding::ok(
            "tailscale",
            "no public Tailscale Funnel exposure detected",
        ));
    } else {
        findings.push(DoctorFinding::info(
            "tailscale",
            "could not inspect Tailscale Serve/Funnel",
            [serve.text, funnel.text].join("\n"),
        ));
    }
}

pub(crate) fn doctor_listeners(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
    let expected_tcp_ports = expected_manifest_tcp_ports(ctx);
    let out = run_command("ss", &["-ltnupH".into()], None, &[]);
    if out.code != 0 {
        findings.push(DoctorFinding::warn(
            "listeners",
            "could not inspect listening sockets",
            out.text,
        ));
        return;
    }

    let unexpected = unexpected_wildcard_tcp_listeners(&out.text, &expected_tcp_ports);
    if unexpected.is_empty() {
        findings.push(DoctorFinding::ok(
            "listeners",
            "no unexpected wildcard TCP listeners detected",
        ));
    } else {
        findings.push(DoctorFinding::warn(
            "listeners",
            format!(
                "{} unexpected wildcard TCP listener(s) detected",
                unexpected.len()
            ),
            limit_lines(&unexpected, 16),
        ));
    }
}
