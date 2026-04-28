use rust_i18n::t;

use crate::ldap::ChildNode;
use crate::model::{DitNode, FlatEntry, Pane, flat_view};

use super::App;
use super::state::{Mode, PendingExpansion};

/// Show a confirmation dialog before expanding when the child count exceeds this threshold.
const EXPAND_CONFIRM_THRESHOLD: usize = 100;

// ── Tree ─────────────────────────────────────────────────────────────────────

/// State for the DIT tree view (root + cursor + base_dn).
/// Wraps the older implementation that exposed a `usize` cursor over flat_view
/// indices behind a safe API. flat_view is rebuilt via DFS on each call
/// (acceptable cost since a typical DIT has fewer than ~1000 nodes).
pub struct Tree {
    root: DitNode,
    cursor: usize,
    base_dn: String,
}

/// Surface information about the cursor position (lightweight summary obtainable in one DFS pass).
pub struct NodeInfo {
    pub dn: String,
    pub depth: usize,
    pub expanded: bool,
    pub has_children: bool,
}

/// Result of `collapse_or_parent`.
pub enum CollapseResult {
    /// Nothing was done (e.g. ← at the root).
    NoOp,
    /// Collapsed an expanded node.
    Collapsed,
    /// Moved the cursor to the parent node (destination DN).
    MovedToParent(String),
}

impl Tree {
    /// Build a Tree from a base DN and the already-fetched children (the root is returned expanded).
    pub fn from_children(base_dn: String, children: Vec<ChildNode>) -> Self {
        let has_children = !children.is_empty();
        let root = DitNode {
            dn: base_dn.clone(),
            has_children,
            expanded: true,
            children: Some(children.into_iter().map(child_to_dit).collect()),
        };
        Self {
            root,
            cursor: 0,
            base_dn,
        }
    }

    /// Build from a single node (e.g. a placeholder used for error display).
    pub fn from_root(root: DitNode, base_dn: String) -> Self {
        Self {
            root,
            cursor: 0,
            base_dn,
        }
    }

    pub fn root(&self) -> &DitNode {
        &self.root
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn base_dn(&self) -> &str {
        &self.base_dn
    }

    pub fn flat(&self) -> Vec<FlatEntry<'_>> {
        flat_view(&self.root)
    }

    pub fn cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn cursor_down(&mut self) {
        let max = self.flat().len().saturating_sub(1);
        if self.cursor < max {
            self.cursor += 1;
        }
    }

    /// Set the cursor directly to an arbitrary flat_view index (out-of-range values clamp to the end).
    /// Used to move to the parent after a deletion.
    pub fn set_cursor(&mut self, idx: usize) {
        let max = self.flat().len().saturating_sub(1);
        self.cursor = idx.min(max);
    }

    /// Return (DN, flat_view index) of the parent of the node at the cursor.
    /// Returns None when the cursor is at the root (depth=0).
    pub fn parent_info(&self) -> Option<(String, usize)> {
        let info = self.selected_info()?;
        if info.depth == 0 {
            return None;
        }
        let flat = self.flat();
        let idx = flat[..self.cursor]
            .iter()
            .rposition(|e| e.depth < info.depth)?;
        Some((flat[idx].node.dn.clone(), idx))
    }

    pub fn selected_dn(&self) -> Option<String> {
        self.flat().get(self.cursor).map(|e| e.node.dn.clone())
    }

    pub fn selected_info(&self) -> Option<NodeInfo> {
        let flat = self.flat();
        let e = flat.get(self.cursor)?;
        Some(NodeInfo {
            dn: e.node.dn.clone(),
            depth: e.depth,
            expanded: e.node.expanded,
            has_children: e.node.has_children,
        })
    }

    /// Collapse or move to parent. If expanded, collapse while keeping the child cache;
    /// if not expanded and not root, move the cursor to the parent.
    pub fn collapse_or_parent(&mut self) -> CollapseResult {
        let info = match self.selected_info() {
            Some(i) => i,
            None => return CollapseResult::NoOp,
        };

        if info.expanded {
            if let Some(node) = self.root.find_mut(&info.dn) {
                node.collapse();
            }
            return CollapseResult::Collapsed;
        }

        if info.depth == 0 {
            return CollapseResult::NoOp;
        }

        let flat = self.flat();
        let Some(idx) = flat[..self.cursor]
            .iter()
            .rposition(|e| e.depth < info.depth)
        else {
            return CollapseResult::NoOp;
        };
        let parent_dn = flat[idx].node.dn.clone();
        self.cursor = idx;
        CollapseResult::MovedToParent(parent_dn)
    }

    /// Set the children on the node at the given DN and mark it expanded.
    pub fn expand_at(&mut self, dn: &str, children: Vec<ChildNode>) {
        if let Some(node) = self.root.find_mut(dn) {
            node.set_children(children.into_iter().map(child_to_dit).collect());
        }
    }

    /// Mark the node as having no children (has_children=false and children=Some([])).
    pub fn mark_no_children_at(&mut self, dn: &str) {
        if let Some(node) = self.root.find_mut(dn) {
            node.mark_no_children();
        }
    }
}

fn child_to_dit(c: ChildNode) -> DitNode {
    DitNode::unloaded(c.dn, c.has_children)
}

// ── App: thin wrappers around tree operations ────────────────────────────────

impl App {
    pub(super) fn toggle_pane(&mut self) {
        self.active_pane = match self.active_pane {
            Pane::Tree => Pane::Detail,
            Pane::Detail => Pane::Tree,
        };
    }

    pub(super) fn tree_up(&mut self) {
        if let Some(t) = &mut self.browse {
            t.cursor_up();
        }
    }

    pub(super) fn tree_down(&mut self) {
        if let Some(t) = &mut self.browse {
            t.cursor_down();
        }
    }

    pub(super) fn selected_tree_dn(&self) -> Option<String> {
        self.browse.as_ref().and_then(|t| t.selected_dn())
    }

    /// → key: always re-fetch children from LDAP and expand.
    pub(super) async fn expand_selected(&mut self) {
        let info = match self.browse.as_ref().and_then(|t| t.selected_info()) {
            Some(i) => i,
            None => return,
        };
        if !info.has_children || info.expanded {
            return;
        }

        let Some(result) = self.ldap_children(&info.dn).await else {
            return;
        };

        match result {
            Ok(children) if children.len() > EXPAND_CONFIRM_THRESHOLD => {
                self.status = t!("status.expand_confirm", count = children.len()).to_string();
                self.mode = Mode::ConfirmExpand(PendingExpansion {
                    dn: info.dn,
                    children,
                });
            }
            Ok(children) if !children.is_empty() => {
                if let Some(t) = &mut self.browse {
                    t.expand_at(&info.dn, children);
                }
            }
            Ok(_) => {
                if let Some(t) = &mut self.browse {
                    t.mark_no_children_at(&info.dn);
                }
            }
            Err(e) => {
                self.status = format!("Expand error: {e}");
            }
        }
    }

    /// Apply an expansion confirmed via ConfirmExpand to the Tree.
    pub(super) fn apply_expansion(&mut self, dn: &str, children: Vec<ChildNode>) {
        if let Some(t) = &mut self.browse {
            t.expand_at(dn, children);
        }
    }

    /// ← key: collapse if expanded, otherwise move to parent.
    pub(super) fn collapse_or_parent(&mut self) {
        let result = match &mut self.browse {
            Some(t) => t.collapse_or_parent(),
            None => return,
        };
        if let CollapseResult::MovedToParent(parent_dn) = result {
            // Clear the detail view if the parent differs from the currently shown entry.
            if self.current_entry.as_ref().map(|e| e.dn.as_str()) != Some(parent_dn.as_str()) {
                self.current_entry = None;
            }
        }
    }
}
