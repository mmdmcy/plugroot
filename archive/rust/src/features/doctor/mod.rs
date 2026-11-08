use crate::audit_public;
use crate::boundary_findings;
use crate::git_dirty_details;
use crate::status_rows;
use crate::BoundarySeverity;
use crate::Context;
use crate::PathBuf;
use std::io;

mod docker;
mod host;
mod system;
mod types;

pub(crate) use docker::doctor_docker;
#[cfg(test)]
pub(crate) use host::add_tcp_ports_from_descriptor;
pub(crate) use host::{
    broad_ufw_allow_rules, doctor_fuse_mounts, doctor_tmux, expected_container_names,
    expected_manifest_tcp_ports, limit_lines, meminfo_kb, run_read_command,
    tailscale_text_has_public_funnel, unexpected_wildcard_tcp_listeners,
};
pub(crate) use system::{
    doctor_failed_units, doctor_listeners, doctor_load, doctor_memory, doctor_root_disk,
    doctor_tailscale, doctor_ufw,
};
pub(crate) use types::{DoctorFinding, DoctorSeverity};

pub(crate) fn cmd_doctor(ctx: &Context, args: &[String]) -> io::Result<u8> {
    let mut json = false;
    let mut strict = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!(
                    r#"Usage:
  plugroot doctor [--json] [--strict]

Runs a read-only host health scan for Plugroot boundaries, declared services,
firewall exposure, Tailscale Funnel, wildcard listeners, stale Docker/tmux/FUSE
state, failed systemd units, and basic resource pressure.

By default doctor exits nonzero only for errors. --strict also fails on warnings.
"#
                );
                return Ok(0);
            }
            "--json" => json = true,
            "--strict" => strict = true,
            _ => {
                eprintln!("usage: plugroot doctor [--json] [--strict]");
                return Ok(2);
            }
        }
    }

    let findings = doctor_findings(ctx)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&findings)?);
    } else {
        print_doctor_report(&findings);
    }

    let has_errors = findings
        .iter()
        .any(|finding| finding.severity == DoctorSeverity::Error);
    let has_warnings = findings
        .iter()
        .any(|finding| finding.severity == DoctorSeverity::Warn);
    if has_errors || (strict && has_warnings) {
        Ok(1)
    } else {
        Ok(0)
    }
}

pub(crate) fn cmd_release_check(ctx: &Context) -> io::Result<u8> {
    let mut failed = false;

    let boundary = boundary_findings(ctx)?;
    if boundary.is_empty() {
        println!("boundary: ok");
    } else {
        failed = true;
        eprintln!("boundary: found {} issue(s)", boundary.len());
        for finding in boundary {
            let location = finding
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".into());
            eprintln!(
                "{}: {location}: {}",
                finding.severity.label(),
                finding.message
            );
        }
    }

    match audit_public(&ctx.root) {
        Ok(findings) if findings.is_empty() => println!("audit-public: ok"),
        Ok(findings) => {
            failed = true;
            eprintln!("audit-public: found {} issue(s)", findings.len());
            for finding in findings {
                match finding.line {
                    Some(line) => eprintln!("{}:{line}: {}", finding.path, finding.message),
                    None => eprintln!("{}: {}", finding.path, finding.message),
                }
            }
        }
        Err(err) => {
            failed = true;
            eprintln!("audit-public: failed: {err}");
        }
    }

    match git_dirty_details(&ctx.root) {
        Ok(None) => println!("git: code checkout clean"),
        Ok(Some(detail)) => {
            failed = true;
            eprintln!("git: code checkout has uncommitted changes\n{detail}");
        }
        Err(err) => {
            failed = true;
            eprintln!("git: could not check code checkout: {err}");
        }
    }

    for repo in &ctx.manifest.repo {
        let path = PathBuf::from(&repo.path);
        match git_dirty_details(&path) {
            Ok(None) => println!("git: {} clean", repo.id),
            Ok(Some(detail)) => {
                failed = true;
                eprintln!("git: {} has uncommitted changes\n{detail}", repo.id);
            }
            Err(err) => {
                failed = true;
                eprintln!("git: could not check {}: {err}", repo.id);
            }
        }
    }

    if failed {
        Ok(1)
    } else {
        println!("release-check: ok");
        Ok(0)
    }
}

pub(crate) fn print_doctor_report(findings: &[DoctorFinding]) {
    let errors = findings
        .iter()
        .filter(|finding| finding.severity == DoctorSeverity::Error)
        .count();
    let warnings = findings
        .iter()
        .filter(|finding| finding.severity == DoctorSeverity::Warn)
        .count();
    println!(
        "doctor: {} check(s), {errors} error(s), {warnings} warning(s)",
        findings.len()
    );
    println!("{:<7} {:<18} MESSAGE", "STATE", "CHECK");
    for finding in findings {
        println!(
            "{:<7} {:<18} {}",
            finding.severity.label(),
            finding.check,
            finding.message
        );
        if let Some(detail) = &finding.detail {
            for line in detail.lines() {
                println!("{:<7} {:<18} {}", "", "", line);
            }
        }
    }
}

pub(crate) fn doctor_findings(ctx: &Context) -> io::Result<Vec<DoctorFinding>> {
    let mut findings = Vec::new();
    doctor_boundary(ctx, &mut findings)?;
    doctor_audit(ctx, &mut findings);
    doctor_git(ctx, &mut findings);
    doctor_declared_services(ctx, &mut findings);
    doctor_resources(&mut findings);
    doctor_failed_units(&mut findings);
    doctor_ufw(&mut findings);
    doctor_tailscale(&mut findings);
    doctor_listeners(ctx, &mut findings);
    doctor_docker(ctx, &mut findings);
    doctor_tmux(&mut findings);
    doctor_fuse_mounts(&mut findings);
    Ok(findings)
}

pub(crate) fn doctor_boundary(ctx: &Context, findings: &mut Vec<DoctorFinding>) -> io::Result<()> {
    let boundary = boundary_findings(ctx)?;
    if boundary.is_empty() {
        findings.push(DoctorFinding::ok(
            "boundary",
            "code and private state roots are separated",
        ));
        return Ok(());
    }

    for finding in boundary {
        let detail = finding
            .path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".into());
        match finding.severity {
            BoundarySeverity::Error => {
                findings.push(DoctorFinding::error("boundary", finding.message, detail))
            }
            BoundarySeverity::Warning => {
                findings.push(DoctorFinding::warn("boundary", finding.message, detail))
            }
        }
    }
    Ok(())
}

pub(crate) fn doctor_audit(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
    match audit_public(&ctx.root) {
        Ok(audit) if audit.is_empty() => findings.push(DoctorFinding::ok(
            "audit-public",
            "tracked files passed private-state audit",
        )),
        Ok(audit) => {
            let detail = audit
                .iter()
                .take(12)
                .map(|finding| match finding.line {
                    Some(line) => format!("{}:{}: {}", finding.path, line, finding.message),
                    None => format!("{}: {}", finding.path, finding.message),
                })
                .collect::<Vec<_>>()
                .join("\n");
            findings.push(DoctorFinding::error(
                "audit-public",
                format!("tracked files have {} private-state issue(s)", audit.len()),
                detail,
            ));
        }
        Err(err) => findings.push(DoctorFinding::warn(
            "audit-public",
            "could not audit tracked files",
            err.to_string(),
        )),
    }
}

pub(crate) fn doctor_git(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
    match git_dirty_details(&ctx.root) {
        Ok(None) => findings.push(DoctorFinding::ok("git", "code checkout is clean")),
        Ok(Some(detail)) => findings.push(DoctorFinding::warn(
            "git",
            "code checkout has uncommitted changes",
            detail,
        )),
        Err(err) => findings.push(DoctorFinding::warn(
            "git",
            "could not check code checkout cleanliness",
            err.to_string(),
        )),
    }

    let mut dirty_repos = Vec::new();
    let mut failed_repos = Vec::new();
    for repo in &ctx.manifest.repo {
        let path = PathBuf::from(&repo.path);
        match git_dirty_details(&path) {
            Ok(None) => {}
            Ok(Some(detail)) => dirty_repos.push(format!("{}:\n{}", repo.id, detail)),
            Err(err) => failed_repos.push(format!("{}: {err}", repo.id)),
        }
    }

    if dirty_repos.is_empty() && failed_repos.is_empty() {
        findings.push(DoctorFinding::ok(
            "repo-git",
            "managed repo checkouts are clean",
        ));
        return;
    }

    if !dirty_repos.is_empty() {
        findings.push(DoctorFinding::warn(
            "repo-git",
            format!("{} managed repo checkout(s) are dirty", dirty_repos.len()),
            limit_lines(&dirty_repos, 12),
        ));
    }
    if !failed_repos.is_empty() {
        findings.push(DoctorFinding::warn(
            "repo-git",
            format!(
                "could not inspect {} managed repo checkout(s)",
                failed_repos.len()
            ),
            limit_lines(&failed_repos, 12),
        ));
    }
}

pub(crate) fn doctor_declared_services(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
    let rows = status_rows(ctx);
    let mut attention = Vec::new();
    for row in rows {
        if row.state == "online" {
            continue;
        }
        if row.optional && matches!(row.state.as_str(), "offline" | "missing") {
            continue;
        }
        attention.push(format!("{}: {} ({})", row.id, row.state, row.detail));
    }

    if attention.is_empty() {
        findings.push(DoctorFinding::ok(
            "services",
            "declared non-optional services are online",
        ));
    } else {
        findings.push(DoctorFinding::warn(
            "services",
            format!("{} declared service(s) need attention", attention.len()),
            limit_lines(&attention, 16),
        ));
    }
}

pub(crate) fn doctor_resources(findings: &mut Vec<DoctorFinding>) {
    doctor_load(findings);
    doctor_memory(findings);
    doctor_root_disk(findings);
}
