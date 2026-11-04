// Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{AppState, SortMode};

#[derive(Default, Debug, PartialEq, Eq)]
pub struct InputOutcome {
    pub quit: bool,
    pub redraw: bool,
    pub resort: bool,
}

pub fn handle_key(app: &mut AppState, key: KeyEvent) -> InputOutcome {
    let mut outcome = InputOutcome::default();
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return outcome;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'C'))
    {
        outcome.quit = true;
        return outcome;
    }
    if app.search_editing {
        match key.code {
            KeyCode::Esc => app.cancel_search(),
            KeyCode::Enter => app.finish_search(),
            KeyCode::Backspace => {
                if let Some((index, _)) = app.process_filter.grapheme_indices(true).next_back() {
                    app.process_filter.truncate(index);
                }
                app.process_offset = 0;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.process_filter.clear();
                app.process_offset = 0;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                app.process_filter.push(c);
                app.process_offset = 0;
            }
            _ => return outcome,
        }
        outcome.redraw = true;
        return outcome;
    }
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return outcome;
    }
    match key.code {
        KeyCode::Char('q') => outcome.quit = true,
        KeyCode::Esc => {
            if app.show_help {
                app.show_help = false;
            } else if !app.process_filter.is_empty() {
                app.process_filter.clear();
                app.process_offset = 0;
            } else {
                outcome.quit = true;
            }
        }
        KeyCode::Char('?') => app.show_help = !app.show_help,
        KeyCode::Char('/') => app.begin_search(),
        KeyCode::Char('{') => app.gpu_page = app.gpu_page.saturating_sub(1),
        KeyCode::Char('}') => {
            app.gpu_page = (app.gpu_page + 1).min(app.gpu_page_count.saturating_sub(1))
        }
        KeyCode::Up => app.scroll_processes(-1),
        KeyCode::Down => app.scroll_processes(1),
        KeyCode::PageUp => app.scroll_processes(-(app.process_viewport_rows as isize)),
        KeyCode::PageDown => app.scroll_processes(app.process_viewport_rows as isize),
        KeyCode::Home => app.process_offset = 0,
        KeyCode::End => app.process_offset = app.max_process_offset(),
        KeyCode::Char(' ') => app.paused = !app.paused,
        KeyCode::Char('+') | KeyCode::Char('=') => app.bump_refresh(0.25),
        KeyCode::Char('-') | KeyCode::Char('_') => app.bump_refresh(-0.25),
        KeyCode::Char('[') => app.bump_top(-1),
        KeyCode::Char(']') => app.bump_top(1),
        KeyCode::Char(c @ ('c' | 'r' | 'g' | 'v' | 't')) => {
            app.sort = match c {
                'r' => SortMode::Ram,
                'g' => SortMode::Gpu,
                'v' => SortMode::Vram,
                't' => SortMode::Total,
                _ => SortMode::Cpu,
            };
            app.process_offset = 0;
            outcome.resort = true;
        }
        _ => return outcome,
    }
    outcome.redraw = !outcome.quit;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn control_c_quits_even_while_searching() {
        let mut app = AppState::default();
        app.begin_search();
        let result = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(result.quit);
        assert!(!result.resort);
    }

    #[test]
    fn search_consumes_shortcuts_and_escape_restores_previous_filter() {
        let mut app = AppState {
            process_filter: "old".into(),
            ..AppState::default()
        };
        handle_key(&mut app, key(KeyCode::Char('/')));
        handle_key(&mut app, key(KeyCode::Char('q')));
        handle_key(&mut app, key(KeyCode::Char(' ')));
        assert_eq!(app.process_filter, "oldq ");
        assert!(!app.paused);
        assert!(!handle_key(&mut app, key(KeyCode::Esc)).quit);
        assert_eq!(app.process_filter, "old");
        assert!(!app.search_editing);
    }

    #[test]
    fn backspace_removes_whole_grapheme_and_enter_applies_search() {
        let mut app = AppState {
            process_filter: "a👩‍💻".into(),
            ..AppState::default()
        };
        app.begin_search();
        handle_key(&mut app, key(KeyCode::Backspace));
        assert_eq!(app.process_filter, "a");
        handle_key(&mut app, key(KeyCode::Enter));
        assert!(!app.search_editing);
        assert!(handle_key(&mut app, key(KeyCode::Char('r'))).resort);
        assert_eq!(app.sort, SortMode::Ram);
    }

    #[test]
    fn page_and_end_navigation_stay_within_visible_limit() {
        let mut app = AppState {
            top_n: 30,
            process_match_count: 25,
            process_viewport_rows: 5,
            ..AppState::default()
        };
        handle_key(&mut app, key(KeyCode::PageDown));
        assert_eq!(app.process_offset, 5);
        handle_key(&mut app, key(KeyCode::End));
        assert_eq!(app.process_offset, 20);
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.process_offset, 19);
        handle_key(&mut app, key(KeyCode::Home));
        assert_eq!(app.process_offset, 0);
    }
}
