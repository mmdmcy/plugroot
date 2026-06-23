use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, Read, Stdout, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use serde::{Deserialize, Serialize};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("plugroot: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> io::Result<u8> {
    let (root, args) = parse_global_args(env::args().skip(1).collect())?;
    let command = args.first().map(String::as_str).unwrap_or("help");

    if matches!(command, "help" | "-h" | "--help") {
        print_help();
        return Ok(0);
    }

    if command == "audit-public" {
        return cmd_audit_public(&root, &args[1..]);
    }

    let ctx = Context::load(root)?;
    match command {
        "boundary" => cmd_boundary(&ctx, &args[1..]),
        "status" => cmd_status(&ctx, &args[1..]),
        "apply" => cmd_apply(&ctx, &args[1..]),
        "repos" => cmd_repos(&ctx, &args[1..]),
        "doctor" => cmd_doctor(&ctx, &args[1..]),
        "release-check" => cmd_release_check(&ctx),
        "up" => cmd_action(&ctx, "start", &args[1..]),
        "down" => cmd_action(&ctx, "stop", &args[1..]),
        "restart" => cmd_action(&ctx, "restart", &args[1..]),
        "logs" => cmd_action(&ctx, "logs", &args[1..]),
        "tui" => cmd_tui(ctx, &args[1..]),
        "web" => cmd_web(ctx, &args[1..]),
        "list" => cmd_list(&ctx),
        _ => {
            eprintln!("unknown command: {command}");
            print_help();
            Ok(2)
        }
    }
}

fn parse_global_args(args: Vec<String>) -> io::Result<(PathBuf, Vec<String>)> {
    let mut root = None;
    let mut rest = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--root" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--root requires a path",
                    ));
                };
                root = Some(PathBuf::from(value));
                index += 2;
            }
            value if value.starts_with("--root=") => {
                root = Some(PathBuf::from(value.trim_start_matches("--root=")));
                index += 1;
            }
            _ => {
                rest.push(args[index].clone());
                index += 1;
            }
        }
    }

    let root = root.unwrap_or(env::current_dir()?);
    Ok((root, rest))
}

fn print_help() {
    println!(
        r#"Plugroot - private-first selfhost harness

Usage:
  plugroot [--root <path>] status [--json]
  plugroot [--root <path>] list
  plugroot [--root <path>] apply [--dry-run]
  plugroot [--root <path>] repos sync
  plugroot [--root <path>] doctor [--json] [--strict]
  plugroot [--root <path>] release-check
  plugroot [--root <path>] up|down|restart|logs <service|all>
  plugroot [--root <path>] tui [--once]
  plugroot [--root <path>] web [--bind <addr:port>]
  plugroot [--root <path>] boundary [--strict]
  plugroot [--root <path>] audit-public [--install-hook]

Files:
  plugroot.toml                         public manifest in the code root
  $PLUGROOT_STATE_ROOT/.env             private local values
  $PLUGROOT_STATE_ROOT/plugroot.local.toml  private local overlay
"#
    );
}

#[derive(Clone)]
struct Context {
    root: PathBuf,
    manifest: Manifest,
    env_values: HashMap<String, String>,
}

impl Context {
    fn load(root: PathBuf) -> io::Result<Self> {
        let mut merged_env: HashMap<String, String> = env::vars().collect();

        for (key, value) in load_env_file(&root.join(".env"))? {
            merged_env.insert(key, value);
        }

        let main_path = root.join("plugroot.toml");
        let mut manifest = load_manifest_file(&main_path, &merged_env)?;
        let state_root = state_root_for_manifest(&root, &manifest);
        for (key, value) in load_env_file(&state_root.join(".env"))? {
            merged_env.insert(key, value);
        }

        manifest = load_manifest_file(&main_path, &merged_env)?;
        let state_root = state_root_for_manifest(&root, &manifest);
        let code_local_path = root.join("plugroot.local.toml");
        let state_local_path = state_root.join("plugroot.local.toml");
        for local_path in [&code_local_path, &state_local_path] {
            if local_path.exists() {
                let local = load_manifest_file(local_path, &merged_env)?;
                manifest = merge_manifest(manifest, local);
            }
        }

        Ok(Self {
            root,
            manifest,
            env_values: merged_env,
        })
    }

    fn service(&self, id: &str) -> Option<Service> {
        self.manifest
            .service
            .iter()
            .find(|service| service.id == id)
            .cloned()
    }

    fn services_for_target(&self, target: &str) -> Vec<Service> {
        if target == "all" {
            self.manifest
                .service
                .iter()
                .filter(|service| {
                    service
                        .controls
                        .as_ref()
                        .map(|controls| !controls.is_empty())
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        } else {
            self.service(target).into_iter().collect()
        }
    }

    fn state_root(&self) -> PathBuf {
        state_root_for_manifest(&self.root, &self.manifest)
    }

    fn repo_dir(&self) -> Option<PathBuf> {
        self.manifest
            .plugroot
            .as_ref()
            .and_then(|config| config.repo_dir.as_deref())
            .map(|path| resolve_path(&self.root, path))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Manifest {
    plugroot: Option<PlugrootConfig>,
    host: Option<HostConfig>,
    #[serde(default)]
    repo: Vec<Repo>,
    #[serde(default)]
    directory: Vec<ManagedDir>,
    #[serde(default)]
    file: Vec<ManagedFile>,
    #[serde(default)]
    service: Vec<Service>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PlugrootConfig {
    code_root: Option<String>,
    root: Option<String>,
    state_root: Option<String>,
    repo_dir: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct HostConfig {
    name: Option<String>,
    private_ip: Option<String>,
    user: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Repo {
    id: String,
    url: String,
    path: String,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ManagedDir {
    path: String,
    owner: Option<String>,
    mode: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ManagedFile {
    source: String,
    target: String,
    owner: Option<String>,
    mode: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Service {
    id: String,
    name: String,
    plane: Option<String>,
    category: Option<String>,
    kind: String,
    path: Option<String>,
    unit: Option<String>,
    user: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    url: Option<String>,
    access: Option<String>,
    description: Option<String>,
    containers: Option<Vec<String>>,
    controls: Option<Vec<String>>,
    ports: Option<Vec<String>>,
    optional: Option<bool>,
    repo: Option<String>,
    working_dir: Option<String>,
    command: Option<Vec<String>>,
    env: Option<Vec<String>>,
    env_file: Option<String>,
    unit_source: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct StatusRow {
    id: String,
    name: String,
    plane: String,
    category: String,
    kind: String,
    state: String,
    detail: String,
    url: Option<String>,
    access: Option<String>,
    description: Option<String>,
    ports: Vec<String>,
    repo: Option<String>,
    controls: Vec<String>,
    optional: bool,
}

#[derive(Debug)]
struct CmdOutput {
    code: i32,
    text: String,
}

#[derive(Debug)]
struct AuditFinding {
    path: String,
    line: Option<usize>,
    message: String,
}

#[derive(Debug, PartialEq, Eq)]
enum BoundarySeverity {
    Error,
    Warning,
}

impl BoundarySeverity {
    fn label(&self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

#[derive(Debug)]
struct BoundaryFinding {
    severity: BoundarySeverity,
    path: Option<PathBuf>,
    message: String,
}

impl BoundaryFinding {
    fn error(path: Option<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            severity: BoundarySeverity::Error,
            path,
            message: message.into(),
        }
    }

    fn warning(path: Option<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            severity: BoundarySeverity::Warning,
            path,
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum DoctorSeverity {
    Ok,
    Info,
    Warn,
    Error,
}

impl DoctorSeverity {
    fn label(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Serialize)]
struct DoctorFinding {
    severity: DoctorSeverity,
    check: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl DoctorFinding {
    fn new(
        severity: DoctorSeverity,
        check: impl Into<String>,
        message: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            severity,
            check: check.into(),
            message: message.into(),
            detail,
        }
    }

    fn ok(check: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(DoctorSeverity::Ok, check, message, None)
    }

    fn info(
        check: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::new(DoctorSeverity::Info, check, message, Some(detail.into()))
    }

    fn warn(
        check: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::new(DoctorSeverity::Warn, check, message, Some(detail.into()))
    }

    fn error(
        check: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::new(DoctorSeverity::Error, check, message, Some(detail.into()))
    }
}

fn cmd_boundary(ctx: &Context, args: &[String]) -> io::Result<u8> {
    let mut strict = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!(
                    r#"Usage:
  plugroot boundary
  plugroot boundary --strict

Checks that the code root is code-only and private state lives outside the
Git checkout. Errors always fail. --strict also fails on warnings.
"#
                );
                return Ok(0);
            }
            "--strict" => strict = true,
            _ => {
                eprintln!("usage: plugroot boundary [--strict]");
                return Ok(2);
            }
        }
    }

    let findings = boundary_findings(ctx)?;
    let code_root = clean_path(&ctx.root);
    let state_root = clean_path(&ctx.state_root());
    let repo_dir = ctx.repo_dir().map(|path| clean_path(&path));

    if findings.is_empty() {
        println!("code root: {}", code_root.display());
        println!("state root: {}", state_root.display());
        if let Some(repo_dir) = &repo_dir {
            println!("repo dir: {}", repo_dir.display());
        }
        println!("boundary: ok");
        return Ok(0);
    }

    let errors = findings
        .iter()
        .filter(|finding| finding.severity == BoundarySeverity::Error)
        .count();
    let warnings = findings.len().saturating_sub(errors);
    eprintln!("code root: {}", code_root.display());
    eprintln!("state root: {}", state_root.display());
    if let Some(repo_dir) = &repo_dir {
        eprintln!("repo dir: {}", repo_dir.display());
    }
    eprintln!("boundary: found {errors} error(s), {warnings} warning(s)");
    for finding in &findings {
        match &finding.path {
            Some(path) => eprintln!(
                "{}: {}: {}",
                finding.severity.label(),
                path.display(),
                finding.message
            ),
            None => eprintln!("{}: {}", finding.severity.label(), finding.message),
        }
    }

    if errors > 0 || (strict && warnings > 0) {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn cmd_audit_public(root: &Path, args: &[String]) -> io::Result<u8> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!(
            r#"Usage:
  plugroot audit-public
  plugroot audit-public --install-hook

Checks tracked files for private paths, common secret markers, private-network
IP leaks, and host-specific denylist terms.
"#
        );
        return Ok(0);
    }

    if args.iter().any(|arg| arg == "--install-hook") {
        install_audit_hook(root)?;
        println!("installed .git/hooks/pre-commit and .git/hooks/pre-push");
    }

    let findings = audit_public(root)?;
    if findings.is_empty() {
        println!("audit-public: ok");
        return Ok(0);
    }

    eprintln!("audit-public: found {} issue(s)", findings.len());
    for finding in &findings {
        match finding.line {
            Some(line) => eprintln!("{}:{}: {}", finding.path, line, finding.message),
            None => eprintln!("{}: {}", finding.path, finding.message),
        }
    }
    Ok(1)
}

fn cmd_doctor(ctx: &Context, args: &[String]) -> io::Result<u8> {
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

fn cmd_release_check(ctx: &Context) -> io::Result<u8> {
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

fn print_doctor_report(findings: &[DoctorFinding]) {
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

fn doctor_findings(ctx: &Context) -> io::Result<Vec<DoctorFinding>> {
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

fn doctor_boundary(ctx: &Context, findings: &mut Vec<DoctorFinding>) -> io::Result<()> {
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

fn doctor_audit(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
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

fn doctor_git(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
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

fn doctor_declared_services(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
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

fn doctor_resources(findings: &mut Vec<DoctorFinding>) {
    doctor_load(findings);
    doctor_memory(findings);
    doctor_root_disk(findings);
}

fn doctor_load(findings: &mut Vec<DoctorFinding>) {
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

fn doctor_memory(findings: &mut Vec<DoctorFinding>) {
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

fn doctor_root_disk(findings: &mut Vec<DoctorFinding>) {
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

fn doctor_failed_units(findings: &mut Vec<DoctorFinding>) {
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

fn doctor_ufw(findings: &mut Vec<DoctorFinding>) {
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

fn doctor_tailscale(findings: &mut Vec<DoctorFinding>) {
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

fn doctor_listeners(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
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

fn doctor_docker(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
    doctor_docker_containers(ctx, findings);
    doctor_docker_networks(findings);
    doctor_docker_volumes(findings);
    doctor_docker_reclaimable(findings);
}

fn doctor_docker_containers(ctx: &Context, findings: &mut Vec<DoctorFinding>) {
    let out = run_command(
        "docker",
        &[
            "ps".into(),
            "-a".into(),
            "--format".into(),
            "{{.Names}}\t{{.Status}}\t{{.Label \"com.docker.compose.project\"}}".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        findings.push(DoctorFinding::info(
            "docker",
            "could not inspect Docker containers",
            out.text,
        ));
        return;
    }

    let expected = expected_container_names(ctx);
    let mut stopped = Vec::new();
    let mut unmanaged = Vec::new();
    for line in out.text.lines().filter(|line| !line.trim().is_empty()) {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        let name = parts.first().copied().unwrap_or("");
        let status = parts.get(1).copied().unwrap_or("");
        let project = parts.get(2).copied().unwrap_or("");
        if !status.starts_with("Up ") {
            stopped.push(format!("{name}: {status}"));
        } else if !expected.is_empty() && !expected.contains(name) {
            unmanaged.push(format!("{name}: {status}, project={project}"));
        }
    }

    if stopped.is_empty() {
        findings.push(DoctorFinding::ok(
            "docker",
            "no stopped Docker containers detected",
        ));
    } else {
        findings.push(DoctorFinding::warn(
            "docker",
            format!("{} stopped Docker container(s) detected", stopped.len()),
            limit_lines(&stopped, 12),
        ));
    }

    if !unmanaged.is_empty() {
        findings.push(DoctorFinding::info(
            "docker",
            format!(
                "{} running Docker container(s) are not declared in Plugroot",
                unmanaged.len()
            ),
            limit_lines(&unmanaged, 12),
        ));
    }
}

fn doctor_docker_networks(findings: &mut Vec<DoctorFinding>) {
    let out = run_command(
        "docker",
        &[
            "network".into(),
            "ls".into(),
            "--format".into(),
            "{{.Name}}\t{{.Driver}}".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        findings.push(DoctorFinding::info(
            "docker-networks",
            "could not inspect Docker networks",
            out.text,
        ));
        return;
    }

    let mut empty_custom = Vec::new();
    for line in out.text.lines().filter(|line| !line.trim().is_empty()) {
        let mut parts = line.split('\t');
        let name = parts.next().unwrap_or("");
        let driver = parts.next().unwrap_or("");
        if matches!(name, "bridge" | "host" | "none") || driver != "bridge" {
            continue;
        }
        let inspected = run_command(
            "docker",
            &[
                "network".into(),
                "inspect".into(),
                "-f".into(),
                "{{len .Containers}}".into(),
                name.into(),
            ],
            None,
            &[],
        );
        if inspected.code == 0 && inspected.text.trim() == "0" {
            empty_custom.push(name.to_string());
        }
    }

    if empty_custom.is_empty() {
        findings.push(DoctorFinding::ok(
            "docker-networks",
            "no empty custom Docker networks detected",
        ));
    } else {
        findings.push(DoctorFinding::warn(
            "docker-networks",
            format!(
                "{} empty custom Docker network(s) detected",
                empty_custom.len()
            ),
            limit_lines(&empty_custom, 12),
        ));
    }
}

fn doctor_docker_volumes(findings: &mut Vec<DoctorFinding>) {
    let out = run_command(
        "docker",
        &[
            "volume".into(),
            "ls".into(),
            "-qf".into(),
            "dangling=true".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        findings.push(DoctorFinding::info(
            "docker-volumes",
            "could not inspect unused Docker volumes",
            out.text,
        ));
        return;
    }
    let volumes: Vec<String> = out
        .text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    if volumes.is_empty() {
        findings.push(DoctorFinding::ok(
            "docker-volumes",
            "no unused Docker volumes detected",
        ));
    } else {
        findings.push(DoctorFinding::warn(
            "docker-volumes",
            format!("{} unused Docker volume(s) detected", volumes.len()),
            limit_lines(&volumes, 12),
        ));
    }
}

fn doctor_docker_reclaimable(findings: &mut Vec<DoctorFinding>) {
    let out = run_command(
        "docker",
        &[
            "system".into(),
            "df".into(),
            "--format".into(),
            "{{json .}}".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        findings.push(DoctorFinding::info(
            "docker-cache",
            "could not inspect Docker reclaimable data",
            out.text,
        ));
        return;
    }

    let mut reclaimable = Vec::new();
    for line in out.text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let item_type = value
            .get("Type")
            .and_then(|value| value.as_str())
            .unwrap_or("Docker data");
        let amount = value
            .get("Reclaimable")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !amount.starts_with("0B") {
            reclaimable.push(format!("{item_type}: {amount} reclaimable"));
        }
    }

    if reclaimable.is_empty() {
        findings.push(DoctorFinding::ok(
            "docker-cache",
            "Docker reports no reclaimable data",
        ));
    } else {
        findings.push(DoctorFinding::info(
            "docker-cache",
            "Docker has reclaimable data",
            limit_lines(&reclaimable, 8),
        ));
    }
}

fn doctor_tmux(findings: &mut Vec<DoctorFinding>) {
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

fn doctor_fuse_mounts(findings: &mut Vec<DoctorFinding>) {
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

fn meminfo_kb(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (name, rest) = line.split_once(':')?;
        if name != key {
            return None;
        }
        rest.split_whitespace().next()?.parse::<u64>().ok()
    })
}

fn broad_ufw_allow_rules(text: &str) -> Vec<String> {
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

fn tailscale_text_has_public_funnel(text: &str) -> bool {
    text.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains("funnel")
            && !lower.contains("funnel off")
            && !lower.contains("funnel is off")
            && !lower.contains("disabled")
    })
}

fn expected_manifest_tcp_ports(ctx: &Context) -> HashSet<u16> {
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

fn add_tcp_ports_from_descriptor(descriptor: &str, ports: &mut HashSet<u16>) {
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

fn add_port_range(range: &str, ports: &mut HashSet<u16>) {
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

fn unexpected_wildcard_tcp_listeners(text: &str, expected_tcp_ports: &HashSet<u16>) -> Vec<String> {
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

fn is_wildcard_listener(local_address: &str) -> bool {
    local_address.starts_with("0.0.0.0:")
        || local_address.starts_with("[::]:")
        || local_address.starts_with("*:")
}

fn listener_port(local_address: &str) -> Option<u16> {
    local_address
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
}

fn expected_container_names(ctx: &Context) -> HashSet<String> {
    ctx.manifest
        .service
        .iter()
        .flat_map(|service| service.containers.as_deref().unwrap_or(&[]))
        .cloned()
        .collect()
}

fn retired_tmux_sessions(text: &str) -> Vec<String> {
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

fn limit_lines(lines: &[String], limit: usize) -> String {
    let mut selected = lines.iter().take(limit).cloned().collect::<Vec<_>>();
    if lines.len() > limit {
        selected.push(format!("... {} more", lines.len() - limit));
    }
    selected.join("\n")
}

fn run_read_command(program: &str, args: &[String]) -> CmdOutput {
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

fn install_audit_hook(root: &Path) -> io::Result<()> {
    let hooks = root.join(".git/hooks");
    fs::create_dir_all(&hooks)?;
    for name in ["pre-commit", "pre-push"] {
        let hook = hooks.join(name);
        fs::write(
            &hook,
            r#"#!/bin/sh
set -eu
repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"
cargo run --quiet -- audit-public
test -f plugroot.toml && cargo run --quiet -- boundary --strict
"#,
        )?;
        ensure_success(run_command(
            "chmod",
            &["0755".into(), hook.display().to_string()],
            None,
            &[],
        ))?;
    }
    Ok(())
}

fn boundary_findings(ctx: &Context) -> io::Result<Vec<BoundaryFinding>> {
    let mut findings = Vec::new();
    let code_root = clean_path(&ctx.root);
    let state_root = clean_path(&ctx.state_root());

    match &ctx.manifest.plugroot {
        Some(config) => {
            if let Some(code_root_value) = &config.code_root {
                let configured = clean_path(&resolve_path(&ctx.root, code_root_value));
                if configured != code_root {
                    findings.push(BoundaryFinding::warning(
                        Some(configured),
                        "configured code_root differs from the active --root checkout",
                    ));
                }
            }
            if config.state_root.is_none() {
                if config.root.is_some() {
                    findings.push(BoundaryFinding::warning(
                        None,
                        "plugroot.root is legacy; use plugroot.state_root for private machine state",
                    ));
                } else {
                    findings.push(BoundaryFinding::warning(
                        None,
                        "plugroot.state_root is not configured; private state falls back to the code root",
                    ));
                }
            }
        }
        None => findings.push(BoundaryFinding::warning(
            None,
            "missing [plugroot] section; private state falls back to the code root",
        )),
    }

    if same_or_inside(&code_root, &state_root) {
        findings.push(BoundaryFinding::error(
            Some(state_root.clone()),
            "state root must live outside the code checkout",
        ));
    }

    if let Some(repo_dir) = ctx.repo_dir() {
        let repo_dir = clean_path(&repo_dir);
        if same_or_inside(&code_root, &repo_dir) {
            findings.push(BoundaryFinding::error(
                Some(repo_dir.clone()),
                "repo_dir must live outside the Plugroot code checkout",
            ));
        }
        if let Some(git_root) = nearest_git_root(&repo_dir) {
            if same_or_inside(&git_root, &repo_dir) {
                findings.push(BoundaryFinding::error(
                    Some(repo_dir),
                    "repo_dir is inside a Git checkout; keep cloned app repos in private state",
                ));
            }
        }
    }

    if let Some(git_root) = nearest_git_root(&state_root) {
        if same_or_inside(&git_root, &state_root) {
            findings.push(BoundaryFinding::error(
                Some(state_root.clone()),
                "state root is inside a Git checkout",
            ));
        }
    }

    for rel in [
        ".env",
        "plugroot.local.toml",
        ".plugroot",
        "repos",
        "data",
        "media",
        "backups",
    ] {
        let path = ctx.root.join(rel);
        if path.exists() {
            findings.push(BoundaryFinding::error(
                Some(path),
                "private runtime path exists inside the code checkout",
            ));
        }
    }

    for path in service_private_paths(&ctx.root) {
        findings.push(BoundaryFinding::error(
            Some(path),
            "private service state exists inside the code checkout",
        ));
    }

    match audit_public(&ctx.root) {
        Ok(audit_findings) => {
            for finding in audit_findings {
                let path = ctx.root.join(&finding.path);
                let message = match finding.line {
                    Some(line) => format!("public audit issue at line {line}: {}", finding.message),
                    None => format!("public audit issue: {}", finding.message),
                };
                findings.push(BoundaryFinding::error(Some(path), message));
            }
        }
        Err(err) => findings.push(BoundaryFinding::warning(
            Some(ctx.root.clone()),
            format!("could not run public audit: {err}"),
        )),
    }

    if nearest_git_root(&ctx.root).is_none() {
        findings.push(BoundaryFinding::warning(
            Some(ctx.root.clone()),
            "code root is not inside a Git checkout; tracked-file auditing is limited",
        ));
    }

    Ok(findings)
}

fn service_private_paths(code_root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let services = code_root.join("services");
    let Ok(entries) = fs::read_dir(services) else {
        return paths;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        for name in [".env", "data", "cache", "config", "secrets"] {
            let path = entry.path().join(name);
            if path.exists() {
                paths.push(path);
            }
        }
    }

    paths
}

fn audit_public(root: &Path) -> io::Result<Vec<AuditFinding>> {
    let files = git_tracked_files(root)?;
    let private_terms = load_audit_denylist(root);
    let mut findings = Vec::new();

    for path in files {
        if let Some(message) = audit_path(&path) {
            findings.push(AuditFinding {
                path,
                line: None,
                message,
            });
            continue;
        }

        let full_path = root.join(&path);
        if fs::metadata(&full_path)
            .map(|metadata| metadata.len() > 1_000_000)
            .unwrap_or(false)
        {
            continue;
        }
        let Ok(text) = fs::read_to_string(&full_path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            for message in audit_line(line, &private_terms) {
                findings.push(AuditFinding {
                    path: path.clone(),
                    line: Some(index + 1),
                    message,
                });
            }
        }
    }

    Ok(findings)
}

fn git_tracked_files(root: &Path) -> io::Result<Vec<String>> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect())
}

fn audit_path(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let name = normalized.rsplit('/').next().unwrap_or(&normalized);
    let denied_exact = [
        "AGENTS.md",
        "plugroot.local.toml",
        ".env",
        ".env.local",
        "id_rsa",
        "id_ed25519",
    ];
    if denied_exact.contains(&normalized.as_str()) || denied_exact.contains(&name) {
        return Some("private file path is tracked".into());
    }
    if name.starts_with(".env.") && name != ".env.example" {
        return Some("private env file is tracked".into());
    }
    if normalized.starts_with("docs/private/")
        || normalized.starts_with(".plugroot/")
        || normalized.starts_with("repos/")
        || normalized.starts_with("backups/")
    {
        return Some("ignored private/runtime path is tracked".into());
    }
    if normalized.contains("/data/")
        || normalized.contains("/cache/")
        || normalized.contains("/config/")
    {
        return Some("service runtime data path is tracked".into());
    }
    if matches!(
        Path::new(name).extension().and_then(|ext| ext.to_str()),
        Some("db" | "sqlite" | "sqlite3" | "log" | "pid" | "pem" | "key" | "p12" | "pfx")
    ) {
        return Some("private state or key-like file is tracked".into());
    }
    None
}

fn load_audit_denylist(root: &Path) -> Vec<String> {
    let mut paths = vec![
        root.join("docs/private/audit-denylist.txt"),
        root.join(".plugroot/audit-denylist.txt"),
    ];
    let state_root = env::var("PLUGROOT_STATE_ROOT").unwrap_or_else(|_| "/var/lib/plugroot".into());
    paths.push(PathBuf::from(state_root).join(".plugroot/audit-denylist.txt"));
    if let Ok(path) = env::var("PLUGROOT_AUDIT_DENYLIST") {
        paths.push(PathBuf::from(path));
    }

    let mut terms = Vec::new();
    for path in paths {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            let term = line.trim();
            if term.is_empty() || term.starts_with('#') {
                continue;
            }
            terms.push(term.to_ascii_lowercase());
        }
    }
    terms
}

fn audit_line(line: &str, private_terms: &[String]) -> Vec<String> {
    let mut findings = Vec::new();
    let lower = line.to_ascii_lowercase();
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return findings;
    }

    if line.contains("-----BEGIN ") && line.contains(&["PRIVATE", " KEY"].concat()) {
        findings.push("private key material".into());
    }
    for marker in token_markers() {
        if contains_token_marker(line, &marker) {
            findings.push(format!("token marker `{marker}`"));
        }
    }
    if contains_tailscale_ipv4(line) {
        findings.push("Tailscale/CGNAT private IP address".into());
    }
    if suspicious_secret_assignment(line) {
        findings.push("non-placeholder secret-looking assignment".into());
    }
    for term in private_terms {
        if !term.is_empty() && lower.contains(term) {
            findings.push("local denylist term".into());
        }
    }

    findings
}

fn token_markers() -> Vec<String> {
    vec![
        ["github", "_pat_"].concat(),
        ["gh", "p_"].concat(),
        ["gh", "o_"].concat(),
        ["gh", "s_"].concat(),
        ["gh", "u_"].concat(),
        ["s", "k-"].concat(),
        ["xo", "xb-"].concat(),
        ["xo", "xp-"].concat(),
    ]
}

fn contains_token_marker(line: &str, marker: &str) -> bool {
    let mut offset = 0;
    while let Some(relative_start) = line[offset..].find(marker) {
        let start = offset + relative_start;
        let suffix_start = start + marker.len();
        let suffix_len = line[suffix_start..]
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
            .count();
        if suffix_len >= token_marker_min_suffix(marker) {
            return true;
        }
        offset = suffix_start;
    }
    false
}

fn token_marker_min_suffix(marker: &str) -> usize {
    let openai_marker = ["s", "k-"].concat();
    let slack_bot_marker = ["xo", "xb-"].concat();
    let slack_user_marker = ["xo", "xp-"].concat();
    if marker == openai_marker || marker == slack_bot_marker || marker == slack_user_marker {
        16
    } else {
        12
    }
}

fn suspicious_secret_assignment(line: &str) -> bool {
    let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) else {
        return false;
    };
    let key = key.trim().to_ascii_lowercase();
    if key.is_empty()
        || key.len() > 80
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return false;
    }
    let secret_keys = [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_key",
        "client_secret",
        "private_key",
    ];
    if !secret_keys.iter().any(|needle| key.contains(needle)) {
        return false;
    }

    let value = value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches(',')
        .trim();
    let allowed = [
        "",
        "example",
        "placeholder",
        "changeme",
        "change-me",
        "redacted",
        "dummy",
        "none",
        "null",
        "false",
        "true",
    ];
    if allowed.contains(&value.to_ascii_lowercase().as_str()) {
        return false;
    }
    !(value.starts_with("${") || value.starts_with('<') || value.starts_with("your-"))
}

fn contains_tailscale_ipv4(line: &str) -> bool {
    line.split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .filter(|token| token.matches('.').count() == 3)
        .any(|token| {
            let octets: Vec<u16> = token
                .split('.')
                .filter_map(|part| part.parse::<u16>().ok())
                .collect();
            octets.len() == 4
                && octets[0] == 100
                && (64..=127).contains(&octets[1])
                && octets.iter().all(|octet| *octet <= 255)
        })
}

fn load_env_file(path: &Path) -> io::Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    let Ok(raw) = fs::read_to_string(path) else {
        return Ok(values);
    };

    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        values.insert(key.trim().to_string(), value);
    }
    Ok(values)
}

fn load_manifest_file(path: &Path, env_values: &HashMap<String, String>) -> io::Result<Manifest> {
    let raw = fs::read_to_string(path)?;
    let expanded = expand_vars(&raw, env_values);
    toml::from_str(&expanded).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn merge_manifest(mut base: Manifest, overlay: Manifest) -> Manifest {
    if overlay.plugroot.is_some() {
        base.plugroot = overlay.plugroot;
    }
    if overlay.host.is_some() {
        base.host = overlay.host;
    }
    merge_by_id(&mut base.repo, overlay.repo, |repo| &repo.id);
    merge_by_id(&mut base.directory, overlay.directory, |directory| {
        &directory.path
    });
    merge_by_id(&mut base.file, overlay.file, |file| &file.target);
    merge_by_id(&mut base.service, overlay.service, |service| &service.id);
    base
}

fn merge_by_id<T, F>(base: &mut Vec<T>, overlay: Vec<T>, id: F)
where
    F: Fn(&T) -> &str,
{
    for item in overlay {
        if let Some(index) = base.iter().position(|existing| id(existing) == id(&item)) {
            base[index] = item;
        } else {
            base.push(item);
        }
    }
}

fn expand_vars(input: &str, values: &HashMap<String, String>) -> String {
    let mut output = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            if let Some(end) = chars[index + 2..].iter().position(|ch| *ch == '}') {
                let body: String = chars[index + 2..index + 2 + end].iter().collect();
                let replacement =
                    expand_var_body(&body, values).unwrap_or_else(|| format!("${{{body}}}"));
                output.push_str(&replacement);
                index += end + 3;
                continue;
            }
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}

fn expand_var_body(body: &str, values: &HashMap<String, String>) -> Option<String> {
    if let Some((key, default)) = body.split_once(":-") {
        return values
            .get(key)
            .filter(|value| !value.is_empty())
            .cloned()
            .or_else(|| Some(default.to_string()));
    }
    values.get(body).cloned()
}

fn cmd_status(ctx: &Context, args: &[String]) -> io::Result<u8> {
    let json = args.iter().any(|arg| arg == "--json");
    let rows = status_rows(ctx);
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        print_status_table(&rows);
    }
    Ok(0)
}

fn cmd_list(ctx: &Context) -> io::Result<u8> {
    for service in &ctx.manifest.service {
        println!(
            "{:<18} {:<13} {}",
            service.id,
            service.kind,
            service
                .url
                .as_deref()
                .or(service.access.as_deref())
                .unwrap_or("-")
        );
    }
    Ok(0)
}

fn print_status_table(rows: &[StatusRow]) {
    println!(
        "{:<9} {:<8} {:<13} {:<22} DETAIL",
        "STATE", "PLANE", "KIND", "SERVICE"
    );
    for row in rows {
        println!(
            "{:<9} {:<8} {:<13} {:<22} {}",
            row.state, row.plane, row.kind, row.name, row.detail
        );
    }
}

fn status_rows(ctx: &Context) -> Vec<StatusRow> {
    ctx.manifest
        .service
        .iter()
        .map(|service| {
            let (state, detail) = service_status(ctx, service);
            StatusRow {
                id: service.id.clone(),
                name: service.name.clone(),
                plane: service.plane.clone().unwrap_or_else(|| "private".into()),
                category: service.category.clone().unwrap_or_else(|| "-".into()),
                kind: service.kind.clone(),
                state,
                detail,
                url: service.url.clone(),
                access: service.access.clone(),
                description: service.description.clone(),
                ports: service.ports.clone().unwrap_or_default(),
                repo: service.repo.clone(),
                controls: service.controls.clone().unwrap_or_default(),
                optional: service.optional.unwrap_or(false),
            }
        })
        .collect()
}

fn service_status(ctx: &Context, service: &Service) -> (String, String) {
    match service.kind.as_str() {
        "compose" => compose_status(service),
        "systemd" => systemd_status(service.unit.as_deref().unwrap_or("")),
        "user-systemd" => user_systemd_status(
            service.user.as_deref().unwrap_or(""),
            service.unit.as_deref().unwrap_or(""),
        ),
        "port" | "manual" => port_status(
            service.host.as_deref().unwrap_or("127.0.0.1"),
            service.port.unwrap_or(0),
        ),
        _ => {
            let _ = ctx;
            ("unknown".into(), format!("unknown kind {}", service.kind))
        }
    }
}

fn compose_status(service: &Service) -> (String, String) {
    let containers = service.containers.clone().unwrap_or_default();
    if containers.is_empty() {
        return ("unknown".into(), "no containers listed".into());
    }

    let mut online = 0usize;
    let mut details = Vec::new();
    for container in &containers {
        let out = run_command(
            "docker",
            &[
                "inspect".into(),
                "--format".into(),
                "{{json .State}}".into(),
                container.clone(),
            ],
            None,
            &[],
        );
        if out.code != 0 {
            details.push(format!("{container}: missing"));
            continue;
        }
        let state: serde_json::Value = serde_json::from_str(&out.text).unwrap_or_default();
        let running = state
            .get("Running")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if running {
            online += 1;
            let health = state
                .get("Health")
                .and_then(|health| health.get("Status"))
                .and_then(|value| value.as_str());
            match health {
                Some(value) => details.push(format!("{container}: {value}")),
                None => details.push(format!("{container}: running")),
            }
        } else {
            let status = state
                .get("Status")
                .and_then(|value| value.as_str())
                .unwrap_or("stopped");
            details.push(format!("{container}: {status}"));
        }
    }

    let state = if online == containers.len() {
        "online"
    } else if online > 0 {
        "partial"
    } else {
        "offline"
    };
    (state.into(), details.join(", "))
}

fn systemd_status(unit: &str) -> (String, String) {
    if unit.is_empty() {
        return ("unknown".into(), "missing unit".into());
    }
    let active = run_command("systemctl", &["is-active".into(), unit.into()], None, &[]);
    let enabled = run_command("systemctl", &["is-enabled".into(), unit.into()], None, &[]);
    let active_text = active.text.lines().next().unwrap_or("unknown").to_string();
    let enabled_text = enabled.text.lines().next().unwrap_or("unknown").to_string();
    let state = if active.code == 0 {
        "online"
    } else if active_text == "inactive" || active_text == "failed" {
        "offline"
    } else {
        "unknown"
    };
    (state.into(), format!("{active_text}, {enabled_text}"))
}

fn user_systemd_status(user: &str, unit: &str) -> (String, String) {
    if unit.is_empty() {
        return ("unknown".into(), "missing unit".into());
    }
    let Some(prefix) = user_systemd_prefix(user) else {
        return ("unknown".into(), format!("user {user} not found"));
    };
    let active = run_prefixed_command(
        &prefix,
        "systemctl",
        &["--user".into(), "is-active".into(), unit.into()],
    );
    let enabled = run_prefixed_command(
        &prefix,
        "systemctl",
        &["--user".into(), "is-enabled".into(), unit.into()],
    );
    let active_text = active.text.lines().next().unwrap_or("unknown").to_string();
    let enabled_text = enabled.text.lines().next().unwrap_or("unknown").to_string();
    let state = if active.code == 0 {
        "online"
    } else if active_text == "inactive" || active_text == "failed" {
        "offline"
    } else if active_text.contains("not-found") || active_text.contains("not found") {
        "missing"
    } else {
        "unknown"
    };
    (state.into(), format!("{active_text}, {enabled_text}"))
}

fn port_status(host: &str, port: u16) -> (String, String) {
    if port == 0 {
        return ("unknown".into(), "missing port".into());
    }
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return ("unknown".into(), format!("{host}:{port} did not resolve"));
    };
    let Some(addr) = addrs.find(|addr| matches!(addr, SocketAddr::V4(_))) else {
        return (
            "unknown".into(),
            format!("{host}:{port} has no IPv4 address"),
        );
    };
    match TcpStream::connect_timeout(&addr, Duration::from_millis(700)) {
        Ok(_) => (
            "online".into(),
            format!("{host}:{port} accepts connections"),
        ),
        Err(err) => (
            "offline".into(),
            format!("{host}:{port} closed ({})", err.kind()),
        ),
    }
}

fn cmd_apply(ctx: &Context, args: &[String]) -> io::Result<u8> {
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let state_root = ctx.state_root();
    let generated_root = state_root.join(".plugroot/generated");
    println!("code root: {}", ctx.root.display());
    println!("state root: {}", state_root.display());
    if let Some(repo_dir) = ctx.repo_dir() {
        println!("repo dir: {}", repo_dir.display());
    }
    if dry_run {
        println!("dry run: no files or services will be changed");
    } else {
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
        println!(
            "{} repo {} -> {}",
            if dry_run { "would sync" } else { "syncing" },
            repo.id,
            repo.path
        );
        if !dry_run {
            sync_repo(repo)?;
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

fn cmd_repos(ctx: &Context, args: &[String]) -> io::Result<u8> {
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

fn sync_repo(repo: &Repo) -> io::Result<()> {
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

fn apply_directory(directory: &ManagedDir) -> io::Result<()> {
    fs::create_dir_all(&directory.path)?;
    apply_owner_mode(
        Path::new(&directory.path),
        directory.owner.as_deref(),
        directory.mode.as_deref(),
    )
}

fn apply_file(ctx: &Context, file: &ManagedFile) -> io::Result<()> {
    let source = resolve_path(&ctx.root, &file.source);
    let target = PathBuf::from(&file.target);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&source, &target)?;
    apply_owner_mode(&target, file.owner.as_deref(), file.mode.as_deref())
}

fn apply_owner_mode(path: &Path, owner: Option<&str>, mode: Option<&str>) -> io::Result<()> {
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

fn generate_unit(ctx: &Context, service: &Service) -> io::Result<Option<String>> {
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

fn systemd_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "/:._-=@".contains(ch))
    {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn install_unit(service: &Service, generated_path: &Path) -> io::Result<()> {
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

fn ensure_success(out: CmdOutput) -> io::Result<()> {
    if out.code == 0 {
        return Ok(());
    }
    Err(io::Error::other(out.text))
}

fn cmd_action(ctx: &Context, action: &str, args: &[String]) -> io::Result<u8> {
    let Some(target) = args.first() else {
        eprintln!("missing service target");
        return Ok(2);
    };
    let services = ctx.services_for_target(target);
    if services.is_empty() {
        eprintln!("unknown service: {target}");
        return Ok(2);
    }

    let mut final_code = 0u8;
    for service in services {
        if !has_control(&service, action) {
            eprintln!("{} does not allow `{action}`", service.id);
            final_code = 2;
            continue;
        }
        println!("==> {} {}", service.id, action);
        let out = if action == "logs" {
            logs_for(ctx, &service)
        } else {
            run_fixed_action(ctx, &service, action)
        };
        print_cmd_output(&out);
        if out.code != 0 {
            final_code = out.code.min(255) as u8;
        }
    }
    Ok(final_code)
}

fn has_control(service: &Service, action: &str) -> bool {
    service
        .controls
        .as_ref()
        .map(|controls| controls.iter().any(|control| control == action))
        .unwrap_or(false)
}

fn compose_args(ctx: &Context, service: &Service) -> Option<Vec<String>> {
    let path = service.path.as_ref()?;
    let dir = resolve_path(&ctx.root, path);
    let mut args = vec!["compose".into()];
    let env_file = service
        .env_file
        .as_deref()
        .map(|path| resolve_path(&ctx.root, path))
        .or_else(|| {
            let path = ctx.state_root().join(".env");
            path.exists().then_some(path)
        })
        .or_else(|| {
            let path = ctx.root.join(".env");
            path.exists().then_some(path)
        });
    if let Some(env_file) = env_file {
        args.extend(["--env-file".into(), env_file.display().to_string()]);
    }
    args.extend([
        "--project-directory".into(),
        dir.display().to_string(),
        "-f".into(),
        dir.join("compose.yaml").display().to_string(),
    ]);
    Some(args)
}

fn logs_for(ctx: &Context, service: &Service) -> CmdOutput {
    match service.kind.as_str() {
        "compose" => {
            let Some(mut args) = compose_args(ctx, service) else {
                return CmdOutput {
                    code: 2,
                    text: "compose service is missing path".into(),
                };
            };
            args.extend([
                "logs".into(),
                "--tail".into(),
                "200".into(),
                "--no-color".into(),
            ]);
            run_command("docker", &args, None, &[])
        }
        "systemd" => run_command(
            "journalctl",
            &[
                "-u".into(),
                service.unit.clone().unwrap_or_default(),
                "-n".into(),
                "200".into(),
                "--no-pager".into(),
            ],
            None,
            &[],
        ),
        "user-systemd" => {
            let Some(prefix) = user_systemd_prefix(service.user.as_deref().unwrap_or("")) else {
                return CmdOutput {
                    code: 2,
                    text: "user not found".into(),
                };
            };
            run_prefixed_command(
                &prefix,
                "journalctl",
                &[
                    "--user".into(),
                    "-u".into(),
                    service.unit.clone().unwrap_or_default(),
                    "-n".into(),
                    "200".into(),
                    "--no-pager".into(),
                ],
            )
        }
        _ => CmdOutput {
            code: 2,
            text: "logs unavailable for this service type".into(),
        },
    }
}

fn run_fixed_action(ctx: &Context, service: &Service, action: &str) -> CmdOutput {
    match service.kind.as_str() {
        "compose" => {
            let Some(mut args) = compose_args(ctx, service) else {
                return CmdOutput {
                    code: 2,
                    text: "compose service is missing path".into(),
                };
            };
            let compose_action = match action {
                "start" => vec!["up".into(), "-d".into()],
                "stop" => vec!["down".into()],
                "restart" => vec!["restart".into()],
                _ => {
                    return CmdOutput {
                        code: 2,
                        text: format!("unsupported action {action}"),
                    }
                }
            };
            args.extend(compose_action);
            run_command("docker", &args, None, &[])
        }
        "systemd" => {
            let systemd_action = match action {
                "start" => "start",
                "stop" => "stop",
                "restart" => "restart",
                _ => {
                    return CmdOutput {
                        code: 2,
                        text: format!("unsupported action {action}"),
                    }
                }
            };
            run_command(
                "systemctl",
                &[
                    systemd_action.into(),
                    service.unit.clone().unwrap_or_default(),
                ],
                None,
                &[],
            )
        }
        "user-systemd" => {
            let Some(prefix) = user_systemd_prefix(service.user.as_deref().unwrap_or("")) else {
                return CmdOutput {
                    code: 2,
                    text: "user not found".into(),
                };
            };
            let systemd_action = match action {
                "start" => "start",
                "stop" => "stop",
                "restart" => "restart",
                _ => {
                    return CmdOutput {
                        code: 2,
                        text: format!("unsupported action {action}"),
                    }
                }
            };
            run_prefixed_command(
                &prefix,
                "systemctl",
                &[
                    "--user".into(),
                    systemd_action.into(),
                    service.unit.clone().unwrap_or_default(),
                ],
            )
        }
        _ => CmdOutput {
            code: 2,
            text: "this service type has no fixed action".into(),
        },
    }
}

fn resolve_path(root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn state_root_for_manifest(code_root: &Path, manifest: &Manifest) -> PathBuf {
    manifest
        .plugroot
        .as_ref()
        .and_then(|config| config.state_root.as_deref().or(config.root.as_deref()))
        .map(|path| resolve_path(code_root, path))
        .unwrap_or_else(|| code_root.to_path_buf())
}

fn clean_path(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            Component::Normal(part) => clean.push(part),
            Component::RootDir | Component::Prefix(_) => clean.push(component.as_os_str()),
        }
    }
    clean
}

fn same_or_inside(parent: &Path, child: &Path) -> bool {
    let parent = clean_path(parent);
    let child = clean_path(child);
    child == parent || child.starts_with(parent)
}

fn nearest_git_root(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };

    loop {
        if current.join(".git").exists() {
            return Some(clean_path(&current));
        }
        if !current.pop() {
            return None;
        }
    }
}

fn git_dirty_details(path: &Path) -> io::Result<Option<String>> {
    if nearest_git_root(path).is_none() {
        return Ok(None);
    }
    let out = run_command(
        "git",
        &[
            "-C".into(),
            path.display().to_string(),
            "status".into(),
            "--porcelain=v1".into(),
        ],
        None,
        &[],
    );
    if out.code != 0 {
        return Err(io::Error::other(out.text));
    }
    let lines = out
        .text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        Ok(None)
    } else {
        Ok(Some(limit_lines(&lines, 20)))
    }
}

fn run_command(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    envs: &[(&str, &str)],
) -> CmdOutput {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    for (key, value) in envs {
        cmd.env(key, value);
    }
    match cmd.output() {
        Ok(output) => {
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            CmdOutput {
                code: output.status.code().unwrap_or(1),
                text: text.trim().to_string(),
            }
        }
        Err(err) => CmdOutput {
            code: 127,
            text: err.to_string(),
        },
    }
}

fn run_prefixed_command(prefix: &[String], program: &str, args: &[String]) -> CmdOutput {
    if prefix.is_empty() {
        return run_command(program, args, None, &[]);
    }
    let mut full_args = prefix[1..].to_vec();
    full_args.push(program.into());
    full_args.extend(args.iter().cloned());
    run_command(&prefix[0], &full_args, None, &[])
}

fn print_cmd_output(out: &CmdOutput) {
    if !out.text.is_empty() {
        println!("{}", out.text);
    }
}

fn command_text(args: &[&str]) -> Option<String> {
    let output = Command::new(args[0]).args(&args[1..]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn user_systemd_prefix(user: &str) -> Option<Vec<String>> {
    let uid = command_text(&["id", "-u", user])?;
    let home = user_home(user).unwrap_or_else(|| format!("/home/{user}"));
    let env_args = vec![
        "env".into(),
        format!("XDG_RUNTIME_DIR=/run/user/{uid}"),
        format!("DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{uid}/bus"),
        format!("HOME={home}"),
    ];
    if command_text(&["id", "-u"]).as_deref() == Some(uid.as_str()) {
        return Some(env_args);
    }
    let mut args = vec!["runuser".into(), "-u".into(), user.into(), "--".into()];
    args.extend(env_args);
    Some(args)
}

fn user_home(user: &str) -> Option<String> {
    command_text(&["getent", "passwd", user]).and_then(|line| {
        line.split(':')
            .nth(5)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn is_root() -> bool {
    command_text(&["id", "-u"]).as_deref() == Some("0")
}

fn cmd_tui(ctx: Context, args: &[String]) -> io::Result<u8> {
    if args.iter().any(|arg| arg == "--once") {
        print_status_table(&status_rows(&ctx));
        return Ok(0);
    }

    let mut app = TuiApp::new(ctx);
    let mut term = TerminalGuard::enter()?;
    loop {
        draw_tui(&mut term.stdout, &mut app)?;
        if event::poll(Duration::from_millis(500))? {
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Down | KeyCode::Char('j') => app.down(),
                    KeyCode::Up | KeyCode::Char('k') => app.up(),
                    KeyCode::Right => app.right(),
                    KeyCode::Left => app.left(),
                    KeyCode::PageDown => app.page_down(tui_list_capacity(terminal::size()?.1)),
                    KeyCode::PageUp => app.page_up(tui_list_capacity(terminal::size()?.1)),
                    KeyCode::Home => app.home(),
                    KeyCode::End => app.end(),
                    KeyCode::Char('r') => app.refresh(),
                    KeyCode::Char('l') => app.action("logs"),
                    KeyCode::Char('o') => app.action("start"),
                    KeyCode::Char('e') => app.action("restart"),
                    KeyCode::Char('f') => app.action("stop"),
                    _ => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    Ok(0)
}

struct TuiApp {
    ctx: Context,
    rows: Vec<StatusRow>,
    selected: usize,
    scroll_x: usize,
    scroll_y: usize,
    message: String,
    last_refresh: Instant,
}

impl TuiApp {
    fn new(ctx: Context) -> Self {
        let rows = status_rows(&ctx);
        Self {
            ctx,
            rows,
            selected: 0,
            scroll_x: 0,
            scroll_y: 0,
            message: String::new(),
            last_refresh: Instant::now(),
        }
    }

    fn refresh(&mut self) {
        let selected_id = self.rows.get(self.selected).map(|row| row.id.clone());
        let selected_index = self.selected;
        match Context::load(self.ctx.root.clone()) {
            Ok(ctx) => self.ctx = ctx,
            Err(err) => {
                self.message = format!("refresh failed: {err}");
                return;
            }
        }
        self.rows = status_rows(&self.ctx);
        self.restore_selection(selected_id.as_deref(), selected_index);
        self.last_refresh = Instant::now();
    }

    fn restore_selection(&mut self, selected_id: Option<&str>, selected_index: usize) {
        if self.rows.is_empty() {
            self.selected = 0;
            self.scroll_y = 0;
            return;
        }
        if let Some(id) = selected_id {
            if let Some(index) = self.rows.iter().position(|row| row.id == id) {
                self.selected = index;
                return;
            }
        }
        self.selected = selected_index.min(self.rows.len().saturating_sub(1));
    }

    fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn down(&mut self) {
        if self.selected + 1 < self.rows.len() {
            self.selected += 1;
        }
    }

    fn right(&mut self) {
        self.scroll_x = self.scroll_x.saturating_add(8);
    }

    fn left(&mut self) {
        self.scroll_x = self.scroll_x.saturating_sub(8);
    }

    fn page_down(&mut self, visible_rows: usize) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = (self.selected + visible_rows.max(1)).min(self.rows.len() - 1);
    }

    fn page_up(&mut self, visible_rows: usize) {
        self.selected = self.selected.saturating_sub(visible_rows.max(1));
    }

    fn home(&mut self) {
        self.selected = 0;
        self.scroll_y = 0;
        self.scroll_x = 0;
    }

    fn end(&mut self) {
        if !self.rows.is_empty() {
            self.selected = self.rows.len() - 1;
        }
    }

    fn clamp_viewport(&mut self, visible_rows: usize, width: usize) {
        if self.rows.is_empty() {
            self.selected = 0;
            self.scroll_y = 0;
            self.scroll_x = 0;
            return;
        }

        let visible_rows = visible_rows.max(1);
        self.selected = self.selected.min(self.rows.len() - 1);
        if self.selected < self.scroll_y {
            self.scroll_y = self.selected;
        } else if self.selected >= self.scroll_y + visible_rows {
            self.scroll_y = self.selected + 1 - visible_rows;
        }
        self.scroll_y = self
            .scroll_y
            .min(self.rows.len().saturating_sub(visible_rows));
        self.scroll_x = self
            .scroll_x
            .min(tui_content_width(self).saturating_sub(width));
    }

    fn selected_service(&self) -> Option<Service> {
        self.rows
            .get(self.selected)
            .and_then(|row| self.ctx.service(&row.id))
    }

    fn action(&mut self, action: &str) {
        let Some(service) = self.selected_service() else {
            return;
        };
        if !has_control(&service, action) {
            self.message = format!("{} does not allow `{action}`", service.name);
            return;
        }
        let output = if action == "logs" {
            logs_for(&self.ctx, &service)
        } else {
            run_fixed_action(&self.ctx, &service, action)
        };
        self.message = format!(
            "{} {}: exit {}\n{}",
            service.name,
            action,
            output.code,
            trim_output(&output.text, 2200)
        );
        self.refresh();
    }
}

struct TerminalGuard {
    stdout: Stdout,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        let mut stdout = io::stdout();
        terminal::enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, Hide)?;
        Ok(Self { stdout })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, Show, LeaveAlternateScreen, ResetColor);
        let _ = terminal::disable_raw_mode();
    }
}

fn draw_tui(stdout: &mut Stdout, app: &mut TuiApp) -> io::Result<()> {
    let (terminal_width, height) = terminal::size()?;
    let width = terminal_width as usize;
    let available = tui_list_capacity(height);
    app.clamp_viewport(available, width);

    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
    let online = app.rows.iter().filter(|row| row.state == "online").count();
    let name = app
        .ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.name.as_deref())
        .unwrap_or("plugroot");
    let ip = app
        .ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.private_ip.as_deref())
        .unwrap_or("unknown");
    let user = app
        .ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.user.as_deref())
        .unwrap_or("-");
    queue!(
        stdout,
        SetAttribute(Attribute::Bold),
        Print(fit(
            &format!("Plugroot - {name} - {ip} - user {user}"),
            width
        )),
        SetAttribute(Attribute::Reset),
        MoveTo(0, 1),
        Print(fit(
            &format!(
                "{} online, {} attention, {} tracked - arrows move/pan - r refresh - l logs - o on - e restart - f off - q quit",
                online,
                app.rows.len().saturating_sub(online),
                app.rows.len()
            ),
            width
        )),
        MoveTo(0, 2),
        Print(line(terminal_width))
    )?;

    let list_top = 3u16;
    let visible = visible_rows(app, available);
    queue!(
        stdout,
        MoveTo(0, list_top),
        SetForegroundColor(Color::DarkGrey),
        Print(fit_window(
            "  STATE    PLANE    CATEGORY/KIND  SERVICE                         ACCESS",
            width,
            app.scroll_x
        )),
        ResetColor
    )?;
    for (row_index, row) in app.rows[visible.clone()].iter().enumerate() {
        let actual = visible.start + row_index;
        let y = list_top + 1 + row_index as u16;
        let marker = if actual == app.selected { ">" } else { " " };
        let category_kind = format!("{}/{}", row.category, row.kind);
        let access = row.url.as_deref().or(row.access.as_deref()).unwrap_or("-");
        let text = tui_row_text(marker, row, &category_kind, access);
        queue!(stdout, MoveTo(0, y))?;
        if actual == app.selected {
            queue!(stdout, SetAttribute(Attribute::Reverse))?;
        }
        queue!(
            stdout,
            SetForegroundColor(color_for_state(&row.state)),
            Print(fit_window(&text, width, app.scroll_x)),
            ResetColor
        )?;
        if actual == app.selected {
            queue!(stdout, SetAttribute(Attribute::Reset))?;
        }
    }

    let base = height.saturating_sub(5);
    queue!(stdout, MoveTo(0, base), Print(line(terminal_width)))?;
    if let Some(row) = app.rows.get(app.selected) {
        let access = row.url.as_deref().or(row.access.as_deref()).unwrap_or("-");
        queue!(
            stdout,
            MoveTo(0, base + 1),
            SetAttribute(Attribute::Bold),
            Print(fit(&format!("{} ({})", row.name, row.id), width)),
            SetAttribute(Attribute::Reset),
            MoveTo(0, base + 2),
            Print(fit(
                row.description.as_deref().unwrap_or(&row.detail),
                width
            )),
            MoveTo(0, base + 3),
            Print(fit_window(
                &tui_detail_text(row, access),
                width,
                app.scroll_x
            )),
            MoveTo(0, base + 4),
            Print(fit_window(&app.message, width, app.scroll_x))
        )?;
    }
    stdout.flush()?;
    Ok(())
}

fn tui_list_capacity(height: u16) -> usize {
    let list_top = 3u16;
    let footer_start = height.saturating_sub(5);
    footer_start.saturating_sub(list_top + 1) as usize
}

fn visible_rows(app: &TuiApp, available: usize) -> Range<usize> {
    let end = (app.scroll_y + available).min(app.rows.len());
    app.scroll_y..end
}

fn tui_row_text(marker: &str, row: &StatusRow, category_kind: &str, access: &str) -> String {
    format!(
        "{marker} {:<8} {:<8} {:<14} {:<30} {}",
        row.state, row.plane, category_kind, row.name, access
    )
}

fn tui_content_width(app: &TuiApp) -> usize {
    let mut max_width =
        visible_width("  STATE    PLANE    CATEGORY/KIND  SERVICE                         ACCESS");
    for row in &app.rows {
        let category_kind = format!("{}/{}", row.category, row.kind);
        let access = row.url.as_deref().or(row.access.as_deref()).unwrap_or("-");
        max_width = max_width.max(visible_width(&tui_row_text(
            ">",
            row,
            &category_kind,
            access,
        )));
        max_width = max_width.max(visible_width(&tui_detail_text(row, access)));
    }
    max_width
}

fn tui_detail_text(row: &StatusRow, access: &str) -> String {
    format!(
        "access: {} | detail: {} | ports: {} | controls: {}",
        access,
        row.detail,
        if row.ports.is_empty() {
            "none".into()
        } else {
            row.ports.join(",")
        },
        if row.controls.is_empty() {
            "none".into()
        } else {
            row.controls.join(",")
        }
    )
}

fn color_for_state(state: &str) -> Color {
    match state {
        "online" => Color::Green,
        "offline" | "missing" => Color::Red,
        "partial" | "unknown" => Color::Yellow,
        _ => Color::White,
    }
}

fn line(width: u16) -> String {
    "-".repeat(width as usize)
}

fn fit(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut text = value.replace('\n', " ");
    if text.chars().count() > width {
        text = text
            .chars()
            .take(width.saturating_sub(1))
            .collect::<String>();
        text.push('~');
    }
    format!("{text:<width$}")
}

fn fit_window(value: &str, width: usize, offset: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let text = value.replace('\n', " ");
    let chars: Vec<char> = text.chars().collect();
    let offset = offset.min(chars.len().saturating_sub(1));
    let mut window: Vec<char> = chars.iter().skip(offset).take(width).copied().collect();
    let has_left = offset > 0;
    let has_right = offset + width < chars.len();

    if has_left && !window.is_empty() {
        window[0] = '<';
    }
    if has_right && !window.is_empty() {
        let last = window.len() - 1;
        window[last] = '>';
    }

    let text: String = window.into_iter().collect();
    format!("{text:<width$}")
}

fn visible_width(value: &str) -> usize {
    value.replace('\n', " ").chars().count()
}

fn trim_output(value: &str, max_chars: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let tail: String = value
        .chars()
        .rev()
        .take(max_chars)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("[truncated]\n{tail}")
}

fn cmd_web(ctx: Context, args: &[String]) -> io::Result<u8> {
    let mut bind = ctx
        .env_values
        .get("PLUGROOT_WEB_BIND")
        .cloned()
        .or_else(|| env::var("PLUGROOT_WEB_BIND").ok())
        .unwrap_or_else(|| "127.0.0.1:8786".into());
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--bind" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--bind requires addr:port",
                    ));
                };
                bind = value.clone();
                index += 2;
            }
            value if value.starts_with("--bind=") => {
                bind = value.trim_start_matches("--bind=").to_string();
                index += 1;
            }
            _ => index += 1,
        }
    }

    let listener = TcpListener::bind(&bind)?;
    println!("Plugroot web listening on http://{bind}");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let _ = handle_http(ctx, stream);
                });
            }
            Err(err) => eprintln!("connection error: {err}"),
        }
    }
    Ok(0)
}

fn handle_http(ctx: Context, mut stream: TcpStream) -> io::Result<()> {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf)?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let Some(first_line) = request.lines().next() else {
        return Ok(());
    };
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0];
    let path = parts[1];

    if !web_authorized(&ctx, &request) {
        return http_unauthorized(&mut stream);
    }

    if method == "GET" && path == "/" {
        let body = render_web(&ctx);
        return http_response(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            body.as_bytes(),
        );
    }
    if method == "GET" && path == "/api/status" {
        let body = serde_json::to_vec_pretty(&status_rows(&ctx))?;
        return http_response(&mut stream, 200, "application/json", &body);
    }
    if method == "POST" && path.starts_with("/api/action/") {
        let segments: Vec<&str> = path.trim_start_matches("/api/action/").split('/').collect();
        if segments.len() != 2 {
            return http_response(&mut stream, 400, "text/plain", b"bad action path");
        }
        let id = segments[0];
        let action = segments[1];
        let Some(service) = ctx.service(id) else {
            return http_response(&mut stream, 404, "text/plain", b"unknown service");
        };
        if !has_control(&service, action) {
            return http_response(&mut stream, 403, "text/plain", b"action not allowed");
        }
        let out = if action == "logs" {
            logs_for(&ctx, &service)
        } else {
            run_fixed_action(&ctx, &service, action)
        };
        let code = if out.code == 0 { 200 } else { 500 };
        return http_response(&mut stream, code, "text/plain", out.text.as_bytes());
    }
    http_response(&mut stream, 404, "text/plain", b"not found")
}

fn web_authorized(ctx: &Context, request: &str) -> bool {
    let password = ctx
        .env_values
        .get("PLUGROOT_WEB_PASSWORD")
        .filter(|value| !value.is_empty());
    let Some(password) = password else {
        return true;
    };
    let user = ctx
        .env_values
        .get("PLUGROOT_WEB_USER")
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .unwrap_or("plugroot");
    let expected = format!("Basic {}", BASE64.encode(format!("{user}:{password}")));
    request.lines().any(|line| {
        line.strip_prefix("Authorization:")
            .map(str::trim)
            .is_some_and(|value| value == expected)
    })
}

fn http_unauthorized(stream: &mut TcpStream) -> io::Result<()> {
    let body = b"authentication required";
    write!(
        stream,
        "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nWWW-Authenticate: Basic realm=\"Plugroot\"\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn http_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn render_web(ctx: &Context) -> String {
    let rows = status_rows(ctx);
    let total = rows.len();
    let online = rows.iter().filter(|row| row.state == "online").count();
    let attention = rows.iter().filter(|row| web_needs_attention(row)).count();
    let optional_idle = rows
        .iter()
        .filter(|row| row.optional && row.state != "online" && !web_needs_attention(row))
        .count();
    let planes = rows
        .iter()
        .map(|row| row.plane.as_str())
        .collect::<HashSet<_>>()
        .len();
    let categories = rows
        .iter()
        .filter(|row| row.category != "-")
        .map(|row| row.category.as_str())
        .collect::<HashSet<_>>()
        .len();

    let host_name = ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.name.as_deref())
        .unwrap_or("plugroot-host");
    let private_ip = ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.private_ip.as_deref())
        .unwrap_or("127.0.0.1");
    let state_root = clean_path(&ctx.state_root()).display().to_string();

    let mut grouped: BTreeMap<(String, String), Vec<&StatusRow>> = BTreeMap::new();
    for row in &rows {
        grouped
            .entry((row.plane.clone(), web_category_label(row)))
            .or_default()
            .push(row);
    }

    let groups = if grouped.is_empty() {
        r#"<section class="empty"><h2>No Services</h2><p>Add service entries to plugroot.toml or the private overlay to populate this overview.</p></section>"#
            .into()
    } else {
        grouped
            .into_iter()
            .map(|((plane, category), mut group_rows)| {
                group_rows.sort_by(|left, right| {
                    web_needs_attention(right)
                        .cmp(&web_needs_attention(left))
                        .then_with(|| left.name.cmp(&right.name))
                });
                let cards = group_rows
                    .iter()
                    .map(|row| render_web_service(row))
                    .collect::<Vec<_>>()
                    .join("");
                let group_online = group_rows
                    .iter()
                    .filter(|row| row.state == "online")
                    .count();
                let group_attention = group_rows
                    .iter()
                    .filter(|row| web_needs_attention(row))
                    .count();
                format!(
                    r#"<section class="group"><div class="group-head"><div><span class="eyebrow">{}</span><h2>{}</h2></div><p>{} service(s), {} online, {} attention</p></div><div class="grid">{}</div></section>"#,
                    html_escape(&plane),
                    html_escape(&category),
                    group_rows.len(),
                    group_online,
                    group_attention,
                    cards
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Plugroot</title>
<style>
:root{{color-scheme:dark;--bg:#101113;--panel:#181a1d;--panel-2:#202327;--line:#30343a;--text:#f3f1eb;--muted:#a9adb4;--ok:#7acb7a;--warn:#e1b65d;--bad:#ef767a;--info:#70a7d9}}
*{{box-sizing:border-box}}
body{{margin:0;font:14px system-ui,sans-serif;background:var(--bg);color:var(--text);letter-spacing:0}}
a{{color:inherit}}
header{{display:flex;justify-content:space-between;align-items:center;gap:16px;padding:20px 24px;border-bottom:1px solid var(--line);background:#15171a}}
h1,h2,h3,p{{margin:0}}
h1{{font-size:24px;line-height:1.1}}
h2{{font-size:18px;line-height:1.2}}
h3{{font-size:16px;line-height:1.25}}
main{{padding:20px 24px 28px}}
.toplink{{color:var(--muted);text-decoration:none;border:1px solid var(--line);border-radius:6px;padding:7px 10px;background:var(--panel)}}
.hero{{display:grid;grid-template-columns:minmax(0,1.2fr) minmax(320px,.8fr);gap:16px;margin-bottom:22px}}
.hero-main{{padding:18px;border:1px solid var(--line);border-radius:8px;background:var(--panel)}}
.hero-main p{{margin-top:8px;color:var(--muted);line-height:1.45}}
.facts{{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px}}
.fact{{min-height:74px;padding:12px;border:1px solid var(--line);border-radius:8px;background:var(--panel-2)}}
.fact strong{{display:block;font-size:24px;line-height:1.1}}
.fact span{{display:block;margin-top:6px;color:var(--muted)}}
.meta{{display:flex;gap:8px;flex-wrap:wrap;margin-top:14px}}
.chip{{display:inline-flex;align-items:center;min-height:26px;border:1px solid var(--line);border-radius:6px;padding:4px 8px;color:var(--muted);background:#141619;max-width:100%;overflow-wrap:anywhere}}
.group{{margin-top:18px}}
.group-head{{display:flex;justify-content:space-between;gap:16px;align-items:flex-end;margin-bottom:10px}}
.group-head p,.eyebrow{{color:var(--muted)}}
.eyebrow{{display:block;margin-bottom:4px;text-transform:uppercase;font-size:11px;letter-spacing:.08em}}
.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(290px,1fr));gap:12px}}
.svc{{display:flex;flex-direction:column;gap:12px;min-height:250px;border:1px solid var(--line);border-radius:8px;padding:14px;background:var(--panel)}}
.svc-head{{display:flex;justify-content:space-between;gap:12px;align-items:flex-start}}
.svc-title{{min-width:0}}
.svc-title p{{margin-top:5px;color:var(--muted);font-size:13px;overflow-wrap:anywhere}}
.state{{flex:0 0 auto;border:1px solid currentColor;border-radius:6px;padding:4px 7px;font-size:12px;text-transform:uppercase}}
.state-online{{color:var(--ok)}}.state-attention{{color:var(--bad)}}.state-idle{{color:var(--muted)}}.state-unknown{{color:var(--warn)}}
.detail{{line-height:1.45;color:#d4d6da;overflow-wrap:anywhere}}
.desc{{color:var(--muted);line-height:1.45}}
.svc-meta{{display:flex;gap:7px;flex-wrap:wrap;margin-top:auto}}
.actions{{display:flex;gap:8px;flex-wrap:wrap;align-items:center}}
button,.launch{{display:inline-flex;align-items:center;min-height:30px;background:#23272d;color:var(--text);border:1px solid #4a515b;border-radius:6px;padding:6px 10px;cursor:pointer;text-decoration:none;font:inherit}}
.launch{{background:#243141;border-color:#3e536d}}
.disabled{{color:var(--muted);cursor:default}}
.empty{{border:1px solid var(--line);border-radius:8px;padding:20px;background:var(--panel)}}
.empty p{{margin-top:8px;color:var(--muted)}}
@media (max-width:760px){{header,.group-head{{align-items:flex-start;flex-direction:column}}main{{padding:16px}}.hero{{grid-template-columns:1fr}}.facts{{grid-template-columns:1fr 1fr}}}}
@media (max-width:430px){{.facts{{grid-template-columns:1fr}}.svc-head{{flex-direction:column}}}}
</style>
</head>
<body>
<header><h1>Plugroot</h1><a class="toplink" href="/api/status">status json</a></header>
<main>
<section class="hero">
  <div class="hero-main">
    <h2>Service Harness Overview</h2>
    <p>{} is supervising {} declared service(s) across {} plane(s) and {} category group(s). Non-optional offline or missing services are counted as attention.</p>
    <div class="meta">
      <span class="chip">host {}</span>
      <span class="chip">private {}</span>
      <span class="chip">state {}</span>
    </div>
  </div>
  <div class="facts">
    <div class="fact"><strong>{}</strong><span>services</span></div>
    <div class="fact"><strong>{}</strong><span>online</span></div>
    <div class="fact"><strong>{}</strong><span>attention</span></div>
    <div class="fact"><strong>{}</strong><span>optional idle</span></div>
  </div>
</section>
{}
</main>
</body>
</html>"#,
        html_escape(host_name),
        total,
        planes,
        categories,
        html_escape(host_name),
        html_escape(private_ip),
        html_escape(&state_root),
        total,
        online,
        attention,
        optional_idle,
        groups
    )
}

fn render_web_service(row: &StatusRow) -> String {
    let actions = row
        .controls
        .iter()
        .map(|action| {
            format!(
                r#"<form method="post" action="/api/action/{}/{}"><button>{}</button></form>"#,
                html_escape(&row.id),
                html_escape(action),
                html_escape(action)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let action_html = if actions.is_empty() {
        r#"<span class="chip disabled">No controls</span>"#.into()
    } else {
        actions
    };
    let launch = row
        .url
        .as_deref()
        .map(|url| format!(r#"<a class="launch" href="{}">open</a>"#, html_escape(url)))
        .unwrap_or_else(|| r#"<span class="chip disabled">No URL</span>"#.into());
    let description = row
        .description
        .as_deref()
        .map(|description| format!(r#"<p class="desc">{}</p>"#, html_escape(description)))
        .unwrap_or_default();
    let ports = if row.ports.is_empty() {
        String::new()
    } else {
        format!(
            r#"<span class="chip">{}</span>"#,
            html_escape(&row.ports.join(", "))
        )
    };
    let repo = row
        .repo
        .as_deref()
        .map(|repo| format!(r#"<span class="chip">repo {}</span>"#, html_escape(repo)))
        .unwrap_or_default();
    let access = row
        .access
        .as_deref()
        .map(|access| {
            format!(
                r#"<span class="chip">access {}</span>"#,
                html_escape(access)
            )
        })
        .unwrap_or_default();
    let optional = if row.optional {
        r#"<span class="chip">optional</span>"#
    } else {
        ""
    };

    format!(
        r#"<article class="svc"><div class="svc-head"><div class="svc-title"><h3>{}</h3><p>{} / {} / {}</p></div><span class="state {}">{}</span></div><p class="detail">{}</p>{}<div class="svc-meta"><span class="chip">id {}</span>{}{}{}{}</div><div class="actions">{}{}</div></article>"#,
        html_escape(&row.name),
        html_escape(&row.plane),
        html_escape(&web_category_label(row)),
        html_escape(&row.kind),
        web_state_class(row),
        html_escape(&row.state),
        html_escape(&row.detail),
        description,
        html_escape(&row.id),
        repo,
        access,
        ports,
        optional,
        launch,
        action_html
    )
}

fn web_category_label(row: &StatusRow) -> String {
    if row.category == "-" {
        "uncategorized".into()
    } else {
        row.category.clone()
    }
}

fn web_needs_attention(row: &StatusRow) -> bool {
    row.state != "online" && !(row.optional && matches!(row.state.as_str(), "offline" | "missing"))
}

fn web_state_class(row: &StatusRow) -> &'static str {
    if row.state == "online" {
        "state-online"
    } else if !web_needs_attention(row) {
        "state-idle"
    } else if matches!(row.state.as_str(), "offline" | "missing") {
        "state-attention"
    } else {
        "state-unknown"
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_vars_with_defaults() {
        let mut values = HashMap::new();
        values.insert("SET".into(), "value".into());
        assert_eq!(
            expand_vars("${SET} ${MISSING:-fallback} ${UNKNOWN}", &values),
            "value fallback ${UNKNOWN}"
        );
    }

    #[test]
    fn parses_minimal_manifest() {
        let raw = r#"
[plugroot]
root = "/opt/plugroot"
state_root = "/var/lib/plugroot"

[[repo]]
id = "example"
url = "https://example.invalid/repo.git"
path = "/tmp/example"

[[service]]
id = "example"
name = "Example"
kind = "port"
host = "127.0.0.1"
port = 9
"#;
        let parsed: Manifest = toml::from_str(raw).unwrap();
        assert_eq!(
            parsed.plugroot.as_ref().unwrap().state_root.as_deref(),
            Some("/var/lib/plugroot")
        );
        assert_eq!(parsed.repo.len(), 1);
        assert_eq!(parsed.service[0].id, "example");
    }

    #[test]
    fn state_root_prefers_state_root_over_legacy_root() {
        let manifest = Manifest {
            plugroot: Some(PlugrootConfig {
                code_root: None,
                root: Some("/legacy".into()),
                state_root: Some("/private-state".into()),
                repo_dir: None,
            }),
            ..Manifest::default()
        };

        assert_eq!(
            state_root_for_manifest(Path::new("/code"), &manifest),
            PathBuf::from("/private-state")
        );
    }

    #[test]
    fn context_loads_state_root_overlay() {
        let code = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        fs::write(
            code.path().join("plugroot.toml"),
            format!(
                r#"
[plugroot]
state_root = "{}"

[[service]]
id = "alpha"
name = "Alpha"
kind = "noop"
"#,
                state.path().display()
            ),
        )
        .unwrap();
        fs::write(
            state.path().join("plugroot.local.toml"),
            r#"
[[service]]
id = "beta"
name = "Beta"
kind = "noop"
"#,
        )
        .unwrap();

        let ctx = Context::load(code.path().to_path_buf()).unwrap();

        assert!(ctx.service("alpha").is_some());
        assert!(ctx.service("beta").is_some());
    }

    #[test]
    fn boundary_rejects_state_root_inside_code_checkout() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(
            dir.path().join("plugroot.toml"),
            r#"
[plugroot]
state_root = "state"
"#,
        )
        .unwrap();
        let ctx = Context::load(dir.path().to_path_buf()).unwrap();

        let findings = boundary_findings(&ctx).unwrap();

        assert!(findings
            .iter()
            .any(|finding| finding.severity == BoundarySeverity::Error
                && finding.message.contains("state root")));
    }

    #[test]
    fn generates_user_unit() {
        let service = Service {
            id: "example-app".into(),
            name: "Example App".into(),
            plane: None,
            category: None,
            kind: "user-systemd".into(),
            path: None,
            unit: Some("example-app.service".into()),
            user: Some("plugroot".into()),
            host: None,
            port: None,
            url: None,
            access: None,
            description: None,
            containers: None,
            controls: None,
            ports: None,
            optional: None,
            repo: None,
            working_dir: Some("/tmp/example-app".into()),
            command: Some(vec!["/usr/bin/python3".into(), "server.py".into()]),
            env: Some(vec!["EXAMPLE_LOG_REQUESTS=0".into()]),
            env_file: None,
            unit_source: None,
        };
        let ctx = Context {
            root: PathBuf::new(),
            manifest: Manifest::default(),
            env_values: HashMap::new(),
        };
        let unit = generate_unit(&ctx, &service).unwrap().unwrap();
        assert!(unit.contains("ExecStart=/usr/bin/python3 server.py"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn tui_refresh_reloads_manifest_overlay() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("plugroot.toml"),
            r#"
[[service]]
id = "alpha"
name = "Alpha"
kind = "noop"
"#,
        )
        .unwrap();
        let ctx = Context::load(dir.path().to_path_buf()).unwrap();
        let mut app = TuiApp::new(ctx);
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.rows[0].id, "alpha");

        fs::write(
            dir.path().join("plugroot.local.toml"),
            r#"
[[service]]
id = "beta"
name = "Beta"
kind = "noop"
"#,
        )
        .unwrap();

        app.refresh();
        assert!(app.rows.iter().any(|row| row.id == "beta"));
        assert_eq!(app.rows[app.selected].id, "alpha");
    }

    #[test]
    fn tui_window_marks_horizontal_overflow() {
        assert_eq!(fit_window("abcdef", 4, 0), "abc>");
        assert_eq!(fit_window("abcdef", 4, 1), "<cd>");
        assert_eq!(fit_window("abc", 5, 0), "abc  ");
    }

    #[test]
    fn tui_viewport_keeps_selection_visible_and_clamps_pan() {
        let ctx = Context {
            root: PathBuf::new(),
            manifest: Manifest::default(),
            env_values: HashMap::new(),
        };
        let mut app = TuiApp {
            ctx,
            rows: vec![
                test_status_row("alpha", "https://example.invalid/a"),
                test_status_row("beta", "https://example.invalid/b"),
                test_status_row("gamma", "https://example.invalid/very/long/service/link"),
                test_status_row("delta", "https://example.invalid/d"),
            ],
            selected: 3,
            scroll_x: usize::MAX,
            scroll_y: 0,
            message: String::new(),
            last_refresh: Instant::now(),
        };

        app.clamp_viewport(2, 24);

        assert_eq!(visible_rows(&app, 2), 2..4);
        assert!(app.scroll_x > 0);
        assert!(app.scroll_x <= tui_content_width(&app).saturating_sub(24));
    }

    fn test_status_row(id: &str, url: &str) -> StatusRow {
        StatusRow {
            id: id.into(),
            name: id.into(),
            plane: "private".into(),
            category: "app".into(),
            kind: "noop".into(),
            state: "online".into(),
            detail: "running".into(),
            url: Some(url.into()),
            access: None,
            description: None,
            ports: Vec::new(),
            repo: None,
            controls: Vec::new(),
            optional: false,
        }
    }

    #[test]
    fn audit_secret_assignment_allows_placeholders() {
        assert!(!suspicious_secret_assignment(&format!("{}=", "TOKEN")));
        assert!(!suspicious_secret_assignment(&format!(
            "{}=<value>",
            "TOKEN"
        )));
        assert!(!suspicious_secret_assignment(&format!(
            "{}=${{VALUE}}",
            "TOKEN"
        )));
        assert!(suspicious_secret_assignment(&format!(
            "{}={}",
            "TOKEN", "abc123"
        )));
    }

    #[test]
    fn audit_detects_cgnat_private_address() {
        let line = format!("bind = 100.{}.0.1", 64);
        assert!(contains_tailscale_ipv4(&line));
        assert!(!contains_tailscale_ipv4("bind = 127.0.0.1"));
    }

    #[test]
    fn audit_token_marker_requires_token_shaped_suffix() {
        let openai_marker = ["s", "k-"].concat();
        let rustdesk_image = format!("image: rust{}{}{}", "desk/rustde", "sk", "-server:latest");
        let token_line = format!("OPENAI_API_KEY={}{}", openai_marker, "abcdefghijklmnop");
        assert!(!contains_token_marker(&rustdesk_image, &openai_marker));
        assert!(contains_token_marker(&token_line, &openai_marker));
    }

    #[test]
    fn audit_rejects_private_paths() {
        assert!(audit_path("docs/private/notes.md").is_some());
        assert!(audit_path("service/data/state.db").is_some());
        assert!(audit_path(".env.example").is_none());
    }

    #[test]
    fn doctor_parses_manifest_tcp_port_ranges() {
        let mut ports = HashSet::new();
        add_tcp_ports_from_descriptor("21115-21119/tcp tailscale", &mut ports);
        add_tcp_ports_from_descriptor("21120/udp tailscale", &mut ports);

        assert!(ports.contains(&21115));
        assert!(ports.contains(&21119));
        assert!(!ports.contains(&21120));
    }

    #[test]
    fn doctor_flags_only_broad_ufw_allows() {
        let rules = broad_ufw_allow_rules(
            r#"
Status: active
22/tcp on tailscale0       ALLOW IN    Anywhere
41641/udp                  ALLOW IN    Anywhere
8080/tcp                   ALLOW IN    Anywhere
"#,
        );

        assert_eq!(
            rules,
            vec!["8080/tcp                   ALLOW IN    Anywhere"]
        );
    }

    #[test]
    fn doctor_flags_unexpected_wildcard_tcp_listeners() {
        let mut expected = HashSet::new();
        expected.insert(22);
        let listeners = unexpected_wildcard_tcp_listeners(
            &format!(
                r#"
tcp LISTEN 0 4096 0.0.0.0:22 0.0.0.0:*
tcp LISTEN 0 4096 100.{}.0.1:8088 0.0.0.0:*
tcp LISTEN 0 4096 *:7345 *:*
"#,
                64
            ),
            &expected,
        );

        assert_eq!(listeners.len(), 1);
        assert!(listeners[0].contains("*:7345"));
    }
}
