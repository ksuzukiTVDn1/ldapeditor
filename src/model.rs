//! Pure domain data types. Mode lives in `app::state` since it is part of the app state machine.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// ── Pane ──────────────────────────────────────────────────────────────────────

#[derive(PartialEq)]
pub enum Pane {
    Tree,
    Detail,
}

// ── DitNode ───────────────────────────────────────────────────────────────────

/// A single DIT node. The tree mirrors the LDAP directory hierarchy.
///
/// - `children: None`     → not yet loaded
/// - `children: Some([])` → loaded, no children
/// - `children: Some([…])` → cached (retained even when collapsed)
pub struct DitNode {
    pub dn: String,
    pub has_children: bool,
    pub expanded: bool,
    pub children: Option<Vec<DitNode>>,
}

impl DitNode {
    /// Unloaded node that potentially has children.
    pub fn unloaded(dn: String, has_children: bool) -> Self {
        Self {
            dn,
            has_children,
            expanded: false,
            children: None,
        }
    }

    /// Recursively search by DN and return a mutable reference.
    pub fn find_mut(&mut self, dn: &str) -> Option<&mut Self> {
        if self.dn == dn {
            return Some(self);
        }
        if let Some(children) = &mut self.children {
            for child in children.iter_mut() {
                if let Some(found) = child.find_mut(dn) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Collapse (retain children as a cache).
    pub fn collapse(&mut self) {
        self.expanded = false;
    }

    /// Replace children and mark as expanded.
    pub fn set_children(&mut self, children: Vec<DitNode>) {
        self.children = Some(children);
        self.expanded = true;
    }

    /// Mark the node as having no children.
    pub fn mark_no_children(&mut self) {
        self.has_children = false;
        self.children = Some(vec![]);
    }
}

// ── FlatEntry / flat_view ─────────────────────────────────────────────────────

/// A flat entry produced by DFS for ratatui rendering.
pub struct FlatEntry<'a> {
    pub node: &'a DitNode,
    pub depth: usize,
    pub is_last: bool,
    /// continuing[i] = true → ancestor at depth i still has siblings → draw ┆.
    pub continuing: Vec<bool>,
}

/// Flatten the expanded nodes via DFS (for rendering and cursor math).
pub fn flat_view(root: &DitNode) -> Vec<FlatEntry<'_>> {
    let mut result = Vec::new();
    collect_flat(root, 0, true, vec![], &mut result);
    result
}

fn collect_flat<'a>(
    node: &'a DitNode,
    depth: usize,
    is_last: bool,
    continuing: Vec<bool>,
    result: &mut Vec<FlatEntry<'a>>,
) {
    result.push(FlatEntry {
        node,
        depth,
        is_last,
        continuing: continuing.clone(),
    });
    if node.expanded {
        if let Some(children) = &node.children {
            let n = children.len();
            for (i, child) in children.iter().enumerate() {
                let child_is_last = i == n - 1;
                let mut child_cont = continuing.clone();
                child_cont.push(!is_last); // if the current node has siblings, children draw ┆
                collect_flat(child, depth + 1, child_is_last, child_cont, result);
            }
        }
    }
}

// ── TextInput ─────────────────────────────────────────────────────────────────

/// Single-line input model with text and a char-based cursor.
/// Shared by the search filter, attribute-value editor, and picker filter.
#[derive(Default, Clone)]
pub struct TextInput {
    text: String,
    cursor: usize, // in chars
}

impl TextInput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize with the cursor positioned at the end of `text`.
    pub fn with_text(text: String) -> Self {
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
    pub fn right(&mut self) {
        let m = self.len();
        if self.cursor < m {
            self.cursor += 1;
        }
    }
    pub fn home(&mut self) {
        self.cursor = 0;
    }
    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    pub fn insert(&mut self, c: char) {
        let mut cs: Vec<char> = self.text.chars().collect();
        cs.insert(self.cursor, c);
        self.text = cs.into_iter().collect();
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let mut cs: Vec<char> = self.text.chars().collect();
            cs.remove(self.cursor - 1);
            self.text = cs.into_iter().collect();
            self.cursor -= 1;
        }
    }

    pub fn delete_fwd(&mut self) {
        if self.cursor < self.len() {
            let mut cs: Vec<char> = self.text.chars().collect();
            cs.remove(self.cursor);
            self.text = cs.into_iter().collect();
        }
    }

    /// Handle the standard editing keys (Left/Right/Home/End/Backspace/Delete/Char).
    /// Returns true if the key was consumed, false for unsupported keys.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Left => {
                self.left();
                true
            }
            KeyCode::Right => {
                self.right();
                true
            }
            KeyCode::Home => {
                self.home();
                true
            }
            KeyCode::End => {
                self.end();
                true
            }
            KeyCode::Backspace => {
                self.backspace();
                true
            }
            KeyCode::Delete => {
                self.delete_fwd();
                true
            }
            KeyCode::Char(c)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.insert(c);
                true
            }
            _ => false,
        }
    }
}

// ── EditState ─────────────────────────────────────────────────────────────────

pub struct EditState {
    pub attr: String,
    pub old_value: String,
    pub input: TextInput,
}

impl EditState {
    /// Edit an existing value (preset `input` to the current value).
    pub fn edit(attr: String, current: String) -> Self {
        Self {
            attr,
            old_value: current.clone(),
            input: TextInput::with_text(current),
        }
    }

    /// Add a new value (`input` starts empty).
    pub fn add(attr: String) -> Self {
        Self {
            attr,
            old_value: String::new(),
            input: TextInput::new(),
        }
    }

    /// Delete an existing value (old_value = current value, `input` is unused).
    pub fn delete(attr: String, current: String) -> Self {
        Self {
            attr,
            old_value: current,
            input: TextInput::new(),
        }
    }

    /// Attribute-name input mode (attr/old_value empty; the name goes into `input`).
    pub fn attr_name() -> Self {
        Self {
            attr: String::new(),
            old_value: String::new(),
            input: TextInput::new(),
        }
    }
}

// ── LdapEntry ─────────────────────────────────────────────────────────────────

pub struct LdapEntry {
    pub dn: String,
    pub oc_values: Vec<String>,
    pub attr_rows: Vec<(String, String)>,
    pub op_rows: Vec<(String, String)>,
    pub attr_w: usize,
}

impl LdapEntry {
    /// Set of existing user attribute names (lowercase) plus "objectclass".
    /// Used by the schema picker and the MUST attribute queue to identify
    /// attributes that already have values.
    pub fn existing_attr_names_lower(&self) -> std::collections::HashSet<String> {
        self.attr_rows
            .iter()
            .map(|(a, _)| a.to_lowercase())
            .chain(std::iter::once("objectclass".to_string()))
            .collect()
    }
}

// ── Selection ─────────────────────────────────────────────────────────────────

pub enum Selection<'e> {
    ObjectClass {
        #[allow(dead_code)]
        index: usize,
        value: &'e str,
    },
    OcPlusRow,
    Attr {
        attr: &'e str,
        value: &'e str,
    },
    AttrPlusRow,
    OpAttr {
        #[allow(dead_code)]
        attr: &'e str,
        #[allow(dead_code)]
        value: &'e str,
    }, // read-only
}

impl Selection<'_> {
    pub fn is_oc(&self) -> bool {
        matches!(self, Selection::ObjectClass { .. })
    }
    pub fn is_oc_plus(&self) -> bool {
        matches!(self, Selection::OcPlusRow)
    }
    pub fn is_attr_plus(&self) -> bool {
        matches!(self, Selection::AttrPlusRow)
    }
    pub fn is_op(&self) -> bool {
        matches!(self, Selection::OpAttr { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(dn: &str) -> DitNode {
        DitNode {
            dn: dn.to_string(),
            has_children: false,
            expanded: false,
            children: Some(vec![]),
        }
    }

    fn branch(dn: &str, children: Vec<DitNode>) -> DitNode {
        DitNode {
            dn: dn.to_string(),
            has_children: true,
            expanded: true,
            children: Some(children),
        }
    }

    fn collapsed(dn: &str, cached_children: Vec<DitNode>) -> DitNode {
        DitNode {
            dn: dn.to_string(),
            has_children: true,
            expanded: false,
            children: Some(cached_children),
        }
    }

    #[test]
    fn flat_view_root_only() {
        let root = leaf("dc=example,dc=com");
        let flat = flat_view(&root);
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].node.dn, "dc=example,dc=com");
        assert_eq!(flat[0].depth, 0);
        assert!(flat[0].is_last);
        assert!(flat[0].continuing.is_empty());
    }

    #[test]
    fn flat_view_two_children_continuing() {
        // root (is_last=true) → child_cont pushed = !true = false
        //   ├─ ou=users  (depth=1, is_last=false, continuing=[false])
        //   └─ ou=groups (depth=1, is_last=true,  continuing=[false])
        let root = branch(
            "dc=example,dc=com",
            vec![
                leaf("ou=users,dc=example,dc=com"),
                leaf("ou=groups,dc=example,dc=com"),
            ],
        );
        let flat = flat_view(&root);
        assert_eq!(flat.len(), 3);

        assert_eq!(flat[1].node.dn, "ou=users,dc=example,dc=com");
        assert!(!flat[1].is_last);
        assert_eq!(flat[1].continuing, vec![false]); // root has no siblings

        assert_eq!(flat[2].node.dn, "ou=groups,dc=example,dc=com");
        assert!(flat[2].is_last);
        assert_eq!(flat[2].continuing, vec![false]);
    }

    #[test]
    fn flat_view_nested_continuing() {
        // root (is_last=true)
        //   ├─ ou=users (depth=1, is_last=false)
        //   │    └─ cn=user1 (depth=2)
        //   │         continuing = [false (root→no sib), true (ou=users has sib)]
        //   └─ ou=groups (depth=1, is_last=true)
        let root = branch(
            "dc=example,dc=com",
            vec![
                branch(
                    "ou=users,dc=example,dc=com",
                    vec![leaf("cn=user1,ou=users,dc=example,dc=com")],
                ),
                leaf("ou=groups,dc=example,dc=com"),
            ],
        );
        let flat = flat_view(&root);
        assert_eq!(flat.len(), 4);

        let user1 = &flat[2];
        assert_eq!(user1.node.dn, "cn=user1,ou=users,dc=example,dc=com");
        assert_eq!(user1.depth, 2);
        // continuing[0]=false (root is_last=true → !true=false)
        // continuing[1]=true  (ou=users is_last=false → !false=true → ┆ drawn at depth1)
        assert_eq!(user1.continuing, vec![false, true]);

        let groups = &flat[3];
        assert_eq!(groups.depth, 1);
        assert!(groups.is_last);
        assert_eq!(groups.continuing, vec![false]);
    }

    #[test]
    fn flat_view_collapsed_hides_children() {
        // Children of a collapsed node must not appear in flat_view.
        let root = branch(
            "dc=example,dc=com",
            vec![collapsed(
                "ou=users,dc=example,dc=com",
                vec![leaf("cn=user1,ou=users,dc=example,dc=com")],
            )],
        );
        let flat = flat_view(&root);
        assert_eq!(flat.len(), 2, "collapsed node's children must not appear");
        assert_eq!(flat[1].node.dn, "ou=users,dc=example,dc=com");
    }

    #[test]
    fn flat_view_unloaded_children_not_expanded() {
        // A node with children=None yields no children even when expanded=true (not loaded yet).
        let root = DitNode {
            dn: "dc=example,dc=com".to_string(),
            has_children: true,
            expanded: true,
            children: None, // not loaded
        };
        let flat = flat_view(&root);
        assert_eq!(flat.len(), 1);
    }
}
