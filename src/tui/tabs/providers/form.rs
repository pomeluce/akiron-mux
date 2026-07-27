use super::super::super::theme;
use super::super::super::widgets::shared::{centered_rect, display_width, pad_label};
use crate::tui::lang;
use crossterm::event::KeyCode;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};

pub struct EditForm {
    pub fields: [String; 6],
    pub cursors: [usize; 6],
    pub focused: usize,
    pub prov_id: String,
    pub is_edit: bool,
}

pub fn edit_labels() -> [&'static str; 6] {
    let l = lang::current();
    [
        l.label_profile_id,
        l.label_profile_name,
        l.label_opus,
        l.label_sonnet,
        l.label_haiku,
        l.label_subagent,
    ]
}

impl EditForm {
    pub fn handle_key(&mut self, code: KeyCode) {
        if self.is_edit && self.focused == 0 && !matches!(code, KeyCode::Tab | KeyCode::BackTab) {
            return;
        }
        match code {
            KeyCode::Tab => {
                self.focused = (self.focused + 1) % 6;
                return;
            }
            KeyCode::BackTab => {
                self.focused = if self.focused == 0 {
                    5
                } else {
                    self.focused - 1
                };
                return;
            }
            _ => {}
        }
        edit_text_field(
            &mut self.fields[self.focused],
            &mut self.cursors[self.focused],
            code,
        );
    }
}

pub fn render_edit_form(form: &EditForm, f: &mut Frame, area: Rect) {
    let popup = centered_rect(65, 26, area);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let pad_w = (inner_w.saturating_sub(40)) / 2;
    let pad = " ".repeat(pad_w);
    let value_w = inner_w.saturating_sub(pad_w * 2 + 17);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    let labels = edit_labels();
    for (i, label) in labels.iter().enumerate() {
        let val = &form.fields[i];
        let pos = form.cursors[i].min(val.len());
        let vis = slice_value(val, pos, value_w);
        let cur = (pos - vis.skip).min(vis.text.len());
        let (left, right) = vis.text.split_at(cur);
        let cursor = if i == form.focused { "▌" } else { "" };
        let tail = " ".repeat(
            value_w.saturating_sub(display_width(&vis.text) + usize::from(i == form.focused)),
        );
        let style = if form.is_edit && i == 0 {
            Style::default().fg(theme::current().dim)
        } else if i == form.focused {
            Style::default().fg(theme::current().cyan)
        } else {
            Style::default().fg(theme::current().fg)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}{}", pad, pad_label(label, 15)),
                Style::default().fg(theme::current().fg),
            ),
            Span::styled(left.to_string(), style),
            Span::styled(cursor.to_string(), style),
            Span::styled(right.to_string(), style),
            Span::raw(tail),
            Span::styled(pad.clone(), Style::default()),
        ]));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(
        Line::from(vec![
            Span::styled(
                lang::current().sc_save,
                Style::default().fg(theme::current().comment),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                lang::current().sc_cancel,
                Style::default().fg(theme::current().comment),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                lang::current().sc_next_field,
                Style::default().fg(theme::current().comment),
            ),
        ])
        .centered(),
    );

    let p = Paragraph::new(lines).block(
        Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .title(
                Line::from(if form.is_edit {
                    lang::current().title_edit_profile
                } else {
                    lang::current().title_add_profile
                })
                .centered(),
            )
            .border_style(Style::default().fg(theme::current().cyan)),
    );
    f.render_widget(Clear, popup);
    f.render_widget(p, popup);
}

// ── Provider Add/Edit form ──────────────────────────────────────

pub struct ProviderForm {
    pub fields: [String; 4], // name, id, api_url, api_key
    pub cursors: [usize; 4],
    pub focused: usize,
    pub is_edit: bool, // true = edit (id readonly), false = add
}

fn provider_labels() -> [&'static str; 4] {
    let l = lang::current();
    [
        l.label_prov_name,
        l.label_prov_id,
        l.label_api_url,
        l.label_api_key,
    ]
}

impl ProviderForm {
    pub fn handle_key(&mut self, code: KeyCode) {
        // Skip readonly id field (index 1) in edit mode
        if self.is_edit && self.focused == 1 && !matches!(code, KeyCode::Tab | KeyCode::BackTab) {
            return;
        }
        match code {
            KeyCode::Tab => {
                self.focused = (self.focused + 1) % 4;
                return;
            }
            KeyCode::BackTab => {
                self.focused = if self.focused == 0 {
                    3
                } else {
                    self.focused - 1
                };
                return;
            }
            _ => {}
        }
        edit_text_field(
            &mut self.fields[self.focused],
            &mut self.cursors[self.focused],
            code,
        );
    }
}

pub fn render_provider_form(form: &ProviderForm, f: &mut Frame, area: Rect) {
    let popup = centered_rect(60, 18, area);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let pad_w = (inner_w.saturating_sub(40)) / 2;
    let pad = " ".repeat(pad_w);
    let value_w = inner_w.saturating_sub(pad_w * 2 + 17);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    let p_labels = provider_labels();
    for (i, label) in p_labels.iter().enumerate() {
        let val = &form.fields[i];
        let pos = form.cursors[i].min(val.len());
        let vis = slice_value(val, pos, value_w);
        let cur = (pos - vis.skip).min(vis.text.len());
        let (left, right) = vis.text.split_at(cur);
        let cursor = if i == form.focused { "\u{258c}" } else { "" };
        let tail = " ".repeat(
            value_w.saturating_sub(display_width(&vis.text) + usize::from(i == form.focused)),
        );
        let style = if form.is_edit && i == 1 {
            // Readonly ID in edit mode
            Style::default().fg(theme::current().dim)
        } else if i == form.focused {
            Style::default().fg(theme::current().cyan)
        } else {
            Style::default().fg(theme::current().fg)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}{}", pad, pad_label(label, 10)),
                Style::default().fg(theme::current().fg),
            ),
            Span::styled(left.to_string(), style),
            Span::styled(cursor.to_string(), style),
            Span::styled(right.to_string(), style),
            Span::raw(tail),
            Span::styled(pad.clone(), Style::default()),
        ]));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(
        Line::from(vec![
            Span::styled(
                lang::current().sc_save,
                Style::default().fg(theme::current().comment),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                lang::current().sc_cancel,
                Style::default().fg(theme::current().comment),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                lang::current().sc_next_field,
                Style::default().fg(theme::current().comment),
            ),
        ])
        .centered(),
    );

    let title = if form.is_edit {
        lang::current().title_edit_provider
    } else {
        lang::current().title_add_provider
    };
    let p = Paragraph::new(lines).block(
        Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .title(Line::from(title).centered())
            .border_style(Style::default().fg(theme::current().cyan)),
    );
    f.render_widget(Clear, popup);
    f.render_widget(p, popup);
}

struct VisSlice {
    text: String,
    skip: usize,
}

fn slice_value(text: &str, cursor: usize, max_w: usize) -> VisSlice {
    if display_width(text) <= max_w || max_w < 4 {
        return VisSlice {
            text: text.to_string(),
            skip: 0,
        };
    }
    let mut boundaries: Vec<usize> = text.char_indices().map(|(index, _)| index).collect();
    boundaries.push(text.len());
    let cursor = previous_char_boundary(text, cursor.min(text.len()));
    let cursor_index = boundaries
        .iter()
        .position(|index| *index == cursor)
        .unwrap_or(0);
    let mut start_index = cursor_index;
    let mut before_width = 0usize;
    while start_index > 0 {
        let ch = text[boundaries[start_index - 1]..boundaries[start_index]]
            .chars()
            .next()
            .unwrap();
        let width = display_width(&ch.to_string());
        if before_width + width > max_w / 2 {
            break;
        }
        before_width += width;
        start_index -= 1;
    }
    let mut end_index = start_index;
    let mut visible_width = 0usize;
    while end_index + 1 < boundaries.len() {
        let ch = text[boundaries[end_index]..boundaries[end_index + 1]]
            .chars()
            .next()
            .unwrap();
        let width = display_width(&ch.to_string());
        if visible_width + width > max_w {
            break;
        }
        visible_width += width;
        end_index += 1;
    }
    let start = boundaries[start_index];
    let end = boundaries[end_index];
    VisSlice {
        text: text[start..end].to_string(),
        skip: start,
    }
}

fn edit_text_field(field: &mut String, cursor: &mut usize, code: KeyCode) {
    *cursor = previous_char_boundary(field, (*cursor).min(field.len()));
    match code {
        KeyCode::Left => *cursor = previous_char_boundary(field, cursor.saturating_sub(1)),
        KeyCode::Right => *cursor = next_char_boundary(field, *cursor),
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = field.len(),
        KeyCode::Backspace if *cursor > 0 => {
            let previous = previous_char_boundary(field, cursor.saturating_sub(1));
            field.replace_range(previous..*cursor, "");
            *cursor = previous;
        }
        KeyCode::Delete if *cursor < field.len() => {
            let next = next_char_boundary(field, *cursor);
            field.replace_range(*cursor..next, "");
        }
        KeyCode::Char(ch) => {
            field.insert(*cursor, ch);
            *cursor += ch.len_utf8();
        }
        _ => {}
    }
}

fn previous_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut next = index + 1;
    while next < text.len() && !text.is_char_boundary(next) {
        next += 1;
    }
    next
}

#[cfg(test)]
mod tests {
    use super::{edit_text_field, slice_value};
    use crossterm::event::KeyCode;

    #[test]
    fn unicode_editing_keeps_cursor_on_char_boundaries() {
        let mut text = "配置A".to_string();
        let mut cursor = text.len();
        edit_text_field(&mut text, &mut cursor, KeyCode::Left);
        edit_text_field(&mut text, &mut cursor, KeyCode::Backspace);
        assert_eq!(text, "配A");
        edit_text_field(&mut text, &mut cursor, KeyCode::Char('置'));
        assert_eq!(text, "配置A");
        assert!(text.is_char_boundary(cursor));
        let visible = slice_value("一个很长的模型名称-model", cursor, 10);
        assert!(visible.text.is_char_boundary(visible.text.len()));
    }
}
