use crate::has_control;
use crate::logs_for;
use crate::run_fixed_action;
use crate::status_rows;
use crate::Context;
use crate::HashMap;
use crate::SystemTime;
use crate::TcpListener;
use crate::TcpStream;
use crate::UNIX_EPOCH;
use std::env;
use std::io;
use std::io::Read;
use std::io::Write;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use sha2::{Digest, Sha256};

mod types;

pub(crate) use types::WebRequest;

pub(crate) fn cmd_web(ctx: Context, args: &[String]) -> io::Result<u8> {
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

pub(crate) fn handle_http(ctx: Context, mut stream: TcpStream) -> io::Result<()> {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf)?;
    let mut data = buf[..n].to_vec();
    read_http_body(&mut stream, &mut data)?;
    let request = String::from_utf8_lossy(&data);
    let Some(web_request) = parse_web_request(&request) else {
        return Ok(());
    };
    let method = web_request.method.as_str();
    let path = web_request.path.as_str();

    if method == "GET" && path == "/login" {
        let body = render_web_login(&ctx, None);
        return http_response(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            body.as_bytes(),
        );
    }
    if method == "POST" && path == "/login" {
        return handle_web_login(&ctx, &mut stream, &web_request);
    }
    if method == "POST" && path == "/logout" {
        return http_redirect_with_cookie(
            &mut stream,
            "/login",
            "plugroot_session=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax",
        );
    }
    if method == "GET" && path == "/.well-known/linuxmice/component" {
        let body = serde_json::to_vec_pretty(&component_status_json(&web_request))?;
        return http_response(&mut stream, 200, "application/json", &body);
    }

    if !web_authorized(&ctx, &web_request) {
        if method == "GET" && path == "/" {
            return http_redirect(&mut stream, "/login");
        }
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

pub(crate) fn component_status_json(request: &WebRequest) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "linuxmice.response.v1",
        "component": "katteke.plugroot",
        "operation": "status",
        "outcome": "success",
        "data": {
            "status": "ready",
            "version": env!("CARGO_PKG_VERSION"),
            "standalone": true,
            "capabilities": [
                "selfhost.manifest",
                "selfhost.lifecycle",
                "selfhost.backup",
                "selfhost.restore"
            ]
        },
        "warnings": [],
        "error": null,
        "correlation_id": component_correlation_id(request)
    })
}

fn component_correlation_id(request: &WebRequest) -> String {
    if let Some(value) = request
        .headers
        .iter()
        .find(|(name, _)| name == "x-correlation-id")
        .map(|(_, value)| value)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
        })
    {
        return value.clone();
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("plugroot-{}-{nanos}", std::process::id())
}

pub(crate) fn read_http_body(stream: &mut TcpStream, data: &mut Vec<u8>) -> io::Result<()> {
    let Some(header_end) = find_header_end(data) else {
        return Ok(());
    };
    let headers = String::from_utf8_lossy(&data[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
        })
        .unwrap_or(0);
    let needed = header_end + 4 + content_length;
    while data.len() < needed {
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    Ok(())
}

pub(crate) fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|window| window == b"\r\n\r\n")
}

pub(crate) fn parse_web_request(request: &str) -> Option<WebRequest> {
    let (head, body) = request.split_once("\r\n\r\n").unwrap_or((request, ""));
    let mut lines = head.lines();
    let first_line = lines.next()?;
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let path = parts[1].split('?').next().unwrap_or(parts[1]).to_string();
    let headers = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();
    Some(WebRequest {
        method: parts[0].to_string(),
        path,
        headers,
        body: body.to_string(),
    })
}

pub(crate) fn web_authorized(ctx: &Context, request: &WebRequest) -> bool {
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
    request
        .headers
        .iter()
        .any(|(name, value)| name == "authorization" && value == &expected)
        || request
            .headers
            .iter()
            .find(|(name, _)| name == "cookie")
            .is_some_and(|(_, value)| web_session_valid(ctx, value))
}

pub(crate) fn handle_web_login(
    ctx: &Context,
    stream: &mut TcpStream,
    request: &WebRequest,
) -> io::Result<()> {
    let fields = parse_form_body(&request.body);
    let expected_user = ctx
        .env_values
        .get("PLUGROOT_WEB_USER")
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .unwrap_or("plugroot");
    let expected_password = ctx
        .env_values
        .get("PLUGROOT_WEB_PASSWORD")
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .unwrap_or("");
    let user = fields.get("user").map(String::as_str).unwrap_or("");
    let password = fields.get("password").map(String::as_str).unwrap_or("");
    if expected_password.is_empty() || (user == expected_user && password == expected_password) {
        let cookie = web_session_cookie(ctx);
        return http_redirect_with_cookie(stream, "/", &cookie);
    }
    let body = render_web_login(ctx, Some("Wrong username or password."));
    http_response(stream, 401, "text/html; charset=utf-8", body.as_bytes())
}

pub(crate) fn parse_form_body(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Some((url_decode(key)?, url_decode(value)?))
        })
        .collect()
}

pub(crate) fn url_decode(value: &str) -> Option<String> {
    let mut out = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

pub(crate) fn web_session_cookie(ctx: &Context) -> String {
    let ttl = ctx
        .env_values
        .get("PLUGROOT_WEB_SESSION_TTL_SECONDS")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(604800);
    let expires = now_unix_seconds().saturating_add(ttl);
    let signature = web_session_signature(ctx, expires);
    format!("plugroot_session={expires}.{signature}; Max-Age={ttl}; Path=/; HttpOnly; SameSite=Lax")
}

pub(crate) fn web_session_valid(ctx: &Context, cookie_header: &str) -> bool {
    let Some(value) = cookie_header.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == "plugroot_session").then_some(value)
    }) else {
        return false;
    };
    let Some((expires, signature)) = value.split_once('.') else {
        return false;
    };
    let Ok(expires) = expires.parse::<u64>() else {
        return false;
    };
    expires >= now_unix_seconds() && signature == web_session_signature(ctx, expires)
}

pub(crate) fn web_session_signature(ctx: &Context, expires: u64) -> String {
    let user = ctx
        .env_values
        .get("PLUGROOT_WEB_USER")
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .unwrap_or("plugroot");
    let password = ctx
        .env_values
        .get("PLUGROOT_WEB_PASSWORD")
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .unwrap_or("");
    let mut hasher = Sha256::new();
    hasher.update(b"plugroot-web-session-v1");
    hasher.update(user.as_bytes());
    hasher.update(b":");
    hasher.update(expires.to_string().as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    to_hex(&hasher.finalize())
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub(crate) fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn http_unauthorized(stream: &mut TcpStream) -> io::Result<()> {
    let body = b"authentication required";
    write!(
        stream,
        "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nWWW-Authenticate: Basic realm=\"Plugroot\"\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

pub(crate) fn http_redirect(stream: &mut TcpStream, location: &str) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 303 See Other\r\nLocation: {location}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    )
}

pub(crate) fn http_redirect_with_cookie(
    stream: &mut TcpStream,
    location: &str,
    cookie: &str,
) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 303 See Other\r\nLocation: {location}\r\nSet-Cookie: {cookie}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    )
}

pub(crate) fn http_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        401 => "Unauthorized",
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

mod views;

pub(crate) use views::{render_web, render_web_login};
#[cfg(test)]
pub(crate) use views::{render_web_service, render_web_service_lists};
