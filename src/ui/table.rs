// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::truncate;
use crate::{
    app::{process_matches, AppState, SortMode},
    metrics::Snapshot,
};
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

pub fn draw(f: &mut Frame, area: Rect, snap: &Snapshot, app: &AppState) {
    let show_user = area.width >= 60;
    let show_gpu = area.width >= 70;
    let show_vram = area.width >= 76;
    let vram_total_bytes = snap.vram_total_bytes();
    let query = app.process_filter.to_lowercase();
    let matching = snap
        .procs
        .iter()
        .filter(|row| process_matches(row, &query))
        .take(app.top_n)
        .collect::<Vec<_>>();
    let visible_rows = usize::from(area.height.saturating_sub(3));
    let offset = app
        .process_offset
        .min(matching.len().saturating_sub(visible_rows));
    let shown = matching
        .iter()
        .skip(offset)
        .take(visible_rows)
        .copied()
        .collect::<Vec<_>>();
    let pid_width = shown
        .iter()
        .map(|row| row.pid.to_string().len())
        .max()
        .unwrap_or(7)
        .max(7) as u16;
    let cpu_width = shown
        .iter()
        .map(|row| format!("{:.1}", row.cpu_pct).len())
        .max()
        .unwrap_or(6)
        .max(6) as u16;
    let mut widths = vec![Constraint::Length(pid_width)];
    let mut fixed = pid_width + cpu_width + 6;
    let mut headers = vec!["PID"];
    if show_user {
        headers.push("USER");
        widths.push(Constraint::Length(10));
        fixed += 10;
    }
    let marked = |label: &'static str, mode: SortMode| -> &'static str {
        if app.sort != mode {
            return label;
        }
        match mode {
            SortMode::Cpu => "CPU%▼",
            SortMode::Ram => "RAM%▼",
            SortMode::Gpu => "GPU%▼",
            SortMode::Vram => "VRAM%▼",
            SortMode::Total => label,
        }
    };
    headers.extend([marked("CPU%", SortMode::Cpu), marked("RAM%", SortMode::Ram)]);
    widths.extend([Constraint::Length(cpu_width), Constraint::Length(6)]);
    if show_gpu {
        headers.push(marked("GPU%", SortMode::Gpu));
        widths.push(Constraint::Length(5));
        fixed += 5;
    }
    if show_vram {
        headers.push(marked("VRAM%", SortMode::Vram));
        widths.push(Constraint::Length(6));
        fixed += 6;
    }
    headers.push("COMMAND");
    widths.push(Constraint::Min(1));
    let command_width = area
        .width
        .saturating_sub(2 + fixed + headers.len() as u16 - 1) as usize;
    let rows = shown
        .iter()
        .map(|row| {
            let mut cells = vec![Cell::from(row.pid.to_string())];
            if show_user {
                cells.push(Cell::from(truncate(&row.user, 10)));
            }
            cells.push(Cell::from(format!(
                "{:>width$.1}",
                row.cpu_pct,
                width = cpu_width as usize
            )));
            cells.push(Cell::from(format!("{:5.1}", row.ram_pct)));
            if show_gpu {
                cells.push(Cell::from(
                    row.gpu_pct
                        .map(|p| format!("{p:3}"))
                        .unwrap_or_else(|| "—".into()),
                ));
            }
            if show_vram {
                cells.push(Cell::from(format_vram(row.vram_bytes, vram_total_bytes)));
            }
            cells.push(Cell::from(truncate(&row.command, command_width)));
            Row::new(cells)
        })
        .collect::<Vec<_>>();
    let title = if matching.is_empty() {
        " Processes · no matches ".into()
    } else {
        format!(
            " Processes {}–{} / {} ",
            offset + 1,
            offset + shown.len(),
            matching.len()
        )
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    if matching.is_empty() {
        f.render_widget(
            Paragraph::new("No processes match the filter.").block(block),
            area,
        );
    } else {
        let header = Row::new(headers).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(
            Table::new(rows, widths)
                .header(header)
                .column_spacing(1)
                .block(block),
            area,
        );
    }
}

fn format_vram(bytes: u64, total_bytes: u64) -> String {
    if bytes == 0 || total_bytes == 0 {
        return "—".into();
    }
    format!("{:5.1}", bytes as f64 / total_bytes as f64 * 100.0)
}
