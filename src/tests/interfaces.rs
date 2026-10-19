use crate::app::{Context, Manifest, PlugrootConfig};
use crate::features::status::StatusRow;
use crate::interfaces::{
    tui::{fit_window, tui_content_width, visible_rows, TuiApp},
    web::{
        bind_is_loopback, component_status_json, ct_eq, declared_content_length, hmac_sha256,
        read_http_body, render_web_service, render_web_service_lists,
        require_password_off_loopback, web_authorized, web_binds, web_session_cookie,
        web_session_secret, web_session_valid, WebRequest, MAX_HTTP_BODY,
    },
};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Instant;

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
fn web_service_exposes_one_tap_name_and_open_links() {
    let row = test_status_row("example", "https://example.invalid");

    let html = render_web_service(&row);
    let launch = html.find("class=\"launch\"").unwrap();
    let details = html.find("class=\"svc-more\"").unwrap();

    assert!(html.starts_with("<article class=\"svc\">"));
    assert!(html.contains("class=\"svc-title svc-direct\""));
    assert!(launch < details);
    assert!(html.contains("aria-label=\"Open example\""));
    assert_eq!(html.matches("href=\"https://example.invalid\"").count(), 2);
}

#[test]
fn web_service_does_not_show_plane_or_category_navigation() {
    let row = test_status_row("example", "https://example.invalid");

    let html = render_web_service(&row);

    assert!(!html.contains("private"));
    assert!(!html.contains("app"));
}

#[test]
fn web_service_lists_put_unlinked_services_at_the_bottom() {
    let linked = test_status_row("linked", "https://example.invalid/linked");
    let mut unlinked = test_status_row("unlinked", "https://example.invalid/unlinked");
    unlinked.url = None;

    let html = render_web_service_lists(&[unlinked, linked]);
    let linked_position = html.find(">linked</h3>").unwrap();
    let unlinked_heading = html.find("No direct link").unwrap();
    let unlinked_position = html.find(">unlinked</h3>").unwrap();

    assert!(linked_position < unlinked_heading);
    assert!(unlinked_heading < unlinked_position);
    assert!(html.contains("Status-only services"));
}

#[test]
fn component_status_is_minimal_versioned_and_safe_without_auth() {
    let request = WebRequest {
        method: "GET".into(),
        path: "/.well-known/linuxmice/component".into(),
        headers: vec![("x-correlation-id".into(), "pilot:request-42".into())],
        body: String::new(),
    };

    let status = component_status_json(&request);
    assert_eq!(status["schema_version"], "linuxmice.response.v1");
    assert_eq!(status["component"], "katteke.plugroot");
    assert_eq!(status["operation"], "status");
    assert_eq!(status["correlation_id"], "pilot:request-42");
    assert!(status["data"].get("services").is_none());
}

#[test]
fn web_binds_default_env_list_and_repeated_flags() {
    let none: Vec<String> = Vec::new();
    assert_eq!(web_binds(None, &none).unwrap(), vec!["127.0.0.1:8786"]);
    assert_eq!(
        web_binds(
            Some("192.0.2.10:8088, 127.0.0.1:8786,192.0.2.10:8088"),
            &none
        )
        .unwrap(),
        vec!["192.0.2.10:8088", "127.0.0.1:8786"]
    );
    let flags: Vec<String> = ["--bind", "10.0.0.1:1", "--bind=127.0.0.1:8786"]
        .iter()
        .map(|value| value.to_string())
        .collect();
    assert_eq!(
        web_binds(Some("ignored:1"), &flags).unwrap(),
        vec!["10.0.0.1:1", "127.0.0.1:8786"]
    );
    assert!(web_binds(None, &["--bind".to_string()]).is_err());
    assert!(web_binds(Some(" , "), &none).is_err());
}

fn web_test_ctx(state_root: &std::path::Path, env: &[(&str, &str)]) -> Context {
    Context {
        root: state_root.to_path_buf(),
        manifest: Manifest {
            plugroot: Some(PlugrootConfig {
                state_root: Some(state_root.to_string_lossy().into_owned()),
                ..PlugrootConfig::default()
            }),
            ..Manifest::default()
        },
        env_values: env
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
    }
}

fn request_with_header(name: &str, value: &str) -> WebRequest {
    WebRequest {
        method: "POST".to_string(),
        path: "/api/action/alpha/restart".to_string(),
        headers: vec![(name.to_string(), value.to_string())],
        body: String::new(),
    }
}

#[test]
fn web_requires_password_off_loopback() {
    assert!(bind_is_loopback("127.0.0.1:8786"));
    assert!(bind_is_loopback("localhost:8786"));
    assert!(bind_is_loopback("[::1]:8786"));
    assert!(!bind_is_loopback("100.64.0.1:8786"));
    assert!(!bind_is_loopback("0.0.0.0:8786"));
    assert!(!bind_is_loopback("[::]:8786"));
    assert!(!bind_is_loopback("plugroot.internal:8786"));

    let loopback = vec!["127.0.0.1:8786".to_string()];
    let mixed = vec!["100.64.0.1:8786".to_string(), "127.0.0.1:8786".to_string()];
    assert!(require_password_off_loopback(&loopback, None).is_ok());
    assert!(require_password_off_loopback(&mixed, Some("pw")).is_ok());
    let err = require_password_off_loopback(&mixed, None).unwrap_err();
    assert!(err.to_string().contains("PLUGROOT_WEB_PASSWORD"));
    assert!(err.to_string().contains("100.64.0.1:8786"));
}

#[test]
fn web_ct_eq_and_hmac_vectors() {
    assert!(ct_eq(b"", b""));
    assert!(ct_eq(b"abc", b"abc"));
    assert!(!ct_eq(b"abc", b"abd"));
    assert!(!ct_eq(b"abc", b"abcd"));
    assert!(!ct_eq(b"abc", b""));

    // RFC 4231 test case 2.
    let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
    assert_eq!(
        crate::interfaces::web::to_hex(&mac),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
    // RFC 4231 test case 6: a key longer than the block size.
    let mac = hmac_sha256(
        &[0xaa; 131],
        b"Test Using Larger Than Block-Size Key - Hash Key First",
    );
    assert_eq!(
        crate::interfaces::web::to_hex(&mac),
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
}

#[test]
fn web_session_cookie_round_trip_uses_persisted_secret() {
    let dir = tempfile::tempdir().unwrap();
    let env = [
        ("PLUGROOT_WEB_PASSWORD", "hunter2"),
        ("PLUGROOT_WEB_USER", "rei"),
    ];
    let ctx = web_test_ctx(dir.path(), &env);

    let cookie = web_session_cookie(&ctx);
    let value = cookie.split(';').next().unwrap();
    assert!(value.starts_with("plugroot_session="));
    assert!(web_session_valid(&ctx, value));
    assert!(web_authorized(&ctx, &request_with_header("cookie", value)));

    // The secret is random, persisted with owner-only permissions, and stable.
    let key_path = dir.path().join("web-session.key");
    let secret = web_session_secret(&ctx);
    assert_eq!(secret, web_session_secret(&ctx));
    assert_ne!(secret, [0u8; 32]);
    assert_eq!(fs::read_to_string(&key_path).unwrap().trim().len(), 64);
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // The cookie no longer depends on the password.
    assert!(!fs::read_to_string(&key_path).unwrap().contains("hunter2"));
    let rotated = web_test_ctx(
        dir.path(),
        &[
            ("PLUGROOT_WEB_PASSWORD", "other"),
            ("PLUGROOT_WEB_USER", "rei"),
        ],
    );
    assert!(web_session_valid(&rotated, value));

    // Tampering, expiry and a different secret are rejected.
    let (expires, signature) = value
        .trim_start_matches("plugroot_session=")
        .split_once('.')
        .unwrap();
    let mut flipped = signature.to_string();
    flipped.replace_range(0..1, if signature.starts_with('0') { "1" } else { "0" });
    assert!(!web_session_valid(
        &ctx,
        &format!("plugroot_session={expires}.{flipped}")
    ));
    assert!(!web_session_valid(
        &ctx,
        &format!("plugroot_session=1.{signature}")
    ));
    assert!(!web_session_valid(&ctx, "plugroot_session=garbage"));
    let other = tempfile::tempdir().unwrap();
    assert!(!web_session_valid(&web_test_ctx(other.path(), &env), value));
}

#[test]
fn web_basic_auth_compares_exact_header() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = web_test_ctx(dir.path(), &[("PLUGROOT_WEB_PASSWORD", "hunter2")]);
    let good = "Basic cGx1Z3Jvb3Q6aHVudGVyMg=="; // plugroot:hunter2
    assert!(web_authorized(
        &ctx,
        &request_with_header("authorization", good)
    ));
    let bad = "Basic cGx1Z3Jvb3Q6aHVudGVyMw=="; // plugroot:hunter3
    assert!(!web_authorized(
        &ctx,
        &request_with_header("authorization", bad)
    ));
    assert!(!web_authorized(
        &ctx,
        &request_with_header("authorization", "")
    ));
}

#[test]
fn web_body_cap_rejects_large_content_length_before_reading() {
    assert_eq!(
        declared_content_length(b"POST / HTTP/1.1\r\nContent-Length: 12"),
        12
    );
    assert_eq!(
        declared_content_length(b"POST / HTTP/1.1\r\ncontent-length: x"),
        0
    );
    assert_eq!(MAX_HTTP_BODY, 65536);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mut client = TcpStream::connect(addr).unwrap();
    let (mut server, _) = listener.accept().unwrap();

    let head = format!(
        "POST /login HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        MAX_HTTP_BODY + 1
    );
    client.write_all(head.as_bytes()).unwrap();
    let mut data = vec![0u8; head.len()];
    server.read_exact(&mut data).unwrap();
    assert!(!read_http_body(&mut server, &mut data).unwrap());
    assert_eq!(data.len(), head.len());

    let head = "POST /login HTTP/1.1\r\nContent-Length: 5\r\n\r\n";
    client.write_all(head.as_bytes()).unwrap();
    client.write_all(b"a=b&c").unwrap();
    let mut data = vec![0u8; head.len()];
    server.read_exact(&mut data).unwrap();
    assert!(read_http_body(&mut server, &mut data).unwrap());
    assert!(data.ends_with(b"a=b&c"));
}
