use crate::app::{expand_vars, Context, Manifest, PlugrootConfig};
use crate::security::{boundary_findings, BoundarySeverity};
use crate::shared::{default_state_root, state_root_for_manifest};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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
root = "/srv/plugroot"
state_root = "/var/lib/plugroot"

[[repo]]
id = "example"
url = "https://example.invalid/repo.git"
path = "/tmp/example"

[[symlink]]
source = "/var/lib/plugroot/workspaces/example/.env"
target = "/tmp/example/.env"

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
    assert_eq!(parsed.symlink.len(), 1);
    assert_eq!(parsed.symlink[0].target, "/tmp/example/.env");
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
fn state_root_defaults_to_a_sibling_directory() {
    assert_eq!(
        default_state_root(Path::new("/srv/plugroot")),
        PathBuf::from("/srv/plugroot-state")
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
