use crate::run_command;
use crate::run_prefixed_command;
use crate::user_systemd_prefix;
use crate::Context;
use crate::Duration;
use crate::Service;
use crate::SocketAddr;
use crate::TcpStream;
use std::io;
use std::net::ToSocketAddrs;

mod types;

pub(crate) use types::StatusRow;

pub(crate) fn cmd_status(ctx: &Context, args: &[String]) -> io::Result<u8> {
    let json = args.iter().any(|arg| arg == "--json");
    let rows = status_rows(ctx);
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        print_status_table(&rows);
    }
    Ok(0)
}

pub(crate) fn cmd_list(ctx: &Context) -> io::Result<u8> {
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

pub(crate) fn print_status_table(rows: &[StatusRow]) {
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

pub(crate) fn status_rows(ctx: &Context) -> Vec<StatusRow> {
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

pub(crate) fn service_status(ctx: &Context, service: &Service) -> (String, String) {
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

pub(crate) fn compose_status(service: &Service) -> (String, String) {
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

pub(crate) fn systemd_status(unit: &str) -> (String, String) {
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

pub(crate) fn user_systemd_status(user: &str, unit: &str) -> (String, String) {
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

pub(crate) fn port_status(host: &str, port: u16) -> (String, String) {
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
