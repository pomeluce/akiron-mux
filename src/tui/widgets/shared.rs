use super::super::theme;
use crate::tui::lang;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Style},
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
    let color = if is_searching {
        theme::current().cyan
    } else {
        theme::current().comment
    };
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
            let label = Span::styled(
                format!(": {}", grp[1].0.clone()),
                Style::default().fg(theme::current().comment),
            );
            vec![
                Span::styled(grp[0].0.clone(), Style::default().fg(grp[0].1)),
                label,
            ]
        })
        .collect();

    let width = area.width.saturating_sub(2).max(10) as usize; // account for border
    let mut rows: Vec<Line> = Vec::new();
    let mut cur: Vec<Span> = Vec::new();
    let mut cur_w = 0usize;

    for g in &group_spans {
        let gw: usize = g.iter().map(|s| s.width()).sum();
        if cur_w + gw > width && !cur.is_empty() {
            rows.push(Line::from(std::mem::take(&mut cur)));
            cur_w = 0;
        }
        if !cur.is_empty() {
            cur.push(sep());
            cur_w += 2;
        }
        cur.extend(g.clone());
        cur_w += gw;
    }
    if !cur.is_empty() {
        rows.push(Line::from(cur));
    }
    if rows.is_empty() {
        rows.push(Line::default());
    }

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
    let mut lines = 1usize;
    let mut current = 0usize;
    for group in groups {
        if group.len() < 2 {
            continue;
        }
        let group_width = display_width(&group[0].0) + 2 + display_width(&group[1].0);
        if current > 0 && current + group_width > width {
            lines += 1;
            current = 0;
        }
        if current > 0 {
            current += 2;
        }
        current += group_width;
    }
    lines
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
            Span::styled(
                text.to_string(),
                Style::default().fg(theme::current().comment),
            ),
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
        Style::default().fg(Color::Black).bg(confirm_color)
    } else {
        Style::default().fg(theme::current().dim)
    };
    let xs = if selected == 1 {
        Style::default().fg(Color::Black).bg(theme::current().cyan)
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
    f.render_widget(Clear, popup);
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
    let popup_height = (message_lines as u16 + 4)
        .min(area.height)
        .max(5.min(area.height));
    let popup = centered_rect(popup_width, popup_height, area);
    let p = Paragraph::new(vec![
        Line::from(msg),
        Line::from(""),
        Line::from(Span::styled(
            lang::current().confirm_ok,
            Style::default().fg(Color::Black).bg(theme::current().cyan),
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
    f.render_widget(Clear, popup);
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
        format!(
            "{}...",
            s.chars().take(max.saturating_sub(3)).collect::<String>()
        )
    } else {
        s.to_string()
    }
}

/// Display-width of a string: ASCII = 1, CJK/etc = 2 columns
pub fn display_width(s: &str) -> usize {
    s.chars().map(|c| if c > '\u{7e}' { 2 } else { 1 }).sum()
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
