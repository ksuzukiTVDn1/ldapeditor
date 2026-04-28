use std::time::Duration;

use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use ldap3::{Ldap, LdapConnAsync, LdapConnSettings, Mod, Scope, SearchEntry};
use zeroize::Zeroizing;

use crate::model::LdapEntry;

// ── ChildNode ─────────────────────────────────────────────────────────────────

pub struct ChildNode {
    pub dn: String,
    pub has_children: bool,
}

// ── LdapClient ────────────────────────────────────────────────────────────────

pub struct LdapClient {
    ldap: Ldap,
}

impl LdapClient {
    /// Connect, bind, and return an LdapClient.
    /// ldapi:// without bind_dn → SASL EXTERNAL; otherwise → Simple Bind.
    pub async fn connect(
        uri: &str,
        bind_dn: Option<&str>,
        password: Option<&Zeroizing<String>>,
    ) -> Result<Self> {
        let settings = LdapConnSettings::new().set_conn_timeout(Duration::from_secs(5));
        let (conn, mut ldap) = LdapConnAsync::with_settings(settings, uri).await?;
        ldap3::drive!(conn);
        if uri.starts_with("ldapi://") && bind_dn.is_none() {
            ldap.sasl_external_bind().await?.success()?;
        } else {
            ldap.simple_bind(
                bind_dn.unwrap_or(""),
                password.map(|p| p.as_str()).unwrap_or(""),
            )
            .await?
            .success()?;
        }
        Ok(Self { ldap })
    }

    /// Return all namingContexts from RootDSE (sorted).
    pub async fn all_naming_contexts(&mut self) -> Result<Vec<String>> {
        let (rs, _) = self
            .ldap
            .search("", Scope::Base, "(objectClass=*)", vec!["namingContexts"])
            .await?
            .success()?;
        let mut contexts = rs
            .into_iter()
            .next()
            .and_then(|raw| SearchEntry::construct(raw).attrs.remove("namingContexts"))
            .unwrap_or_default();
        contexts.sort();
        Ok(contexts)
    }

    /// Return the first namingContexts entry from RootDSE (for single-context use).
    #[allow(dead_code)]
    pub async fn base_dn(&mut self) -> Result<String> {
        let (rs, _) = self
            .ldap
            .search("", Scope::Base, "(objectClass=*)", vec!["namingContexts"])
            .await?
            .success()?;
        rs.into_iter()
            .next()
            .and_then(|raw| {
                SearchEntry::construct(raw)
                    .attrs
                    .remove("namingContexts")
                    .and_then(|v| v.into_iter().next())
            })
            .ok_or_else(|| anyhow::anyhow!("No namingContexts in RootDSE"))
    }

    /// List child entries directly under parent_dn (with hasSubordinates).
    pub async fn children(&mut self, parent_dn: &str) -> Result<Vec<ChildNode>> {
        let (rs, _) = self
            .ldap
            .search(
                parent_dn,
                Scope::OneLevel,
                "(objectClass=*)",
                vec!["hasSubordinates"],
            )
            .await?
            .success()?;
        let mut nodes: Vec<ChildNode> = rs
            .into_iter()
            .map(|raw| {
                let entry = SearchEntry::construct(raw);
                let has_children = entry
                    .attrs
                    .get("hasSubordinates")
                    .and_then(|v| v.first())
                    .map(|v| v.eq_ignore_ascii_case("TRUE"))
                    .unwrap_or(true);
                ChildNode {
                    dn: entry.dn,
                    has_children,
                }
            })
            .collect();
        nodes.sort_by_key(|n| n.dn.to_lowercase());
        Ok(nodes)
    }

    /// Fetch user attributes ("*") + operational attributes ("+") for the given DN and return an LdapEntry.
    pub async fn fetch_entry(&mut self, dn: &str) -> Result<LdapEntry> {
        let (user_rs, _) = self
            .ldap
            .search(dn, Scope::Base, "(objectClass=*)", vec!["*"])
            .await?
            .success()?;
        let (op_rs, _) = self
            .ldap
            .search(dn, Scope::Base, "(objectClass=*)", vec!["+"])
            .await?
            .success()?;
        Ok(parse_entry(dn, user_rs, op_rs))
    }

    /// Run a scope=Subtree search with the given LDAP filter and return the matching DNs.
    pub async fn search(&mut self, base_dn: &str, filter: &str) -> Result<Vec<String>> {
        let (rs, _) = self
            .ldap
            .search(base_dn, Scope::Subtree, filter, vec!["1.1"])
            .await?
            .success()?;
        let mut dns: Vec<String> = rs
            .into_iter()
            .map(|raw| SearchEntry::construct(raw).dn)
            .collect();
        dns.sort_by_key(|d| d.to_lowercase());
        Ok(dns)
    }

    /// Apply Add / Delete / Replace modifications to attribute values.
    pub async fn modify(&mut self, dn: &str, mods: Vec<Mod<String>>) -> Result<()> {
        self.ldap.modify(dn, mods).await?.success()?;
        Ok(())
    }

    /// Delete the entry at the given DN. If the entry has children,
    /// the LDAP server is expected to reject the operation with NotAllowedOnNonLeaf.
    pub async fn delete(&mut self, dn: &str) -> Result<()> {
        self.ldap.delete(dn).await?.success()?;
        Ok(())
    }

    /// Create a new entry via LDAP add. `attrs` is a list of (attribute name, value set) pairs.
    pub async fn add(
        &mut self,
        dn: &str,
        attrs: Vec<(String, std::collections::HashSet<String>)>,
    ) -> Result<()> {
        self.ldap.add(dn, attrs).await?.success()?;
        Ok(())
    }

    /// Unbind and close the connection (consumes self).
    /// Fetch subschemaSubentry, parse it, and return a SchemaCache.
    pub async fn fetch_subschema(&mut self) -> Result<crate::schema::SchemaCache> {
        // Get the subschemaSubentry DN from RootDSE.
        let (rs, _) = self
            .ldap
            .search(
                "",
                Scope::Base,
                "(objectClass=*)",
                vec!["subschemaSubentry"],
            )
            .await?
            .success()?;
        let subschema_dn = rs
            .into_iter()
            .next()
            .and_then(|raw| {
                SearchEntry::construct(raw)
                    .attrs
                    .remove("subschemaSubentry")
                    .and_then(|v| v.into_iter().next())
            })
            .unwrap_or_else(|| "cn=Subschema".to_string());

        // Fetch the subschema entry.
        let (rs, _) = self
            .ldap
            .search(
                &subschema_dn,
                Scope::Base,
                "(objectClass=subschema)",
                vec!["attributeTypes", "objectClasses"],
            )
            .await?
            .success()?;

        let (attr_vals, oc_vals) = rs
            .into_iter()
            .next()
            .map(|raw| {
                let entry = SearchEntry::construct(raw);
                let ats = entry
                    .attrs
                    .get("attributeTypes")
                    .cloned()
                    .unwrap_or_default();
                let ocs = entry
                    .attrs
                    .get("objectClasses")
                    .cloned()
                    .unwrap_or_default();
                (ats, ocs)
            })
            .unwrap_or_default();

        Ok(crate::schema::SchemaCache::from_raw(&attr_vals, &oc_vals))
    }

    pub async fn unbind(mut self) {
        let _ = self.ldap.unbind().await;
    }
}

// ── entry parsing ─────────────────────────────────────────────────────────────

fn parse_entry(
    dn: &str,
    user_rs: Vec<ldap3::ResultEntry>,
    op_rs: Vec<ldap3::ResultEntry>,
) -> LdapEntry {
    let mut oc_values: Vec<String> = Vec::new();
    let mut attr_rows: Vec<(String, String)> = Vec::new();

    for raw in user_rs {
        let mut entry = SearchEntry::construct(raw);
        if let Some(mut ocs) = entry.attrs.remove("objectClass") {
            ocs.sort_by_key(|s| s.to_lowercase());
            oc_values.extend(ocs);
        }
        let mut text: Vec<_> = entry.attrs.into_iter().collect();
        text.sort_by_key(|(k, _)| k.to_lowercase());
        for (attr, values) in text {
            for v in values {
                attr_rows.push((attr.clone(), v));
            }
        }
        let mut bin: Vec<_> = entry.bin_attrs.into_iter().collect();
        bin.sort_by_key(|(k, _)| k.to_lowercase());
        for (attr, values) in bin {
            for v in values {
                attr_rows.push((attr.clone(), B64.encode(&v)));
            }
        }
    }

    let mut op_rows: Vec<(String, String)> = Vec::new();
    for raw in op_rs {
        let entry = SearchEntry::construct(raw);
        let mut op: Vec<_> = entry.attrs.into_iter().collect();
        op.sort_by_key(|(k, _)| k.to_lowercase());
        for (attr, values) in op {
            for v in values {
                op_rows.push((attr.clone(), v));
            }
        }
        let mut op_bin: Vec<_> = entry.bin_attrs.into_iter().collect();
        op_bin.sort_by_key(|(k, _)| k.to_lowercase());
        for (attr, values) in op_bin {
            for v in values {
                op_rows.push((attr.clone(), B64.encode(&v)));
            }
        }
    }

    let attr_w = attr_rows
        .iter()
        .map(|(a, _)| a.len())
        .max()
        .unwrap_or(8)
        .clamp(8, 28);

    LdapEntry {
        dn: dn.to_string(),
        oc_values,
        attr_rows,
        op_rows,
        attr_w,
    }
}
