mod actions;
mod state;
mod tree;

use std::collections::VecDeque;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rust_i18n::t;
use zeroize::Zeroizing;

use crate::ldap::{ChildNode, LdapClient};
use crate::model::{DitNode, EditState, LdapEntry, Pane};
use crate::ui::detail::DetailView;

pub use state::{
    CreatePhase, FormFocus, FormState, Mode, OcDeleteState, PickerState, SearchState,
    SuffixSelection,
};
pub use tree::Tree;

// ── App ──────────────────────────────────────────────────────────────────────

pub struct App {
    pub ldap: Option<LdapClient>,
    pub schema_cache: Option<crate::schema::SchemaCache>,
    pub current_entry: Option<LdapEntry>,
    pub detail_view: DetailView,
    /// DIT tree (root + cursor + base_dn). None while in SelectSuffix mode.
    pub browse: Option<Tree>,
    pub mode: Mode,
    pub active_pane: Pane,
    pub search: SearchState,
    /// Queue of MUST attributes that need values after adding an objectClass (persists across multiple AddValue rounds).
    pub pending_must_queue: VecDeque<String>,
    pub status: String,
}

impl App {
    pub async fn init(
        uri: &str,
        bind_dn: Option<&str>,
        password: Option<&Zeroizing<String>>,
        base_dn_override: Option<&str>,
    ) -> Self {
        match Self::try_init(uri, bind_dn, password, base_dn_override).await {
            Ok(app) => app,
            Err(e) => Self::error_state(uri, e),
        }
    }

    fn empty(status: String) -> Self {
        Self {
            ldap: None,
            schema_cache: None,
            current_entry: None,
            detail_view: DetailView::default(),
            browse: None,
            mode: Mode::Browse,
            active_pane: Pane::Tree,
            search: SearchState::new(),
            pending_must_queue: VecDeque::new(),
            status,
        }
    }

    fn error_state(uri: &str, e: anyhow::Error) -> Self {
        let mut app = Self::empty(t!("status.connection_failed", uri = uri).to_string());
        // On error, show a single-node placeholder Tree.
        let err_node = DitNode {
            dn: t!("status.error_prefix", error = e.to_string()).to_string(),
            has_children: false,
            expanded: false,
            children: Some(vec![]),
        };
        app.browse = Some(Tree::from_root(err_node, String::new()));
        app
    }

    async fn try_init(
        uri: &str,
        bind_dn: Option<&str>,
        password: Option<&Zeroizing<String>>,
        base_dn_override: Option<&str>,
    ) -> anyhow::Result<Self> {
        let mut client = LdapClient::connect(uri, bind_dn, password).await?;

        let all_contexts = client.all_naming_contexts().await?;

        // -b given, or single context → initialize directly.
        // Multiple contexts without -b → start in SelectSuffix mode.
        let (base_dn, need_select, candidates) = if let Some(dn) = base_dn_override {
            (dn.to_string(), false, vec![])
        } else if all_contexts.len() == 1 {
            (all_contexts.into_iter().next().unwrap(), false, vec![])
        } else {
            (String::new(), true, all_contexts)
        };

        if need_select {
            let mut app = Self::empty(t!("status.connected_select_context", uri = uri).to_string());
            app.ldap = Some(client);
            app.mode = Mode::SelectSuffix(SuffixSelection::new(candidates));
            return Ok(app);
        }

        let children = client.children(&base_dn).await?;

        let mut app = Self::empty(t!("status.connected", uri = uri).to_string());
        app.ldap = Some(client);
        app.browse = Some(Tree::from_children(base_dn, children));

        app.fetch_selected_entry().await;
        app.load_schema().await;
        Ok(app)
    }

    /// Initialize the tree using the suffix selected in SelectSuffix mode.
    pub async fn select_suffix(&mut self) {
        let dn = match &self.mode {
            Mode::SelectSuffix(sel) => match sel.selected() {
                Some(d) => d.clone(),
                None => return,
            },
            _ => return,
        };

        let Some(result) = self.ldap_children(&dn).await else {
            return;
        };
        match result {
            Ok(children) => {
                self.browse = Some(Tree::from_children(dn, children));
                self.mode = Mode::Browse;
                self.fetch_selected_entry().await;
                self.load_schema().await;
            }
            Err(e) => {
                self.status = t!("status.error_prefix", error = e.to_string()).to_string();
            }
        }
    }

    pub async fn load_schema(&mut self) {
        let Some(result) = self.ldap_fetch_subschema().await else {
            return;
        };
        match result {
            Ok(cache) => {
                self.status = t!(
                    "status.schema_summary",
                    prev = self.status.clone(),
                    attrs = cache.attr_count(),
                    ocs = cache.oc_count(),
                )
                .to_string();
                self.schema_cache = Some(cache);
            }
            Err(e) => {
                self.status = t!(
                    "status.schema_error",
                    prev = self.status.clone(),
                    error = e.to_string(),
                )
                .to_string();
            }
        }
    }

    /// Replace Mode with Browse and return the previous Mode (used to extract its payload).
    pub(super) fn take_mode(&mut self) -> Mode {
        std::mem::replace(&mut self.mode, Mode::Browse)
    }

    /// End the LDAP session (called when the event loop terminates).
    pub async fn unbind(&mut self) {
        if let Some(client) = self.ldap.take() {
            client.unbind().await;
        }
    }

    // ── LDAP call helpers ────────────────────────────────────────────────────
    // Common pattern: return None if ldap is None; the caller exits via `else { return }`.
    // Some(Ok)  → success / Some(Err) → LDAP error

    pub(super) async fn ldap_modify(
        &mut self,
        dn: &str,
        mods: Vec<ldap3::Mod<String>>,
    ) -> Option<anyhow::Result<()>> {
        Some(self.ldap.as_mut()?.modify(dn, mods).await)
    }

    pub(super) async fn ldap_delete(&mut self, dn: &str) -> Option<anyhow::Result<()>> {
        Some(self.ldap.as_mut()?.delete(dn).await)
    }

    pub(super) async fn ldap_add(
        &mut self,
        dn: &str,
        attrs: Vec<(String, std::collections::HashSet<String>)>,
    ) -> Option<anyhow::Result<()>> {
        Some(self.ldap.as_mut()?.add(dn, attrs).await)
    }

    pub(super) async fn ldap_children(
        &mut self,
        dn: &str,
    ) -> Option<anyhow::Result<Vec<ChildNode>>> {
        Some(self.ldap.as_mut()?.children(dn).await)
    }

    pub(super) async fn ldap_search(
        &mut self,
        base: &str,
        filter: &str,
    ) -> Option<anyhow::Result<Vec<String>>> {
        Some(self.ldap.as_mut()?.search(base, filter).await)
    }

    pub(super) async fn ldap_fetch_entry(&mut self, dn: &str) -> Option<anyhow::Result<LdapEntry>> {
        Some(self.ldap.as_mut()?.fetch_entry(dn).await)
    }

    pub(super) async fn ldap_fetch_subschema(
        &mut self,
    ) -> Option<anyhow::Result<crate::schema::SchemaCache>> {
        Some(self.ldap.as_mut()?.fetch_subschema().await)
    }

    // ── Key handlers ─────────────────────────────────────────────────────────

    pub async fn handle_key(&mut self, key: KeyEvent) -> bool {
        // Input modes (text input takes priority).
        match &self.mode {
            Mode::AddAttrName(_) => {
                self.handle_add_attr_name(key).await;
                return true;
            }
            Mode::EditValue(_) | Mode::AddValue(_) => {
                self.handle_edit_input(key).await;
                return true;
            }
            Mode::SearchInput => {
                self.handle_search_input(key).await;
                return true;
            }
            Mode::SelectSuffix(_) => {
                return self.handle_select_suffix(key).await;
            }
            Mode::Picker(_) | Mode::OcPicker(_) => {
                self.handle_picker(key).await;
                return true;
            }
            Mode::ConfirmExpand(_) => {
                self.handle_confirm_expand(key);
                return true;
            }
            Mode::ConfirmOcDelete(_) => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => self.apply_oc_delete().await,
                    _ => {
                        self.mode = Mode::Browse;
                    }
                }
                return true;
            }
            Mode::ConfirmDelete(_) => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => self.apply_delete().await,
                    _ => {
                        self.mode = Mode::Browse;
                    }
                }
                return true;
            }
            Mode::ConfirmEntryDelete(_) => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => self.apply_entry_delete().await,
                    _ => {
                        self.mode = Mode::Browse;
                    }
                }
                return true;
            }
            Mode::CreateChild(_) => {
                self.handle_create_child(key).await;
                return true;
            }
            Mode::ActionDialog => {
                match key.code {
                    KeyCode::Char('e') => self.enter_edit(),
                    KeyCode::Char('a') => self.enter_add(),
                    KeyCode::Char('d') => self.enter_delete(),
                    _ => {
                        self.mode = Mode::Browse;
                    }
                }
                return true;
            }
            Mode::Browse | Mode::SearchResults => {}
        }

        self.handle_browse_key(key).await
    }

    async fn handle_browse_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => return false,
            KeyCode::Tab => self.toggle_pane(),

            // Ctrl+r / F5: force-reload the current node (pick up external changes).
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
                self.current_entry = None;
                self.fetch_selected_entry().await;
            }
            KeyCode::F(5) => {
                self.current_entry = None;
                self.fetch_selected_entry().await;
            }

            KeyCode::Char('/') => {
                self.mode = Mode::SearchInput;
                self.search.input.end();
            }
            KeyCode::Esc if matches!(self.mode, Mode::SearchResults) => {
                self.mode = Mode::Browse;
                self.search.reset_input();
                self.fetch_selected_entry().await;
            }

            KeyCode::Up | KeyCode::Char('k') => match self.active_pane {
                Pane::Tree => match &self.mode {
                    Mode::Browse => {
                        self.tree_up();
                        self.fetch_selected_entry().await;
                    }
                    Mode::SearchResults => {
                        self.search.up();
                        self.fetch_search_selected_entry().await;
                    }
                    _ => {}
                },
                Pane::Detail => {
                    if let Some(e) = self.current_entry.as_ref() {
                        self.detail_view.row_up(e);
                    }
                }
            },
            KeyCode::Down | KeyCode::Char('j') => match self.active_pane {
                Pane::Tree => match &self.mode {
                    Mode::Browse => {
                        self.tree_down();
                        self.fetch_selected_entry().await;
                    }
                    Mode::SearchResults => {
                        self.search.down();
                        self.fetch_search_selected_entry().await;
                    }
                    _ => {}
                },
                Pane::Detail => {
                    if let Some(e) = self.current_entry.as_ref() {
                        self.detail_view.row_down(e);
                    }
                }
            },

            KeyCode::Left | KeyCode::Char('h') => match self.active_pane {
                Pane::Tree if matches!(self.mode, Mode::Browse) => {
                    self.collapse_or_parent();
                    self.fetch_selected_entry().await;
                }
                Pane::Detail => self.detail_view.col_left(),
                _ => {}
            },
            KeyCode::Right | KeyCode::Char('l') => match self.active_pane {
                Pane::Tree if matches!(self.mode, Mode::Browse) => {
                    self.expand_selected().await;
                }
                Pane::Detail => self.detail_view.col_right(),
                _ => {}
            },

            KeyCode::Enter if self.active_pane == Pane::Detail => self.enter_action_dialog(),
            KeyCode::Char('e') if self.active_pane == Pane::Detail => {
                if let Some(e) = &self.current_entry {
                    if !self.detail_view.is_attr_plus_row(e) {
                        self.enter_edit();
                    }
                }
            }
            KeyCode::Char('a') if self.active_pane == Pane::Detail => {
                if let Some(e) = &self.current_entry {
                    if self.detail_view.is_attr_plus_row(e) {
                        self.enter_add_attr_name();
                    } else if self.detail_view.is_oc_plus_row(e) {
                        self.open_oc_picker();
                    } else {
                        self.enter_add();
                    }
                }
            }
            KeyCode::Char('d') if self.active_pane == Pane::Detail => {
                if let Some(e) = &self.current_entry {
                    if !self.detail_view.is_attr_plus_row(e) {
                        self.enter_delete();
                    }
                }
            }
            // In the tree pane, `d` enters delete-confirmation mode for the selected entry.
            KeyCode::Char('d')
                if self.active_pane == Pane::Tree && matches!(self.mode, Mode::Browse) =>
            {
                self.enter_tree_delete();
            }
            // In the tree pane, `a` creates a new entry directly under the selected entry.
            KeyCode::Char('a')
                if self.active_pane == Pane::Tree && matches!(self.mode, Mode::Browse) =>
            {
                self.enter_create_child();
            }

            _ => {}
        }
        true
    }

    // ── Key handler subroutines ──────────────────────────────────────────────

    async fn handle_select_suffix(&mut self, key: KeyEvent) -> bool {
        let Mode::SelectSuffix(sel) = &mut self.mode else {
            return true;
        };
        match key.code {
            KeyCode::Char('q') => return false,
            KeyCode::Up | KeyCode::Char('k') => sel.up(),
            KeyCode::Down | KeyCode::Char('j') => sel.down(),
            KeyCode::Enter => {
                self.select_suffix().await;
            }
            _ => {}
        }
        true
    }

    async fn handle_picker(&mut self, key: KeyEvent) {
        let is_oc = matches!(self.mode, Mode::OcPicker(_));
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => {
                if is_oc {
                    self.apply_oc_picker().await;
                } else {
                    self.apply_picker();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(p) = self.picker_mut() {
                    p.up();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(p) = self.picker_mut() {
                    p.down();
                }
            }
            KeyCode::Backspace => {
                if let Some(p) = self.picker_mut() {
                    p.backspace();
                }
            }
            KeyCode::Char(c)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                if let Some(p) = self.picker_mut() {
                    p.insert_char(c);
                }
            }
            _ => {}
        }
    }

    fn picker_mut(&mut self) -> Option<&mut PickerState> {
        match &mut self.mode {
            Mode::Picker(p) | Mode::OcPicker(p) => Some(p),
            _ => None,
        }
    }

    fn handle_confirm_expand(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let mode = self.take_mode();
                if let Mode::ConfirmExpand(p) = mode {
                    self.apply_expansion(&p.dn, p.children);
                }
            }
            _ => {
                self.mode = Mode::Browse;
                self.status.clear();
            }
        }
    }

    async fn handle_add_attr_name(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => {
                let attr = match &self.mode {
                    Mode::AddAttrName(es) => es.input.text().trim().to_string(),
                    _ => String::new(),
                };
                if !attr.is_empty() {
                    self.mode = Mode::AddValue(EditState::add(attr));
                }
            }
            _ => {
                if let Some(es) = self.mode.edit_state_mut() {
                    es.input.handle_key(key);
                }
            }
        }
    }

    async fn handle_edit_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                self.pending_must_queue.clear(); // discard the queue on cancel
            }
            KeyCode::Enter => {
                let is_edit = matches!(self.mode, Mode::EditValue(_));
                if is_edit {
                    self.apply_edit().await;
                } else {
                    self.apply_add().await;
                }
                // Open the next entry if the MUST attribute queue is non-empty.
                if matches!(self.mode, Mode::Browse) && !self.pending_must_queue.is_empty() {
                    self.open_next_must_attr();
                }
            }
            _ => {
                if let Some(es) = self.mode.edit_state_mut() {
                    es.input.handle_key(key);
                }
            }
        }
    }

    async fn handle_search_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => {
                self.execute_search().await;
            }
            _ => {
                self.search.input.handle_key(key);
            }
        }
    }
}
