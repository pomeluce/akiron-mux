use std::sync::Arc;

use alacritty_terminal::{
    event::{Event, EventListener},
    grid::Dimensions,
    grid::Scroll,
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, Term},
    term::{TermMode, cell::Flags},
    vte::ansi,
};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalDimensions {
    pub rows: usize,
    pub columns: usize,
}

impl Dimensions for TerminalDimensions {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub rows: usize,
    pub columns: usize,
    pub cursor_line: i32,
    pub cursor_column: usize,
    pub content_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderCell {
    pub line: i32,
    pub column: usize,
    pub text: String,
    pub foreground: u32,
    pub background: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub selected: bool,
    pub cursor: bool,
    pub hyperlink: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSnapshot {
    pub rows: usize,
    pub columns: usize,
    pub cells: Vec<RenderCell>,
}

#[derive(Clone, Default)]
struct Listener {
    pty_writes: Arc<Mutex<Vec<Vec<u8>>>>,
    wakeups: Arc<Mutex<u64>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => self.pty_writes.lock().push(text.into_bytes()),
            Event::Wakeup => *self.wakeups.lock() += 1,
            _ => {}
        }
    }
}

pub struct TerminalModel {
    term: Term<Listener>,
    parser: ansi::Processor,
    listener: Listener,
}

impl TerminalModel {
    pub fn new(dimensions: TerminalDimensions) -> Self {
        let listener = Listener::default();
        let config = Config {
            scrolling_history: 10_000,
            ..Config::default()
        };
        let term = Term::new(config, &dimensions, listener.clone());
        Self {
            term,
            parser: ansi::Processor::new(),
            listener,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn replace(&mut self, bytes: &[u8]) {
        let dimensions = TerminalDimensions {
            rows: self.term.screen_lines(),
            columns: self.term.columns(),
        };
        *self = Self::new(dimensions);
        self.feed(bytes);
    }

    pub fn resize(&mut self, dimensions: TerminalDimensions) {
        self.term.resize(dimensions);
    }

    pub fn take_pty_writes(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.listener.pty_writes.lock())
    }

    pub fn take_wakeups(&self) -> u64 {
        std::mem::take(&mut *self.listener.wakeups.lock())
    }

    pub fn application_cursor_mode(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    pub fn bracketed_paste_mode(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    pub fn mouse_reporting_mode(&self) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    pub fn sgr_mouse_mode(&self) -> bool {
        self.term.mode().contains(TermMode::SGR_MOUSE)
    }

    pub fn start_selection(&mut self, row: usize, column: usize, click_count: usize) {
        let selection_type = match click_count {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        self.term.selection = Some(Selection::new(selection_type, self.viewport_point(row, column), Side::Left));
    }

    pub fn update_selection(&mut self, row: usize, column: usize) {
        let point = self.viewport_point(row, column);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Side::Right);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    pub fn hyperlink_at(&self, row: usize, column: usize) -> Option<String> {
        let explicit = self
            .term
            .renderable_content()
            .display_iter
            .find(|indexed| indexed.point.line.0 == row as i32 && indexed.point.column.0 == column)
            .and_then(|indexed| indexed.cell.hyperlink())
            .map(|link| link.uri().to_owned());
        if explicit.is_some() {
            return explicit;
        }

        let mut line = String::new();
        let mut target_offset = None;
        for indexed in self.term.renderable_content().display_iter.filter(|indexed| indexed.point.line.0 == row as i32) {
            if indexed.point.column.0 == column {
                target_offset = Some(line.len());
            }
            line.push(indexed.cell.c);
        }
        let target_offset = target_offset?;
        let mut offset = 0;
        for token in line.split_whitespace() {
            let start = line[offset..].find(token).map(|relative| offset + relative)?;
            let end = start + token.len();
            offset = end;
            if start <= target_offset && target_offset < end {
                let candidate = token.trim_matches(|character: char| matches!(character, '(' | ')' | '[' | ']' | '<' | '>' | ',' | '.' | ';' | '"' | '\''));
                if candidate.starts_with("https://") || candidate.starts_with("http://") {
                    return Some(candidate.to_owned());
                }
            }
        }
        None
    }

    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    fn viewport_point(&self, row: usize, column: usize) -> Point {
        let display_offset = self.term.grid().display_offset().min(i32::MAX as usize) as i32;
        Point::new(
            Line(row.min(self.term.screen_lines().saturating_sub(1)) as i32 - display_offset),
            Column(column.min(self.term.columns().saturating_sub(1))),
        )
    }

    pub fn render_snapshot(&self) -> RenderSnapshot {
        let content = self.term.renderable_content();
        let cursor = content.cursor.point;
        let selection = content.selection;
        let cells = content
            .display_iter
            .map(|indexed| {
                let cell = indexed.cell;
                let inverse = cell.flags.contains(Flags::INVERSE);
                let (foreground, background) = if inverse {
                    (resolve_color(cell.bg), resolve_color(cell.fg))
                } else {
                    (resolve_color(cell.fg), resolve_color(cell.bg))
                };
                let mut text = cell.c.to_string();
                if let Some(zerowidth) = cell.zerowidth() {
                    text.extend(zerowidth);
                }
                RenderCell {
                    line: indexed.point.line.0,
                    column: indexed.point.column.0,
                    text,
                    foreground,
                    background,
                    bold: cell.flags.contains(Flags::BOLD),
                    italic: cell.flags.contains(Flags::ITALIC),
                    underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                    selected: selection.is_some_and(|selection| selection.contains(indexed.point)),
                    cursor: cursor == indexed.point,
                    hyperlink: cell.hyperlink().map(|link| link.uri().to_owned()),
                }
            })
            .collect();
        RenderSnapshot {
            rows: self.term.screen_lines(),
            columns: self.term.columns(),
            cells,
        }
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        let cursor = self.term.grid().cursor.point;
        let mut digest = Sha256::new();
        for line in 0..self.term.screen_lines() {
            for column in 0..self.term.columns() {
                let cell = &self.term.grid()[Line(line as i32)][Column(column)];
                digest.update(cell.c.to_string().as_bytes());
                digest.update([cell.flags.bits() as u8]);
            }
        }
        TerminalSnapshot {
            rows: self.term.screen_lines(),
            columns: self.term.columns(),
            cursor_line: cursor.line.0,
            cursor_column: cursor.column.0,
            content_digest: format!("{:x}", digest.finalize()),
        }
    }
}

fn resolve_color(color: ansi::Color) -> u32 {
    use ansi::{Color, NamedColor};
    match color {
        Color::Spec(rgb) => ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32,
        Color::Indexed(index) => indexed_color(index),
        Color::Named(NamedColor::Foreground | NamedColor::BrightForeground) => 0xD8DEE9,
        Color::Named(NamedColor::Background) => 0x0F1117,
        Color::Named(NamedColor::Cursor) => 0xECEFF4,
        Color::Named(named) => indexed_color(named as u8),
    }
}

fn indexed_color(index: u8) -> u32 {
    const ANSI: [u32; 16] = [
        0x1B1D23, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xABB2BF, 0x5C6370, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xFFFFFF,
    ];
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let value = index - 16;
            let channel = |part: u8| if part == 0 { 0 } else { 55 + part as u32 * 40 };
            (channel(value / 36) << 16) | (channel((value / 6) % 6) << 8) | channel(value % 6)
        }
        232..=255 => {
            let gray = 8 + (index as u32 - 232) * 10;
            (gray << 16) | (gray << 8) | gray
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_color_and_alternate_screen_without_losing_dimensions() {
        let mut terminal = TerminalModel::new(TerminalDimensions { rows: 24, columns: 80 });
        terminal.feed(b"hello\r\n\x1b[38;2;1;2;3mcolor\x1b[0m");
        let first = terminal.snapshot();
        terminal.feed(b"\x1b[?1049halternate\x1b[?1049l");
        let second = terminal.snapshot();
        assert_eq!((first.rows, first.columns), (24, 80));
        assert_eq!((second.rows, second.columns), (24, 80));
        assert_ne!(
            first.content_digest,
            TerminalModel::new(TerminalDimensions { rows: 24, columns: 80 }).snapshot().content_digest
        );
    }

    #[test]
    fn replacement_discards_stale_terminal_content() {
        let mut terminal = TerminalModel::new(TerminalDimensions { rows: 10, columns: 40 });
        terminal.feed(b"stale");
        terminal.replace(b"fresh");
        let replaced = terminal.snapshot();
        let mut expected = TerminalModel::new(TerminalDimensions { rows: 10, columns: 40 });
        expected.feed(b"fresh");
        assert_eq!(replaced, expected.snapshot());
    }

    #[test]
    fn render_snapshot_preserves_truecolor_combining_text_and_links() {
        let mut terminal = TerminalModel::new(TerminalDimensions { rows: 4, columns: 20 });
        terminal.feed(b"\x1b[38;2;1;2;3me\xcc\x81\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\");
        let snapshot = terminal.render_snapshot();
        assert!(snapshot.cells.iter().any(|cell| cell.text == "e\u{301}" && cell.foreground == 0x010203));
        assert!(snapshot.cells.iter().any(|cell| cell.hyperlink.as_deref() == Some("https://example.com")));
    }
}
