use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use rust_i18n::t;

use crate::app::{FormFocus, FormState, OcDeleteState, PickerState};
use crate::model::{LdapEntry, Selection};
use crate::ui::detail::DetailView;
use crate::ui::input::inject_cursor;

/// When `is_oc = true`, render the objectClass picker badges ([STR]/[AUX]).
/// objectClass deletion confirmation window (with an LDIF-style list of managed attributes).
pub fn render_oc_delete_confirm(frame: &mut Frame, area: Rect, state: &OcDeleteState) {
    let red_bold = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let yellow = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let gray = Style::default().fg(Color::Gray);
    let dim = Style::default().fg(Color::Gray);
    let attr_sty = Style::default().fg(Color::Cyan);

    const MAX_SHOWN: usize = 14;
    let shown = state.orphaned.len().min(MAX_SHOWN);
    let overflow = state.orphaned.len().saturating_sub(MAX_SHOWN);

    // Window width: attribute name + ": " + truncated value (up to 13 chars) + margin.
    let max_line = state
        .orphaned
        .iter()
        .take(MAX_SHOWN)
        .map(|(a, v)| a.len() + 2 + v.chars().count().min(13)) // "attr: value..."
        .max()
        .unwrap_or(0);
    let w = (max_line as u16 + 6).max(52).min(area.width);

    let content_rows: u16 = 2  // OC line + blank line
        + if state.orphaned.is_empty() { 0 }
          else { 1 + shown as u16 + if overflow > 0 { 1 } else { 0 } + 1 }
        + 1; // confirmation line
    let h = (content_rows + 2).min(area.height);

    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // OC to delete.
    lines.push(Line::from(vec![
        Span::styled(t!("ui.oc_delete_target").to_string(), gray),
        Span::styled(state.oc_name.clone(), red_bold),
    ]));
    lines.push(Line::raw(""));

    if !state.orphaned.is_empty() {
        lines.push(Line::from(Span::styled(
            t!("ui.oc_delete_managed_attrs").to_string(),
            yellow,
        )));
        for (attr, value) in state.orphaned.iter().take(MAX_SHOWN) {
            let display_val = if value.chars().count() > 10 {
                format!("{}...", value.chars().take(10).collect::<String>())
            } else {
                value.clone()
            };
            lines.push(Line::from(vec![
                Span::styled("   ", dim),
                Span::styled(attr.to_string(), attr_sty),
                Span::styled(": ", dim),
                Span::styled(display_val, Style::default().fg(Color::Red)),
            ]));
        }
        if overflow > 0 {
            lines.push(Line::from(Span::styled(
                t!("ui.oc_delete_overflow", count = overflow).to_string(),
                dim,
            )));
        }
        lines.push(Line::raw(""));
    }

    lines.push(Line::from(Span::styled(
        t!("ui.oc_delete_confirm").to_string(),
        gray,
    )));

    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(t!("ui.oc_delete_title").to_string())
                .border_style(Style::default().fg(Color::Red)),
        ),
        popup,
    );
}

/// Shared picker rendering. `is_oc` toggles item formatting (true: STR/AUX, false: MUST/MAY + SYNTAX).
/// The caller passes a title string appropriate for the use case.
pub fn render_picker(
    frame: &mut Frame,
    area: Rect,
    picker: &PickerState,
    is_oc: bool,
    title: &str,
) {
    let w = (area.width * 7 / 10).max(42).min(area.width);
    let h = (area.height * 65 / 100).max(10).min(area.height);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);

    let [input_area, list_area, hint_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(popup);

    // ── Filter input ─────────────────────────────────────────────────────────
    let picker_title = format!(
        "{}  ({} / {})",
        title,
        picker.filtered.len(),
        picker.entries.len()
    );
    let mut spans = vec![Span::styled(
        t!("ui.filter_label").to_string(),
        Style::default().fg(Color::Cyan),
    )];
    let plain = vec![Span::styled(
        picker.input.text().to_string(),
        Style::default().fg(Color::Gray),
    )];
    spans.extend(inject_cursor(plain, picker.input.cursor()));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(picker_title)
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        input_area,
    );

    // ── Candidate list ───────────────────────────────────────────────────────
    // OC picker: name on the left, STR/AUX tag at the right edge.
    // Attr picker: [MUST]/[MAY] badge + name + SYNTAX.
    let inner_w = list_area.width.saturating_sub(2) as usize; // excluding border

    let items: Vec<ListItem> = picker
        .filtered
        .iter()
        .map(|&idx| {
            let e = &picker.entries[idx];
            if is_oc {
                let (tag, tag_sty) = if e.is_must {
                    (
                        "[STR]",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("[AUX]", Style::default().fg(Color::Cyan))
                };
                let name_len = e.attr_name.chars().count();
                let gap = inner_w.saturating_sub(name_len + tag.len()).max(1);
                ListItem::new(Line::from(vec![
                    Span::raw(e.attr_name.clone()),
                    Span::raw(" ".repeat(gap)),
                    Span::styled(tag, tag_sty),
                ]))
            } else {
                let (badge, badge_sty) = if e.is_must {
                    (
                        "[MUST]",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("[MAY] ", Style::default().fg(Color::Green))
                };
                // Right-edge flags: N = MUST, S = SINGLE-VALUE.
                let flags = match (e.is_must, e.single_value) {
                    (true, true) => "[NS]",
                    (true, false) => "[N] ",
                    (false, true) => "[S] ",
                    (false, false) => "    ",
                };
                let flags_sty = Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD);
                // Compute remaining space after badge(6) + space(1) + name + syntax.
                let left = format!(
                    "{} {}{}",
                    badge,
                    e.attr_name,
                    if e.syntax.is_empty() {
                        String::new()
                    } else {
                        format!("  {}", e.syntax)
                    }
                );
                let left_len = left.chars().count();
                let gap = inner_w
                    .saturating_sub(left_len + flags.trim_end().len())
                    .max(1);
                ListItem::new(Line::from(vec![
                    Span::styled(badge, badge_sty),
                    Span::raw(format!(" {}", e.attr_name)),
                    Span::styled(
                        if e.syntax.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", e.syntax)
                        },
                        Style::default().fg(Color::Gray),
                    ),
                    Span::raw(" ".repeat(gap)),
                    Span::styled(flags, flags_sty),
                ]))
            }
        })
        .collect();

    let mut list_state = ListState::default();
    if !picker.filtered.is_empty() {
        list_state.select(Some(picker.selected));
    }

    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Gray)),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
        list_area,
        &mut list_state,
    );

    // ── Hint ─────────────────────────────────────────────────────────────────
    frame.render_widget(
        Paragraph::new(t!("hint.picker_window").to_string())
            .style(Style::default().bg(Color::Black).fg(Color::Gray)),
        hint_area,
    );
}

/// Render the Form phase of the child-entry creation wizard.
/// Each MUST attribute gets an input row; [Submit] / [Cancel] buttons sit at the bottom.
/// The RDN attribute row carries an `← (RDN)` badge and a DN preview is shown at the top.
pub fn render_create_child_form(frame: &mut Frame, area: Rect, parent_dn: &str, form: &FormState) {
    let gray = Style::default().fg(Color::Gray);
    let cyan = Style::default().fg(Color::Cyan);
    let yellow = Style::default().fg(Color::Yellow);
    let green = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);

    // Window size: width is 80% of area (min 60, max area.width).
    // Height: title(2) + DN preview(2) + field count + button row(2) + borders(2)
    let w = (area.width * 80 / 100).max(60).min(area.width);
    let content_h = 4 // dn / oc / blank / hint
                  + form.fields.len() as u16
                  + 2; // submit + blank
    let h = (content_h + 2).min(area.height);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);

    let title = t!("ui.create_form_title", oc = form.oc_name.clone()).to_string();
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Cyan)),
        popup,
    );

    // Inner area.
    let inner = Rect::new(popup.x + 1, popup.y + 1, popup.width - 2, popup.height - 2);

    // ── Header: parent DN / DN preview ───────────────────────────────────────
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(t!("ui.parent_label").to_string(), gray),
        Span::styled(parent_dn.to_string(), cyan),
    ]));
    lines.push(Line::from(vec![
        Span::styled(t!("ui.dn_label").to_string(), gray),
        Span::styled(form.dn_preview(parent_dn), green),
    ]));
    lines.push(Line::raw(""));

    // ── Field rows ───────────────────────────────────────────────────────────
    let attr_w = form
        .fields
        .iter()
        .map(|f| f.attr.len())
        .max()
        .unwrap_or(8)
        .min(20);
    for (i, field) in form.fields.iter().enumerate() {
        let focused = form.focus == FormFocus::Field(i);
        let attr_pad = format!("{:width$}", field.attr, width = attr_w);

        // Label.
        let label_sty = if focused {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            gray
        };

        let mut spans: Vec<Span<'static>> = vec![
            Span::styled(if focused { " ▶ " } else { "   " }, label_sty),
            Span::styled(attr_pad, label_sty),
            Span::styled(" : ", gray),
        ];

        // Input field: show the cursor when focused.
        if focused {
            let plain = vec![Span::styled(
                field.input.text().to_string(),
                Style::default().fg(Color::Gray),
            )];
            spans.extend(inject_cursor(plain, field.input.cursor()));
        } else {
            spans.push(Span::styled(
                field.input.text().to_string(),
                Style::default().fg(Color::Gray),
            ));
        }

        // RDN badge.
        if i == form.rdn_idx {
            spans.push(Span::styled(t!("ui.rdn_badge").to_string(), yellow));
        }

        lines.push(Line::from(spans));
    }

    lines.push(Line::raw(""));

    // ── Button row ───────────────────────────────────────────────────────────
    let button = |label: &str, focused: bool, color: Color| {
        let sty = if focused {
            Style::default()
                .bg(color)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color)
        };
        let text = if focused {
            format!(" ▶ {label} ")
        } else {
            format!("   {label}  ")
        };
        Span::styled(text, sty)
    };
    lines.push(Line::from(vec![
        Span::raw("   "),
        button("[ Submit ]", form.focus == FormFocus::Submit, Color::Green),
        Span::raw("   "),
        button("[ Cancel ]", form.focus == FormFocus::Cancel, Color::Red),
    ]));

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

pub fn render_action_dialog(frame: &mut Frame, area: Rect, entry: &LdapEntry, view: &DetailView) {
    let is_oc = view.is_oc_row(entry);
    let h = (if is_oc { 6u16 } else { 7u16 }).min(area.height);
    let w = 38u16.min(area.width);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);

    let label = match view.selected(entry) {
        Some(Selection::ObjectClass { value, .. }) => format!("objectClass: {value}"),
        Some(Selection::Attr { attr, value }) => {
            let v = if value.chars().count() > 20 {
                let end = value
                    .char_indices()
                    .nth(20)
                    .map(|(i, _)| i)
                    .unwrap_or(value.len());
                format!("{}…", &value[..end])
            } else {
                value.to_string()
            };
            format!("{attr}: {v}")
        }
        _ => String::new(),
    };

    let gray = Style::default().fg(Color::Gray);
    let cyan = Style::default().fg(Color::Cyan);
    let text = if is_oc {
        Text::from(vec![
            Line::from(Span::styled(format!(" {label}"), gray)),
            Line::raw(""),
            Line::from(Span::styled(t!("action.del_only").to_string(), cyan)),
        ])
    } else {
        Text::from(vec![
            Line::from(Span::styled(format!(" {label}"), gray)),
            Line::raw(""),
            Line::from(Span::styled(t!("action.edit_add").to_string(), cyan)),
            Line::from(Span::styled(t!("action.del_cancel").to_string(), cyan)),
        ])
    };

    frame.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(t!("ui.action_dialog").to_string())
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        popup,
    );
}
