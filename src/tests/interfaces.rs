use crate::app::{Context, Manifest};
use crate::features::status::StatusRow;
use crate::interfaces::{
    tui::{fit_window, tui_content_width, visible_rows, TuiApp},
    web::{component_status_json, render_web_service, render_web_service_lists, WebRequest},
};
use std::collections::HashMap;
use std::fs;
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
