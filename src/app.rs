// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::time::Duration;

use crate::{
    cli::{Cli, SortArg},
    metrics::ProcRow,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SortMode {
    Cpu,
    Ram,
    Gpu,
    Vram,
    Total,
}

impl SortMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Ram => "ram",
            Self::Gpu => "gpu",
            Self::Vram => "vram",
            Self::Total => "total",
        }
    }
}

impl From<SortArg> for SortMode {
    fn from(s: SortArg) -> Self {
        match s {
            SortArg::Cpu => Self::Cpu,
            SortArg::Ram => Self::Ram,
            SortArg::Gpu => Self::Gpu,
            SortArg::Vram => Self::Vram,
            SortArg::Total => Self::Total,
        }
    }
}

pub struct AppState {
    pub sort: SortMode,
    pub paused: bool,
    pub refresh_secs: f64,
    pub top_n: usize,
    pub show_help: bool,
    pub process_filter: String,
    pub process_offset: usize,
    pub process_match_count: usize,
    pub process_viewport_rows: usize,
    pub search_editing: bool,
    pub gpu_page: usize,
    pub gpu_page_count: usize,
    pub(crate) search_original_filter: String,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            sort: SortMode::Cpu,
            paused: false,
            refresh_secs: 1.0,
            top_n: 10,
            show_help: false,
            process_filter: String::new(),
            process_offset: 0,
            process_match_count: 0,
            process_viewport_rows: 1,
            search_editing: false,
            gpu_page: 0,
            gpu_page_count: 1,
            search_original_filter: String::new(),
        }
    }
}

impl AppState {
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            sort: cli.sort.into(),
            refresh_secs: cli.refresh,
            top_n: cli.top,
            process_filter: cli.filter.clone(),
            ..Self::default()
        }
    }

    pub fn refresh_interval(&self) -> Duration {
        Duration::from_millis((self.refresh_secs * 1000.0) as u64)
    }

    pub fn bump_refresh(&mut self, delta: f64) {
        let new = (self.refresh_secs + delta).clamp(0.25, 5.0);
        self.refresh_secs = (new * 4.0).round() / 4.0;
    }

    pub fn bump_top(&mut self, delta: isize) {
        self.top_n = self.top_n.saturating_add_signed(delta).clamp(1, 200);
        self.clamp_process_offset();
    }

    pub fn max_process_offset(&self) -> usize {
        self.process_match_count
            .min(self.top_n)
            .saturating_sub(self.process_viewport_rows.max(1))
    }

    pub fn clamp_process_offset(&mut self) {
        self.process_offset = self.process_offset.min(self.max_process_offset());
    }

    pub fn scroll_processes(&mut self, delta: isize) {
        self.process_offset = self
            .process_offset
            .saturating_add_signed(delta)
            .min(self.max_process_offset());
    }

    pub fn begin_search(&mut self) {
        self.search_original_filter = self.process_filter.clone();
        self.search_editing = true;
        self.show_help = false;
    }

    pub fn cancel_search(&mut self) {
        self.process_filter = std::mem::take(&mut self.search_original_filter);
        self.search_editing = false;
        self.process_offset = 0;
    }

    pub fn finish_search(&mut self) {
        self.search_original_filter.clear();
        self.search_editing = false;
    }
}

/// The query is normalized once per scan, rather than once per process.
pub fn process_matches(row: &ProcRow, lowercase_query: &str) -> bool {
    lowercase_query.is_empty()
        || row.command.to_lowercase().contains(lowercase_query)
        || row.user.to_lowercase().contains(lowercase_query)
        || row.pid.to_string().contains(lowercase_query)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_bump_clamps_and_snaps_to_quarters() {
        let mut app = AppState::default();
        app.bump_refresh(0.26);
        assert_eq!(app.refresh_secs, 1.25);
        app.bump_refresh(-10.0);
        assert_eq!(app.refresh_secs, 0.25);
        app.bump_refresh(10.0);
        assert_eq!(app.refresh_secs, 5.0);
    }

    #[test]
    fn scrolling_tracks_viewport_and_matching_count() {
        let mut app = AppState {
            process_match_count: 100,
            process_viewport_rows: 5,
            ..AppState::default()
        };
        app.scroll_processes(100);
        assert_eq!(app.process_offset, 5);
        app.bump_top(-8);
        assert_eq!(app.top_n, 2);
        assert_eq!(app.process_offset, 0);
        app.bump_top(-100);
        assert_eq!(app.top_n, 1);
        app.bump_top(500);
        assert_eq!(app.top_n, 200);
    }

    #[test]
    fn process_filter_matches_command_user_and_pid() {
        let row = ProcRow {
            pid: 1234567,
            user: "Alice".into(),
            cpu_pct: 0.0,
            ram_pct: 0.0,
            gpu_pct: None,
            vram_bytes: 0,
            command: "Python TRAIN.py".into(),
        };
        for query in ["", "alice", "train", "1234567"] {
            assert!(process_matches(&row, query));
        }
        assert!(!process_matches(&row, "missing"));
    }
}
