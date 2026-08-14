use super::super::tabs::Tab;
use super::super::theme;
use crate::tui::lang;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

pub fn render_sidebar(f: &mut Frame, area: Rect, active_tab: Tab) {
    let tabs = vec![
        (Tab::Providers, lang::current().tab_providers),
        (Tab::Usage, lang::current().tab_usage),
        (Tab::History, lang::current().tab_history),
        (Tab::Settings, lang::current().tab_settings),
    ];

    let tab_lines = (tabs.len() * 2) as u16;
    let header_lines = 1u16;
    let inner_h = area.height.saturating_sub(2); // border
    let avail = inner_h.saturating_sub(header_lines);
    let pad_bottom = avail.saturating_sub(tab_lines);
    let inner_w = area.width.saturating_sub(2) as usize;

    // Compute max label width and left pad for centered block
    let max_w = tabs
        .iter()
        .map(|(_, l)| l.chars().map(|c| if c > '\u{7e}' { 2 } else { 1 }).sum::<usize>())
        .max()
        .unwrap_or(8);
    let tab_pad = " ".repeat(inner_w.saturating_sub(max_w + 2) / 2);
    let mut lines: Vec<Line> = vec![Line::from("")];
    for (tab, label) in &tabs {
        let style = if *tab == active_tab {
            Style::default().fg(theme::current().cyan)
        } else {
            Style::default().fg(theme::current().dim)
        };
        let dw = label.chars().map(|c| if c > '\u{7e}' { 2 } else { 1 }).sum::<usize>();
        let rpad = " ".repeat(max_w.saturating_sub(dw));
        let marker = if *tab == active_tab { "› " } else { "  " };
        lines.push(Line::from(Span::styled(format!("{}{}{}{}", tab_pad, marker, label, rpad), style)));
        lines.push(Line::from(""));
    }

    for _ in 0..pad_bottom {
        lines.push(Line::from(""));
    }

    let p = Paragraph::new(lines).block(
        Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .border_style(Style::default().fg(theme::current().dim)),
    );
    f.render_widget(p, area);
}
