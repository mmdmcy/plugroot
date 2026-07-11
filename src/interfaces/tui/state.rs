use crate::has_control;
use crate::logs_for;
use crate::run_fixed_action;
use crate::status_rows;
use crate::trim_output;
use crate::tui_content_width;
use crate::Context;
use crate::Instant;
use crate::Service;
use crate::StatusRow;
use crate::Stdout;
use std::io;

use crossterm::{
    cursor::{Hide, Show},
    execute,
    style::ResetColor,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};

pub(crate) struct TuiApp {
    pub(crate) ctx: Context,
    pub(crate) rows: Vec<StatusRow>,
    pub(crate) selected: usize,
    pub(crate) scroll_x: usize,
    pub(crate) scroll_y: usize,
    pub(crate) message: String,
    pub(crate) last_refresh: Instant,
}

impl TuiApp {
    pub(crate) fn new(ctx: Context) -> Self {
        let rows = status_rows(&ctx);
        Self {
            ctx,
            rows,
            selected: 0,
            scroll_x: 0,
            scroll_y: 0,
            message: String::new(),
            last_refresh: Instant::now(),
        }
    }

    pub(crate) fn refresh(&mut self) {
        let selected_id = self.rows.get(self.selected).map(|row| row.id.clone());
        let selected_index = self.selected;
        match Context::load(self.ctx.root.clone()) {
            Ok(ctx) => self.ctx = ctx,
            Err(err) => {
                self.message = format!("refresh failed: {err}");
                return;
            }
        }
        self.rows = status_rows(&self.ctx);
        self.restore_selection(selected_id.as_deref(), selected_index);
        self.last_refresh = Instant::now();
    }

    pub(crate) fn restore_selection(&mut self, selected_id: Option<&str>, selected_index: usize) {
        if self.rows.is_empty() {
            self.selected = 0;
            self.scroll_y = 0;
            return;
        }
        if let Some(id) = selected_id {
            if let Some(index) = self.rows.iter().position(|row| row.id == id) {
                self.selected = index;
                return;
            }
        }
        self.selected = selected_index.min(self.rows.len().saturating_sub(1));
    }

    pub(crate) fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub(crate) fn down(&mut self) {
        if self.selected + 1 < self.rows.len() {
            self.selected += 1;
        }
    }

    pub(crate) fn right(&mut self) {
        self.scroll_x = self.scroll_x.saturating_add(8);
    }

    pub(crate) fn left(&mut self) {
        self.scroll_x = self.scroll_x.saturating_sub(8);
    }

    pub(crate) fn page_down(&mut self, visible_rows: usize) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = (self.selected + visible_rows.max(1)).min(self.rows.len() - 1);
    }

    pub(crate) fn page_up(&mut self, visible_rows: usize) {
        self.selected = self.selected.saturating_sub(visible_rows.max(1));
    }

    pub(crate) fn home(&mut self) {
        self.selected = 0;
        self.scroll_y = 0;
        self.scroll_x = 0;
    }

    pub(crate) fn end(&mut self) {
        if !self.rows.is_empty() {
            self.selected = self.rows.len() - 1;
        }
    }

    pub(crate) fn clamp_viewport(&mut self, visible_rows: usize, width: usize) {
        if self.rows.is_empty() {
            self.selected = 0;
            self.scroll_y = 0;
            self.scroll_x = 0;
            return;
        }

        let visible_rows = visible_rows.max(1);
        self.selected = self.selected.min(self.rows.len() - 1);
        if self.selected < self.scroll_y {
            self.scroll_y = self.selected;
        } else if self.selected >= self.scroll_y + visible_rows {
            self.scroll_y = self.selected + 1 - visible_rows;
        }
        self.scroll_y = self
            .scroll_y
            .min(self.rows.len().saturating_sub(visible_rows));
        self.scroll_x = self
            .scroll_x
            .min(tui_content_width(self).saturating_sub(width));
    }

    pub(crate) fn selected_service(&self) -> Option<Service> {
        self.rows
            .get(self.selected)
            .and_then(|row| self.ctx.service(&row.id))
    }

    pub(crate) fn action(&mut self, action: &str) {
        let Some(service) = self.selected_service() else {
            return;
        };
        if !has_control(&service, action) {
            self.message = format!("{} does not allow `{action}`", service.name);
            return;
        }
        let output = if action == "logs" {
            logs_for(&self.ctx, &service)
        } else {
            run_fixed_action(&self.ctx, &service, action)
        };
        self.message = format!(
            "{} {}: exit {}\n{}",
            service.name,
            action,
            output.code,
            trim_output(&output.text, 2200)
        );
        self.refresh();
    }
}

pub(crate) struct TerminalGuard {
    pub(crate) stdout: Stdout,
}

impl TerminalGuard {
    pub(crate) fn enter() -> io::Result<Self> {
        let mut stdout = io::stdout();
        terminal::enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, Hide)?;
        Ok(Self { stdout })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, Show, LeaveAlternateScreen, ResetColor);
        let _ = terminal::disable_raw_mode();
    }
}
