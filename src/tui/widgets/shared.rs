use super::super::theme;
use crate::tui::lang;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Wrap},
    Frame,
};

/// Centered rectangle helper
pub fn centered_rect(w: u16, h: u16, r: Rect) -> Rect {
    Rect {
        x: r.x + (r.width.saturating_sub(w)) / 2,
        y: r.y + (r.height.saturating_sub(h)) / 2,
        width: w.min(r.width),
        height: h.min(r.height),
    }
}

/// Clear one extra column on each side so an underlying double-width glyph
/// cannot straddle a popup border.
pub fn clear_popup_area(f: &mut Frame, popup: Rect) {
    let bounds = f.area();
    let left = popup.x.saturating_sub(1).max(bounds.x);
    let right = popup.right().saturating_add(1).min(bounds.right());
    let clear_area = Rect::new(left, popup.y, right.saturating_sub(left), popup.height);
    f.render_widget(Clear, clear_area);
}

/// Render a search box input widget
pub fn render_search_box(f: &mut Frame, area: Rect, query: &str, is_searching: bool) {
    let l = lang::current();
    let cursor = if is_searching { "\u{258c}" } else { "" };
    let text = if query.is_empty() && !is_searching {
        format!("\u{2315} {}", l.search_hint)
    } else if !query.is_empty() && !is_searching {
        format!("\u{2315} {} (/) — {} {}", query, l.sc_cancel, l.sc_back)
    } else {
        format!("\u{2315} {}{}", query, cursor)
    };
    let color = if is_searching { theme::current().cyan } else { theme::current().comment };
    let p = Paragraph::new(Line::from(Span::styled(text, Style::default().fg(color)))).block(
        Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .border_style(Style::default().fg(theme::current().dim)),
    );
    f.render_widget(p, area);
}

/// Render a shortcut bar that wraps at group boundaries for narrow windows
pub fn render_shortcut_bar(f: &mut Frame, area: Rect, groups: &[Vec<(String, Color)>]) {
    let sep = || Span::styled("  ".to_string(), Style::default());
    let group_spans: Vec<Vec<Span>> = groups
        .iter()
        .map(|grp| {
            let label = Span::styled(format!(": {}", grp[1].0.clone()), Style::default().fg(theme::current().comment));
            vec![Span::styled(grp[0].0.clone(), Style::default().fg(grp[0].1)), label]
        })
        .collect();

    let width = area.width.saturating_sub(2).max(10) as usize;
    let rows = shortcut_rows(width, groups)
        .into_iter()
        .map(|indices| {
            let mut spans = Vec::new();
            for index in indices {
                if !spans.is_empty() {
                    spans.push(sep());
                }
                spans.extend(group_spans[index].clone());
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();

    f.render_widget(
        Paragraph::new(rows).centered().block(
            Block::bordered()
                .border_set(ratatui::symbols::border::ROUNDED)
                .border_style(Style::default().fg(theme::current().dim)),
        ),
        area,
    );
}

pub fn shortcut_line_count(available_width: u16, groups: &[Vec<(String, Color)>]) -> usize {
    let width = available_width.saturating_sub(2).max(10) as usize;
    shortcut_rows(width, groups).len()
}

fn shortcut_rows(width: usize, groups: &[Vec<(String, Color)>]) -> Vec<Vec<usize>> {
    let mut rows = vec![Vec::new()];
    let mut current_width = 0usize;
    for (index, group) in groups.iter().enumerate() {
        if group.len() < 2 {
            continue;
        }
        // Span::width uses the same terminal-width rules as the renderer.
        let group_width = Span::raw(&group[0].0).width() + 2 + Span::raw(&group[1].0).width();
        let separator_width = usize::from(!rows.last().is_some_and(Vec::is_empty)) * 2;
        if current_width + separator_width + group_width > width && !rows.last().is_some_and(Vec::is_empty) {
            rows.push(Vec::new());
            current_width = 0;
        }
        let separator_width = usize::from(!rows.last().is_some_and(Vec::is_empty)) * 2;
        rows.last_mut().expect("shortcut row exists").push(index);
        current_width += separator_width + group_width;
    }
    rows
}

pub fn render_status_bar(f: &mut Frame, area: Rect, text: &str) {
    let (marker, color) = if text.starts_with("Error") || text.starts_with("Failed") {
        ("!", theme::current().red)
    } else if text.contains("scanning") || text.contains("扫描") {
        ("⟳", theme::current().purple)
    } else {
        ("✓", theme::current().green)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {} ", marker), Style::default().fg(color)),
            Span::styled(text.to_string(), Style::default().fg(theme::current().comment)),
        ])),
        area,
    );
}

/// Render a confirmation popup with two buttons
pub fn render_confirm_popup(
    f: &mut Frame,
    area: Rect,
    title: &str,
    msg: &str,
    labels: (&str, &str),
    state: (Color, usize), // color, selected button (0=confirm, 1=cancel)
) {
    let (confirm_label, cancel_label) = labels;
    let (confirm_color, selected) = state;
    let popup = centered_rect(44, 6, area);
    let cs = if selected == 0 {
        Style::default().fg(confirm_color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::current().dim)
    };
    let xs = if selected == 1 {
        Style::default().fg(theme::current().cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::current().dim)
    };

    let p = Paragraph::new(vec![
        Line::from(msg).centered(),
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("  {}  ", confirm_label), cs),
            Span::raw("     "),
            Span::styled(format!("  {}  ", cancel_label), xs),
        ])
        .centered(),
    ])
    .block(
        Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .title(Line::from(title).centered())
            .border_style(Style::default().fg(confirm_color)),
    );
    clear_popup_area(f, popup);
    f.render_widget(p, popup);
}

/// Render a simple message/notice popup with OK button
pub fn render_message_popup(f: &mut Frame, area: Rect, msg: &str) {
    let popup_width = area.width.saturating_sub(4).clamp(20, 80).min(area.width);
    let text_width = popup_width.saturating_sub(4).max(1) as usize;
    let message_lines = msg
        .lines()
        .map(|line| {
            let width = display_width(line).max(1);
            width.div_ceil(text_width)
        })
        .sum::<usize>()
        .max(1);
    let popup_height = (message_lines as u16 + 4).min(area.height).max(5.min(area.height));
    let popup = centered_rect(popup_width, popup_height, area);
    let p = Paragraph::new(vec![
        Line::from(msg),
        Line::from(""),
        Line::from(Span::styled(
            lang::current().confirm_ok,
            Style::default().fg(theme::current().cyan).add_modifier(Modifier::BOLD),
        ))
        .centered(),
    ])
    .alignment(Alignment::Center)
    .wrap(Wrap { trim: true })
    .block(
        Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .title(Line::from(lang::current().notice_title).centered())
            .border_style(Style::default().fg(theme::current().yellow)),
    );
    clear_popup_area(f, popup);
    f.render_widget(p, popup);
}

// === Format helpers ===
pub fn format_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn format_date(iso: &str) -> String {
    iso.get(5..16).unwrap_or(iso).to_string()
}

pub fn relative_time(iso: &str) -> String {
    if iso.len() < 19 {
        return format_date(iso);
    }
    let Some(timestamp) = iso.get(..19) else {
        return format_date(iso);
    };
    let parsed = chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S");
    let dt = match parsed {
        Ok(d) => d.and_utc(),
        Err(_) => return format_date(iso),
    };
    let dur = chrono::Utc::now() - dt;
    let secs = dur.num_seconds().max(0);
    let mins = dur.num_minutes().max(0);
    let hrs = dur.num_hours().max(0);
    let days = dur.num_days().max(0);
    if secs < 60 {
        format!("{} seconds ago", secs)
    } else if mins < 60 {
        format!("{} mins ago", mins)
    } else if hrs < 24 {
        format!("{} hours ago", hrs)
    } else if days < 7 {
        format!("{} days ago", days)
    } else if days < 30 {
        format!("{} weeks ago", days / 7)
    } else {
        format!("{} months ago", days / 30)
    }
}

pub fn format_tokens(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}...", s.chars().take(max.saturating_sub(3)).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Display-width of a string: ASCII = 1, CJK/etc = 2 columns
pub fn display_width(s: &str) -> usize {
    Span::raw(s).width()
}

/// Pad a label to the given display-width, appending `: ` suffix
pub fn pad_label(label: &str, w: usize) -> String {
    let dw = display_width(label);
    if dw >= w {
        format!("{}: ", label)
    } else {
        format!("{}{}: ", label, " ".repeat(w - dw))
    }
}

#[cfg(test)]
mod tests {
    use super::{centered_rect, shortcut_line_count, shortcut_rows};
    use ratatui::{layout::Rect, style::Color, text::Span};

    fn groups() -> Vec<Vec<(String, Color)>> {
        [(" J/K ", "导航"), (" H/← ", "返回"), (" C ", "Catalog"), (" V ", "Preview")]
            .into_iter()
            .map(|(key, label)| vec![(key.to_string(), Color::White), (label.to_string(), Color::White)])
            .collect()
    }

    #[test]
    fn shortcut_height_uses_the_same_rows_as_rendering() {
        let groups = groups();
        for available_width in 20u16..80 {
            let inner_width = available_width.saturating_sub(2).max(10) as usize;
            assert_eq!(shortcut_line_count(available_width, &groups), shortcut_rows(inner_width, &groups).len());
        }
    }

    #[test]
    fn shortcut_wrap_accounts_for_separator_width() {
        let groups = groups();
        let group_width = |index: usize| Span::raw(&groups[index][0].0).width() + 2 + Span::raw(&groups[index][1].0).width();
        let first_two_width = group_width(0) + 2 + group_width(1);
        assert_eq!(shortcut_rows(first_two_width - 1, &groups)[0], vec![0]);
        assert_eq!(shortcut_rows(first_two_width, &groups)[0], vec![0, 1]);
    }

    #[test]
    fn centered_popup_leaves_equal_horizontal_margins() {
        let area = Rect::new(0, 0, 120, 40);
        let popup = centered_rect(98, 30, area);
        assert_eq!(popup.x - area.x, area.right() - popup.right());
    }
}
