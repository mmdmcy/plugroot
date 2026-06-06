use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Stdout, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
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
        "status" => cmd_status(&ctx, &args[1..]),
        "apply" => cmd_apply(&ctx, &args[1..]),
        "repos" => cmd_repos(&ctx, &args[1..]),
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
  plugroot [--root <path>] up|down|restart|logs <service|all>
  plugroot [--root <path>] tui [--once]
  plugroot [--root <path>] web [--bind <addr:port>]
  plugroot [--root <path>] audit-public [--install-hook]

Files:
  plugroot.toml        public manifest
  plugroot.local.toml  ignored local overlay
  .env                ignored local values
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
        let env_values = load_env_file(&root.join(".env"))?;
        let mut merged_env: HashMap<String, String> = env::vars().collect();
        for (key, value) in env_values {
            merged_env.insert(key, value);
        }

        let main = load_manifest_file(&root.join("plugroot.toml"), &merged_env)?;
        let local_path = root.join("plugroot.local.toml");
        let manifest = if local_path.exists() {
            let local = load_manifest_file(&local_path, &merged_env)?;
            merge_manifest(main, local)
        } else {
            main
        };

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
    root: Option<String>,
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
        println!("installed .git/hooks/pre-push");
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

fn install_audit_hook(root: &Path) -> io::Result<()> {
    let hook = root.join(".git/hooks/pre-push");
    let parent = hook
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing .git/hooks"))?;
    fs::create_dir_all(parent)?;
    fs::write(
        &hook,
        r#"#!/bin/sh
set -eu
repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"
cargo run --quiet -- audit-public
"#,
    )?;
    ensure_success(run_command(
        "chmod",
        &["0755".into(), hook.display().to_string()],
        None,
        &[],
    ))
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
        return Err(io::Error::new(
            io::ErrorKind::Other,
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
        if line.contains(&marker) {
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
        "{:<9} {:<8} {:<13} {:<22} {}",
        "STATE", "PLANE", "KIND", "SERVICE", "DETAIL"
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
    let generated_root = ctx.root.join(".plugroot/generated");
    if let Some(config) = &ctx.manifest.plugroot {
        let runtime_root = config.root.as_deref().unwrap_or("-");
        let repo_dir = config.repo_dir.as_deref().unwrap_or("-");
        println!("runtime root: {runtime_root}");
        println!("repo dir: {repo_dir}");
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
            return Err(io::Error::new(io::ErrorKind::Other, "git fetch failed"));
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
                return Err(io::Error::new(io::ErrorKind::Other, "git checkout failed"));
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
                return Err(io::Error::new(io::ErrorKind::Other, "git pull failed"));
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
        return Err(io::Error::new(io::ErrorKind::Other, "git clone failed"));
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
            return Err(io::Error::new(io::ErrorKind::Other, "git checkout failed"));
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
    Err(io::Error::new(io::ErrorKind::Other, out.text))
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
        draw_tui(&mut term.stdout, &app)?;
        if event::poll(Duration::from_millis(500))? {
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Down | KeyCode::Char('j') => app.down(),
                    KeyCode::Up | KeyCode::Char('k') => app.up(),
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

fn draw_tui(stdout: &mut Stdout, app: &TuiApp) -> io::Result<()> {
    let (width, height) = terminal::size()?;
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
            width as usize
        )),
        SetAttribute(Attribute::Reset),
        MoveTo(0, 1),
        Print(fit(
            &format!(
                "{} online, {} attention, {} tracked - r refresh - l logs - o on - e restart - f off - q quit",
                online,
                app.rows.len().saturating_sub(online),
                app.rows.len()
            ),
            width as usize
        )),
        MoveTo(0, 2),
        Print(line(width))
    )?;

    let list_top = 3u16;
    let footer_rows = 6u16;
    let available = height.saturating_sub(list_top + footer_rows).max(1) as usize;
    let start = app.selected.saturating_sub(available.saturating_sub(1));
    let end = (start + available).min(app.rows.len());
    queue!(
        stdout,
        MoveTo(0, list_top),
        SetForegroundColor(Color::DarkGrey),
        Print(fit(
            "  STATE    PLANE    CATEGORY/KIND  SERVICE                         ACCESS",
            width as usize
        )),
        ResetColor
    )?;
    for (row_index, row) in app.rows[start..end].iter().enumerate() {
        let actual = start + row_index;
        let y = list_top + 1 + row_index as u16;
        let marker = if actual == app.selected { ">" } else { " " };
        let category_kind = format!("{}/{}", row.category, row.kind);
        let access = row.url.as_deref().or(row.access.as_deref()).unwrap_or("-");
        let text = format!(
            "{marker} {:<8} {:<8} {:<14} {:<30} {}",
            row.state, row.plane, category_kind, row.name, access
        );
        queue!(stdout, MoveTo(0, y))?;
        if actual == app.selected {
            queue!(stdout, SetAttribute(Attribute::Reverse))?;
        }
        queue!(
            stdout,
            SetForegroundColor(color_for_state(&row.state)),
            Print(fit(&text, width as usize)),
            ResetColor
        )?;
        if actual == app.selected {
            queue!(stdout, SetAttribute(Attribute::Reset))?;
        }
    }

    let base = height.saturating_sub(5);
    queue!(stdout, MoveTo(0, base), Print(line(width)))?;
    if let Some(row) = app.rows.get(app.selected) {
        queue!(
            stdout,
            MoveTo(0, base + 1),
            SetAttribute(Attribute::Bold),
            Print(fit(&format!("{} ({})", row.name, row.id), width as usize)),
            SetAttribute(Attribute::Reset),
            MoveTo(0, base + 2),
            Print(fit(
                row.description.as_deref().unwrap_or(&row.detail),
                width as usize
            )),
            MoveTo(0, base + 3),
            Print(fit(
                &format!(
                    "detail: {} | ports: {} | controls: {}",
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
                ),
                width as usize
            )),
            MoveTo(0, base + 4),
            Print(fit(&app.message, width as usize))
        )?;
    }
    stdout.flush()?;
    Ok(())
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
    let mut cards = String::new();
    for row in rows {
        let repo = row.repo.as_deref().unwrap_or("-");
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
        cards.push_str(&format!(
            r#"<section class="svc"><div><strong>{}</strong><span>{} / {}</span></div><p class="{}">{}</p><p>{}</p><p class="muted">repo: {}</p><div class="actions">{}</div></section>"#,
            html_escape(&row.name),
            html_escape(&row.plane),
            html_escape(&row.kind),
            html_escape(&row.state),
            html_escape(&row.state),
            html_escape(&row.detail),
            html_escape(repo),
            if actions.is_empty() { "<span class=\"muted\">No controls</span>".into() } else { actions },
        ));
    }

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Plugroot</title>
<style>
body{{margin:0;font:14px system-ui,sans-serif;background:#111;color:#eee}}
header{{display:flex;justify-content:space-between;align-items:center;padding:16px 20px;border-bottom:1px solid #333}}
main{{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:12px;padding:16px}}
.svc{{border:1px solid #333;border-radius:6px;padding:12px;background:#181818}}
.svc div:first-child{{display:flex;justify-content:space-between;gap:10px}}
.svc span,.muted{{color:#aaa}}
.online{{color:#62d36f}}.offline,.missing{{color:#ff716f}}.unknown,.partial{{color:#ffd166}}
.actions{{display:flex;gap:8px;flex-wrap:wrap}}button{{background:#242424;color:#eee;border:1px solid #555;border-radius:4px;padding:5px 9px;cursor:pointer}}
</style>
</head>
<body>
<header><h1>Plugroot</h1><a href="/api/status">status json</a></header>
<main>{cards}</main>
</body>
</html>"#
    )
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
        assert_eq!(parsed.repo.len(), 1);
        assert_eq!(parsed.service[0].id, "example");
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
    fn audit_rejects_private_paths() {
        assert!(audit_path("docs/private/notes.md").is_some());
        assert!(audit_path("service/data/state.db").is_some());
        assert!(audit_path(".env.example").is_none());
    }
}
