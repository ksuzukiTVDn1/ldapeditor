use std::collections::HashSet;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};
use rust_i18n::t;

use crate::model::{LdapEntry, Selection};
use crate::schema::SchemaCache;

/// Width of the badge column for attributes (4 chars, e.g. "[NS]").
const BADGE_W: usize = 4;

// ── DetailView ────────────────────────────────────────────────────────────────

pub struct DetailView {
    pub row: usize,
    pub col: usize,
    scroll: u16,
    height: u16,
    value_col_width: u16,
}

impl Default for DetailView {
    fn default() -> Self {
        Self {
            row: 0,
            col: 1,
            scroll: 0,
            height: 0,
            value_col_width: 0,
        }
    }
}

impl DetailView {
    pub fn reset(&mut self) {
        self.row = 0;
        self.col = 1;
        self.scroll = 0;
    }

    // ── Navigation ───────────────────────────────────────────────────────────

    pub fn nav_len(&self, e: &LdapEntry) -> usize {
        e.oc_values.len() + 1 + e.attr_rows.len() + 1 + e.op_rows.len()
    }

    // Row-kind checks are derived through Selection.
    // Row-layout arithmetic is centralized in selected().
    pub fn is_oc_row(&self, e: &LdapEntry) -> bool {
        self.selected(e).is_some_and(|s| s.is_oc())
    }
    pub fn is_oc_plus_row(&self, e: &LdapEntry) -> bool {
        self.selected(e).is_some_and(|s| s.is_oc_plus())
    }
    pub fn is_attr_plus_row(&self, e: &LdapEntry) -> bool {
        self.selected(e).is_some_and(|s| s.is_attr_plus())
    }
    pub fn is_op_row(&self, e: &LdapEntry) -> bool {
        self.selected(e).is_some_and(|s| s.is_op())
    }

    pub fn selected<'e>(&self, e: &'e LdapEntry) -> Option<Selection<'e>> {
        let oc_end = e.oc_values.len();
        let attr_end = oc_end + 1 + e.attr_rows.len();
        let op_start = attr_end + 1;

        if self.row < oc_end {
            Some(Selection::ObjectClass {
                index: self.row,
                value: &e.oc_values[self.row],
            })
        } else if self.row == oc_end {
            Some(Selection::OcPlusRow)
        } else if self.row < attr_end {
            let j = self.row - (oc_end + 1);
            let (attr, value) = &e.attr_rows[j];
            Some(Selection::Attr { attr, value })
        } else if self.row == attr_end {
            Some(Selection::AttrPlusRow)
        } else {
            let j = self.row - op_start;
            e.op_rows
                .get(j)
                .map(|(attr, value)| Selection::OpAttr { attr, value })
        }
    }

    pub fn row_up(&mut self, e: &LdapEntry) {
        if self.row > 0 {
            self.row -= 1;
            self.ensure_visible(e);
        }
    }

    pub fn row_down(&mut self, e: &LdapEntry) {
        if self.row + 1 < self.nav_len(e) {
            self.row += 1;
            self.ensure_visible(e);
        }
    }

    pub fn col_left(&mut self) {
        self.col = 0;
    }
    pub fn col_right(&mut self) {
        self.col = 1;
    }

    // ── Scroll math ──────────────────────────────────────────────────────────

    fn attr_row_lines(&self, j: usize, e: &LdapEntry) -> u16 {
        if self.value_col_width == 0 {
            return 1;
        }
        let len = e.attr_rows[j].1.chars().count() as u16;
        len.div_ceil(self.value_col_width).max(1)
    }

    fn visual_line_of_row(&self, i: usize, e: &LdapEntry) -> u16 {
        let oc_len = e.oc_values.len();
        let attr_len = e.attr_rows.len();

        if i <= oc_len {
            return 1 + i as u16;
        }

        let attr_base: u16 = oc_len as u16 + 4;
        let j = i - (oc_len + 1);

        if j <= attr_len {
            let mut line = attr_base;
            for k in 0..j.min(attr_len) {
                line += self.attr_row_lines(k, e);
            }
            return line;
        }

        let total_attr: u16 = (0..attr_len).map(|k| self.attr_row_lines(k, e)).sum();
        let op_base = attr_base + total_attr + 3;
        let op_j = j - attr_len - 1;
        op_base + op_j as u16
    }

    fn ensure_visible(&mut self, e: &LdapEntry) {
        if self.height == 0 {
            return;
        }
        let line = self.visual_line_of_row(self.row, e);
        if line < self.scroll {
            self.scroll = line;
        } else if line >= self.scroll + self.height {
            self.scroll = line.saturating_sub(self.height - 1);
        }
    }

    // ── ratatui rendering ────────────────────────────────────────────────────

    /// `schema` is used to render the N/S badges. When None, render without badges.
    pub fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        entry: Option<&LdapEntry>,
        active: bool,
        schema: Option<&SchemaCache>,
    ) {
        let Some(e) = entry else {
            frame.render_widget(
                Paragraph::new(t!("ui.detail_select_entry").to_string())
                    .block(section_block("Detail", active)),
                area,
            );
            return;
        };

        let [dn_area, content_area] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).areas(area);

        // DN (fixed)
        let dn_title = t!("ui.detail_dn").to_string();
        frame.render_widget(
            Paragraph::new(e.dn.clone()).block(section_block_owned(dn_title, active)),
            dn_area,
        );

        // Combined scrollable area.
        // Attribute column width = attr_w (name) + BADGE_W (badge) + 3 (│) + value
        self.height = content_area.height.saturating_sub(2);
        self.value_col_width = content_area
            .width
            .saturating_sub(2 + e.attr_w as u16 + BADGE_W as u16 + 3);

        let lines = self.build_content_lines(e, active, schema);
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .scroll((self.scroll, 0))
                .block(content_block(active)),
            content_area,
        );
    }

    fn build_content_lines(
        &self,
        e: &LdapEntry,
        active: bool,
        schema: Option<&SchemaCache>,
    ) -> Vec<Line<'static>> {
        let sel = Style::default()
            .bg(Color::Blue)
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(Color::Gray);
        let hdr = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let sep_sty = Style::default().fg(Color::Gray);
        let plus_sty = Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD);
        let badge_sty = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);

        let oc_len = e.oc_values.len();
        let attr_len = e.attr_rows.len();
        let attr_w = e.attr_w;
        let name_w = attr_w.saturating_sub(BADGE_W); // width of the name part
        let op_start = oc_len + 2 + attr_len;
        let mut lines: Vec<Line<'static>> = Vec::new();

        // Compute the MUST set from the current entry's OCs once.
        let must_set: HashSet<String> = if let Some(sc) = schema {
            let mut s = HashSet::new();
            for oc in &e.oc_values {
                let (must, _) = sc.expanded_attrs(oc);
                s.extend(must);
            }
            s
        } else {
            HashSet::new()
        };

        // ── objectClass ──────────────────────────────────────────────────────
        lines.push(Line::from(Span::styled("objectClass", hdr)));
        for (i, v) in e.oc_values.iter().enumerate() {
            lines.push(if active && self.row == i {
                Line::from(Span::styled(format!(" {v}"), sel))
            } else {
                Line::from(vec![Span::styled(" ", dim), Span::raw(v.clone())])
            });
        }
        let oc_plus_sel = active && self.is_oc_plus_row(e);
        lines.push(Line::from(Span::styled(
            t!("ui.detail_add_oc").to_string(),
            if oc_plus_sel { sel } else { plus_sty },
        )));
        lines.push(Line::raw(""));

        // ── attributes ───────────────────────────────────────────────────────
        lines.push(Line::from(Span::styled("attributes", hdr)));
        for (j, (attr, value)) in e.attr_rows.iter().enumerate() {
            let row_idx = oc_len + 1 + j;
            let is_sel = active && self.row == row_idx;

            // Compute the N/S badge.
            let (is_must, is_sv) = if let Some(sc) = schema {
                let lc = attr.to_lowercase();
                (
                    must_set.contains(&lc),
                    sc.attr_type(&lc).map(|a| a.single_value).unwrap_or(false),
                )
            } else {
                (false, false)
            };

            let badge: &'static str = match (is_must, is_sv) {
                (true, true) => "[NS]",
                (true, false) => "[N] ",
                (false, true) => "[S] ",
                (false, false) => "    ",
            };
            let cur_badge_sty = if badge.trim().is_empty() {
                dim
            } else {
                badge_sty
            };

            // Attribute name part (trimmed and padded to name_w chars).
            let name_part: String = attr.chars().take(name_w).collect();
            let name_padded = format!("{:<width$}", name_part, width = name_w);
            let spaces_name = " ".repeat(name_w);

            let chunks = split_value(value, self.value_col_width as usize);
            for (ci, chunk) in chunks.into_iter().enumerate() {
                let line = if ci == 0 {
                    if is_sel {
                        match self.col {
                            0 => Line::from(vec![
                                Span::styled(name_padded.clone(), sel),
                                Span::styled(badge, cur_badge_sty),
                                Span::styled(" │ ", sep_sty),
                                Span::raw(chunk),
                            ]),
                            _ => Line::from(vec![
                                Span::styled(name_padded.clone(), dim),
                                Span::styled(badge, cur_badge_sty),
                                Span::styled(" │ ", sep_sty),
                                Span::styled(chunk, sel),
                            ]),
                        }
                    } else {
                        Line::from(vec![
                            Span::raw(name_padded.clone()),
                            Span::styled(badge, cur_badge_sty),
                            Span::styled(" │ ", sep_sty),
                            Span::raw(chunk),
                        ])
                    }
                } else {
                    // Continuation line: blank out the name and badge area.
                    if is_sel && self.col == 1 {
                        Line::from(vec![
                            Span::raw(spaces_name.clone()),
                            Span::styled("    ", dim),
                            Span::styled("   ", sep_sty),
                            Span::styled(chunk, sel),
                        ])
                    } else {
                        Line::from(vec![
                            Span::raw(spaces_name.clone()),
                            Span::styled("    ", dim),
                            Span::styled("   ", sep_sty),
                            Span::raw(chunk),
                        ])
                    }
                };
                lines.push(line);
            }
        }
        let attr_plus_sel = active && self.is_attr_plus_row(e);
        lines.push(Line::from(Span::styled(
            t!("ui.detail_add_attr").to_string(),
            if attr_plus_sel { sel } else { plus_sty },
        )));

        // ── operational ──────────────────────────────────────────────────────
        if !e.op_rows.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled("operational", hdr),
                Span::styled(t!("ui.detail_op_readonly").to_string(), dim),
            ]));
            for (j, (attr, value)) in e.op_rows.iter().enumerate() {
                let row_idx = op_start + j;
                let is_sel = active && self.row == row_idx;
                let name_part: String = attr.chars().take(name_w).collect();
                let name_padded = format!("{:<width$}", name_part, width = name_w);
                let spaces_name = " ".repeat(name_w);
                let chunks = split_value(value, self.value_col_width as usize);

                for (ci, chunk) in chunks.into_iter().enumerate() {
                    let line = if ci == 0 {
                        if is_sel {
                            Line::from(vec![
                                Span::styled(name_padded.clone(), sel),
                                Span::styled("    ", sel),
                                Span::styled(" │ ", sep_sty),
                                Span::styled(chunk, sel),
                            ])
                        } else {
                            Line::from(vec![
                                Span::styled(name_padded.clone(), dim),
                                Span::styled("    ", dim),
                                Span::styled(" │ ", sep_sty),
                                Span::styled(chunk, dim),
                            ])
                        }
                    } else if is_sel {
                        Line::from(vec![
                            Span::raw(spaces_name.clone()),
                            Span::styled("    ", dim),
                            Span::styled("   ", sep_sty),
                            Span::styled(chunk, sel),
                        ])
                    } else {
                        Line::from(vec![
                            Span::raw(spaces_name.clone()),
                            Span::styled("    ", dim),
                            Span::styled("   ", sep_sty),
                            Span::styled(chunk, dim),
                        ])
                    };
                    lines.push(line);
                }
            }
        }

        lines
    }
}

// ── value wrapping ────────────────────────────────────────────────────────────

fn split_value(value: &str, width: usize) -> Vec<String> {
    if width == 0 || value.chars().count() <= width {
        return vec![value.to_string()];
    }
    value
        .chars()
        .collect::<Vec<_>>()
        .chunks(width)
        .map(|c| c.iter().collect())
        .collect()
}

// ── block helpers ─────────────────────────────────────────────────────────────

pub fn section_block(title: &'static str, active: bool) -> Block<'static> {
    section_block_owned(title.to_string(), active)
}

pub fn section_block_owned(title: String, active: bool) -> Block<'static> {
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

fn content_block(active: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(if active {
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        })
}
