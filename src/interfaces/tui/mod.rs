use crate::print_status_table;
use crate::status_rows;
use crate::Context;
use crate::Duration;
use crate::Range;
use crate::StatusRow;
use crate::Stdout;
use std::io;
use std::io::Write;

use crossterm::cursor::MoveTo;
use crossterm::{
    event::{self, Event, KeyCode},
    queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};

mod state;

pub(crate) use state::{TerminalGuard, TuiApp};

pub(crate) fn cmd_tui(ctx: Context, args: &[String]) -> io::Result<u8> {
    if args.iter().any(|arg| arg == "--once") {
        print_status_table(&status_rows(&ctx));
        return Ok(0);
    }

    let mut app = TuiApp::new(ctx);
    let mut term = TerminalGuard::enter()?;
    loop {
        draw_tui(&mut term.stdout, &mut app)?;
        if event::poll(Duration::from_millis(500))? {
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Down | KeyCode::Char('j') => app.down(),
                    KeyCode::Up | KeyCode::Char('k') => app.up(),
                    KeyCode::Right => app.right(),
                    KeyCode::Left => app.left(),
                    KeyCode::PageDown => app.page_down(tui_list_capacity(terminal::size()?.1)),
                    KeyCode::PageUp => app.page_up(tui_list_capacity(terminal::size()?.1)),
                    KeyCode::Home => app.home(),
                    KeyCode::End => app.end(),
                    KeyCode::Char('r') => app.refresh(),
                    KeyCode::Char('l') => app.action("logs"),
                    KeyCode::Char('o') => app.action("start"),
                    KeyCode::Char('e') => app.action("restart"),
                    KeyCode::Char('f') => app.action("stop"),
                    _ => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    Ok(0)
}

pub(crate) fn draw_tui(stdout: &mut Stdout, app: &mut TuiApp) -> io::Result<()> {
    let (terminal_width, height) = terminal::size()?;
    let width = terminal_width as usize;
    let available = tui_list_capacity(height);
    app.clamp_viewport(available, width);

    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
    let online = app.rows.iter().filter(|row| row.state == "online").count();
    let name = app
        .ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.name.as_deref())
        .unwrap_or("plugroot");
    let ip = app
        .ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.private_ip.as_deref())
        .unwrap_or("unknown");
    let user = app
        .ctx
        .manifest
        .host
        .as_ref()
        .and_then(|host| host.user.as_deref())
        .unwrap_or("-");
    queue!(
        stdout,
        SetAttribute(Attribute::Bold),
        Print(fit(
            &format!("Plugroot - {name} - {ip} - user {user}"),
            width
        )),
        SetAttribute(Attribute::Reset),
        MoveTo(0, 1),
        Print(fit(
            &format!(
                "{} online, {} attention, {} tracked - arrows move/pan - r refresh - l logs - o on - e restart - f off - q quit",
                online,
                app.rows.len().saturating_sub(online),
                app.rows.len()
            ),
            width
        )),
        MoveTo(0, 2),
        Print(line(terminal_width))
    )?;

    let list_top = 3u16;
    let visible = visible_rows(app, available);
    queue!(
        stdout,
        MoveTo(0, list_top),
        SetForegroundColor(Color::DarkGrey),
        Print(fit_window(
            "  STATE    PLANE    CATEGORY/KIND  SERVICE                         ACCESS",
            width,
            app.scroll_x
        )),
        ResetColor
    )?;
    for (row_index, row) in app.rows[visible.clone()].iter().enumerate() {
        let actual = visible.start + row_index;
        let y = list_top + 1 + row_index as u16;
        let marker = if actual == app.selected { ">" } else { " " };
        let category_kind = format!("{}/{}", row.category, row.kind);
        let access = row.url.as_deref().or(row.access.as_deref()).unwrap_or("-");
        let text = tui_row_text(marker, row, &category_kind, access);
        queue!(stdout, MoveTo(0, y))?;
        if actual == app.selected {
            queue!(stdout, SetAttribute(Attribute::Reverse))?;
        }
        queue!(
            stdout,
            SetForegroundColor(color_for_state(&row.state)),
            Print(fit_window(&text, width, app.scroll_x)),
            ResetColor
        )?;
        if actual == app.selected {
            queue!(stdout, SetAttribute(Attribute::Reset))?;
        }
    }

    let base = height.saturating_sub(5);
    queue!(stdout, MoveTo(0, base), Print(line(terminal_width)))?;
    if let Some(row) = app.rows.get(app.selected) {
        let access = row.url.as_deref().or(row.access.as_deref()).unwrap_or("-");
        queue!(
            stdout,
            MoveTo(0, base + 1),
            SetAttribute(Attribute::Bold),
            Print(fit(&format!("{} ({})", row.name, row.id), width)),
            SetAttribute(Attribute::Reset),
            MoveTo(0, base + 2),
            Print(fit(
                row.description.as_deref().unwrap_or(&row.detail),
                width
            )),
            MoveTo(0, base + 3),
            Print(fit_window(
                &tui_detail_text(row, access),
                width,
                app.scroll_x
            )),
            MoveTo(0, base + 4),
            Print(fit_window(&app.message, width, app.scroll_x))
        )?;
    }
    stdout.flush()?;
    Ok(())
}

pub(crate) fn tui_list_capacity(height: u16) -> usize {
    let list_top = 3u16;
    let footer_start = height.saturating_sub(5);
    footer_start.saturating_sub(list_top + 1) as usize
}

pub(crate) fn visible_rows(app: &TuiApp, available: usize) -> Range<usize> {
    let end = (app.scroll_y + available).min(app.rows.len());
    app.scroll_y..end
}

pub(crate) fn tui_row_text(
    marker: &str,
    row: &StatusRow,
    category_kind: &str,
    access: &str,
) -> String {
    format!(
        "{marker} {:<8} {:<8} {:<14} {:<30} {}",
        row.state, row.plane, category_kind, row.name, access
    )
}

pub(crate) fn tui_content_width(app: &TuiApp) -> usize {
    let mut max_width =
        visible_width("  STATE    PLANE    CATEGORY/KIND  SERVICE                         ACCESS");
    for row in &app.rows {
        let category_kind = format!("{}/{}", row.category, row.kind);
        let access = row.url.as_deref().or(row.access.as_deref()).unwrap_or("-");
        max_width = max_width.max(visible_width(&tui_row_text(
            ">",
            row,
            &category_kind,
            access,
        )));
        max_width = max_width.max(visible_width(&tui_detail_text(row, access)));
    }
    max_width
}

pub(crate) fn tui_detail_text(row: &StatusRow, access: &str) -> String {
    format!(
        "access: {} | detail: {} | ports: {} | controls: {}",
        access,
        row.detail,
        if row.ports.is_empty() {
            "none".into()
        } else {
            row.ports.join(",")
        },
        if row.controls.is_empty() {
            "none".into()
        } else {
            row.controls.join(",")
        }
    )
}

pub(crate) fn color_for_state(state: &str) -> Color {
    match state {
        "online" => Color::Green,
        "offline" | "missing" => Color::Red,
        "partial" | "unknown" => Color::Yellow,
        _ => Color::White,
    }
}

pub(crate) fn line(width: u16) -> String {
    "-".repeat(width as usize)
}

pub(crate) fn fit(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut text = value.replace('\n', " ");
    if text.chars().count() > width {
        text = text
            .chars()
            .take(width.saturating_sub(1))
            .collect::<String>();
        text.push('~');
    }
    format!("{text:<width$}")
}

pub(crate) fn fit_window(value: &str, width: usize, offset: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let text = value.replace('\n', " ");
    let chars: Vec<char> = text.chars().collect();
    let offset = offset.min(chars.len().saturating_sub(1));
    let mut window: Vec<char> = chars.iter().skip(offset).take(width).copied().collect();
    let has_left = offset > 0;
    let has_right = offset + width < chars.len();

    if has_left && !window.is_empty() {
        window[0] = '<';
    }
    if has_right && !window.is_empty() {
        let last = window.len() - 1;
        window[last] = '>';
    }

    let text: String = window.into_iter().collect();
    format!("{text:<width$}")
}

pub(crate) fn visible_width(value: &str) -> usize {
    value.replace('\n', " ").chars().count()
}

pub(crate) fn trim_output(value: &str, max_chars: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let tail: String = value
        .chars()
        .rev()
        .take(max_chars)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("[truncated]\n{tail}")
}
