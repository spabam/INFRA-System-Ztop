// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

mod app;
mod cli;
mod config;
mod input;
mod metrics;
mod output;
mod ui;

use anyhow::{Context, Result};
use clap::{CommandFactory, FromArgMatches};
use crossterm::{
    cursor::Show,
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io,
    time::{Duration, Instant},
};

use app::AppState;
use cli::Cli;
use metrics::Collector;

struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        disable_raw_mode().ok();
        execute!(stdout, LeaveAlternateScreen, Show).ok();
    }
}

fn main() -> Result<()> {
    let matches = Cli::command().get_matches();
    let mut cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    config::apply_default_file(&mut cli, &matches).context("config file")?;

    if cli.json || cli.json_stream {
        return output::run(&cli);
    }

    let mut app = AppState::from_cli(&cli);
    let mut collector = Collector::new(&cli)?;

    let mut stdout = io::stdout();
    enable_raw_mode().context("enable raw mode")?;
    let _cleanup = TerminalCleanup;
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    run(&mut terminal, &mut app, &mut collector)
}

fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut AppState,
    collector: &mut Collector,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let mut last_tick = Instant::now();
    collector.sample_gpu();
    let mut snapshot = collector.collect(app);
    terminal.draw(|f| {
        ui::prepare_frame(app, &snapshot, f.area());
        ui::draw(f, app, &snapshot);
    })?;

    loop {
        let now = Instant::now();
        let tick = app.refresh_interval();
        let elapsed = now.duration_since(last_tick);
        let refresh_wait = if app.paused {
            Duration::from_millis(100)
        } else if elapsed >= tick {
            Duration::ZERO
        } else {
            (tick - elapsed).min(Duration::from_millis(100))
        };
        // While paused, "quiet" means quiet: GPU sampling stops too, so a
        // paused ztop costs (almost) nothing.
        let poll_for = if app.paused {
            refresh_wait
        } else {
            collector
                .gpu_sample_wait(now)
                .map(|gpu_wait| refresh_wait.min(gpu_wait))
                .unwrap_or(refresh_wait)
        };

        let mut needs_draw = false;
        if event::poll(poll_for)? {
            match event::read()? {
                Event::Key(key) => {
                    let action = input::handle_key(app, key);
                    if action.quit {
                        return Ok(());
                    }
                    if action.resort {
                        snapshot.sort_processes(app.sort);
                    }
                    needs_draw = action.redraw;
                }
                // Without this, a resized terminal keeps the stale layout
                // until the next tick — or forever while paused.
                Event::Resize(_, _) => needs_draw = true,
                _ => {}
            }
        }

        let now = Instant::now();
        if !app.paused {
            collector.sample_gpu_if_due(now);
        }
        let now = Instant::now();
        if !app.paused && now.duration_since(last_tick) >= app.refresh_interval() {
            snapshot = collector.collect(app);
            last_tick = now;
            needs_draw = true;
        }

        if needs_draw {
            terminal.draw(|f| {
                ui::prepare_frame(app, &snapshot, f.area());
                ui::draw(f, app, &snapshot);
            })?;
        }
    }
}
