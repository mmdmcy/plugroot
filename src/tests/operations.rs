use crate::app::{Context, Manifest, PlugrootConfig, Service};
use crate::features::{
    apply::{generate_unit, parse_apply_options, ApplyOptions},
    backup::{
        emergency_tar_excludes, parse_emergency_export_options, parse_simple_backup_args,
        sanitize_filename_component, tar_relative_path, validate_emergency_output_boundary,
        validate_emergency_output_dir,
    },
    restore::{classify_restore_archive, find_restore_archive, RestoreArchiveKind},
    services::{compose_args, compose_env_values},
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[test]
fn emergency_export_parses_safety_options() {
    let args = vec![
        "--output=/mnt/usb".to_string(),
        "--yes".to_string(),
        "--dry-run".to_string(),
        "--passphrase-file".to_string(),
        "/tmp/passphrase".to_string(),
    ];

    let options = parse_emergency_export_options(&args).unwrap().unwrap();

    assert_eq!(options.output_dir, PathBuf::from("/mnt/usb"));
    assert!(options.yes);
    assert!(options.dry_run);
    assert_eq!(
        options.passphrase_file,
        Some(PathBuf::from("/tmp/passphrase"))
    );
    assert!(!options.create_output_dir);
    assert_eq!(options.owner_user, None);
}

#[test]
fn simple_backup_args_use_words_not_flags() {
    let default_dir = Path::new("/home/example/Documents/plugroot-emergency-exports");
    let args = vec!["check".to_string(), "/media/example/USB".to_string()];

    let (output_dir, dry_run) = parse_simple_backup_args(&args, default_dir)
        .unwrap()
        .unwrap();

    assert_eq!(output_dir, PathBuf::from("/media/example/USB"));
    assert!(dry_run);
    assert!(parse_simple_backup_args(&["--output=/tmp".into()], default_dir).is_err());
}

#[test]
fn apply_options_keep_repo_sync_explicit() {
    assert_eq!(
        parse_apply_options(&["--dry-run".into()]).unwrap(),
        ApplyOptions {
            dry_run: true,
            sync_repos: false
        }
    );
    assert_eq!(
        parse_apply_options(&["--sync-repos".into()]).unwrap(),
        ApplyOptions {
            dry_run: false,
            sync_repos: true
        }
    );
    assert!(parse_apply_options(&["--pull".into()]).is_err());
}

#[test]
fn compose_uses_process_env_instead_of_raw_env_file() {
    let mut env_values = HashMap::new();
    env_values.insert("PLUGDECK_PASSWORD_HASH".into(), "$argon2id$v=19$m=1".into());
    let ctx = Context {
        root: PathBuf::from("/srv/plugroot"),
        manifest: Manifest::default(),
        env_values,
    };
    let service = Service {
        id: "example".into(),
        name: "Example".into(),
        plane: None,
        category: None,
        kind: "compose".into(),
        path: Some("/srv/plugroot-state/.plugroot/local/compose/example".into()),
        unit: None,
        user: None,
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
        working_dir: None,
        command: None,
        env: None,
        env_file: None,
        unit_source: None,
    };

    let args = compose_args(&ctx, &service).unwrap();
    let envs = compose_env_values(&ctx, &service);

    assert!(!args.iter().any(|arg| arg == "--env-file"));
    assert_eq!(
        envs.get("PLUGDECK_PASSWORD_HASH").map(String::as_str),
        Some("$argon2id$v=19$m=1")
    );
}

#[test]
fn restore_classifies_supported_archives() {
    assert_eq!(
        classify_restore_archive(Path::new("plugroot-emergency-host.tar.zst.gpg")),
        Some(RestoreArchiveKind::Encrypted)
    );
    assert_eq!(
        classify_restore_archive(Path::new("plugroot-plain-emergency-host.tar.zst")),
        Some(RestoreArchiveKind::Plain)
    );
    assert_eq!(classify_restore_archive(Path::new("notes.txt")), None);
}

#[test]
fn restore_finds_newest_archive_in_directory() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("plugroot-emergency-old.tar.zst.gpg");
    let new = dir.path().join("plugroot-emergency-new.tar.zst.gpg");
    fs::write(&old, "old").unwrap();
    std::thread::sleep(Duration::from_millis(5));
    fs::write(&new, "new").unwrap();

    assert_eq!(find_restore_archive(dir.path()).unwrap(), new);
}

#[test]
fn restore_prefers_encrypted_archive_over_plain() {
    let dir = tempfile::tempdir().unwrap();
    let encrypted = dir.path().join("plugroot-emergency-old.tar.zst.gpg");
    let plain = dir.path().join("plugroot-plain-emergency-new.tar.zst");
    fs::write(&encrypted, "encrypted").unwrap();
    std::thread::sleep(Duration::from_millis(5));
    fs::write(&plain, "plain").unwrap();

    assert_eq!(find_restore_archive(dir.path()).unwrap(), encrypted);
}

#[test]
fn emergency_export_rejects_output_inside_code_or_state_root() {
    let code = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let code_output = code.path().join("exports");
    let state_output = state.path().join("exports");
    fs::create_dir(&code_output).unwrap();
    fs::create_dir(&state_output).unwrap();
    let ctx = Context {
        root: code.path().to_path_buf(),
        manifest: Manifest {
            plugroot: Some(PlugrootConfig {
                code_root: None,
                root: None,
                state_root: Some(state.path().display().to_string()),
                repo_dir: None,
            }),
            ..Manifest::default()
        },
        env_values: HashMap::new(),
    };

    assert!(validate_emergency_output_boundary(&ctx, &code_output).is_err());
    assert!(validate_emergency_output_boundary(&ctx, &state_output).is_err());
    assert!(validate_emergency_output_boundary(&ctx, outside.path()).is_ok());
    assert!(validate_emergency_output_dir(outside.path()).is_ok());
}

#[test]
fn emergency_export_sanitizes_archive_name_parts() {
    assert_eq!(
        sanitize_filename_component("mint laptop/desk"),
        "mint-laptop-desk"
    );
    assert_eq!(sanitize_filename_component("!!!"), "plugroot-host");
}

#[test]
fn emergency_tar_paths_drop_leading_root() {
    assert_eq!(
        tar_relative_path(Path::new("/srv/plugroot/target")).as_deref(),
        Some("srv/plugroot/target")
    );
}

#[test]
fn emergency_tar_excludes_state_repo_build_caches() {
    let ctx = Context {
        root: PathBuf::from("/srv/plugroot"),
        manifest: Manifest {
            plugroot: Some(PlugrootConfig {
                code_root: None,
                root: None,
                state_root: Some("/srv/plugroot-state".into()),
                repo_dir: None,
            }),
            ..Manifest::default()
        },
        env_values: HashMap::new(),
    };

    let excludes = emergency_tar_excludes(&ctx);

    assert!(excludes.contains(&"srv/plugroot/target".into()));
    assert!(excludes.contains(&"srv/plugroot-state/repos/*/target".into()));
    assert!(excludes.contains(&"srv/plugroot-state/repos/*/.cache".into()));
    assert!(excludes.contains(&"srv/plugroot-state/repos/*/node_modules".into()));
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
