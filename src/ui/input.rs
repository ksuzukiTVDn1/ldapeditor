use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use rust_i18n::t;

use crate::model::{EditState, TextInput};

// ── LDAP filter syntax highlight ─────────────────────────────────────────────

/// Linux VT compatible: uses only SGR 30-37 + SGR 1 (Bold).
/// & → Cyan+Bold, | → Green+Bold, ! → Red+Bold, normal → Gray
pub fn colorize_filter(input: &str) -> Line<'static> {
    let and_sty = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let or_sty = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let not_sty = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let norm_sty = Style::default().fg(Color::Gray);

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut stack: Vec<Style> = Vec::new();
    let mut cur = norm_sty;
    let mut buf = String::new();

    for (i, c) in input.char_indices() {
        match c {
            '(' => {
                if !buf.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buf), cur));
                }
                let new = match input[i + 1..].chars().next() {
                    Some('&') => and_sty,
                    Some('|') => or_sty,
                    Some('!') => not_sty,
                    _ => cur,
                };
                stack.push(cur);
                cur = new;
                buf.push('(');
            }
            ')' => {
                buf.push(')');
                if !buf.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut buf), cur));
                }
                cur = stack.pop().unwrap_or(norm_sty);
            }
            other => buf.push(other),
        }
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, cur));
    }
    Line::from(spans)
}

/// Returns a Span sequence with the cursor position highlighted via Modifier::REVERSED (SGR 7).
pub fn inject_cursor(spans: Vec<Span<'static>>, cursor: usize) -> Vec<Span<'static>> {
    let cur_sty = Style::default().add_modifier(Modifier::REVERSED);
    let mut result: Vec<Span<'static>> = Vec::new();
    let mut pos = 0usize;
    let mut done = false;

    for span in spans {
        let chars: Vec<char> = span.content.chars().collect();
        let len = chars.len();
        if !done && cursor >= pos && cursor < pos + len {
            let rel = cursor - pos;
            if rel > 0 {
                result.push(Span::styled(
                    chars[..rel].iter().collect::<String>(),
                    span.style,
                ));
            }
            result.push(Span::styled(chars[rel].to_string(), cur_sty));
            if rel + 1 < len {
                result.push(Span::styled(
                    chars[rel + 1..].iter().collect::<String>(),
                    span.style,
                ));
            }
            done = true;
        } else {
            result.push(span);
        }
        pos += len;
    }
    if !done {
        result.push(Span::styled(" ", cur_sty));
    }
    result
}

// ── Input bar rendering ──────────────────────────────────────────────────────

pub fn render_search_input(frame: &mut Frame, area: Rect, input: &TextInput) {
    let bg = bar_bg();
    let mut spans = vec![Span::styled(" / ", Style::default().fg(Color::Gray))];
    spans.extend(inject_cursor(
        colorize_filter(input.text()).spans,
        input.cursor(),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(bg), area);
}

pub fn render_edit_input(frame: &mut Frame, area: Rect, es: &EditState, is_add: bool) {
    let bg = bar_bg();
    let verb = if is_add {
        t!("input.add")
    } else {
        t!("input.edit")
    };
    let mut spans = vec![Span::styled(
        format!(" {verb} {}: ", es.attr),
        Style::default().fg(Color::Cyan),
    )];
    let plain = vec![Span::styled(
        es.input.text().to_string(),
        Style::default().fg(Color::Gray),
    )];
    spans.extend(inject_cursor(plain, es.input.cursor()));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(bg), area);
}

pub fn render_attr_name_input(frame: &mut Frame, area: Rect, es: &EditState) {
    let bg = bar_bg();
    let mut spans = vec![Span::styled(
        t!("ui.attr_name_input").to_string(),
        Style::default().fg(Color::Cyan),
    )];
    let plain = vec![Span::styled(
        es.input.text().to_string(),
        Style::default().fg(Color::Gray),
    )];
    spans.extend(inject_cursor(plain, es.input.cursor()));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(bg), area);
}

fn bar_bg() -> Style {
    Style::default().bg(Color::Black).fg(Color::Gray)
}
