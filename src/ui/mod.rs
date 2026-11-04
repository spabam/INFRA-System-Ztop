// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

mod bars;
mod table;

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{process_matches, AppState, History},
    metrics::{GpuSampleStatus, GpuSnapshot, Snapshot},
};

const CORE_LABEL_WIDTH: usize = 6;
const MIN_CORE_INDEX_WIDTH: usize = 2;
const CORE_USAGE_WIDTH: usize = 4;
const BAR_START_COL: usize = 34;
const LABEL: Style = Style::new().fg(Color::Cyan);
const BRAND: Style = Style::new().fg(Color::Gray);
const TEMP: Style = Style::new().fg(Color::Yellow);
const TITLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

struct PanelLayout {
    header: Rect,
    cpu: Rect,
    memory: Rect,
    gpus: Vec<(usize, Rect)>,
    gpu_none: Option<Rect>,
    history: Option<Rect>,
    table: Rect,
    footer: Rect,
    compact: bool,
    core_rows: usize,
    gpu_pages: usize,
}

fn take_rows(remaining: &mut Rect, height: u16) -> Rect {
    let height = height.min(remaining.height);
    let result = Rect::new(remaining.x, remaining.y, remaining.width, height);
    remaining.y += height;
    remaining.height -= height;
    result
}

fn panel_layout(area: Rect, app: &AppState, snap: &Snapshot) -> PanelLayout {
    let compact = area.height < 22;
    let n_gpus = snap.gpus.len().max(1);
    let mut core_rows = 0;
    let mut history_height = 0;
    let (gpu_count, gpu_height) = if compact {
        (1, 1)
    } else {
        // Header + footer use three rows. Always reserve eight for processes.
        let budget = area.height.saturating_sub(3 + 8 + 4 + 4);
        // Per-core rows are mandatory: no key hides them, and they take the
        // flexible space first, ahead of GPU panel height, extra GPU panels,
        // and the history strip. Only one three-row GPU panel is kept ahead
        // of them, so they are trimmed solely when the terminal cannot hold
        // them next to the eight process rows at all.
        let n_cores = snap.cpu_per_core_pct.len().max(1);
        let desired = n_cores.div_ceil(cores_per_row(area.width, n_cores));
        core_rows = desired.min(usize::from(budget.saturating_sub(3)));
        let gpu_budget = budget.saturating_sub(core_rows as u16);
        let count = usize::from(gpu_budget / 3).max(1).min(n_gpus);
        let height = if gpu_budget >= count as u16 * 5 { 5 } else { 3 };
        let spare = gpu_budget.saturating_sub(count as u16 * height);
        let requested_history = if snap.gpus.is_empty() { 4 } else { 6 };
        if !app.histories.cpu.is_empty() && spare >= requested_history + 2 {
            history_height = requested_history;
        }
        (count, height)
    };
    let gpu_pages = n_gpus.div_ceil(gpu_count);
    let page = app.gpu_page.min(gpu_pages.saturating_sub(1));
    let mut remaining = area;
    let header = take_rows(&mut remaining, 1);
    let cpu = take_rows(
        &mut remaining,
        if compact { 1 } else { 4 + core_rows as u16 },
    );
    let memory = take_rows(&mut remaining, if compact { 1 } else { 4 });
    let mut gpus = Vec::new();
    let mut gpu_none = None;
    if snap.gpus.is_empty() {
        gpu_none = Some(take_rows(&mut remaining, if compact { 1 } else { 3 }));
    } else {
        for index in (page * gpu_count)..((page + 1) * gpu_count).min(snap.gpus.len()) {
            gpus.push((index, take_rows(&mut remaining, gpu_height)));
        }
    }
    let history = (history_height > 0).then(|| take_rows(&mut remaining, history_height));
    let table_height = remaining.height.saturating_sub(2);
    let table = take_rows(&mut remaining, table_height);
    let footer = remaining;
    PanelLayout {
        header,
        cpu,
        memory,
        gpus,
        gpu_none,
        history,
        table,
        footer,
        compact,
        core_rows,
        gpu_pages,
    }
}

/// Refresh navigation bounds before rendering or processing the next key.
pub fn prepare_frame(app: &mut AppState, snap: &Snapshot, area: Rect) {
    let layout = panel_layout(area, app, snap);
    let query = app.process_filter.to_lowercase();
    app.process_match_count = snap
        .procs
        .iter()
        .filter(|row| process_matches(row, &query))
        .count();
    app.process_viewport_rows = usize::from(layout.table.height.saturating_sub(3)).max(1);
    app.clamp_process_offset();
    app.gpu_page_count = layout.gpu_pages;
    app.gpu_page = app.gpu_page.min(layout.gpu_pages.saturating_sub(1));
}

pub fn draw(f: &mut Frame, app: &AppState, snap: &Snapshot) {
    let area = f.area();
    if area.width < 40 || area.height < 10 {
        let text = if area.width < 40 {
            "terminal too narrow (need ≥ 40 cols)"
        } else {
            "terminal too short (need ≥ 10 rows)"
        };
        f.render_widget(
            Paragraph::new(text).style(Style::default().fg(Color::Red)),
            area,
        );
        return;
    }
    let layout = panel_layout(area, app, snap);
    draw_header(f, layout.header, app, snap);
    if layout.compact {
        draw_compact_cpu(f, layout.cpu, snap);
        draw_compact_memory(f, layout.memory, snap);
    } else {
        draw_cpu(f, layout.cpu, snap, layout.core_rows);
        draw_memory(f, layout.memory, snap);
    }
    if let Some(area) = layout.gpu_none {
        draw_gpu_none(f, area);
    }
    for &(index, area) in &layout.gpus {
        draw_gpu(f, area, &snap.gpus[index], snap, layout.gpu_pages);
    }
    if let Some(area) = layout.history {
        draw_history(
            f,
            area,
            app,
            snap,
            layout.gpus.first().map(|(index, _)| *index),
        );
    }
    table::draw(f, layout.table, snap, app);
    draw_footer(f, layout.footer, app);
    if app.show_help {
        draw_help(f, area);
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &AppState, snap: &Snapshot) {
    let version = if area.width >= 100 {
        concat!(" v", env!("CARGO_PKG_VERSION"))
    } else {
        concat!(
            " v",
            env!("CARGO_PKG_VERSION_MAJOR"),
            ".",
            env!("CARGO_PKG_VERSION_MINOR"),
            ".",
            env!("CARGO_PKG_VERSION_PATCH")
        )
    };
    let details = format!(
        "  {:.2}s  {}  up {}",
        app.refresh_secs,
        snap.hostname,
        format_uptime(snap.uptime_secs)
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("ZTOP", TITLE),
            Span::styled(version, BRAND),
            Span::raw(details),
        ])),
        area,
    );
}

fn draw_compact_cpu(f: &mut Frame, area: Rect, snap: &Snapshot) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("CPU ", LABEL),
            Span::raw(format!(
                "{:5.1}%  {}  temp {}",
                snap.cpu_total_pct,
                cpu_topology(snap),
                cpu_temperature(snap)
            )),
        ])),
        area,
    );
}

fn cpu_temperature(snap: &Snapshot) -> String {
    snap.cpu_temp_c
        .map(|t| format!("{t:.1}°C"))
        .unwrap_or_else(|| "n/a".into())
}

/// `12 cores / 14 threads`, or just `14 threads` when the platform hides the
/// physical core count (some containers and virtual machines do).
fn cpu_topology(snap: &Snapshot) -> String {
    let threads = snap.cpu_per_core_pct.len().max(1);
    match snap.cpu_physical_cores {
        Some(cores) => format!("{cores} cores / {threads} threads"),
        None => format!("{threads} threads"),
    }
}

/// CPU panel title: ` CPU · 12 cores / 14 threads `, or the bare ` CPU ` when
/// the counts do not fit. No key hints: `C` is documented in the help overlay.
fn cpu_title(snap: &Snapshot, max_width: usize) -> String {
    let title = format!(" CPU · {} ", cpu_topology(snap));
    if title.width() <= max_width {
        title
    } else {
        " CPU ".to_string()
    }
}

fn draw_cpu(f: &mut Frame, area: Rect, snap: &Snapshot, core_rows: usize) {
    let prefix = format!(
        "   {:5.1}%   temp: {}",
        snap.cpu_total_pct,
        cpu_temperature(snap)
    );
    let mut total = vec![
        Span::styled("CPU", LABEL),
        Span::raw(format!("   {:5.1}%   ", snap.cpu_total_pct)),
        Span::styled("temp", LABEL),
        Span::raw(": "),
        Span::styled(cpu_temperature(snap), TEMP),
    ];
    append_aligned_bar(&mut total, 3 + prefix.width(), snap.cpu_total_pct, area);
    let mut lines = vec![
        Line::styled(
            truncate(&snap.cpu_brand, area.width.saturating_sub(2) as usize),
            BRAND,
        ),
        Line::from(total),
    ];
    let cols = cores_per_row(area.width, snap.cpu_per_core_pct.len().max(1));
    let index_width = core_index_width(snap.cpu_per_core_pct.len().max(1));
    for (row, cores) in snap
        .cpu_per_core_pct
        .chunks(cols)
        .take(core_rows)
        .enumerate()
    {
        let cells = cores
            .iter()
            .enumerate()
            .map(|(col, p)| format!("{index:>index_width$}:{p:3.0}%", index = row * cols + col))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(Line::from(vec![
            Span::styled(if row == 0 { "Cores " } else { "      " }, LABEL),
            Span::raw(cells),
        ]));
    }
    let title = cpu_title(snap, area.width.saturating_sub(2) as usize);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title, TITLE)),
        ),
        area,
    );
}

fn draw_compact_memory(f: &mut Frame, area: Rect, snap: &Snapshot) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("RAM ", LABEL),
            Span::raw(format!(
                "{:.1}/{:.1} GiB  swap {:.1} GiB",
                gib(snap.ram_used_bytes),
                gib(snap.ram_total_bytes),
                gib(snap.swap_used_bytes)
            )),
        ])),
        area,
    );
}

fn memory_line(label: &'static str, used: u64, total: u64, area: Rect) -> Line<'static> {
    let pct = used as f64 / total.max(1) as f64 * 100.0;
    let text = format!("  {:5.1} / {:5.1} GiB {pct:5.1}%", gib(used), gib(total));
    let mut spans = vec![Span::styled(label, LABEL), Span::raw(text.clone())];
    append_aligned_bar(&mut spans, label.width() + text.width(), pct as f32, area);
    Line::from(spans)
}

fn draw_memory(f: &mut Frame, area: Rect, snap: &Snapshot) {
    let swap = if snap.swap_total_bytes == 0 {
        Line::from(vec![
            Span::styled("SWAP", LABEL),
            Span::raw("  none configured"),
        ])
    } else {
        memory_line("SWAP", snap.swap_used_bytes, snap.swap_total_bytes, area)
    };
    f.render_widget(
        Paragraph::new(vec![
            memory_line("RAM ", snap.ram_used_bytes, snap.ram_total_bytes, area),
            swap,
        ])
        .block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_gpu_none(f: &mut Frame, area: Rect) {
    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled("GPU", LABEL),
        Span::raw(" n/a (no device or disabled)"),
    ]));
    f.render_widget(
        if area.height > 1 {
            paragraph.block(Block::default().borders(Borders::ALL))
        } else {
            paragraph
        },
        area,
    );
}

fn gpu_state(g: &GpuSnapshot, snap: &Snapshot) -> String {
    match g.status {
        GpuSampleStatus::Fresh => String::new(),
        GpuSampleStatus::Stale => format!(
            " · stale {:.1}s",
            snap.sampled_at_unix_ms.saturating_sub(g.sampled_at_unix_ms) as f64 / 1000.0
        ),
        GpuSampleStatus::Unavailable => " · unavailable".into(),
    }
}

fn draw_gpu(f: &mut Frame, area: Rect, g: &GpuSnapshot, snap: &Snapshot, pages: usize) {
    let index = g.index;
    let available = g.status != GpuSampleStatus::Unavailable;
    let util = if available {
        g.util_pct.map(|v| format!("{v}%"))
    } else {
        None
    }
    .unwrap_or_else(|| "n/a".into());
    let temp = if available {
        g.temp_c.map(|v| format!("{v}°C"))
    } else {
        None
    }
    .unwrap_or_else(|| "n/a".into());
    let vram = if available {
        format!(
            "{:.1}/{:.1} GiB",
            gib(g.vram_used_bytes),
            gib(g.vram_total_bytes)
        )
    } else {
        "n/a".into()
    };
    if area.height == 1 {
        let state = gpu_state(g, snap);
        f.render_widget(
            Paragraph::new(format!("GPU{index} {util}  VRAM {vram}{state}")),
            area,
        );
        return;
    }
    let page_hint = if pages > 1 { " · { } GPUs" } else { "" };
    let title = format!(" GPU{index}{}{} ", gpu_state(g, snap), page_hint);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, TITLE));
    if area.height < 5 {
        let text = if area.width < 60 {
            format!("GPU {util}  VRAM {vram}")
        } else {
            format!("GPU {util}  temp {temp}  VRAM {vram}")
        };
        let block = block.title_bottom(truncate(&g.name, area.width.saturating_sub(2) as usize));
        f.render_widget(Paragraph::new(text).block(block), area);
        return;
    }
    let peak = if available {
        g.util_peak_pct
            .map(|p| format!(" peak {p}%"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let gpu_text = format!("   {util}{peak}  temp: {temp}");
    let mut spans = vec![
        Span::styled("GPU", LABEL),
        Span::raw(format!("   {util}{peak}  ")),
        Span::styled("temp", LABEL),
        Span::raw(": "),
        Span::styled(temp, TEMP),
    ];
    if available {
        if let Some(pct) = g.util_pct {
            append_aligned_bar(&mut spans, 3 + gpu_text.width(), pct as f32, area);
        }
    }
    let memory = if available {
        memory_line("VRAM", g.vram_used_bytes, g.vram_total_bytes, area)
    } else {
        Line::from(vec![Span::styled("VRAM", LABEL), Span::raw(" n/a")])
    };
    let mut name = g.name.clone();
    if available {
        if let Some(peak) = g.vram_peak_used_bytes {
            name.push_str(&format!(" | peak VRAM {:.1} GiB", gib(peak)));
        }
    }
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                truncate(&name, area.width.saturating_sub(2) as usize),
                BRAND,
            ),
            Line::from(spans),
            memory,
        ])
        .block(block),
        area,
    );
}

fn draw_history(
    f: &mut Frame,
    area: Rect,
    app: &AppState,
    snap: &Snapshot,
    gpu_index: Option<usize>,
) {
    let width = area.width.saturating_sub(10) as usize;
    let mut lines = vec![
        history_line("CPU", &app.histories.cpu, width),
        history_line("RAM", &app.histories.ram, width),
    ];
    if let Some(index) = gpu_index {
        let gpu = &snap.gpus[index];
        if let Some(history) = app.histories.gpu.get(&gpu.id) {
            lines.push(history_line(&format!("GPU{}", gpu.index), history, width));
        }
        if let Some(history) = app.histories.vram.get(&gpu.id) {
            lines.push(history_line("VRAM", history, width));
        }
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(" Recent samples · 0–100% ", TITLE)),
        ),
        area,
    );
}

fn history_line(label: &str, history: &History, width: usize) -> Line<'static> {
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let spark: String = history
        .iter()
        .skip(history.len().saturating_sub(width))
        .map(|value| {
            value
                .map(|v| LEVELS[(v.min(100) * 7 / 100) as usize])
                .unwrap_or(' ')
        })
        .collect();
    Line::from(vec![
        Span::styled(format!("{label:<6}"), LABEL),
        Span::styled(
            format!(
                "{:>width$}{spark}",
                "",
                width = width.saturating_sub(history.len())
            ),
            Style::default().fg(Color::Green),
        ),
    ])
}

fn draw_footer(f: &mut Frame, area: Rect, app: &AppState) {
    let limit = app.process_match_count.min(app.top_n);
    let first = if limit == 0 {
        0
    } else {
        app.process_offset + 1
    };
    let last = (app.process_offset + app.process_viewport_rows).min(limit);
    let status = format!(
        "sort:{} {} rows:{first}–{last}/{limit} matches:{}",
        app.sort.label(),
        if app.paused { "PAUSED" } else { "live" },
        app.process_match_count
    );
    let search = if app.search_editing {
        format!("/ {}▏  Enter apply · Esc cancel", app.process_filter)
    } else if !app.process_filter.is_empty() {
        format!("/ {}  Esc clear · ↑↓ scroll · ? help", app.process_filter)
    } else {
        "/ filter · ↑↓ scroll · ? help · q quit".into()
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(truncate(&status, area.width as usize)),
            Line::styled(truncate(&search, area.width as usize), LABEL),
        ]),
        area,
    );
}

fn draw_help(f: &mut Frame, area: Rect) {
    let keys = [
        ("q Ctrl-C", "quit"),
        ("Esc", "close help / clear filter / quit"),
        ("c r g v t", "sort CPU RAM GPU VRAM total"),
        ("Space", "pause / resume"),
        ("/", "search command, user, or PID"),
        ("Enter Esc", "apply / cancel search"),
        ("↑ ↓", "scroll process rows"),
        ("PgUp PgDn", "scroll a page"),
        ("Home End", "first / last process page"),
        ("{ }", "previous / next GPU page"),
        ("+ -", "refresh ±0.25s"),
        ("[ ]", "process limit ±1"),
        ("?", "toggle help"),
    ];
    let width = area.width.min(64);
    let height = (keys.len() as u16 + 2).min(area.height);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let lines = keys
        .iter()
        .map(|(key, action)| {
            Line::from(vec![
                Span::styled(format!("{key:<10}"), LABEL),
                Span::raw(*action),
            ])
        })
        .collect::<Vec<_>>();
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(" Keys ", TITLE)),
        ),
        popup,
    );
}

fn format_uptime(secs: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}
fn gib(bytes: u64) -> f64 {
    bytes as f64 / 1_073_741_824.0
}

/// Truncate in terminal columns without splitting combining sequences or emoji.
pub(super) fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for grapheme in s.graphemes(true) {
        let grapheme_width = grapheme.width();
        if used + grapheme_width > width - 1 {
            break;
        }
        out.push_str(grapheme);
        used += grapheme_width;
    }
    out.push('…');
    out
}

fn append_aligned_bar(spans: &mut Vec<Span<'static>>, prefix_width: usize, pct: f32, area: Rect) {
    let padding = BAR_START_COL.saturating_sub(prefix_width).max(1);
    let width = usize::from(area.width.saturating_sub(2)).saturating_sub(prefix_width + padding);
    if width >= 4 {
        spans.push(Span::raw(" ".repeat(padding)));
        spans.extend(bars::bar_spans(pct, width as u16));
    }
}

fn cores_per_row(area_width: u16, n_cores: usize) -> usize {
    let available = usize::from(area_width.saturating_sub(2)).saturating_sub(CORE_LABEL_WIDTH);
    ((available + 1) / (core_cell_width(n_cores.max(1)) + 1)).max(1)
}
fn core_cell_width(n_cores: usize) -> usize {
    core_index_width(n_cores) + 1 + CORE_USAGE_WIDTH
}
fn core_index_width(n_cores: usize) -> usize {
    n_cores
        .saturating_sub(1)
        .to_string()
        .len()
        .max(MIN_CORE_INDEX_WIDTH)
}

#[cfg(test)]
mod tests;
