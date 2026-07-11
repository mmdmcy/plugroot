use crate::clean_path;
use crate::status_rows;
use crate::Context;
use crate::StatusRow;

pub(crate) fn render_web_login(ctx: &Context, error: Option<&str>) -> String {
    let user = ctx
        .env_values
        .get("PLUGROOT_WEB_USER")
        .filter(|value| !value.is_empty())
        .map(String::as_str)
        .unwrap_or("plugroot");
    let error_html = error
        .map(|message| format!(r#"<p class="error">{}</p>"#, html_escape(message)))
        .unwrap_or_default();
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-title" content="Plugroot">
<meta name="theme-color" content="#111418">
<title>Plugroot Login</title>
<style>
:root{{color-scheme:dark;--bg:#111418;--panel:#1b2026;--line:#343b45;--text:#f5f2ea;--muted:#aeb5bf;--accent:#7fb4e8;--bad:#ff8b86}}
*{{box-sizing:border-box}}
html,body{{min-height:100%;margin:0;background:var(--bg);color:var(--text);font:15px system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;letter-spacing:0}}
body{{display:grid;place-items:center;padding:18px}}
main{{width:min(380px,100%);border:1px solid var(--line);border-radius:8px;background:var(--panel);padding:18px;box-shadow:0 18px 48px rgba(0,0,0,.24)}}
h1{{font-size:24px;line-height:1.1;margin:0 0 6px}}
p{{margin:0 0 14px;color:var(--muted);line-height:1.4}}
form{{display:grid;gap:10px}}
label{{display:grid;gap:5px;color:var(--muted);font-size:13px;font-weight:700}}
input{{width:100%;min-height:42px;border:1px solid var(--line);border-radius:7px;background:#11161b;color:var(--text);padding:9px 10px;font:inherit;font-size:16px}}
button{{min-height:42px;border:1px solid #47627f;border-radius:7px;background:#223247;color:var(--text);font:inherit;font-weight:800;cursor:pointer}}
.error{{color:var(--bad);font-weight:800;margin-bottom:10px}}
</style>
</head>
<body>
<main>
  <h1>Plugroot</h1>
  <p>Sign in to the service overview.</p>
  {error_html}
  <form method="post" action="/login">
    <label>Username <input name="user" value="{}" autocomplete="username" required autofocus></label>
    <label>Password <input name="password" type="password" autocomplete="current-password" required></label>
    <button type="submit">Sign In</button>
  </form>
</main>
</body>
</html>"##,
        html_escape(user)
    )
}

pub(crate) fn render_web(ctx: &Context) -> String {
    let rows = status_rows(ctx);
    let total = rows.len();
    let online = rows.iter().filter(|row| row.state == "online").count();
    let attention = rows.iter().filter(|row| web_needs_attention(row)).count();
    let optional_idle = rows
        .iter()
        .filter(|row| row.optional && row.state != "online" && !web_needs_attention(row))
        .count();

    let host_name = ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.name.as_deref())
        .unwrap_or("plugroot-host");
    let private_ip = ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.private_ip.as_deref())
        .unwrap_or("127.0.0.1");
    let state_root = clean_path(&ctx.state_root()).display().to_string();

    let services = render_web_service_lists(&rows);

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-title" content="Plugroot">
<meta name="theme-color" content="#111418">
<title>Plugroot</title>
<style>
:root{{color-scheme:dark;--bg:#111418;--panel:#191e24;--panel-2:#202630;--line:#343b45;--text:#f5f2ea;--muted:#aeb5bf;--ok:#7fd17c;--warn:#e6be63;--bad:#f47c80;--info:#7fb4e8}}
*{{box-sizing:border-box}}
html,body{{min-height:100%;margin:0;background:var(--bg);color:var(--text);font:13px system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;letter-spacing:0}}
a{{color:inherit;text-decoration:none}}
header{{position:sticky;top:0;z-index:2;display:flex;justify-content:space-between;align-items:center;gap:10px;min-height:48px;padding:8px max(12px,env(safe-area-inset-right)) 8px max(12px,env(safe-area-inset-left));border-bottom:1px solid var(--line);background:#151a20}}
h1,h2,h3,p{{margin:0}}
h1{{font-size:20px;line-height:1.1}}
h2{{font-size:15px;line-height:1.2}}
h3{{font-size:14px;line-height:1.2}}
main{{padding:10px max(10px,env(safe-area-inset-right)) max(16px,env(safe-area-inset-bottom)) max(10px,env(safe-area-inset-left))}}
.top-actions{{display:flex;align-items:center;gap:7px}}
.toplink,button,.launch{{display:inline-flex;align-items:center;justify-content:center;min-height:30px;color:var(--text);border:1px solid var(--line);border-radius:6px;padding:5px 9px;background:var(--panel);font:inherit;cursor:pointer}}
.toplink{{color:var(--muted)}}
.hero{{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:8px;align-items:stretch;margin-bottom:8px}}
.hero-main{{min-width:0;padding:8px 10px;border:1px solid var(--line);border-radius:8px;background:var(--panel)}}
.hero-main p{{margin-top:4px;color:var(--muted);line-height:1.3}}
.facts{{display:grid;grid-template-columns:repeat(4,minmax(58px,1fr));gap:5px}}
.fact{{min-height:46px;padding:6px 7px;border:1px solid var(--line);border-radius:8px;background:var(--panel-2)}}
.fact strong{{display:block;font-size:17px;line-height:1}}
.fact span{{display:block;margin-top:3px;color:var(--muted);font-size:10px;white-space:nowrap}}
.meta{{display:flex;gap:6px;flex-wrap:wrap;margin-top:8px}}
.chip{{display:inline-flex;align-items:center;min-height:22px;border:1px solid var(--line);border-radius:6px;padding:2px 6px;color:var(--muted);background:#141920;max-width:100%;overflow-wrap:anywhere;font-size:12px}}
.services{{margin-top:8px}}
.unlinked{{margin-top:14px;padding-top:9px;border-top:1px solid var(--line)}}
.list-head{{display:flex;justify-content:space-between;align-items:center;gap:10px;margin-bottom:5px}}
.list-head p{{color:var(--muted)}}
.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:5px}}
.svc{{border:1px solid var(--line);border-radius:8px;background:var(--panel);overflow:hidden}}
.svc-row{{display:grid;grid-template-columns:minmax(0,1fr) auto auto;gap:7px;align-items:center;min-height:42px;padding:7px 8px}}
.svc-title{{min-width:0;display:grid;gap:2px}}
.svc-direct{{border-radius:5px}}.svc-direct:focus-visible{{outline:2px solid var(--info);outline-offset:2px}}
.svc-title h3{{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}
.svc-title p{{color:var(--muted);font-size:11px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}
.state{{justify-self:end;flex:0 0 auto;border:1px solid currentColor;border-radius:6px;padding:3px 6px;font-size:11px;text-transform:uppercase}}
.state-online{{color:var(--ok)}}.state-attention{{color:var(--bad)}}.state-idle{{color:var(--muted)}}.state-unknown{{color:var(--warn)}}
.svc-more{{border-top:1px solid var(--line)}}
.svc-more summary{{list-style:none;cursor:pointer;padding:5px 8px;color:var(--muted);font-size:11px}}
.svc-more summary::-webkit-details-marker{{display:none}}
.svc-more summary::after{{content:" +"}}.svc-more[open] summary::after{{content:" −"}}
.svc-body{{display:grid;gap:6px;padding:0 8px 8px}}
.detail{{padding-top:7px;color:#d7dbe0;overflow-wrap:anywhere;line-height:1.3}}
.desc{{color:var(--muted);line-height:1.3}}
.svc-meta{{display:flex;gap:5px;flex-wrap:wrap}}
.actions{{display:flex;gap:6px;flex-wrap:wrap;align-items:center}}
.actions form{{margin:0}}
.launch{{background:#233246;border-color:#40556f;font-weight:800}}
.no-launch{{color:var(--muted);font-size:11px;white-space:nowrap}}
.disabled{{color:var(--muted);cursor:default}}
.empty{{border:1px solid var(--line);border-radius:8px;padding:14px;background:var(--panel)}}
.empty p{{margin-top:8px;color:var(--muted)}}
@media (max-width:760px){{.hero{{grid-template-columns:1fr}}.facts{{grid-template-columns:repeat(4,minmax(0,1fr))}}}}
@media (max-width:430px){{body{{font-size:12px}}header{{min-height:42px}}h1{{font-size:18px}}main{{padding-top:7px}}.toplink,.top-actions button{{min-height:27px;padding:4px 7px}}.hero{{gap:6px;margin-bottom:6px}}.hero-main{{display:none}}.facts{{grid-template-columns:repeat(4,minmax(0,1fr));gap:4px}}.fact{{min-height:38px;padding:5px}}.fact strong{{font-size:15px}}.fact span{{font-size:9px}}.services{{margin-top:6px}}.list-head{{display:none}}.unlinked{{margin-top:12px;padding-top:8px}}.unlinked .list-head{{display:flex}}.grid{{grid-template-columns:1fr;gap:4px}}.svc-row{{min-height:48px;padding:5px 6px}}.launch{{min-height:38px;padding:5px 9px}}.svc-more{{display:none}}}}
</style>
</head>
<body>
<header><h1>Plugroot</h1><div class="top-actions"><a class="toplink" href="/api/status">json</a><form method="post" action="/logout"><button type="submit">log out</button></form></div></header>
<main>
<section class="hero">
  <div class="hero-main">
    <h2>Service Harness Overview</h2>
    <p>{} is supervising {} declared service(s). Tap a service name or Open to launch it directly.</p>
    <div class="meta">
      <span class="chip">host {}</span>
      <span class="chip">private {}</span>
      <span class="chip">state {}</span>
    </div>
  </div>
  <div class="facts">
    <div class="fact"><strong>{}</strong><span>services</span></div>
    <div class="fact"><strong>{}</strong><span>online</span></div>
    <div class="fact"><strong>{}</strong><span>attention</span></div>
    <div class="fact"><strong>{}</strong><span>idle</span></div>
  </div>
</section>
{}
</main>
</body>
</html>"##,
        html_escape(host_name),
        total,
        html_escape(host_name),
        html_escape(private_ip),
        html_escape(&state_root),
        total,
        online,
        attention,
        optional_idle,
        services
    )
}

pub(crate) fn render_web_service_lists(rows: &[StatusRow]) -> String {
    if rows.is_empty() {
        return r#"<section class="empty"><h2>No Services</h2><p>Add service entries to plugroot.toml or the private overlay to populate this overview.</p></section>"#
            .into();
    }

    let mut launchable = rows
        .iter()
        .filter(|row| row.url.is_some())
        .collect::<Vec<_>>();
    let mut unlinked = rows
        .iter()
        .filter(|row| row.url.is_none())
        .collect::<Vec<_>>();
    let sort_rows = |rows: &mut Vec<&StatusRow>| {
        rows.sort_by(|left, right| {
            web_needs_attention(right)
                .cmp(&web_needs_attention(left))
                .then_with(|| left.name.cmp(&right.name))
        });
    };
    sort_rows(&mut launchable);
    sort_rows(&mut unlinked);

    let launchable_html = if launchable.is_empty() {
        String::new()
    } else {
        let cards = launchable
            .into_iter()
            .map(render_web_service)
            .collect::<Vec<_>>()
            .join("");
        format!(
            r#"<section class="services"><div class="list-head"><h2>Services</h2><p>Tap a service name or Open to launch it.</p></div><div class="grid">{cards}</div></section>"#
        )
    };
    let unlinked_html = if unlinked.is_empty() {
        String::new()
    } else {
        let cards = unlinked
            .into_iter()
            .map(render_web_service)
            .collect::<Vec<_>>()
            .join("");
        format!(
            r#"<section class="services unlinked"><div class="list-head"><h2>No direct link</h2><p>Status-only services</p></div><div class="grid">{cards}</div></section>"#
        )
    };

    format!("{launchable_html}{unlinked_html}")
}

pub(crate) fn render_web_service(row: &StatusRow) -> String {
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
    let action_html = if actions.is_empty() {
        r#"<span class="chip disabled">No controls</span>"#.into()
    } else {
        actions
    };
    let title = row
        .url
        .as_deref()
        .map(|url| {
            format!(
                r#"<a class="svc-title svc-direct" href="{}"><h3>{}</h3><p>{}</p></a>"#,
                html_escape(url),
                html_escape(&row.name),
                html_escape(&row.detail)
            )
        })
        .unwrap_or_else(|| {
            format!(
                r#"<div class="svc-title"><h3>{}</h3><p>{}</p></div>"#,
                html_escape(&row.name),
                html_escape(&row.detail)
            )
        });
    let launch = row
        .url
        .as_deref()
        .map(|url| {
            format!(
                r#"<a class="launch" href="{}" aria-label="Open {}">Open</a>"#,
                html_escape(url),
                html_escape(&row.name)
            )
        })
        .unwrap_or_else(|| r#"<span class="no-launch">No link</span>"#.into());
    let description = row
        .description
        .as_deref()
        .map(|description| format!(r#"<p class="desc">{}</p>"#, html_escape(description)))
        .unwrap_or_default();
    let ports = if row.ports.is_empty() {
        String::new()
    } else {
        format!(
            r#"<span class="chip">{}</span>"#,
            html_escape(&row.ports.join(", "))
        )
    };
    let repo = row
        .repo
        .as_deref()
        .map(|repo| format!(r#"<span class="chip">repo {}</span>"#, html_escape(repo)))
        .unwrap_or_default();
    let access = row
        .access
        .as_deref()
        .map(|access| {
            format!(
                r#"<span class="chip">access {}</span>"#,
                html_escape(access)
            )
        })
        .unwrap_or_default();
    let optional = if row.optional {
        r#"<span class="chip">optional</span>"#
    } else {
        ""
    };
    format!(
        r#"<article class="svc"><div class="svc-row">{}<span class="state {}">{}</span>{}</div><details class="svc-more"><summary>Details</summary><div class="svc-body"><p class="detail">{}</p>{}<div class="svc-meta"><span class="chip">id {}</span><span class="chip">{}</span>{}{}{}{}</div><div class="actions">{}</div></div></details></article>"#,
        title,
        web_state_class(row),
        html_escape(&row.state),
        launch,
        html_escape(&row.detail),
        description,
        html_escape(&row.id),
        html_escape(&row.kind),
        repo,
        access,
        ports,
        optional,
        action_html
    )
}

pub(crate) fn web_needs_attention(row: &StatusRow) -> bool {
    row.state != "online" && !(row.optional && matches!(row.state.as_str(), "offline" | "missing"))
}

pub(crate) fn web_state_class(row: &StatusRow) -> &'static str {
    if row.state == "online" {
        "state-online"
    } else if !web_needs_attention(row) {
        "state-idle"
    } else if matches!(row.state.as_str(), "offline" | "missing") {
        "state-attention"
    } else {
        "state-unknown"
    }
}

pub(crate) fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
