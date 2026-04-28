use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};
use rust_i18n::t;

use crate::model::{DitNode, FlatEntry, flat_view};

// ── Tree pane rendering ──────────────────────────────────────────────────────

/// Build a flat_view from the DitNode tree and render it in the tree pane.
/// The cursor position is given by `cursor` (an index into flat_view).
pub fn render_tree(
    frame: &mut Frame,
    area: Rect,
    tree_root: Option<&DitNode>,
    cursor: usize,
    active: bool,
) {
    let flat = match tree_root {
        Some(root) => flat_view(root),
        None => vec![],
    };

    let items: Vec<ListItem> = flat
        .iter()
        .map(|e| ListItem::new(flat_entry_line(e)))
        .collect();

    let mut state = ListState::default();
    if !flat.is_empty() {
        state.select(Some(cursor.min(flat.len() - 1)));
    }

    frame.render_stateful_widget(
        List::new(items)
            .block(pane_block(t!("ui.tree_pane").to_string(), active))
            .highlight_style(highlight_style()),
        area,
        &mut state,
    );
}

pub fn render_search_results(
    frame: &mut Frame,
    area: Rect,
    results: &[String],
    filter: &str,
    state: &mut ListState,
    active: bool,
) {
    let title = t!("ui.search_title", filter = filter, hits = results.len()).to_string();
    let items: Vec<ListItem> = results
        .iter()
        .map(|dn| ListItem::new(Line::raw(dn.clone())))
        .collect();
    frame.render_stateful_widget(
        List::new(items)
            .block(pane_block(title, active))
            .highlight_style(highlight_style()),
        area,
        state,
    );
}

/// Linux VT compatible list highlight: bg Blue + fg Gray + Bold
fn highlight_style() -> Style {
    Style::default()
        .bg(Color::Blue)
        .fg(Color::Gray)
        .add_modifier(Modifier::BOLD)
}

// ── FlatEntry → Line ──────────────────────────────────────────────────────────

fn flat_entry_line(entry: &FlatEntry<'_>) -> Line<'static> {
    let node = entry.node;
    let tree_sty = Style::default().fg(Color::Gray);
    let norm_sty = Style::default();
    let mut spans: Vec<Span<'static>> = Vec::new();

    if entry.depth == 0 {
        spans.push(Span::styled(
            if node.expanded { "▼ " } else { "▶ " },
            norm_sty,
        ));
    } else {
        for &cont in &entry.continuing {
            spans.push(Span::styled(if cont { "┆  " } else { "   " }, tree_sty));
        }
        spans.push(Span::styled(
            if entry.is_last { "└─ " } else { "├─ " },
            tree_sty,
        ));
        spans.push(Span::styled(
            match (node.has_children, node.expanded) {
                (true, true) => "▼ ",
                (true, false) => "▶ ",
                (false, _) => "  ",
            },
            norm_sty,
        ));
    }

    // Show the full DN for the root; show only the RDN for descendants.
    let label = if entry.depth == 0 {
        node.dn.clone()
    } else {
        node.dn.split(',').next().unwrap_or(&node.dn).to_string()
    };
    spans.push(Span::styled(label, norm_sty));
    Line::from(spans)
}

fn pane_block(title: String, active: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if active {
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        })
}
