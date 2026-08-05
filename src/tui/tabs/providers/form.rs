use super::super::super::theme;
use super::super::super::widgets::shared::{
    centered_rect, clear_popup_area, display_width, pad_label,
};
use crate::tui::lang;
use crossterm::event::KeyCode;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
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
    clear_popup_area(f, popup);
    f.render_widget(p, popup);
}

// ── Provider Add/Edit form ──────────────────────────────────────

pub struct ProviderForm {
    pub fields: [String; 4], // name, id, api_url, api_key
    pub cursors: [usize; 4],
    pub focused: usize,
    pub is_edit: bool, // true = edit (id readonly), false = add
    pub show_catalog: bool,
    pub custom_catalog: bool,
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
            KeyCode::Char(' ') if self.show_catalog && self.focused == 4 => {
                self.custom_catalog = !self.custom_catalog;
                return;
            }
            KeyCode::Tab => {
                self.focused = (self.focused + 1) % if self.show_catalog { 5 } else { 4 };
                return;
            }
            KeyCode::BackTab => {
                self.focused = if self.focused == 0 {
                    if self.show_catalog {
                        4
                    } else {
                        3
                    }
                } else {
                    self.focused - 1
                };
                return;
            }
            _ => {}
        }
        if self.focused < 4 {
            edit_text_field(
                &mut self.fields[self.focused],
                &mut self.cursors[self.focused],
                code,
            );
        }
    }
}

pub fn render_provider_form(form: &ProviderForm, is_codex: bool, f: &mut Frame, area: Rect) {
    let popup = centered_rect(64, if is_codex { 20 } else { 18 }, area);
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
    if is_codex {
        let selected = if form.custom_catalog {
            lang::pick("Third-party models", "第三方模型")
        } else {
            lang::pick("Codex built-in", "Codex 内置")
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "{}{}",
                    pad,
                    pad_label(lang::pick("Catalog", "模型来源"), 10)
                ),
                Style::default().fg(theme::current().fg),
            ),
            Span::styled(
                selected,
                Style::default().fg(if form.focused == 4 {
                    theme::current().cyan
                } else {
                    theme::current().fg
                }),
            ),
            Span::styled(
                "  Space toggle",
                Style::default().fg(theme::current().comment),
            ),
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
    clear_popup_area(f, popup);
    f.render_widget(p, popup);
}

pub struct CodexModelForm {
    pub fields: [String; 6],
    pub cursors: [usize; 6],
    pub focused: usize,
    pub is_edit: bool,
    pub provider_id: String,
    pub supported_efforts: [bool; REASONING_EFFORTS.len()],
    pub effort_cursor: usize,
    pub default_effort: usize,
    pub default_model: bool,
    pub supports_images: bool,
    pub supports_parallel_tools: bool,
    pub support_verbosity: bool,
    pub supports_search: bool,
}

pub const REASONING_EFFORTS: [&str; 8] = [
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

impl CodexModelForm {
    pub fn supported_reasoning_efforts(&self) -> Vec<String> {
        REASONING_EFFORTS
            .iter()
            .enumerate()
            .filter(|(index, _)| self.supported_efforts[*index])
            .map(|(_, effort)| (*effort).to_string())
            .collect()
    }

    pub fn default_reasoning_effort(&self) -> String {
        REASONING_EFFORTS[self.default_effort].to_string()
    }

    fn cycle_default_effort(&mut self, forward: bool) {
        for distance in 1..=REASONING_EFFORTS.len() {
            let index = if forward {
                (self.default_effort + distance) % REASONING_EFFORTS.len()
            } else {
                (self.default_effort + REASONING_EFFORTS.len() - distance) % REASONING_EFFORTS.len()
            };
            if self.supported_efforts[index] {
                self.default_effort = index;
                break;
            }
        }
    }

    pub fn handle_key(&mut self, code: KeyCode) {
        if self.is_edit && self.focused == 0 && !matches!(code, KeyCode::Tab | KeyCode::BackTab) {
            return;
        }
        match code {
            KeyCode::Tab => self.focused = (self.focused + 1) % 13,
            KeyCode::BackTab => {
                self.focused = if self.focused == 0 {
                    12
                } else {
                    self.focused - 1
                }
            }
            KeyCode::Left | KeyCode::Char('h') if self.focused == 6 => {
                self.cycle_default_effort(false)
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') if self.focused == 6 => {
                self.cycle_default_effort(true)
            }
            KeyCode::Left | KeyCode::Char('h') if self.focused == 7 => {
                self.effort_cursor = self
                    .effort_cursor
                    .checked_sub(1)
                    .unwrap_or(REASONING_EFFORTS.len() - 1)
            }
            KeyCode::Right | KeyCode::Char('l') if self.focused == 7 => {
                self.effort_cursor = (self.effort_cursor + 1) % REASONING_EFFORTS.len()
            }
            KeyCode::Char(' ') if self.focused == 7 => {
                let selected = self
                    .supported_efforts
                    .iter()
                    .filter(|value| **value)
                    .count();
                if self.supported_efforts[self.effort_cursor] && selected == 1 {
                    return;
                }
                self.supported_efforts[self.effort_cursor] =
                    !self.supported_efforts[self.effort_cursor];
                if !self.supported_efforts[self.default_effort] {
                    self.default_effort = self
                        .supported_efforts
                        .iter()
                        .position(|value| *value)
                        .expect("at least one reasoning effort remains selected");
                }
            }
            KeyCode::Char(' ') if self.focused >= 8 => match self.focused {
                8 => self.default_model = !self.default_model,
                9 => self.supports_images = !self.supports_images,
                10 => self.supports_parallel_tools = !self.supports_parallel_tools,
                11 => self.support_verbosity = !self.support_verbosity,
                12 => self.supports_search = !self.supports_search,
                _ => {}
            },
            _ if self.focused < 6 => edit_text_field(
                &mut self.fields[self.focused],
                &mut self.cursors[self.focused],
                code,
            ),
            _ => {}
        }
    }
}

pub fn render_codex_model_form(form: &CodexModelForm, f: &mut Frame, area: Rect) {
    let popup = centered_rect(82, 20, area);
    let block = Block::bordered()
        .border_set(ratatui::symbols::border::ROUNDED)
        .title(
            Line::from(if form.is_edit {
                lang::pick(" Edit Codex Model ", " 编辑 Codex 模型 ")
            } else {
                lang::pick(" Add Codex Model ", " 添加 Codex 模型 ")
            })
            .centered(),
        )
        .border_style(Style::default().fg(theme::current().cyan));
    let inner = block.inner(popup);
    let labels = [
        lang::pick("Slug", "模型标识"),
        lang::pick("Name", "显示名称"),
        lang::pick("Description", "描述"),
        lang::pick("Context", "上下文窗口"),
        lang::pick("Max context", "最大上下文"),
        lang::pick("Effective %", "有效比例 %"),
    ];
    let label_width = labels
        .iter()
        .copied()
        .chain([
            lang::pick("Default effort", "默认推理强度"),
            lang::pick("Supported efforts", "支持的推理强度"),
            lang::pick("Default model", "默认模型"),
            lang::pick("Parallel tools", "并行工具"),
        ])
        .map(display_width)
        .max()
        .unwrap_or(18);
    let value_width = inner.width as usize - (label_width + 4).min(inner.width as usize);
    let mut lines = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        let value = &form.fields[index];
        let cursor_pos = form.cursors[index].min(value.len());
        let visible = slice_value(value, cursor_pos, value_width.max(1));
        let cursor = (cursor_pos - visible.skip).min(visible.text.len());
        let (left, right) = visible.text.split_at(cursor);
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}", pad_label(label, label_width)),
                Style::default().fg(theme::current().purple),
            ),
            Span::styled(
                left.to_string(),
                Style::default().fg(if form.focused == index {
                    theme::current().cyan
                } else {
                    theme::current().fg
                }),
            ),
            if form.focused == index {
                Span::raw("▌")
            } else {
                Span::raw("")
            },
            Span::styled(
                right.to_string(),
                Style::default().fg(if form.focused == index {
                    theme::current().cyan
                } else {
                    theme::current().fg
                }),
            ),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(
            format!(
                "  {}",
                pad_label(lang::pick("Default effort", "默认推理强度"), label_width)
            ),
            Style::default().fg(theme::current().purple),
        ),
        Span::styled(
            format!("< {} >", form.default_reasoning_effort()),
            Style::default().fg(if form.focused == 6 {
                theme::current().cyan
            } else {
                theme::current().fg
            }),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        format!(
            "  {}",
            pad_label(
                lang::pick("Supported efforts", "支持的推理强度"),
                label_width
            )
        ),
        Style::default().fg(theme::current().purple),
    )));
    for indices in [0..4, 4..8] {
        let mut spans = vec![Span::raw("  ")];
        spans.extend(indices.map(|index| {
            let effort = REASONING_EFFORTS[index];
            let marker = if form.supported_efforts[index] {
                "x"
            } else {
                " "
            };
            Span::styled(
                format!(" [{}] {:<7}", marker, effort),
                Style::default().fg(if form.focused == 7 && form.effort_cursor == index {
                    theme::current().cyan
                } else {
                    theme::current().fg
                }),
            )
        }));
        lines.push(Line::from(spans));
    }
    for (offset, (label, value)) in [
        (lang::pick("Default model", "默认模型"), form.default_model),
        (lang::pick("Image input", "图片输入"), form.supports_images),
        (
            lang::pick("Parallel tools", "并行工具"),
            form.supports_parallel_tools,
        ),
        (lang::pick("Verbosity", "详细程度"), form.support_verbosity),
        (lang::pick("Web search", "网络搜索"), form.supports_search),
    ]
    .into_iter()
    .enumerate()
    {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {}", pad_label(label, label_width)),
                Style::default().fg(theme::current().purple),
            ),
            Span::styled(
                if value { "[x]" } else { "[ ]" },
                Style::default().fg(if form.focused == offset + 8 {
                    theme::current().cyan
                } else {
                    theme::current().fg
                }),
            ),
        ]));
    }
    lines.push(
        Line::from(lang::pick(
            "Enter Save · Esc Cancel · Tab Next · ←/→ Select · Space Toggle",
            "Enter 保存 · Esc 取消 · Tab 下一项 · ←/→ 选择 · Space 切换",
        ))
        .centered(),
    );
    clear_popup_area(f, popup);
    f.render_widget(block, popup);
    f.render_widget(Paragraph::new(lines), inner);
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
    use super::{edit_text_field, slice_value, CodexModelForm};
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

    fn model_form() -> CodexModelForm {
        CodexModelForm {
            fields: std::array::from_fn(|_| String::new()),
            cursors: [0; 6],
            focused: 7,
            is_edit: false,
            provider_id: "test".into(),
            supported_efforts: [false, false, true, true, true, false, false, false],
            effort_cursor: 4,
            default_effort: 3,
            default_model: false,
            supports_images: false,
            supports_parallel_tools: true,
            support_verbosity: true,
            supports_search: false,
        }
    }

    #[test]
    fn reasoning_efforts_are_selected_and_default_stays_supported() {
        let mut form = model_form();
        form.handle_key(KeyCode::Char(' '));
        assert_eq!(form.supported_reasoning_efforts(), vec!["low", "medium"]);

        form.focused = 6;
        form.handle_key(KeyCode::Right);
        assert_eq!(form.default_reasoning_effort(), "low");
    }
}
