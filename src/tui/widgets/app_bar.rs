use crate::core::models::AppType;
use crate::tui::theme;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

pub fn render_app_bar(f: &mut Frame, area: Rect, app: AppType) {
    let inner_w = area.width.saturating_sub(2) as usize;
    let claude = " Claude ";
    let separator = " | ";
    let codex = " Codex ";
    let dw = claude.len() + separator.len() + codex.len();
    let pad = " ".repeat(inner_w.saturating_sub(dw) / 2);

    let block = Block::bordered()
        .border_set(ratatui::symbols::border::ROUNDED)
        .border_style(Style::default().fg(theme::current().dim));

    let active = Style::default().fg(theme::current().cyan);
    let inactive = Style::default().fg(theme::current().dim);
    let p = Paragraph::new(Line::from(vec![
        Span::raw(pad),
        Span::styled(claude, if app == AppType::Claude { active } else { inactive }),
        Span::styled(separator, Style::default().fg(theme::current().comment)),
        Span::styled(codex, if app == AppType::Codex { active } else { inactive }),
    ])).block(block);
    f.render_widget(p, area);
}
