//! Daily usage chart rendering — extracted from usage/mod.rs
use crate::tui::lang;
use crate::tui::theme;
use crate::tui::widgets::shared::format_tokens;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

pub fn title_case(s: &str) -> String {
    let mut result = String::new();
    let mut upper = true;
    for c in s.chars() {
        if c == '-' || c == '.' || c == '_' {
            upper = true;
            result.push(c);
        } else if upper {
            result.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// Render the 7-day usage bar chart for a given model.
/// `daily` is the query_daily_usage result; `chart_scroll` is mutated to clamp within bounds.
pub fn render_daily_chart(daily: &[(String, i64, i64, i64, i64)], model_name: &str, chart_scroll: &mut usize, f: &mut Frame, area: Rect) {
    let label = title_case(model_name);
    if daily.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(lang::current().no_usage_7d, Style::default().fg(theme::current().comment))).centered(),
            Line::from(""),
        ])
        .block(
            Block::bordered()
                .border_set(ratatui::symbols::border::ROUNDED)
                .title(format!("{} — {}", lang::current().chart_title, label))
                .border_style(Style::default().fg(theme::current().dim)),
        );
        f.render_widget(p, area);
        return;
    }

    let today_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let days: Vec<(String, i64, i64, i64, i64, bool)> = (0..7)
        .filter_map(|offset| {
            let d = chrono::Local::now() - chrono::Duration::days(offset);
            let date_str = d.format("%Y-%m-%d").to_string();
            let (in_tok, out_tok, cr_tok, cc_tok) = daily
                .iter()
                .find(|(dt, _, _, _, _)| dt == &date_str)
                .map(|(_, i, o, cr, cc)| (*i, *o, *cr, *cc))
                .unwrap_or((0, 0, 0, 0));
            let total = in_tok + out_tok + cr_tok + cc_tok;
            if total == 0 {
                None
            } else {
                Some((d.format("%m-%d").to_string(), in_tok, out_tok, cr_tok, cc_tok, date_str == today_date))
            }
        })
        .collect();

    if days.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(lang::current().no_usage_7d, Style::default().fg(theme::current().comment))).centered(),
            Line::from(""),
        ])
        .block(
            Block::bordered()
                .border_set(ratatui::symbols::border::ROUNDED)
                .title(format!("{} — {}", lang::current().chart_title, label))
                .border_style(Style::default().fg(theme::current().dim)),
        );
        f.render_widget(p, area);
        return;
    }

    let max_val = days.iter().map(|(_, i, o, cr, cc, _)| i + o + cr + cc).max().unwrap_or(1).max(1);
    let lines: Vec<Line> = days
        .iter()
        .flat_map(|(date, in_tok, out_tok, cr_tok, cc_tok, is_today)| {
            let total = in_tok + out_tok + cr_tok + cc_tok;
            let w = if max_val > 0 { (total as f64 / max_val as f64 * 30.0) as usize } else { 0 };
            let w = if total > 0 { w.max(1) } else { 0 };
            let bar_max = (area.width as usize).saturating_sub(24);
            let bar = "\u{2500}".repeat(w.min(bar_max));
            let color = if *is_today { theme::current().orange } else { theme::current().purple };
            let indent = "  ";
            let detail_lines: Vec<Line> = if total > 0 {
                let text = format!(
                    "{}: {}  {}: {}  {}: {}  {}: {}",
                    lang::current().chart_input,
                    format_tokens(*in_tok),
                    lang::current().chart_output,
                    format_tokens(*out_tok),
                    lang::current().chart_cache_read,
                    format_tokens(*cr_tok),
                    lang::current().chart_cache_create,
                    format_tokens(*cc_tok)
                );
                let max_w = (area.width as usize).saturating_sub(4).max(20);
                let (first, rest) = split_dw(&text, max_w);
                let mut result = vec![Line::from(vec![
                    Span::styled(indent, Style::default()),
                    Span::styled(first, Style::default().fg(theme::current().comment)),
                ])];
                for line in rest {
                    result.push(Line::from(Span::styled(format!("{}{}", indent, line), Style::default().fg(theme::current().comment))));
                }
                result
            } else {
                vec![]
            };
            let mut day_lines = vec![Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{}  ", date), Style::default().fg(theme::current().comment)),
                Span::styled(bar, Style::default().fg(color)),
                Span::styled(
                    format!(" {}", format_tokens(total)),
                    Style::default().fg(if *is_today { theme::current().orange } else { theme::current().dim }),
                ),
            ])];
            day_lines.extend(detail_lines);
            day_lines.push(Line::from(""));
            day_lines
        })
        .collect();

    let visible = (area.height as usize).saturating_sub(2);
    let max_scroll = lines.len().saturating_sub(visible);
    *chart_scroll = (*chart_scroll).min(max_scroll);
    let lines: Vec<Line> = lines.into_iter().skip(*chart_scroll).take(visible).collect();

    let p = Paragraph::new(lines).block(
        Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .title(format!("{} — {}", lang::current().chart_title, label))
            .border_style(Style::default().fg(theme::current().dim)),
    );
    f.render_widget(p, area);
}

fn dw(c: char) -> usize {
    if c > '\u{7e}' {
        2
    } else {
        1
    }
}

fn split_dw(text: &str, max_w: usize) -> (String, Vec<String>) {
    let mut first = String::new();
    let mut rest = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for c in text.chars() {
        let cw = dw(c);
        if cur_w + cw > max_w && !cur.is_empty() {
            if first.is_empty() {
                first = cur;
            } else {
                rest.push(cur);
            }
            cur = String::new();
            cur_w = 0;
        }
        cur.push(c);
        cur_w += cw;
    }
    if first.is_empty() {
        first = cur;
    } else {
        rest.push(cur);
    }
    (first, rest)
}
