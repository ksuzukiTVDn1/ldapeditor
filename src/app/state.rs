use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::widgets::ListState;

use crate::ldap::ChildNode;
use crate::model::{EditState, TextInput};
use crate::schema::PickerEntry;

// ── Mode ─────────────────────────────────────────────────────────────────────

/// The app's main state machine. Each mode carries the state it needs as a payload.
/// Data-less variants only identify the currently shown view; their state is read
/// from persistent fields on App (search / pending_must_queue, etc.).
pub enum Mode {
    /// Normal tree + detail view.
    Browse,
    /// Editing the search filter opened by `/`. The text uses `App.search.input`.
    SearchInput,
    /// Showing the list of search hits. Data lives in `App.search.results / list_state`.
    SearchResults,
    /// Pick one of several namingContexts at startup.
    SelectSuffix(SuffixSelection),
    /// e/a/d overlay shown when pressing `Enter` on a detail-pane row.
    ActionDialog,
    /// Edit an existing attribute value.
    EditValue(EditState),
    /// Add a new attribute value.
    AddValue(EditState),
    /// Typing an attribute name manually (when the picker is not used).
    AddAttrName(EditState),
    /// y/N confirmation for deleting an attribute value.
    ConfirmDelete(EditState),
    /// y/N confirmation when expanding more than 100 children.
    ConfirmExpand(PendingExpansion),
    /// Picker for adding an attribute.
    Picker(PickerState),
    /// Picker for adding an objectClass.
    OcPicker(PickerState),
    /// objectClass deletion confirmation (with the list of orphaned attributes).
    ConfirmOcDelete(OcDeleteState),
    /// y/N confirmation for deleting the entry selected in the tree.
    ConfirmEntryDelete(EntryDeleteState),
    /// Wizard for creating a new entry directly under the tree's selected entry.
    CreateChild(CreateChildState),
}

impl Mode {
    /// Mutable reference to the currently shown EditState (for editing modes).
    pub fn edit_state_mut(&mut self) -> Option<&mut EditState> {
        match self {
            Mode::EditValue(es)
            | Mode::AddValue(es)
            | Mode::AddAttrName(es)
            | Mode::ConfirmDelete(es) => Some(es),
            _ => None,
        }
    }
}

// ── SuffixSelection ───────────────────────────────────────────────────────────

pub struct SuffixSelection {
    pub candidates: Vec<String>,
    pub state: ListState,
}

impl SuffixSelection {
    pub fn new(candidates: Vec<String>) -> Self {
        let mut state = ListState::default();
        if !candidates.is_empty() {
            state.select(Some(0));
        }
        Self { candidates, state }
    }

    pub fn up(&mut self) {
        let i = self
            .state
            .selected()
            .map(|i| i.saturating_sub(1))
            .unwrap_or(0);
        self.state.select(Some(i));
    }

    pub fn down(&mut self) {
        let max = self.candidates.len().saturating_sub(1);
        let i = self.state.selected().map(|i| (i + 1).min(max)).unwrap_or(0);
        self.state.select(Some(i));
    }

    pub fn selected(&self) -> Option<&String> {
        self.candidates.get(self.state.selected()?)
    }
}

// ── PendingExpansion ─────────────────────────────────────────────────────────

/// Holds the already-fetched child list while the user answers the y/N prompt for a large expansion.
pub struct PendingExpansion {
    pub dn: String,
    pub children: Vec<ChildNode>,
}

// ── OcDeleteState ────────────────────────────────────────────────────────────

/// State needed by the objectClass deletion confirmation dialog.
pub struct OcDeleteState {
    pub oc_name: String,
    /// (attr, value) pairs of attributes governed by the OC being deleted (multi-valued attrs become multiple rows).
    pub orphaned: Vec<(String, String)>,
}

// ── EntryDeleteState ─────────────────────────────────────────────────────────

/// State needed by the tree-entry deletion confirmation dialog.
pub struct EntryDeleteState {
    pub dn: String,
    /// Entries with `hasSubordinates=TRUE` are rejected by the LDAP server with
    /// NotAllowedOnNonLeaf. Used to show a warning in the confirmation dialog.
    pub has_children: bool,
}

// ── CreateChildState ─────────────────────────────────────────────────────────

/// State for the "create child entry" wizard triggered by `a` in the tree pane.
/// Walks through the PickOc → Form phases within a single window.
pub struct CreateChildState {
    pub parent_dn: String,
    pub phase: CreatePhase,
}

pub enum CreatePhase {
    /// Pick a STRUCTURAL objectClass (picker).
    PickOc(PickerState),
    /// Pick the attribute used as the new entry's RDN from the MUST list.
    PickRdn(RdnPickState),
    /// Form for entering MUST attribute values.
    Form(FormState),
}

/// State for the RDN-attribute selection phase.
/// Pick one attribute, with fuzzy filtering, from MUST ∪ MAY (excluding objectClass / operational attributes).
/// The `is_must` badge distinguishes MUST and MAY in the UI.
pub struct RdnPickState {
    pub oc_name: String,
    /// List of OC names sent as objectClass values on LDAP add (including the SUP chain).
    /// Retained so it can be passed to the Form phase.
    pub oc_chain: Vec<String>,
    pub picker: PickerState,
}

/// State of the input form. Holds one TextInput per MUST attribute.
pub struct FormState {
    pub oc_name: String,
    /// List of OC names sent as objectClass values on LDAP add (including the SUP chain).
    pub oc_chain: Vec<String>,
    pub fields: Vec<FormField>,
    pub focus: FormFocus,
    /// Index in `fields` of the attribute used as the RDN.
    pub rdn_idx: usize,
}

pub struct FormField {
    pub attr: String,
    pub input: TextInput,
}

#[derive(Clone, Copy, PartialEq)]
pub enum FormFocus {
    Field(usize),
    Submit,
    Cancel,
}

impl FormState {
    /// Move focus to the next slot via Tab / ↓. Cycles field → Submit → Cancel → first field.
    pub fn focus_next(&mut self) {
        self.focus = match self.focus {
            FormFocus::Field(i) if i + 1 < self.fields.len() => FormFocus::Field(i + 1),
            FormFocus::Field(_) => FormFocus::Submit,
            FormFocus::Submit => FormFocus::Cancel,
            FormFocus::Cancel => FormFocus::Field(0),
        };
    }

    /// Move focus to the previous slot via Shift+Tab / ↑.
    pub fn focus_prev(&mut self) {
        self.focus = match self.focus {
            FormFocus::Field(0) => FormFocus::Cancel,
            FormFocus::Field(i) => FormFocus::Field(i - 1),
            FormFocus::Submit => FormFocus::Field(self.fields.len().saturating_sub(1)),
            FormFocus::Cancel => FormFocus::Submit,
        };
    }

    /// Mutable reference to the TextInput currently in focus.
    pub fn focused_input_mut(&mut self) -> Option<&mut TextInput> {
        match self.focus {
            FormFocus::Field(i) => self.fields.get_mut(i).map(|f| &mut f.input),
            _ => None,
        }
    }

    /// DN preview: rdn_attr=rdn_value,parent_dn
    pub fn dn_preview(&self, parent_dn: &str) -> String {
        let rdn_attr = self
            .fields
            .get(self.rdn_idx)
            .map(|f| f.attr.as_str())
            .unwrap_or("");
        let rdn_val = self
            .fields
            .get(self.rdn_idx)
            .map(|f| f.input.text())
            .unwrap_or("");
        if rdn_attr.is_empty() {
            parent_dn.to_string()
        } else if rdn_val.is_empty() {
            format!("{rdn_attr}=<...>,{parent_dn}")
        } else {
            format!("{rdn_attr}={rdn_val},{parent_dn}")
        }
    }
}

/// Choose the initial highlight (`PickerState::selected`) from the RDN picker candidates.
/// Priority: cn → ou → uid → dc → first. Brings the conventional RDN attribute to the top if present.
pub fn pick_rdn_initial(entries: &[crate::schema::PickerEntry]) -> usize {
    for preferred in ["cn", "ou", "uid", "dc"] {
        if let Some(i) = entries
            .iter()
            .position(|e| e.attr_name.eq_ignore_ascii_case(preferred))
        {
            return i;
        }
    }
    0
}

// ── SearchState ──────────────────────────────────────────────────────────────

/// Search-related state shared across SearchInput and SearchResults.
pub struct SearchState {
    pub input: TextInput,
    pub results: Vec<String>,
    pub list_state: ListState,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            input: TextInput::new(),
            results: Vec::new(),
            list_state: ListState::default(),
        }
    }

    pub fn set_results(&mut self, results: Vec<String>) {
        let mut st = ListState::default();
        if !results.is_empty() {
            st.select(Some(0));
        }
        self.results = results;
        self.list_state = st;
    }

    pub fn up(&mut self) {
        let i = self
            .list_state
            .selected()
            .map(|i| i.saturating_sub(1))
            .unwrap_or(0);
        self.list_state.select(Some(i));
    }

    pub fn down(&mut self) {
        let max = self.results.len().saturating_sub(1);
        let i = self
            .list_state
            .selected()
            .map(|i| (i + 1).min(max))
            .unwrap_or(0);
        self.list_state.select(Some(i));
    }

    pub fn selected_dn(&self) -> Option<String> {
        self.results.get(self.list_state.selected()?).cloned()
    }

    pub fn reset_input(&mut self) {
        self.input.clear();
    }
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

// ── PickerState ──────────────────────────────────────────────────────────────

pub struct PickerState {
    pub input: TextInput,
    pub entries: Vec<PickerEntry>,
    pub filtered: Vec<usize>,
    pub selected: usize,
}

impl PickerState {
    pub fn new(entries: Vec<PickerEntry>) -> Self {
        let filtered = (0..entries.len()).collect();
        Self {
            input: TextInput::new(),
            entries,
            filtered,
            selected: 0,
        }
    }

    pub fn update_filter(&mut self) {
        if self.input.is_empty() {
            self.filtered = (0..self.entries.len()).collect();
        } else {
            let matcher = SkimMatcherV2::default();
            let mut scored: Vec<(usize, i64)> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    matcher
                        .fuzzy_match(&e.attr_name, self.input.text())
                        .map(|s| (i, s))
                })
                .collect();
            scored.sort_by_key(|&(_, score)| std::cmp::Reverse(score));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }
        if self.selected >= self.filtered.len().max(1) {
            self.selected = 0;
        }
    }

    pub fn selected_entry(&self) -> Option<&PickerEntry> {
        self.entries.get(*self.filtered.get(self.selected)?)
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
    pub fn down(&mut self) {
        let max = self.filtered.len().saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.input.insert(c);
        self.update_filter();
    }

    pub fn backspace(&mut self) {
        self.input.backspace();
        self.update_filter();
    }
}
