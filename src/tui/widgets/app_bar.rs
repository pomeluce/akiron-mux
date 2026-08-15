use crate::core::models::AppType;
use crate::tui::lang;
use crate::tui::theme;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

pub fn render_app_bar(f: &mut Frame, area: Rect, app: AppType, proxy_mode: bool, active_context: &str) {
    let claude = " Claude ";
    let separator = " | ";
    let codex = " Codex ";
    let block = Block::bordered()
        .border_set(ratatui::symbols::border::ROUNDED)
        .border_style(Style::default().fg(theme::current().dim));

    let inner = block.inner(area);
    f.render_widget(block, area);
    let active = Style::default().fg(theme::current().cyan);
    let inactive = Style::default().fg(theme::current().dim);
    let tabs_width = (claude.len() + separator.len() + codex.len()) as u16;
    let tabs_area = Rect {
        x: inner.x + inner.width.saturating_sub(tabs_width) / 2,
        y: inner.y,
        width: tabs_width.min(inner.width),
        height: inner.height,
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(claude, if app == AppType::Claude { active } else { inactive }),
            Span::styled(separator, Style::default().fg(theme::current().comment)),
            Span::styled(codex, if app == AppType::Codex { active } else { inactive }),
        ])),
        tabs_area,
    );
    if inner.width < 60 {
        return;
    }
    let [left, _, right] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(20.min(inner.width)), Constraint::Min(18), Constraint::Length(34.min(inner.width / 2))])
        .areas(inner);
    f.render_widget(
        Paragraph::new(Span::styled(
            format!(" AkironMux {}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(theme::current().comment),
        )),
        left,
    );
    let mode = if app == AppType::Claude {
        if proxy_mode {
            "proxy"
        } else {
            "local"
        }
    } else {
        ""
    };
    let context = if active_context.is_empty() {
        lang::pick("not selected", "未选择")
    } else {
        active_context
    };
    let right_text = if mode.is_empty() {
        format!("{} ", context)
    } else {
        format!("{} · {} ", mode, context)
    };
    if inner.width >= 90 {
        f.render_widget(
            Paragraph::new(Span::styled(right_text, Style::default().fg(theme::current().green))).alignment(Alignment::Right),
            right,
        );
    }
}
