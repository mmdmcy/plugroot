use crate::default_state_root;
use crate::ensure_success;
use crate::run_command;
use crate::AuditFinding;
use crate::Command;
use crate::Path;
use crate::PathBuf;
use std::env;
use std::fs;
use std::io;

pub(crate) fn cmd_audit_public(root: &Path, args: &[String]) -> io::Result<u8> {
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

pub(crate) fn install_audit_hook(root: &Path) -> io::Result<()> {
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

pub(crate) fn audit_public(root: &Path) -> io::Result<Vec<AuditFinding>> {
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

pub(crate) fn git_tracked_files(root: &Path) -> io::Result<Vec<String>> {
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

pub(crate) fn audit_path(path: &str) -> Option<String> {
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

pub(crate) fn load_audit_denylist(root: &Path) -> Vec<String> {
    let mut paths = vec![
        root.join("docs/private/audit-denylist.txt"),
        root.join(".plugroot/audit-denylist.txt"),
    ];
    let state_root = env::var("PLUGROOT_STATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_state_root(root));
    paths.push(state_root.join(".plugroot/audit-denylist.txt"));
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
    terms.sort();
    terms.dedup();
    terms
}

pub(crate) fn contains_denylist_term(line: &str, term: &str) -> bool {
    if term.is_empty() {
        return false;
    }

    let lower = line.to_ascii_lowercase();
    if !term
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return lower.contains(term);
    }

    let mut offset = 0;
    while let Some(relative_start) = lower[offset..].find(term) {
        let start = offset + relative_start;
        let end = start + term.len();
        let left_is_identifier = lower[..start]
            .chars()
            .next_back()
            .is_some_and(denylist_identifier_char);
        let right_is_identifier = lower[end..]
            .chars()
            .next()
            .is_some_and(denylist_identifier_char);
        if !left_is_identifier && !right_is_identifier {
            return true;
        }
        offset = end;
    }
    false
}

fn denylist_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')
}

pub(crate) fn audit_line(line: &str, private_terms: &[String]) -> Vec<String> {
    let mut findings = Vec::new();
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
        if contains_denylist_term(line, term) {
            findings.push("local denylist term".into());
        }
    }

    findings
}

pub(crate) fn token_markers() -> Vec<String> {
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

pub(crate) fn contains_token_marker(line: &str, marker: &str) -> bool {
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

pub(crate) fn token_marker_min_suffix(marker: &str) -> usize {
    let openai_marker = ["s", "k-"].concat();
    let slack_bot_marker = ["xo", "xb-"].concat();
    let slack_user_marker = ["xo", "xp-"].concat();
    if marker == openai_marker || marker == slack_bot_marker || marker == slack_user_marker {
        16
    } else {
        12
    }
}

pub(crate) fn suspicious_secret_assignment(line: &str) -> bool {
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

pub(crate) fn contains_tailscale_ipv4(line: &str) -> bool {
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
