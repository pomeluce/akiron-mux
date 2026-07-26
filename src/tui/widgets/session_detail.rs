use super::shared::{format_size, format_tokens};
use crate::db::sessions::SessionRecord;
use crate::core::models::AppType;
use crate::tui::lang;
use crate::tui::theme;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

/// Render a session detail panel in the given area.
pub fn render_session_detail(f: &mut Frame, area: Rect, session: &SessionRecord, tokens: Option<(i64, i64)>, active_provider: Option<&str>, active_profile: Option<&str>, app: AppType) {
    let home = std::env::var("HOME").unwrap_or_default();
    let path_short = session.project_path.replace(&home, "~");
    let max_w = (area.width as usize).saturating_sub(4).max(20);

    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(session.title.as_deref().unwrap_or(&session.id), Style::default().fg(theme::current().cyan)),
    ]));
    lines.push(Line::from(""));

    // Project
    lines.extend(line_with_wrap(
        lang::current().detail_project,
        &path_short,
        max_w,
        theme::current().purple,
        theme::current().yellow,
    ));
    lines.push(Line::from(""));

    // Profile: provider_id · profile_id
    let pid = session.profile_id.as_deref().or(active_provider).unwrap_or("-");
    let pfid = active_profile.unwrap_or("-");
    let profile_text = if app == AppType::Codex { pid.to_string() } else { format!("{} \u{b7} {}", active_provider.unwrap_or("-"), pfid) };
    lines.extend(line_with_wrap(
        if app == AppType::Codex { lang::current().detail_provider } else { lang::current().detail_profile },
        &profile_text,
        max_w,
        theme::current().purple,
        theme::current().fg,
    ));
    lines.push(Line::from(""));

    if app == AppType::Claude {
        lines.extend(line_with_wrap(
            lang::current().detail_mode,
            &session.mode,
            max_w,
            theme::current().purple,
            theme::current().green,
        ));
        lines.push(Line::from(""));
    }

    // Tokens
    let token_text = if let Some((p, c)) = tokens {
        format!("{} prompt / {} completion", format_tokens(p), format_tokens(c))
    } else {
        format!("{} prompt / {} completion", format_tokens(session.prompt_tokens), format_tokens(session.completion_tokens))
    };
    lines.extend(line_with_wrap(
        lang::current().detail_tokens,
        &token_text,
        max_w,
        theme::current().purple,
        theme::current().fg,
    ));
    lines.push(Line::from(""));

    // Started
    lines.extend(line_with_wrap(
        lang::current().detail_started,
        &session.start_time,
        max_w,
        theme::current().purple,
        theme::current().dim,
    ));
    lines.push(Line::from(""));

    // Messages
    lines.extend(line_with_wrap(
        lang::current().detail_messages,
        &session.message_count.to_string(),
        max_w,
        theme::current().purple,
        theme::current().fg,
    ));
    lines.push(Line::from(""));

    // Size
    lines.extend(line_with_wrap(
        lang::current().detail_size,
        &format_size(session.size_bytes),
        max_w,
        theme::current().purple,
        theme::current().fg,
    ));

    let p = Paragraph::new(lines)
        .block(
            Block::bordered()
                .border_set(ratatui::symbols::border::ROUNDED)
                .title(lang::current().detail_title)
                .border_style(Style::default().fg(theme::current().dim)),
        )
        .style(Style::default());
    f.render_widget(p, area);
}

/// Render empty detail placeholder
pub fn render_empty_detail(f: &mut Frame, area: Rect, hint: &str) {
    let p = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(hint, Style::default().fg(theme::current().comment))).centered(),
    ])
    .block(
        Block::bordered()
            .border_set(ratatui::symbols::border::ROUNDED)
            .title(lang::current().detail_title)
            .border_style(Style::default().fg(theme::current().dim)),
    );
    f.render_widget(p, area);
}

/// Build labeled lines with left pad + fixed-width label: "  Label:  value"
fn line_with_wrap(label: &str, value: &str, max_w: usize, label_color: ratatui::style::Color, value_color: ratatui::style::Color) -> Vec<Line<'static>> {
    let dw = label.chars().map(|c| if c > '\u{7e}' { 2 } else { 1 }).sum::<usize>();
    let prefix = if dw >= 8 {
        format!("  {}:  ", label)
    } else {
        format!("  {}{}:  ", label, " ".repeat(8 - dw))
    };
    let indent = " ".repeat(prefix.len());
    let first_value_w = max_w.saturating_sub(prefix.len());
    let (first_part, rest_lines) = split_value(value, first_value_w);

    let mut lines = vec![Line::from(vec![
        Span::styled(prefix, Style::default().fg(label_color)),
        Span::styled(first_part, Style::default().fg(value_color)),
    ])];

    for rest in rest_lines {
        lines.push(Line::from(vec![
            Span::styled(indent.clone(), Style::default()),
            Span::styled(rest, Style::default().fg(value_color)),
        ]));
    }

    lines
}

/// Split a value string: first line fits in max_w chars, remainder in max_w-char chunks
fn split_value(text: &str, max_w: usize) -> (String, Vec<String>) {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_w || max_w < 4 {
        return (text.to_string(), vec![]);
    }
    let first: String = chars[..max_w].iter().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut idx = max_w;
    while idx < chars.len() {
        let end = (idx + max_w).min(chars.len());
        parts.push(chars[idx..end].iter().collect());
        idx = end;
    }
    (first, parts)
}
