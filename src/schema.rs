//! RFC 4512 subschema fetch, parser, and cache.
//! Independent of ratatui / ldap3.

use std::collections::{HashMap, HashSet};

// ── Public data types ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AttributeType {
    pub oid: String,
    /// NAME aliases (normalized to lowercase).
    pub names: Vec<String>,
    /// SUP (lowercase).
    pub sup: Option<String>,
    /// SYNTAX OID ({len} stripped).
    pub syntax: Option<String>,
    pub single_value: bool,
    pub no_user_mod: bool,
    /// USAGE other than userApplications → operational attribute.
    pub is_operational: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ObjectClass {
    pub oid: String,
    /// NAME aliases (normalized to lowercase).
    pub names: Vec<String>,
    /// SUP (lowercase).
    pub sup: Vec<String>,
    pub kind: OcKind,
    /// MUST attribute names (lowercase).
    pub must: Vec<String>,
    /// MAY attribute names (lowercase).
    pub may: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OcKind {
    Abstract,
    Structural,
    Auxiliary,
}

// ── SchemaCache ──────────────────────────────────────────────────────────────

/// Cache of parsed schema definitions.
/// All keys are normalized to lowercase.
pub struct SchemaCache {
    /// NAME / OID → AttributeType
    attr_map: HashMap<String, AttributeType>,
    /// NAME / OID → ObjectClass
    oc_map: HashMap<String, ObjectClass>,
}

impl SchemaCache {
    /// Build the cache from the raw attributeTypes / objectClasses string lists.
    pub fn from_raw(attr_values: &[String], oc_values: &[String]) -> Self {
        let mut attr_map = HashMap::new();
        let mut oc_map = HashMap::new();

        for v in attr_values {
            if let Some(at) = parse_attribute_type(v) {
                for name in &at.names {
                    attr_map.insert(name.clone(), at.clone());
                }
                attr_map.insert(at.oid.to_lowercase(), at);
            }
        }

        for v in oc_values {
            if let Some(oc) = parse_object_class(v) {
                for name in &oc.names {
                    oc_map.insert(name.clone(), oc.clone());
                }
                oc_map.insert(oc.oid.to_lowercase(), oc);
            }
        }

        Self { attr_map, oc_map }
    }

    pub fn attr_count(&self) -> usize {
        let mut seen = HashSet::new();
        self.attr_map
            .values()
            .filter(|a| seen.insert(a.oid.as_str()))
            .count()
    }

    pub fn oc_count(&self) -> usize {
        let mut seen = HashSet::new();
        self.oc_map
            .values()
            .filter(|o| seen.insert(o.oid.as_str()))
            .count()
    }

    /// Look up an attribute by name (case-insensitive).
    pub fn attr_type(&self, name: &str) -> Option<&AttributeType> {
        self.attr_map.get(&name.to_lowercase())
    }

    /// Look up an objectClass by name (case-insensitive).
    #[allow(dead_code)]
    pub fn object_class(&self, name: &str) -> Option<&ObjectClass> {
        self.oc_map.get(&name.to_lowercase())
    }

    /// Return the MUST / MAY attributes of the given OC, expanded across the SUP chain.
    /// Returned as (must_list, may_list) — both lowercase, sorted, and deduplicated.
    pub fn expanded_attrs(&self, oc_name: &str) -> (Vec<String>, Vec<String>) {
        let mut must = Vec::new();
        let mut may = Vec::new();
        let mut visited = HashSet::new();
        self.collect_attrs(&oc_name.to_lowercase(), &mut must, &mut may, &mut visited);
        must.sort_unstable();
        may.sort_unstable();
        must.dedup();
        may.dedup();
        (must, may)
    }

    fn collect_attrs(
        &self,
        name: &str,
        must: &mut Vec<String>,
        may: &mut Vec<String>,
        visited: &mut HashSet<String>,
    ) {
        if !visited.insert(name.to_string()) {
            return;
        }
        let Some(oc) = self.oc_map.get(name) else {
            return;
        };
        must.extend(oc.must.iter().cloned());
        may.extend(oc.may.iter().cloned());
        for sup in &oc.sup {
            self.collect_attrs(sup, must, may, visited);
        }
    }

    /// Convert a SYNTAX OID to a short display label (used by the M11 picker).
    pub fn syntax_label(oid: &str) -> &'static str {
        match oid {
            "1.3.6.1.4.1.1466.115.121.1.15" => "DirectoryString",
            "1.3.6.1.4.1.1466.115.121.1.26" => "IA5String",
            "1.3.6.1.4.1.1466.115.121.1.7" => "Boolean",
            "1.3.6.1.4.1.1466.115.121.1.27" => "Integer",
            "1.3.6.1.4.1.1466.115.121.1.12" => "DN",
            "1.3.6.1.4.1.1466.115.121.1.36" => "Numeric",
            "1.3.6.1.4.1.1466.115.121.1.38" => "OID",
            "1.3.6.1.4.1.1466.115.121.1.5" => "Binary",
            "1.3.6.1.4.1.1466.115.121.1.8" => "Certificate",
            "1.3.6.1.4.1.1466.115.121.1.28" => "JPEG",
            "1.3.6.1.4.1.1466.115.121.1.40" => "OctetString",
            "1.3.6.1.4.1.1466.115.121.1.50" => "TelephoneNumber",
            "1.3.6.1.4.1.1466.115.121.1.22" => "FacsimileTelephoneNumber",
            "1.3.6.1.4.1.1466.115.121.1.53" => "UTCTime",
            "1.3.6.1.4.1.1466.115.121.1.24" => "GeneralizedTime",
            _ => "OctetString",
        }
    }
}

// ── Tokenizer ────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum Token {
    LParen,
    RParen,
    Word(String),   // OID, keyword, or bare word
    Quoted(String), // 'content'
}

/// Split an RFC 4512 schema description string into tokens.
/// `$` is a list separator (treated as whitespace).
fn tokenize(s: &str) -> Vec<Token> {
    let mut result = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' | '\r' | '$' => {
                chars.next();
            }
            '(' => {
                result.push(Token::LParen);
                chars.next();
            }
            ')' => {
                result.push(Token::RParen);
                chars.next();
            }
            '\'' => {
                chars.next(); // opening quote
                let mut buf = String::new();
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    buf.push(c);
                }
                result.push(Token::Quoted(buf));
            }
            _ => {
                let mut buf = String::new();
                while let Some(&c) = chars.peek() {
                    if matches!(c, '(' | ')' | '\'' | ' ' | '\t' | '\n' | '\r' | '$') {
                        break;
                    }
                    buf.push(c);
                    chars.next();
                }
                result.push(Token::Word(buf));
            }
        }
    }
    result
}

// ── Parser helpers ───────────────────────────────────────────────────────────

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next_token(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// Consume one Word or Quoted token (without lowercasing).
    fn word_raw(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token::Word(w)) => {
                let s = w.clone();
                self.pos += 1;
                Some(s)
            }
            Some(Token::Quoted(q)) => {
                let s = q.clone();
                self.pos += 1;
                Some(s)
            }
            _ => None,
        }
    }

    /// Return one or more names/OIDs, lowercased; the input may be a single token or `( ... )` enclosed.
    fn qdescrs(&mut self) -> Vec<String> {
        match self.peek() {
            Some(Token::LParen) => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    match self.peek() {
                        Some(Token::RParen) => {
                            self.pos += 1;
                            break;
                        }
                        Some(Token::Word(w)) => {
                            let s = w.to_lowercase();
                            self.pos += 1;
                            items.push(s);
                        }
                        Some(Token::Quoted(q)) => {
                            let s = q.to_lowercase();
                            self.pos += 1;
                            items.push(s);
                        }
                        None => break,
                        _ => {
                            self.pos += 1;
                        }
                    }
                }
                items
            }
            Some(Token::Quoted(q)) => {
                let s = q.to_lowercase();
                self.pos += 1;
                vec![s]
            }
            Some(Token::Word(w)) => {
                let s = w.to_lowercase();
                self.pos += 1;
                vec![s]
            }
            _ => vec![],
        }
    }

    /// Skip the remaining tokens until the matching RParen (nesting-aware).
    fn skip_to_rparen(&mut self) {
        let mut depth = 1i32;
        while let Some(t) = self.next_token() {
            match t {
                Token::LParen => depth += 1,
                Token::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
}

// ── attributeTypes parser ────────────────────────────────────────────────────

fn parse_attribute_type(s: &str) -> Option<AttributeType> {
    let tokens = tokenize(s);
    let mut p = Parser {
        tokens: &tokens,
        pos: 0,
    };

    if p.next_token() != Some(&Token::LParen) {
        return None;
    }
    let oid = p.word_raw()?;

    let mut names = Vec::new();
    let mut sup = None;
    let mut syntax = None;
    let mut single_val = false;
    let mut no_user_mod = false;
    let mut is_op = false;

    loop {
        match p.peek() {
            None | Some(Token::RParen) => break,
            Some(Token::Word(w)) => {
                let kw = w.to_uppercase();
                p.pos += 1;
                match kw.as_str() {
                    "NAME" => {
                        names = p.qdescrs();
                    }
                    "DESC" => {
                        p.qdescrs();
                    }
                    "SUP" => {
                        sup = p.word_raw().map(|w| w.to_lowercase());
                    }
                    "SYNTAX" => {
                        syntax = p.word_raw().map(|w| {
                            // Strip the {len} suffix: "1.2.3.4{128}" → "1.2.3.4".
                            w.split('{').next().unwrap_or(&w).to_string()
                        });
                    }
                    "EQUALITY" | "ORDERING" | "SUBSTR" => {
                        p.word_raw();
                    }
                    "SINGLE-VALUE" => {
                        single_val = true;
                    }
                    "NO-USER-MODIFICATION" => {
                        no_user_mod = true;
                    }
                    "COLLECTIVE" | "OBSOLETE" => {}
                    "USAGE" => {
                        if let Some(u) = p.word_raw() {
                            is_op = u.to_lowercase() != "userapplications";
                        }
                    }
                    _ if kw.starts_with("X-") => {
                        p.qdescrs();
                    }
                    _ => {} // ignore unknown keywords
                }
            }
            Some(Token::LParen) => {
                p.pos += 1;
                p.skip_to_rparen();
            }
            _ => {
                p.pos += 1;
            }
        }
    }

    if names.is_empty() {
        names.push(oid.to_lowercase());
    }

    Some(AttributeType {
        oid,
        names,
        sup,
        syntax,
        single_value: single_val,
        no_user_mod,
        is_operational: is_op,
    })
}

// ── objectClasses parser ─────────────────────────────────────────────────────

fn parse_object_class(s: &str) -> Option<ObjectClass> {
    let tokens = tokenize(s);
    let mut p = Parser {
        tokens: &tokens,
        pos: 0,
    };

    if p.next_token() != Some(&Token::LParen) {
        return None;
    }
    let oid = p.word_raw()?;

    let mut names = Vec::new();
    let mut sup = Vec::new();
    let mut kind = OcKind::Structural;
    let mut must = Vec::new();
    let mut may = Vec::new();

    loop {
        match p.peek() {
            None | Some(Token::RParen) => break,
            Some(Token::Word(w)) => {
                let kw = w.to_uppercase();
                p.pos += 1;
                match kw.as_str() {
                    "NAME" => {
                        names = p.qdescrs();
                    }
                    "DESC" => {
                        p.qdescrs();
                    }
                    "SUP" => {
                        sup = p.qdescrs();
                    }
                    "ABSTRACT" => {
                        kind = OcKind::Abstract;
                    }
                    "STRUCTURAL" => {
                        kind = OcKind::Structural;
                    }
                    "AUXILIARY" => {
                        kind = OcKind::Auxiliary;
                    }
                    "MUST" => {
                        must = p.qdescrs();
                    }
                    "MAY" => {
                        may = p.qdescrs();
                    }
                    "OBSOLETE" => {}
                    _ if kw.starts_with("X-") => {
                        p.qdescrs();
                    }
                    _ => {}
                }
            }
            Some(Token::LParen) => {
                p.pos += 1;
                p.skip_to_rparen();
            }
            _ => {
                p.pos += 1;
            }
        }
    }

    if names.is_empty() {
        names.push(oid.to_lowercase());
    }

    Some(ObjectClass {
        oid,
        names,
        sup,
        kind,
        must,
        may,
    })
}

// ── Picker entries ───────────────────────────────────────────────────────────

/// An attribute candidate to show in the picker.
#[derive(Clone)]
pub struct PickerEntry {
    pub attr_name: String, // lowercase
    pub is_must: bool,
    pub syntax: String,     // short display label
    pub single_value: bool, // SINGLE-VALUE constraint
}

/// Build the list of attributes that can be added, based on the current entry's objectClass values.
///
/// Excluded:
/// - operational attributes (is_operational)
/// - existing MUST attributes (already have a value)
/// - existing SINGLE-VALUE attributes (no further values can be added)
/// - objectClass itself (OC operations have their own flow)
pub fn build_picker_entries(
    entry: &crate::model::LdapEntry,
    cache: &SchemaCache,
) -> Vec<PickerEntry> {
    use std::collections::HashSet;

    // Expand and collect MUST / MAY for every objectClass.
    let mut must_all: HashSet<String> = HashSet::new();
    let mut may_all: HashSet<String> = HashSet::new();
    for oc in &entry.oc_values {
        let (m, y) = cache.expanded_attrs(oc);
        must_all.extend(m);
        may_all.extend(y);
    }

    // Existing attribute names (lowercase, deduplicated).
    let existing = entry.existing_attr_names_lower();

    let all_names: HashSet<String> = must_all.union(&may_all).cloned().collect();
    let mut candidates: Vec<PickerEntry> = Vec::new();

    for name in &all_names {
        if name == "objectclass" {
            continue;
        }

        let at = cache.attr_type(name);

        // Exclude operational attributes.
        if at.map(|a| a.is_operational).unwrap_or(false) {
            continue;
        }

        let is_must = must_all.contains(name);

        // An existing MUST does not need to be added again.
        if is_must && existing.contains(name) {
            continue;
        }

        // Skip SINGLE-VALUE attributes that already have a value.
        if at.map(|a| a.single_value).unwrap_or(false) && existing.contains(name) {
            continue;
        }

        let syntax = at
            .and_then(|a| a.syntax.as_deref())
            .map(|oid| SchemaCache::syntax_label(oid).to_string())
            .unwrap_or_default();

        let single_value = at.map(|a| a.single_value).unwrap_or(false);
        candidates.push(PickerEntry {
            attr_name: name.clone(),
            is_must,
            syntax,
            single_value,
        });
    }

    // MUST first; alphabetical within each group.
    candidates.sort_by(|a, b| {
        b.is_must
            .cmp(&a.is_must)
            .then(a.attr_name.cmp(&b.attr_name))
    });
    candidates
}

// ── OC picker entries ────────────────────────────────────────────────────────

impl SchemaCache {
    /// Return a deduplicated list of objectClasses (dedup by OID).
    pub fn unique_object_classes(&self) -> Vec<&ObjectClass> {
        let mut result = Vec::new();
        let mut seen: std::collections::HashSet<&str> = Default::default();
        for oc in self.oc_map.values() {
            if seen.insert(oc.oid.as_str()) {
                result.push(oc);
            }
        }
        result
    }
}

/// Build the candidate objectClasses that can be added to the current entry.
/// ABSTRACT and already-present OCs are excluded.
/// is_must = true → STRUCTURAL (red badge); false → AUXILIARY (green badge).
pub fn build_oc_picker_entries(
    entry: &crate::model::LdapEntry,
    cache: &SchemaCache,
) -> Vec<PickerEntry> {
    use std::collections::HashSet;
    let existing: HashSet<String> = entry.oc_values.iter().map(|o| o.to_lowercase()).collect();

    let mut candidates: Vec<PickerEntry> = cache
        .unique_object_classes()
        .into_iter()
        .filter(|oc| oc.kind != OcKind::Abstract)
        .filter(|oc| !oc.names.iter().any(|n| existing.contains(n.as_str())))
        .map(|oc| PickerEntry {
            attr_name: oc.names.first().cloned().unwrap_or_default(),
            is_must: oc.kind == OcKind::Structural,
            syntax: match oc.kind {
                OcKind::Structural => "STRUCTURAL".to_string(),
                OcKind::Auxiliary => "AUXILIARY".to_string(),
                OcKind::Abstract => "ABSTRACT".to_string(),
            },
            single_value: false,
        })
        .collect();

    // STRUCTURAL first; alphabetical within each group.
    candidates.sort_by(|a, b| {
        b.is_must
            .cmp(&a.is_must)
            .then(a.attr_name.cmp(&b.attr_name))
    });
    candidates
}

/// Build the RDN-attribute candidates for child-entry creation.
/// Takes MUST ∪ MAY for the given OC (including the SUP chain), excluding `objectClass`
/// and operational attributes. MUST entries are tagged `is_must=true` so the UI can distinguish them.
/// MUST first; alphabetical within each group.
pub fn build_rdn_picker_entries(cache: &SchemaCache, oc_name: &str) -> Vec<PickerEntry> {
    let (must, may) = cache.expanded_attrs(oc_name);
    let must_set: HashSet<String> = must.iter().cloned().collect();
    let mut all: HashSet<String> = HashSet::new();
    all.extend(must);
    all.extend(may);

    let mut candidates: Vec<PickerEntry> = Vec::new();
    for name in &all {
        if name == "objectclass" {
            continue;
        }
        let at = cache.attr_type(name);
        if at.map(|a| a.is_operational).unwrap_or(false) {
            continue;
        }

        let is_must = must_set.contains(name);
        let syntax = at
            .and_then(|a| a.syntax.as_deref())
            .map(|oid| SchemaCache::syntax_label(oid).to_string())
            .unwrap_or_default();
        let single_value = at.map(|a| a.single_value).unwrap_or(false);
        candidates.push(PickerEntry {
            attr_name: name.clone(),
            is_must,
            syntax,
            single_value,
        });
    }
    candidates.sort_by(|a, b| {
        b.is_must
            .cmp(&a.is_must)
            .then(a.attr_name.cmp(&b.attr_name))
    });
    candidates
}

/// For child-entry creation: list only STRUCTURAL objectClasses as candidates.
/// ABSTRACT / AUXILIARY are excluded; deduplicated by OID.
pub fn build_structural_oc_picker_entries(cache: &SchemaCache) -> Vec<PickerEntry> {
    let mut candidates: Vec<PickerEntry> = cache
        .unique_object_classes()
        .into_iter()
        .filter(|oc| oc.kind == OcKind::Structural)
        .map(|oc| PickerEntry {
            attr_name: oc.names.first().cloned().unwrap_or_default(),
            is_must: true, // highlight STRUCTURAL
            syntax: "STRUCTURAL".to_string(),
            single_value: false,
        })
        .collect();
    candidates.sort_by(|a, b| a.attr_name.cmp(&b.attr_name));
    candidates
}

impl SchemaCache {
    /// Return the entire SUP chain that contains the given OC (leaf → root order).
    /// Used to build the full objectClass list to send on LDAP add.
    /// The result preserves the original NAME casing and is deduplicated.
    pub fn oc_sup_chain(&self, oc_name: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut visited = HashSet::new();
        self.walk_sup(&oc_name.to_lowercase(), &mut out, &mut visited);
        out
    }

    fn walk_sup(&self, name: &str, out: &mut Vec<String>, visited: &mut HashSet<String>) {
        if !visited.insert(name.to_string()) {
            return;
        }
        if let Some(oc) = self.oc_map.get(name) {
            // Emit the original NAME (the first NAME from the schema).
            if let Some(canon) = oc.names.first() {
                out.push(canon.clone());
            }
            for sup in &oc.sup {
                self.walk_sup(sup, out, visited);
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_attr_single_name() {
        let s = "( 2.5.4.41 NAME 'name' EQUALITY caseIgnoreMatch \
                  SYNTAX 1.3.6.1.4.1.1466.115.121.1.15{32768} )";
        let at = parse_attribute_type(s).unwrap();
        assert_eq!(at.oid, "2.5.4.41");
        assert_eq!(at.names, vec!["name"]);
        assert!(!at.single_value);
        assert!(!at.is_operational);
        assert_eq!(at.syntax.as_deref(), Some("1.3.6.1.4.1.1466.115.121.1.15"));
    }

    #[test]
    fn parse_attr_multi_name() {
        let s = "( 2.5.4.3 NAME ( 'cn' 'commonName' ) SUP name )";
        let at = parse_attribute_type(s).unwrap();
        assert_eq!(at.names, vec!["cn", "commonname"]);
        assert_eq!(at.sup.as_deref(), Some("name"));
    }

    #[test]
    fn parse_attr_operational() {
        let s = "( 2.5.18.1 NAME 'createTimestamp' NO-USER-MODIFICATION \
                  USAGE directoryOperation SYNTAX 1.3.6.1.4.1.1466.115.121.1.24 SINGLE-VALUE )";
        let at = parse_attribute_type(s).unwrap();
        assert!(at.no_user_mod);
        assert!(at.single_value);
        assert!(at.is_operational);
    }

    #[test]
    fn parse_oc_with_must_may() {
        let s = "( 2.5.6.6 NAME 'person' SUP top STRUCTURAL \
                  MUST ( sn $ cn ) \
                  MAY ( userPassword $ telephoneNumber ) )";
        let oc = parse_object_class(s).unwrap();
        assert_eq!(oc.names, vec!["person"]);
        assert_eq!(oc.sup, vec!["top"]);
        assert_eq!(oc.kind, OcKind::Structural);
        assert!(oc.must.contains(&"sn".to_string()));
        assert!(oc.must.contains(&"cn".to_string()));
        assert!(oc.may.contains(&"userpassword".to_string()));
    }

    // ── Parser edge cases ────────────────────────────────────────────────────

    #[test]
    fn parse_attr_name_dollar_separator() {
        // RFC 4512 allows multiple NAME values separated by $.
        let s = "( 2.5.4.3 NAME ( 'cn' $ 'commonName' ) SUP name )";
        let at = parse_attribute_type(s).unwrap();
        assert!(at.names.contains(&"cn".to_string()));
        assert!(at.names.contains(&"commonname".to_string()));
    }

    #[test]
    fn parse_attr_syntax_length_stripped() {
        // The {128} suffix on SYNTAX must be stripped.
        let s = "( 2.5.4.4 NAME 'sn' SYNTAX 1.3.6.1.4.1.1466.115.121.1.15{32768} )";
        let at = parse_attribute_type(s).unwrap();
        assert_eq!(at.syntax.as_deref(), Some("1.3.6.1.4.1.1466.115.121.1.15"));
    }

    #[test]
    fn parse_attr_x_extension_ignored() {
        // OpenLDAP X-* extensions must be ignored without crashing.
        let s = "( 1.2.3.4 NAME 'test' SYNTAX 1.3.6.1.4.1.1466.115.121.1.15 \
                  X-ORDERED 'VALUES' X-NOT-HUMAN-READABLE 'TRUE' )";
        let at = parse_attribute_type(s).unwrap();
        assert_eq!(at.oid, "1.2.3.4");
    }

    #[test]
    fn sup_chain_circular_no_infinite_loop() {
        // Circular SUP definitions must not panic.
        let ocs = vec![
            "( 1.1.1 NAME 'a' SUP b STRUCTURAL )".to_string(),
            "( 1.1.2 NAME 'b' SUP a STRUCTURAL )".to_string(),
        ];
        let cache = SchemaCache::from_raw(&[], &ocs);
        // Verify it does not loop infinitely (the return value is irrelevant).
        let _ = cache.expanded_attrs("a");
    }

    // ── build_picker_entries ─────────────────────────────────────────────────

    fn make_entry(oc_values: Vec<&str>, attr_rows: Vec<(&str, &str)>) -> crate::model::LdapEntry {
        crate::model::LdapEntry {
            dn: "cn=test,dc=example,dc=com".to_string(),
            oc_values: oc_values.into_iter().map(|s| s.to_string()).collect(),
            attr_rows: attr_rows
                .into_iter()
                .map(|(a, v)| (a.to_string(), v.to_string()))
                .collect(),
            op_rows: vec![],
            attr_w: 16,
        }
    }

    fn make_schema_for_picker() -> SchemaCache {
        let attrs = vec![
            "( 2.5.4.0  NAME 'objectClass'   SYNTAX 1.3.6.1.4.1.1466.115.121.1.38 )".to_string(),
            "( 2.5.4.3  NAME ( 'cn' 'commonName' ) SUP name )".to_string(),
            "( 2.5.4.4  NAME 'sn'   SUP name )".to_string(),
            "( 2.5.4.41 NAME 'name' SYNTAX 1.3.6.1.4.1.1466.115.121.1.15 )".to_string(),
            "( 2.5.4.42 NAME 'givenName' SUP name )".to_string(),
            "( 2.5.18.1 NAME 'createTimestamp' SINGLE-VALUE NO-USER-MODIFICATION USAGE directoryOperation SYNTAX 1.3.6.1.4.1.1466.115.121.1.24 )".to_string(),
            "( 2.5.4.99 NAME 'uid' SINGLE-VALUE SYNTAX 1.3.6.1.4.1.1466.115.121.1.26 )".to_string(),
        ];
        let ocs = vec![
            "( 2.5.6.0 NAME 'top' ABSTRACT MUST objectClass )".to_string(),
            "( 2.5.6.6 NAME 'person' SUP top STRUCTURAL MUST ( sn $ cn ) MAY ( givenName ) )"
                .to_string(),
        ];
        SchemaCache::from_raw(&attrs, &ocs)
    }

    #[test]
    fn picker_entries_excludes_existing_must_attrs() {
        // Entry already has cn and sn: MUST but existing, so excluded from candidates.
        let cache = make_schema_for_picker();
        let entry = make_entry(vec!["person"], vec![("cn", "admin"), ("sn", "Smith")]);
        let candidates = build_picker_entries(&entry, &cache);
        let names: Vec<&str> = candidates.iter().map(|e| e.attr_name.as_str()).collect();
        assert!(!names.contains(&"cn"), "existing MUST should be excluded");
        assert!(!names.contains(&"sn"), "existing MUST should be excluded");
    }

    #[test]
    fn picker_entries_includes_unset_must_attrs() {
        // cn is missing → MUST attribute without a value, so it must appear in the candidates.
        let cache = make_schema_for_picker();
        let entry = make_entry(vec!["person"], vec![("sn", "Smith")]); // no cn
        let candidates = build_picker_entries(&entry, &cache);
        let names: Vec<&str> = candidates.iter().map(|e| e.attr_name.as_str()).collect();
        assert!(
            names.contains(&"cn"),
            "unset MUST should be in the candidates"
        );
        let cn_entry = candidates.iter().find(|e| e.attr_name == "cn").unwrap();
        assert!(cn_entry.is_must);
    }

    #[test]
    fn picker_entries_excludes_operational_attrs() {
        let cache = make_schema_for_picker();
        let entry = make_entry(vec!["person"], vec![]);
        let candidates = build_picker_entries(&entry, &cache);
        let names: Vec<&str> = candidates.iter().map(|e| e.attr_name.as_str()).collect();
        assert!(
            !names.contains(&"createtimestamp"),
            "operational attributes should be excluded"
        );
    }

    #[test]
    fn picker_entries_excludes_existing_single_value() {
        // uid is SINGLE-VALUE and already present → must be excluded.
        // Extended schema that adds uid to person's MAY set.
        let attrs = vec![
            "( 2.5.4.0  NAME 'objectClass' SYNTAX 1.3.6.1.4.1.1466.115.121.1.38 )".to_string(),
            "( 2.5.4.3  NAME 'cn'  SUP name )".to_string(),
            "( 2.5.4.4  NAME 'sn'  SUP name )".to_string(),
            "( 2.5.4.41 NAME 'name' SYNTAX 1.3.6.1.4.1.1466.115.121.1.15 )".to_string(),
            "( 2.5.4.99 NAME 'uid' SINGLE-VALUE SYNTAX 1.3.6.1.4.1.1466.115.121.1.26 )".to_string(),
        ];
        let ocs = vec![
            "( 2.5.6.0 NAME 'top' ABSTRACT MUST objectClass )".to_string(),
            "( 2.5.6.6 NAME 'person' SUP top STRUCTURAL MUST ( sn $ cn ) MAY uid )".to_string(),
        ];
        let cache2 = SchemaCache::from_raw(&attrs, &ocs);
        let entry = make_entry(
            vec!["person"],
            vec![("cn", "x"), ("sn", "y"), ("uid", "john")],
        );
        let candidates = build_picker_entries(&entry, &cache2);
        let names: Vec<&str> = candidates.iter().map(|e| e.attr_name.as_str()).collect();
        assert!(
            !names.contains(&"uid"),
            "existing SINGLE-VALUE should be excluded"
        );
    }

    #[test]
    fn sup_chain_expansion() {
        let attrs = vec![
            "( 2.5.4.0 NAME 'objectClass' SYNTAX 1.3.6.1.4.1.1466.115.121.1.38 )".to_string(),
            "( 2.5.4.4 NAME 'sn' SUP name )".to_string(),
            "( 2.5.4.3 NAME ( 'cn' 'commonName' ) SUP name )".to_string(),
        ];
        let ocs = vec![
            "( 2.5.6.0 NAME 'top' ABSTRACT MUST objectClass )".to_string(),
            "( 2.5.6.6 NAME 'person' SUP top STRUCTURAL MUST ( sn $ cn ) MAY userPassword )"
                .to_string(),
            "( 2.5.6.7 NAME 'organizationalPerson' SUP person STRUCTURAL MAY title )".to_string(),
        ];
        let cache = SchemaCache::from_raw(&attrs, &ocs);
        let (must, may) = cache.expanded_attrs("organizationalPerson");
        // MUST: objectClass (from top), sn, cn (from person)
        assert!(must.contains(&"objectclass".to_string()));
        assert!(must.contains(&"sn".to_string()));
        assert!(must.contains(&"cn".to_string()));
        // MAY: userPassword (from person), title (from organizationalPerson)
        assert!(may.contains(&"userpassword".to_string()));
        assert!(may.contains(&"title".to_string()));
    }
}
