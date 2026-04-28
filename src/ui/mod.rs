pub mod detail;
pub mod input;
pub mod overlay;
pub mod tree;

use std::io;

use anyhow::Result;
use crossterm::event::{Event, EventStream};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use rust_i18n::t;
use tokio_stream::StreamExt;

use crate::app::{App, Mode, SuffixSelection};
use crate::model::Pane;

// ── Event loop ───────────────────────────────────────────────────────────────

pub async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let mut events = EventStream::new();
    loop {
        terminal.draw(|frame| draw(frame, app))?;
        let Some(Ok(Event::Key(key))) = events.next().await else {
            break;
        };
        if !app.handle_key(key).await {
            break;
        }
    }
    app.unbind().await;
    Ok(())
}

// ── Rendering ────────────────────────────────────────────────────────────────

fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let needs_2line = matches!(
        app.mode,
        Mode::SearchInput | Mode::EditValue(_) | Mode::AddValue(_) | Mode::AddAttrName(_)
    );
    let main_h = if needs_2line {
        area.height.saturating_sub(2)
    } else {
        area.height.saturating_sub(1)
    };
    let aux_h = if needs_2line { 2u16 } else { 1u16 };

    let [main_area, aux_area] =
        Layout::vertical([Constraint::Length(main_h), Constraint::Length(aux_h)]).areas(area);

    let [tree_area, detail_area] =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
            .areas(main_area);

    // ── Left pane ────────────────────────────────────────────────────────────
    let tree_active = app.active_pane == Pane::Tree;
    match &mut app.mode {
        Mode::SelectSuffix(sel) => render_suffix_list(frame, tree_area, sel),
        Mode::SearchResults => tree::render_search_results(
            frame,
            tree_area,
            &app.search.results,
            app.search.input.text(),
            &mut app.search.list_state,
            tree_active,
        ),
        _ => {
            let (root, cursor) = match &app.browse {
                Some(t) => (Some(t.root()), t.cursor()),
                None => (None, 0),
            };
            tree::render_tree(frame, tree_area, root, cursor, tree_active);
        }
    }

    // ── Right pane ───────────────────────────────────────────────────────────
    let detail_active = app.active_pane == Pane::Detail;
    if matches!(app.mode, Mode::SelectSuffix(_)) {
        render_suffix_help(frame, detail_area);
    } else {
        app.detail_view.render(
            frame,
            detail_area,
            app.current_entry.as_ref(),
            detail_active,
            app.schema_cache.as_ref(),
        );
    }

    // ── Status bar / input bar ───────────────────────────────────────────────
    render_aux(frame, aux_area, app);

    // ── Overlays ─────────────────────────────────────────────────────────────
    match &app.mode {
        Mode::ActionDialog => {
            if let Some(entry) = &app.current_entry {
                overlay::render_action_dialog(frame, area, entry, &app.detail_view);
            }
        }
        Mode::ConfirmOcDelete(state) => {
            overlay::render_oc_delete_confirm(frame, area, state);
        }
        Mode::Picker(picker) => {
            let title = t!("ui.attr_picker_title").to_string();
            overlay::render_picker(frame, area, picker, false, &title);
        }
        Mode::OcPicker(picker) => {
            let title = t!("ui.oc_picker_title").to_string();
            overlay::render_picker(frame, area, picker, true, &title);
        }
        Mode::CreateChild(state) => match &state.phase {
            crate::app::CreatePhase::PickOc(picker) => {
                let title =
                    t!("ui.create_pick_oc_title", parent = state.parent_dn.clone(),).to_string();
                overlay::render_picker(frame, area, picker, true, &title);
            }
            crate::app::CreatePhase::PickRdn(rdn_state) => {
                let title =
                    t!("ui.create_pick_rdn_title", oc = rdn_state.oc_name.clone(),).to_string();
                overlay::render_picker(frame, area, &rdn_state.picker, false, &title);
            }
            crate::app::CreatePhase::Form(form) => {
                overlay::render_create_child_form(frame, area, &state.parent_dn, form);
            }
        },
        _ => {}
    }
}

fn render_suffix_list(frame: &mut Frame, area: ratatui::layout::Rect, sel: &mut SuffixSelection) {
    let items: Vec<ListItem> = sel
        .candidates
        .iter()
        .map(|s| ListItem::new(Line::raw(s.clone())))
        .collect();
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(t!("ui.naming_context").to_string())
                    .border_style(Style::default().fg(Color::Gray)),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
        area,
        &mut sel.state,
    );
}

fn render_suffix_help(frame: &mut Frame, area: ratatui::layout::Rect) {
    let line = |s: String| Line::from(Span::styled(s, Style::default().fg(Color::Gray)));
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::raw(""),
            line(t!("suffix_help.select").to_string()),
            line(t!("suffix_help.browse").to_string()),
            line(t!("suffix_help.quit").to_string()),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        ),
        area,
    );
}

fn render_aux(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let bar_bg = Style::default().bg(Color::Black).fg(Color::Gray);
    let bar_dim = Style::default().bg(Color::Black).fg(Color::Gray);

    match &app.mode {
        Mode::SearchInput => {
            let [in_line, hint_line] =
                Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
            input::render_search_input(frame, in_line, &app.search.input);
            frame.render_widget(
                Paragraph::new(t!("hint.search_input").to_string()).style(bar_dim),
                hint_line,
            );
        }
        Mode::AddAttrName(es) => {
            let [in_line, hint_line] =
                Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
            input::render_attr_name_input(frame, in_line, es);
            frame.render_widget(
                Paragraph::new(t!("hint.attr_name").to_string()).style(bar_dim),
                hint_line,
            );
        }
        Mode::EditValue(es) | Mode::AddValue(es) => {
            let [in_line, hint_line] =
                Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
            let is_add = matches!(app.mode, Mode::AddValue(_));
            input::render_edit_input(frame, in_line, es, is_add);
            frame.render_widget(
                Paragraph::new(t!("hint.edit_value").to_string()).style(bar_dim),
                hint_line,
            );
        }
        Mode::ConfirmExpand(_) => {
            frame.render_widget(
                Paragraph::new(Line::raw(format!(" {}", app.status)))
                    .style(Style::default().bg(Color::Yellow).fg(Color::Black)),
                area,
            );
        }
        Mode::ConfirmOcDelete(_) => {
            // The details are shown in the overlay, so keep the status bar simple.
            frame.render_widget(
                Paragraph::new(t!("hint.oc_delete").to_string())
                    .style(Style::default().bg(Color::Red).fg(Color::Black)),
                area,
            );
        }
        Mode::ConfirmDelete(es) => {
            let msg = t!(
                "ui.attr_value_delete_confirm",
                attr = es.attr.clone(),
                value = es.old_value.clone(),
            )
            .to_string();
            frame.render_widget(
                Paragraph::new(msg).style(Style::default().bg(Color::Red).fg(Color::Black)),
                area,
            );
        }
        Mode::ConfirmEntryDelete(state) => {
            let warn = if state.has_children {
                t!("ui.has_children_warn").to_string()
            } else {
                String::new()
            };
            let msg = t!(
                "ui.entry_delete_confirm",
                dn = state.dn.clone(),
                warn = warn,
            )
            .to_string();
            frame.render_widget(
                Paragraph::new(msg).style(Style::default().bg(Color::Red).fg(Color::Black)),
                area,
            );
        }
        Mode::SelectSuffix(_) => {
            frame.render_widget(
                Paragraph::new(Line::raw(
                    t!("hint.suffix_select", status = app.status.clone()).to_string(),
                ))
                .style(Style::default().bg(Color::Black).fg(Color::Gray)),
                area,
            );
        }
        Mode::Picker(_) | Mode::OcPicker(_) => {
            frame.render_widget(
                Paragraph::new(Line::raw(
                    t!("hint.picker_status", status = app.status.clone()).to_string(),
                ))
                .style(Style::default().bg(Color::Black).fg(Color::Gray)),
                area,
            );
        }
        Mode::CreateChild(state) => {
            let hint = match &state.phase {
                crate::app::CreatePhase::PickOc(_) => t!("hint.create_pick_oc").to_string(),
                crate::app::CreatePhase::PickRdn(_) => t!("hint.create_pick_rdn").to_string(),
                crate::app::CreatePhase::Form(_) => t!("hint.create_form").to_string(),
            };
            frame.render_widget(
                Paragraph::new(Line::raw(
                    t!(
                        "hint.create_status",
                        status = app.status.clone(),
                        hint = hint
                    )
                    .to_string(),
                ))
                .style(Style::default().bg(Color::Black).fg(Color::Gray)),
                area,
            );
        }
        Mode::Browse | Mode::ActionDialog => {
            let hint = match app.active_pane {
                Pane::Tree => t!("hint.browse_tree").to_string(),
                Pane::Detail => t!("hint.browse_detail").to_string(),
            };
            frame.render_widget(
                Paragraph::new(Line::raw(
                    t!(
                        "hint.browse_status",
                        status = app.status.clone(),
                        hint = hint
                    )
                    .to_string(),
                ))
                .style(bar_bg),
                area,
            );
        }
        Mode::SearchResults => {
            let hint = match app.active_pane {
                Pane::Tree => t!("hint.search_results_tree").to_string(),
                Pane::Detail => t!("hint.search_results_detail").to_string(),
            };
            frame.render_widget(
                Paragraph::new(Line::raw(
                    t!(
                        "hint.browse_status",
                        status = app.status.clone(),
                        hint = hint
                    )
                    .to_string(),
                ))
                .style(bar_bg),
                area,
            );
        }
    }
}
