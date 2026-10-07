//! Virtual keys: identification of callers by a hashed bearer key, behind a store trait so a
//! database-backed store can replace the config-backed one without touching callers.

use std::collections::HashMap;

use async_trait::async_trait;
use axum::http::HeaderMap;
use sha2::{Digest, Sha256};

use crate::config::{KeyConfig, Limits, OverBudget};

/// A caller the gateway has authenticated. Everything downstream (budgets, ledger, policy) uses
/// the stable `id`, never the key or its hash.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyRecord {
    pub id: String,
    pub allowed_routes: Option<Vec<String>>,
    pub limits: Limits,
    pub over_budget: OverBudget,
}

impl KeyRecord {
    pub fn may_use(&self, route: &str) -> bool {
        self.allowed_routes
            .as_ref()
            .is_none_or(|allowed| allowed.iter().any(|r| r == route))
    }
}

#[async_trait]
pub trait KeyStore: Send + Sync {
    /// Whether requests must carry a valid key. A store with no keys leaves the gateway open.
    fn enforces_auth(&self) -> bool;

    async fn lookup(&self, hash: &[u8; 32]) -> Option<KeyRecord>;
}

/// Keys declared in the configuration file.
pub struct ConfigKeyStore {
    by_hash: HashMap<[u8; 32], KeyRecord>,
}

impl ConfigKeyStore {
    pub fn new(keys: &std::collections::BTreeMap<String, KeyConfig>) -> Self {
        let by_hash = keys
            .iter()
            .map(|(id, key)| {
                (
                    key.sha256,
                    KeyRecord {
                        id: id.clone(),
                        allowed_routes: key.allowed_routes.clone(),
                        limits: key.limits,
                        over_budget: key.over_budget,
                    },
                )
            })
            .collect();
        Self { by_hash }
    }
}

#[async_trait]
impl KeyStore for ConfigKeyStore {
    fn enforces_auth(&self) -> bool {
        !self.by_hash.is_empty()
    }

    async fn lookup(&self, hash: &[u8; 32]) -> Option<KeyRecord> {
        self.by_hash.get(hash).cloned()
    }
}

pub fn hash_key(key: &str) -> [u8; 32] {
    Sha256::digest(key.as_bytes()).into()
}

/// The key a client presented: `Authorization: Bearer <key>` or `x-api-key: <key>`.
pub fn presented_key(headers: &HeaderMap) -> Option<&str> {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            let (scheme, rest) = v.split_once(' ')?;
            scheme.eq_ignore_ascii_case("bearer").then_some(rest.trim())
        });
    bearer
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()))
        .map(str::trim)
        .filter(|k| !k.is_empty())
}

/// A fresh random key and the config value (`sha256:<hex>`) that authenticates it.
pub fn generate_key() -> Result<(String, String), getrandom::Error> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)?;
    let key = format!("sk-conductor-{}", hex::encode(bytes));
    let hash = format!("sha256:{}", hex::encode(hash_key(&key)));
    Ok((key, hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn key_is_read_from_bearer_or_x_api_key() {
        assert_eq!(
            presented_key(&headers(&[("authorization", "Bearer abc")])),
            Some("abc")
        );
        assert_eq!(
            presented_key(&headers(&[("authorization", "bearer abc")])),
            Some("abc")
        );
        assert_eq!(
            presented_key(&headers(&[("x-api-key", "xyz")])),
            Some("xyz")
        );
        assert_eq!(
            presented_key(&headers(&[("authorization", "Basic abc")])),
            None
        );
        assert_eq!(
            presented_key(&headers(&[("authorization", "Bearer ")])),
            None
        );
        assert_eq!(presented_key(&headers(&[])), None);
    }

    #[test]
    fn generated_key_matches_its_hash_and_is_unique() {
        let (key, hash) = generate_key().unwrap();
        assert!(key.starts_with("sk-conductor-"));
        assert_eq!(hash, format!("sha256:{}", hex::encode(hash_key(&key))));
        assert_ne!(key, generate_key().unwrap().0);
    }

    #[test]
    fn allowlist_checks() {
        let mut record = KeyRecord {
            id: "a".into(),
            allowed_routes: None,
            limits: Limits::default(),
            over_budget: OverBudget::Block,
        };
        assert!(record.may_use("anything"));
        record.allowed_routes = Some(vec!["auto".into()]);
        assert!(record.may_use("auto") && !record.may_use("smart"));
    }

    #[tokio::test]
    async fn config_store_finds_keys_by_hash_only() {
        let (key, hash) = generate_key().unwrap();
        let parsed: [u8; 32] = hex::decode(hash.trim_start_matches("sha256:"))
            .unwrap()
            .try_into()
            .unwrap();
        let keys = [(
            "alice".to_string(),
            KeyConfig {
                sha256: parsed,
                allowed_routes: None,
                over_budget: OverBudget::Block,
                limits: Limits::default(),
            },
        )]
        .into();
        let store = ConfigKeyStore::new(&keys);
        assert!(store.enforces_auth());
        assert_eq!(store.lookup(&hash_key(&key)).await.unwrap().id, "alice");
        assert!(
            store
                .lookup(&hash_key("sk-conductor-wrong"))
                .await
                .is_none()
        );
        assert!(!ConfigKeyStore::new(&Default::default()).enforces_auth());
    }
}
