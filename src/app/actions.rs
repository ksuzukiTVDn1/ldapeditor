use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ldap3::Mod;
use rust_i18n::t;

use crate::model::{EditState, Selection, TextInput};

use super::App;
use super::state::{
    CreateChildState, CreatePhase, EntryDeleteState, FormField, FormFocus, FormState, Mode,
    OcDeleteState, PickerState, RdnPickState, pick_rdn_initial,
};

impl App {
    // ── Entry fetch ──────────────────────────────────────────────────────────

    pub(super) async fn fetch_selected_entry(&mut self) {
        let dn = match self.selected_tree_dn() {
            Some(d) => d,
            None => return,
        };
        self.fetch_entry_by_dn(dn).await;
    }

    pub(super) async fn fetch_search_selected_entry(&mut self) {
        let dn = match self.search.selected_dn() {
            Some(d) => d,
            None => return,
        };
        self.fetch_entry_by_dn(dn).await;
    }

    pub(super) async fn fetch_entry_by_dn(&mut self, dn: String) {
        if self.current_entry.as_ref().map(|e| &e.dn) == Some(&dn) {
            return;
        }
        let Some(result) = self.ldap_fetch_entry(&dn).await else {
            return;
        };
        match result {
            Ok(entry) => {
                self.detail_view.reset();
                self.current_entry = Some(entry);
            }
            Err(e) => {
                self.status = t!("status.fetch_error", error = e.to_string()).to_string();
            }
        }
    }

    // ── LDAP search ──────────────────────────────────────────────────────────

    pub(super) async fn execute_search(&mut self) {
        let filter = self
            .search
            .input
            .text()
            .trim()
            .trim_matches('\'')
            .trim()
            .to_string();
        if filter.is_empty() {
            self.mode = Mode::Browse;
            return;
        }
        let Some(base) = self.browse.as_ref().map(|t| t.base_dn().to_string()) else {
            return;
        };
        let Some(result) = self.ldap_search(&base, &filter).await else {
            return;
        };
        match result {
            Ok(dns) => {
                self.search.set_results(dns);
                self.current_entry = None;
                self.mode = Mode::SearchResults;
                self.fetch_search_selected_entry().await;
            }
            Err(e) => {
                self.status = t!("status.search_error", error = e.to_string()).to_string();
                self.mode = Mode::Browse;
            }
        }
    }

    // ── Attribute edit mode transitions ──────────────────────────────────────

    fn current_selection(&self) -> Option<(String, String)> {
        let e = self.current_entry.as_ref()?;
        match self.detail_view.selected(e)? {
            Selection::ObjectClass { value, .. } => {
                Some(("objectClass".to_string(), value.to_string()))
            }
            Selection::Attr { attr, value } => Some((attr.to_string(), value.to_string())),
            Selection::OcPlusRow | Selection::AttrPlusRow | Selection::OpAttr { .. } => None,
        }
    }

    pub(super) fn enter_action_dialog(&mut self) {
        if let Some(e) = &self.current_entry {
            if self.detail_view.is_attr_plus_row(e) {
                self.enter_add_attr_name();
                return;
            }
            if self.detail_view.is_oc_plus_row(e) {
                self.open_oc_picker();
                return;
            }
            if self.detail_view.is_op_row(e) {
                self.status = t!("status.op_readonly").to_string();
                return;
            }
        }
        if self.current_entry.is_some() {
            self.mode = Mode::ActionDialog;
        }
    }

    pub(super) fn enter_add_attr_name(&mut self) {
        // Prefer the picker when a schema is loaded, otherwise fall back to manual input.
        if self.open_picker() {
            return;
        }
        self.mode = Mode::AddAttrName(EditState::attr_name());
    }

    pub(super) fn enter_edit(&mut self) {
        if let Some(e) = &self.current_entry {
            if self.detail_view.is_oc_row(e) {
                self.status = t!("status.oc_delete_only").to_string();
                return;
            }
            if self.detail_view.is_op_row(e) {
                self.status = t!("status.op_readonly").to_string();
                return;
            }
        }
        let Some((attr, val)) = self.current_selection() else {
            return;
        };
        self.mode = Mode::EditValue(EditState::edit(attr, val));
    }

    pub(super) fn enter_add(&mut self) {
        if let Some(e) = &self.current_entry {
            if self.detail_view.is_oc_row(e) {
                self.status = t!("status.oc_delete_only").to_string();
                return;
            }
            if self.detail_view.is_op_row(e) {
                self.status = t!("status.op_readonly").to_string();
                return;
            }
        }
        let Some((attr, _)) = self.current_selection() else {
            return;
        };
        self.mode = Mode::AddValue(EditState::add(attr));
    }

    pub(super) fn enter_delete(&mut self) {
        if let Some(e) = &self.current_entry {
            if self.detail_view.is_op_row(e) {
                self.status = t!("status.op_readonly").to_string();
                return;
            }
        }

        // OC row: compute orphaned attributes from the schema and open the dedicated dialog.
        let oc_val: Option<String> = self.current_entry.as_ref().and_then(|e| {
            if !self.detail_view.is_oc_row(e) {
                return None;
            }
            match self.detail_view.selected(e) {
                Some(Selection::ObjectClass { value, .. }) => Some(value.to_string()),
                _ => None,
            }
        });

        if let Some(oc_name) = oc_val {
            let orphaned = self.calc_orphaned_attrs(&oc_name);
            self.mode = Mode::ConfirmOcDelete(OcDeleteState { oc_name, orphaned });
            return;
        }

        // Normal attribute deletion.
        let Some((attr, val)) = self.current_selection() else {
            return;
        };
        self.mode = Mode::ConfirmDelete(EditState::delete(attr, val));
    }

    // ── Attribute picker ─────────────────────────────────────────────────────

    /// Open the picker. Returns true when candidates could be generated.
    fn open_picker(&mut self) -> bool {
        let Some(entry) = &self.current_entry else {
            return false;
        };
        let Some(schema) = &self.schema_cache else {
            return false;
        };
        let entries = crate::schema::build_picker_entries(entry, schema);
        if entries.is_empty() {
            return false;
        }
        self.mode = Mode::Picker(PickerState::new(entries));
        true
    }

    pub(super) fn apply_picker(&mut self) {
        let mode = self.take_mode();
        let Mode::Picker(state) = mode else {
            self.mode = mode;
            return;
        };
        let Some(entry) = state.selected_entry() else {
            return;
        };
        let attr = entry.attr_name.clone();
        self.mode = Mode::AddValue(EditState::add(attr));
    }

    // ── Add objectClass ──────────────────────────────────────────────────────

    pub(super) fn open_oc_picker(&mut self) {
        let Some(entry) = &self.current_entry else {
            return;
        };
        let Some(schema) = &self.schema_cache else {
            self.status = t!("status.schema_not_loaded").to_string();
            return;
        };
        let entries = crate::schema::build_oc_picker_entries(entry, schema);
        if entries.is_empty() {
            self.status = t!("status.no_oc_to_add").to_string();
            return;
        }
        self.mode = Mode::OcPicker(PickerState::new(entries));
    }

    pub(super) async fn apply_oc_picker(&mut self) {
        let mode = self.take_mode();
        let Mode::OcPicker(state) = mode else {
            self.mode = mode;
            return;
        };
        let Some(entry) = state.selected_entry() else {
            return;
        };
        let oc_name = entry.attr_name.clone();
        self.apply_oc_add(oc_name).await;
    }

    async fn apply_oc_add(&mut self, oc_name: String) {
        let Some(dn) = self.current_entry.as_ref().map(|e| e.dn.clone()) else {
            return;
        };
        let mods = vec![Mod::Add(
            "objectClass".to_string(),
            one_val(oc_name.clone()),
        )];
        let Some(result) = self.ldap_modify(&dn, mods).await else {
            return;
        };
        match result {
            Err(e) => {
                self.status = t!("status.oc_add_error", error = e.to_string()).to_string();
            }
            Ok(_) => {
                self.status = t!("status.oc_added", name = oc_name).to_string();
                self.current_entry = None;
                self.fetch_entry_by_dn(dn).await;
                self.queue_missing_must_attrs(&oc_name);
                if !self.pending_must_queue.is_empty() {
                    self.open_next_must_attr();
                }
            }
        }
    }

    fn queue_missing_must_attrs(&mut self, oc_name: &str) {
        let Some(entry) = &self.current_entry else {
            return;
        };
        let Some(schema) = &self.schema_cache else {
            return;
        };
        let (must, _) = schema.expanded_attrs(oc_name);
        let existing = entry.existing_attr_names_lower();
        for attr in must {
            if attr != "objectclass" && !existing.contains(&attr) {
                self.pending_must_queue.push_back(attr);
            }
        }
    }

    pub(super) fn open_next_must_attr(&mut self) {
        if let Some(attr) = self.pending_must_queue.pop_front() {
            self.status = t!("status.input_must_attr", attr = attr.clone()).to_string();
            self.mode = Mode::AddValue(EditState::add(attr));
        }
    }

    // ── Delete objectClass ───────────────────────────────────────────────────

    /// Return (attr, value) pairs of attributes governed by the OC being deleted (empty when no schema is loaded).
    fn calc_orphaned_attrs(&self, oc_to_delete: &str) -> Vec<(String, String)> {
        let Some(entry) = &self.current_entry else {
            return vec![];
        };
        let Some(schema) = &self.schema_cache else {
            return vec![];
        };
        orphaned_attrs_for_oc_delete(&entry.attr_rows, &entry.oc_values, oc_to_delete, schema)
    }

    /// Delete the objectClass and its governed attributes in a single modify.
    pub(super) async fn apply_oc_delete(&mut self) {
        let mode = self.take_mode();
        let Mode::ConfirmOcDelete(state) = mode else {
            self.mode = mode;
            return;
        };
        let Some(dn) = self.current_entry.as_ref().map(|e| e.dn.clone()) else {
            return;
        };

        let mut mods: Vec<Mod<String>> = vec![Mod::Delete(
            "objectClass".to_string(),
            one_val(state.oc_name.clone()),
        )];
        // For multi-valued attributes, an empty value set deletes all values.
        let mut seen: HashSet<String> = HashSet::new();
        for (attr, _) in &state.orphaned {
            if seen.insert(attr.to_lowercase()) {
                mods.push(Mod::Delete(attr.clone(), HashSet::new()));
            }
        }

        let Some(result) = self.ldap_modify(&dn, mods).await else {
            return;
        };
        self.status = match result {
            Ok(_) => t!("status.oc_deleted", name = state.oc_name).to_string(),
            Err(e) => t!("status.error_prefix", error = e.to_string()).to_string(),
        };
        self.current_entry = None;
        self.fetch_entry_by_dn(dn).await;
    }

    // ── LDAP Modify ──────────────────────────────────────────────────────────

    pub(super) async fn apply_edit(&mut self) {
        let mode = self.take_mode();
        let Mode::EditValue(es) = mode else {
            self.mode = mode;
            return;
        };
        let new_val = es.input.text().trim().to_string();
        if new_val.is_empty() {
            return;
        }
        let Some(dn) = self.current_entry.as_ref().map(|e| e.dn.clone()) else {
            return;
        };
        let mods = vec![
            Mod::Delete(es.attr.clone(), one_val(es.old_value.clone())),
            Mod::Add(es.attr.clone(), one_val(new_val)),
        ];
        let Some(result) = self.ldap_modify(&dn, mods).await else {
            return;
        };
        self.status = match result {
            Ok(_) => t!("status.modified", attr = es.attr).to_string(),
            Err(e) => t!("status.error_prefix", error = e.to_string()).to_string(),
        };
        self.current_entry = None;
        self.fetch_entry_by_dn(dn).await;
    }

    pub(super) async fn apply_add(&mut self) {
        let mode = self.take_mode();
        let Mode::AddValue(es) = mode else {
            self.mode = mode;
            return;
        };
        let new_val = es.input.text().trim().to_string();
        if new_val.is_empty() {
            return;
        }
        let Some(dn) = self.current_entry.as_ref().map(|e| e.dn.clone()) else {
            return;
        };
        let mods = vec![Mod::Add(es.attr.clone(), one_val(new_val))];
        let Some(result) = self.ldap_modify(&dn, mods).await else {
            return;
        };
        self.status = match result {
            Ok(_) => t!("status.added", attr = es.attr).to_string(),
            Err(e) => t!("status.error_prefix", error = e.to_string()).to_string(),
        };
        self.current_entry = None;
        self.fetch_entry_by_dn(dn).await;
    }

    pub(super) async fn apply_delete(&mut self) {
        let mode = self.take_mode();
        let Mode::ConfirmDelete(es) = mode else {
            self.mode = mode;
            return;
        };
        let Some(dn) = self.current_entry.as_ref().map(|e| e.dn.clone()) else {
            return;
        };
        let mods = vec![Mod::Delete(es.attr.clone(), one_val(es.old_value.clone()))];
        let Some(result) = self.ldap_modify(&dn, mods).await else {
            return;
        };
        self.status = match result {
            Ok(_) => t!("status.deleted", dn = es.attr).to_string(),
            Err(e) => t!("status.error_prefix", error = e.to_string()).to_string(),
        };
        self.current_entry = None;
        self.fetch_entry_by_dn(dn).await;
    }

    // ── Entry deletion ───────────────────────────────────────────────────────

    /// Called when `d` is pressed in the tree pane. Enter the delete-confirmation mode for the selected entry.
    /// The root (base_dn) cannot be deleted; no-op when nothing is selected or no Tree exists.
    pub(super) fn enter_tree_delete(&mut self) {
        let Some(tree) = self.browse.as_ref() else {
            return;
        };
        let Some(info) = tree.selected_info() else {
            return;
        };
        if info.depth == 0 {
            self.status = t!("status.root_undeletable").to_string();
            return;
        }
        self.mode = Mode::ConfirmEntryDelete(EntryDeleteState {
            dn: info.dn,
            has_children: info.has_children,
        });
    }

    // ── Create child entry ───────────────────────────────────────────────────

    /// Called when `a` is pressed in the tree pane. Opens the wizard that creates a new entry directly under the
    /// selected node. Rejected when no schema is loaded.
    pub(super) fn enter_create_child(&mut self) {
        let Some(parent_dn) = self.selected_tree_dn() else {
            return;
        };
        let Some(schema) = &self.schema_cache else {
            self.status = t!("status.schema_required_for_create").to_string();
            return;
        };
        let entries = crate::schema::build_structural_oc_picker_entries(schema);
        if entries.is_empty() {
            self.status = t!("status.no_structural_oc").to_string();
            return;
        }
        self.mode = Mode::CreateChild(CreateChildState {
            parent_dn,
            phase: CreatePhase::PickOc(PickerState::new(entries)),
        });
    }

    /// Handler for Enter in the PickOc phase. Collect the selected OC's MUST/MAY
    /// attributes and transition to the PickRdn phase.
    fn apply_create_child_pick_oc(&mut self) {
        let mode = self.take_mode();
        let Mode::CreateChild(st) = mode else {
            self.mode = mode;
            return;
        };
        let CreateChildState { parent_dn, phase } = st;
        let CreatePhase::PickOc(picker) = phase else {
            self.mode = Mode::CreateChild(CreateChildState { parent_dn, phase });
            return;
        };
        let Some(entry) = picker.selected_entry() else {
            // No candidate selected → cancel without rebuilding the picker.
            self.mode = Mode::Browse;
            return;
        };
        let oc_name = entry.attr_name.clone();

        let Some(schema) = &self.schema_cache else {
            self.status = t!("status.schema_required_for_create").to_string();
            self.mode = Mode::Browse;
            return;
        };

        let rdn_entries = crate::schema::build_rdn_picker_entries(schema, &oc_name);
        if rdn_entries.is_empty() {
            self.status = t!("status.no_must_may_attrs", oc = oc_name).to_string();
            self.mode = Mode::Browse;
            return;
        }

        let oc_chain = schema.oc_sup_chain(&oc_name);
        let mut rdn_picker = PickerState::new(rdn_entries);
        // Align the initial highlight with the conventional RDN attributes (cn/ou/uid/dc) if present.
        // `filtered` initially mirrors all candidates by identity, so the entries-index equals the filtered-index.
        rdn_picker.selected = pick_rdn_initial(&rdn_picker.entries);

        self.mode = Mode::CreateChild(CreateChildState {
            parent_dn,
            phase: CreatePhase::PickRdn(RdnPickState {
                oc_name,
                oc_chain,
                picker: rdn_picker,
            }),
        });
    }

    /// Handler for Enter in the PickRdn phase. Carry the chosen attribute over to the Form phase.
    /// If the RDN is a MAY attribute, add it to the MUST field set so the user must fill it in.
    fn apply_create_child_pick_rdn(&mut self) {
        let mode = self.take_mode();
        let Mode::CreateChild(st) = mode else {
            self.mode = mode;
            return;
        };
        let CreateChildState { parent_dn, phase } = st;
        let CreatePhase::PickRdn(rdn_state) = phase else {
            self.mode = Mode::CreateChild(CreateChildState { parent_dn, phase });
            return;
        };

        let RdnPickState {
            oc_name,
            oc_chain,
            picker,
        } = rdn_state;
        let Some(rdn_entry) = picker.selected_entry() else {
            self.mode = Mode::Browse;
            return;
        };
        let rdn_attr = rdn_entry.attr_name.clone();

        // Form inputs: all MUST attributes + (RDN if it is a MAY attribute).
        let mut field_attrs: Vec<String> = picker
            .entries
            .iter()
            .filter(|e| e.is_must)
            .map(|e| e.attr_name.clone())
            .collect();
        if !field_attrs
            .iter()
            .any(|a| a.eq_ignore_ascii_case(&rdn_attr))
        {
            field_attrs.push(rdn_attr.clone());
        }

        let rdn_idx = field_attrs
            .iter()
            .position(|a| a.eq_ignore_ascii_case(&rdn_attr))
            .unwrap_or(0);

        let fields = field_attrs
            .into_iter()
            .map(|attr| FormField {
                attr,
                input: TextInput::new(),
            })
            .collect();

        self.mode = Mode::CreateChild(CreateChildState {
            parent_dn,
            phase: CreatePhase::Form(FormState {
                oc_name,
                oc_chain,
                fields,
                focus: FormFocus::Field(rdn_idx),
                rdn_idx,
            }),
        });
    }

    /// Called when Submit is pressed in the Form phase. Issues an LDAP add if every MUST has a value
    /// and, on success, re-fetches the parent's children and updates the tree.
    async fn apply_create_child(&mut self) {
        // Extract parameters, then reset mode to Browse.
        let mode = self.take_mode();
        let Mode::CreateChild(st) = mode else {
            self.mode = mode;
            return;
        };
        let CreateChildState { parent_dn, phase } = st;
        let CreatePhase::Form(form) = phase else {
            self.mode = Mode::CreateChild(CreateChildState { parent_dn, phase });
            return;
        };

        // Validation: every field must be non-empty.
        if let Some(empty) = form
            .fields
            .iter()
            .find(|f| f.input.text().trim().is_empty())
        {
            self.status = t!("status.field_empty", field = empty.attr.clone()).to_string();
            self.mode = Mode::CreateChild(CreateChildState {
                parent_dn,
                phase: CreatePhase::Form(form),
            });
            return;
        }

        // Build the DN.
        let rdn_attr = form.fields[form.rdn_idx].attr.clone();
        let rdn_val = form.fields[form.rdn_idx].input.text().trim().to_string();
        let new_dn = format!("{rdn_attr}={rdn_val},{parent_dn}");

        // Build the attribute list.
        let mut attrs: Vec<(String, HashSet<String>)> = Vec::new();
        let mut oc_set: HashSet<String> = HashSet::new();
        for oc in &form.oc_chain {
            oc_set.insert(oc.clone());
        }
        attrs.push(("objectClass".to_string(), oc_set));
        for f in &form.fields {
            attrs.push((f.attr.clone(), one_val(f.input.text().trim().to_string())));
        }

        let Some(result) = self.ldap_add(&new_dn, attrs).await else {
            return;
        };
        match result {
            Err(e) => {
                self.status = t!("status.add_error", error = e.to_string()).to_string();
                // On error, return to the form so the user can edit again.
                self.mode = Mode::CreateChild(CreateChildState {
                    parent_dn,
                    phase: CreatePhase::Form(form),
                });
                return;
            }
            Ok(_) => {
                self.status = t!("status.created", dn = new_dn.clone()).to_string();
            }
        }

        // Re-fetch the parent's children and update the tree.
        let Some(children_result) = self.ldap_children(&parent_dn).await else {
            return;
        };
        if let Ok(children) = children_result {
            // Find the new entry's index among the children and place the cursor on it.
            if let Some(t) = &mut self.browse {
                t.expand_at(&parent_dn, children);
                // Locate the new DN in flat_view and set the cursor.
                let flat = t.flat();
                if let Some((idx, _)) = flat.iter().enumerate().find(|(_, e)| e.node.dn == new_dn) {
                    t.set_cursor(idx);
                }
            }
            self.current_entry = None;
            self.fetch_selected_entry().await;
        }
    }

    /// Handle key input while in CreateChild mode.
    /// Branches by whether PickOc / Form / Submit button / Cancel button has focus.
    pub(super) async fn handle_create_child(&mut self, key: KeyEvent) {
        // Esc always cancels.
        if key.code == KeyCode::Esc {
            self.mode = Mode::Browse;
            self.status = t!("status.cancelled").to_string();
            return;
        }

        // PickOc phase.
        let phase_is_pick = matches!(
            &self.mode,
            Mode::CreateChild(CreateChildState {
                phase: CreatePhase::PickOc(_),
                ..
            })
        );
        if phase_is_pick {
            match key.code {
                KeyCode::Enter => {
                    self.apply_create_child_pick_oc();
                    return;
                }
                _ => {
                    if let Mode::CreateChild(CreateChildState {
                        phase: CreatePhase::PickOc(picker),
                        ..
                    }) = &mut self.mode
                    {
                        match key.code {
                            KeyCode::Up | KeyCode::Char('k')
                                if key.modifiers == KeyModifiers::NONE =>
                            {
                                picker.up()
                            }
                            KeyCode::Down | KeyCode::Char('j')
                                if key.modifiers == KeyModifiers::NONE =>
                            {
                                picker.down()
                            }
                            KeyCode::Backspace => picker.backspace(),
                            KeyCode::Char(c)
                                if key.modifiers == KeyModifiers::NONE
                                    || key.modifiers == KeyModifiers::SHIFT =>
                            {
                                picker.insert_char(c);
                            }
                            _ => {}
                        }
                    }
                    return;
                }
            }
        }

        // PickRdn phase (same picker controls as PickOc).
        let phase_is_rdn = matches!(
            &self.mode,
            Mode::CreateChild(CreateChildState {
                phase: CreatePhase::PickRdn(_),
                ..
            })
        );
        if phase_is_rdn {
            match key.code {
                KeyCode::Enter => {
                    self.apply_create_child_pick_rdn();
                    return;
                }
                _ => {
                    if let Mode::CreateChild(CreateChildState {
                        phase: CreatePhase::PickRdn(state),
                        ..
                    }) = &mut self.mode
                    {
                        match key.code {
                            KeyCode::Up | KeyCode::Char('k')
                                if key.modifiers == KeyModifiers::NONE =>
                            {
                                state.picker.up()
                            }
                            KeyCode::Down | KeyCode::Char('j')
                                if key.modifiers == KeyModifiers::NONE =>
                            {
                                state.picker.down()
                            }
                            KeyCode::Backspace => state.picker.backspace(),
                            KeyCode::Char(c)
                                if key.modifiers == KeyModifiers::NONE
                                    || key.modifiers == KeyModifiers::SHIFT =>
                            {
                                state.picker.insert_char(c);
                            }
                            _ => {}
                        }
                    }
                    return;
                }
            }
        }

        // Form phase.
        let Mode::CreateChild(CreateChildState {
            phase: CreatePhase::Form(form),
            ..
        }) = &mut self.mode
        else {
            return;
        };

        match (key.code, form.focus) {
            // Focus navigation.
            (KeyCode::Tab, _) if key.modifiers == KeyModifiers::NONE => form.focus_next(),
            (KeyCode::BackTab, _) => form.focus_prev(),
            (KeyCode::Down, _) => form.focus_next(),
            (KeyCode::Up, _) => form.focus_prev(),

            // Buttons: confirmed via Enter / Space.
            (KeyCode::Enter, FormFocus::Submit) | (KeyCode::Char(' '), FormFocus::Submit) => {
                self.apply_create_child().await;
            }
            (KeyCode::Enter, FormFocus::Cancel) | (KeyCode::Char(' '), FormFocus::Cancel) => {
                self.mode = Mode::Browse;
                self.status = t!("status.cancelled").to_string();
            }

            // Enter while editing a field moves to the next field (or Submit when on the last field).
            (KeyCode::Enter, FormFocus::Field(_)) => form.focus_next(),

            // Text input (only when a Field has focus).
            _ => {
                if let Some(input) = form.focused_input_mut() {
                    input.handle_key(key);
                }
            }
        }
    }

    /// Execute the entry deletion after confirmation. On success, re-fetch the parent's
    /// children, refresh the tree, and move the cursor to the parent.
    pub(super) async fn apply_entry_delete(&mut self) {
        let mode = self.take_mode();
        let Mode::ConfirmEntryDelete(state) = mode else {
            self.mode = mode;
            return;
        };

        // Record the parent DN and cursor position before the delete (flat_view changes afterwards).
        let parent = self.browse.as_ref().and_then(|t| t.parent_info());

        let Some(result) = self.ldap_delete(&state.dn).await else {
            return;
        };
        match result {
            Err(e) => {
                self.status = t!("status.delete_error", error = e.to_string()).to_string();
                return;
            }
            Ok(_) => {
                self.status = t!("status.deleted", dn = state.dn.clone()).to_string();
            }
        }

        // Re-fetch the parent's children and update the tree.
        let Some((parent_dn, parent_idx)) = parent else {
            return;
        };
        let Some(children_result) = self.ldap_children(&parent_dn).await else {
            return;
        };
        match children_result {
            Ok(children) => {
                if let Some(t) = &mut self.browse {
                    if children.is_empty() {
                        t.mark_no_children_at(&parent_dn);
                    } else {
                        t.expand_at(&parent_dn, children);
                    }
                    t.set_cursor(parent_idx);
                }
                self.current_entry = None;
                self.fetch_selected_entry().await;
            }
            Err(e) => {
                self.status = t!(
                    "status.parent_refetch_failed",
                    prev = self.status.clone(),
                    error = e.to_string(),
                )
                .to_string();
            }
        }
    }
}

// ── Free functions ───────────────────────────────────────────────────────────

/// Compute the attributes that become orphaned when an OC is deleted (testable pure function).
pub(crate) fn orphaned_attrs_for_oc_delete(
    attr_rows: &[(String, String)],
    oc_values: &[String],
    oc_to_delete: &str,
    schema: &crate::schema::SchemaCache,
) -> Vec<(String, String)> {
    let mut supported: HashSet<String> = HashSet::new();
    supported.insert("objectclass".to_string());
    for oc in oc_values {
        if !oc.eq_ignore_ascii_case(oc_to_delete) {
            let (must, may) = schema.expanded_attrs(oc);
            supported.extend(must);
            supported.extend(may);
        }
    }
    attr_rows
        .iter()
        .filter(|(attr, _)| !supported.contains(&attr.to_lowercase()))
        .cloned()
        .collect()
}

fn one_val(v: String) -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert(v);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::SchemaCache;

    fn make_schema() -> SchemaCache {
        let attrs = vec![
            "( 2.5.4.0  NAME 'objectClass' SYNTAX 1.3.6.1.4.1.1466.115.121.1.38 )".to_string(),
            "( 2.5.4.3  NAME 'cn'  SUP name )".to_string(),
            "( 2.5.4.4  NAME 'sn'  SUP name )".to_string(),
            "( 2.5.4.41 NAME 'name' SYNTAX 1.3.6.1.4.1.1466.115.121.1.15 )".to_string(),
            "( 2.5.4.99 NAME 'carLicense' SYNTAX 1.3.6.1.4.1.1466.115.121.1.15 )".to_string(),
            "( 2.5.4.98 NAME 'displayName' SYNTAX 1.3.6.1.4.1.1466.115.121.1.15 )".to_string(),
        ];
        let ocs = vec![
            "( 2.5.6.0 NAME 'top' ABSTRACT MUST objectClass )".to_string(),
            "( 2.5.6.6 NAME 'person' SUP top STRUCTURAL MUST ( sn $ cn ) )".to_string(),
            "( 2.16.840.1.113730.3.2.2 NAME 'inetOrgPerson' SUP person STRUCTURAL MAY ( carLicense $ displayName ) )".to_string(),
        ];
        SchemaCache::from_raw(&attrs, &ocs)
    }

    fn rows(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, v)| (a.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn orphaned_attrs_removes_oc_exclusive_attrs() {
        let schema = make_schema();
        let oc_values = vec!["person".to_string(), "inetOrgPerson".to_string()];
        let attr_rows = rows(&[
            ("cn", "admin"),
            ("sn", "Smith"),
            ("carLicense", "ABC"),
            ("displayName", "Admin"),
        ]);
        let orphaned =
            orphaned_attrs_for_oc_delete(&attr_rows, &oc_values, "inetOrgPerson", &schema);
        let orphaned_names: Vec<&str> = orphaned.iter().map(|(a, _)| a.as_str()).collect();
        assert!(orphaned_names.contains(&"carLicense"));
        assert!(orphaned_names.contains(&"displayName"));
        assert!(!orphaned_names.contains(&"cn"));
        assert!(!orphaned_names.contains(&"sn"));
    }

    #[test]
    fn orphaned_attrs_empty_when_shared_attrs() {
        let schema = make_schema();
        let oc_values = vec!["person".to_string(), "inetOrgPerson".to_string()];
        let attr_rows = rows(&[("cn", "admin"), ("sn", "Smith")]);
        let orphaned =
            orphaned_attrs_for_oc_delete(&attr_rows, &oc_values, "inetOrgPerson", &schema);
        assert!(orphaned.is_empty());
    }

    #[test]
    fn orphaned_attrs_multi_value_all_included() {
        let schema = make_schema();
        let oc_values = vec!["person".to_string(), "inetOrgPerson".to_string()];
        let attr_rows = rows(&[
            ("cn", "admin"),
            ("carLicense", "ABC-111"),
            ("carLicense", "DEF-222"),
        ]);
        let orphaned =
            orphaned_attrs_for_oc_delete(&attr_rows, &oc_values, "inetOrgPerson", &schema);
        assert_eq!(
            orphaned.iter().filter(|(a, _)| a == "carLicense").count(),
            2
        );
    }
}
