use crate::features::doctor::{
    add_tcp_ports_from_descriptor, broad_ufw_allow_rules, unexpected_wildcard_tcp_listeners,
};
use crate::security::audit::{
    audit_path, contains_tailscale_ipv4, contains_token_marker, suspicious_secret_assignment,
};
use std::collections::HashSet;

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
