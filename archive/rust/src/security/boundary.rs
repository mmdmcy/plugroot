use crate::audit_public;
use crate::clean_path;
use crate::nearest_git_root;
use crate::resolve_path;
use crate::same_or_inside;
use crate::BoundaryFinding;
use crate::BoundarySeverity;
use crate::Context;
use crate::Path;
use crate::PathBuf;
use std::fs;
use std::io;

pub(crate) fn cmd_boundary(ctx: &Context, args: &[String]) -> io::Result<u8> {
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

pub(crate) fn boundary_findings(ctx: &Context) -> io::Result<Vec<BoundaryFinding>> {
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

    for managed_symlink in &ctx.manifest.symlink {
        let source = clean_path(&resolve_path(&ctx.root, &managed_symlink.source));
        if !same_or_inside(&state_root, &source) {
            findings.push(BoundaryFinding::error(
                Some(source),
                "managed symlink source must live inside the state root",
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

pub(crate) fn service_private_paths(code_root: &Path) -> Vec<PathBuf> {
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
