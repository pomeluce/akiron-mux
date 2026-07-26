use crate::tui::lang;
use super::super::super::theme;
use super::super::super::widgets::shared::{centered_rect, pad_label};
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
        let field = &mut self.fields[self.focused];
        let cur = &mut self.cursors[self.focused];
        match code {
            KeyCode::Tab => {
                self.focused = (self.focused + 1) % 6;
            }
            KeyCode::BackTab => {
                self.focused = if self.focused == 0 { 5 } else { self.focused - 1 };
            }
            KeyCode::Left => {
                *cur = cur.saturating_sub(1);
            }
            KeyCode::Right => {
                *cur = (*cur + 1).min(field.len());
            }
            KeyCode::Home => {
                *cur = 0;
            }
            KeyCode::End => {
                *cur = field.len();
            }
            KeyCode::Backspace => {
                if *cur > 0 {
                    *cur -= 1;
                    field.remove(*cur);
                }
            }
            KeyCode::Delete => {
                if *cur < field.len() {
                    field.remove(*cur);
                }
            }
            KeyCode::Char(c) => {
                field.insert(*cur, c);
                *cur += 1;
            }
            _ => {}
        }
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
        let style = if form.is_edit && i == 0 {
            Style::default().fg(theme::current().dim)
        } else if i == form.focused {
            Style::default().fg(theme::current().cyan)
        } else {
            Style::default().fg(theme::current().fg)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{}{}", pad, pad_label(label, 15)), Style::default().fg(theme::current().fg)),
            Span::styled(left.to_string(), style),
            Span::styled(cursor.to_string(), style),
            Span::styled(right.to_string(), style),
            Span::styled(pad.clone(), Style::default()),
        ]));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(
        Line::from(vec![
            Span::styled(lang::current().sc_save, Style::default().fg(theme::current().comment)),
            Span::styled("  ", Style::default()),
            Span::styled(lang::current().sc_cancel, Style::default().fg(theme::current().comment)),
            Span::styled("  ", Style::default()),
            Span::styled(lang::current().sc_next_field, Style::default().fg(theme::current().comment)),
        ])
        .centered(),
    );

    let p = Paragraph::new(lines).block(
        Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .title(Line::from(if form.is_edit { lang::current().title_edit_profile } else { lang::current().title_add_profile }).centered())
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
    [l.label_prov_name, l.label_prov_id, l.label_api_url, l.label_api_key]
}

impl ProviderForm {
    pub fn handle_key(&mut self, code: KeyCode) {
        // Skip readonly id field (index 1) in edit mode
        if self.is_edit && self.focused == 1 && !matches!(code, KeyCode::Tab | KeyCode::BackTab) {
            return;
        }
        let field = &mut self.fields[self.focused];
        let cur = &mut self.cursors[self.focused];
        match code {
            KeyCode::Tab => self.focused = (self.focused + 1) % 4,
            KeyCode::BackTab => self.focused = if self.focused == 0 { 3 } else { self.focused - 1 },
            KeyCode::Left => *cur = cur.saturating_sub(1),
            KeyCode::Right => *cur = (*cur + 1).min(field.len()),
            KeyCode::Home => *cur = 0,
            KeyCode::End => *cur = field.len(),
            KeyCode::Backspace => { if *cur > 0 { *cur -= 1; field.remove(*cur); } }
            KeyCode::Delete => { if *cur < field.len() { field.remove(*cur); } }
            KeyCode::Char(c) => { field.insert(*cur, c); *cur += 1; }
            _ => {}
        }
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
        let style = if form.is_edit && i == 1 {
            // Readonly ID in edit mode
            Style::default().fg(theme::current().dim)
        } else if i == form.focused {
            Style::default().fg(theme::current().cyan)
        } else {
            Style::default().fg(theme::current().fg)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{}{}", pad, pad_label(label, 10)), Style::default().fg(theme::current().fg)),
            Span::styled(left.to_string(), style),
            Span::styled(cursor.to_string(), style),
            Span::styled(right.to_string(), style),
            Span::styled(pad.clone(), Style::default()),
        ]));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.push(
        Line::from(vec![
            Span::styled(lang::current().sc_save, Style::default().fg(theme::current().comment)),
            Span::styled("  ", Style::default()),
            Span::styled(lang::current().sc_cancel, Style::default().fg(theme::current().comment)),
            Span::styled("  ", Style::default()),
            Span::styled(lang::current().sc_next_field, Style::default().fg(theme::current().comment)),
        ]).centered(),
    );

    let title = if form.is_edit { lang::current().title_edit_provider } else { lang::current().title_add_provider };
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
    if text.len() <= max_w || max_w < 4 {
        return VisSlice {
            text: text.to_string(),
            skip: 0,
        };
    }
    let half = max_w / 2;
    let mut start = if cursor > half { cursor - half } else { 0 };
    let end = (start + max_w).min(text.len());
    if end == text.len() && end > max_w {
        start = end - max_w;
    }
    VisSlice {
        text: text[start..end].to_string(),
        skip: start,
    }
}
