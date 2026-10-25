//! Admin authentication (ADR 0027): who a presented secret is, and what that principal may do on `/admin/v1`.
//!
//! This is deliberately separate from the client [`KeyStore`](crate::auth::KeyStore). It reads only the admin
//! credentials of the configuration and never a client key, so a client key can never open the admin API and an
//! admin credential can never serve inference; neither authenticator falls through to the other.

use std::collections::BTreeSet;

use crate::auth::hash_key;
use crate::config::{AdminConfig, AdminScope};

/// A caller the admin API has authenticated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    /// The credential's id.
    pub id: String,
    pub scopes: BTreeSet<AdminScope>,
    /// When the credential stops working, as a UTC timestamp; `None` when it never does.
    pub expires: Option<String>,
}

impl Principal {
    /// Whether the principal holds `scope`.
    #[must_use]
    pub fn may(&self, scope: AdminScope) -> bool {
        self.scopes.contains(&scope)
    }

    /// How the principal is named in audit rows: `admin:<id>`, so an admin id can never be mistaken for a
    /// client key id.
    #[must_use]
    pub fn audit_name(&self) -> String {
        format!("admin:{}", self.id)
    }
}

/// A secret that does not open the admin API. There is one shape for every reason (missing, unknown,
/// expired), so a caller learns nothing about which credentials exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused;

/// Compares two hashes in time that does not depend on where they differ.
fn same(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0_u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// Identifies the principal that presented `secret`, if it is a configured credential that has not expired at
/// `now_ms`. Every credential is compared, so the time taken does not say which one matched.
///
/// # Errors
/// [`Refused`] when no secret was presented, none matches, or the match has expired.
pub fn authenticate(
    admin: &AdminConfig,
    secret: Option<&str>,
    now_ms: i64,
) -> Result<Principal, Refused> {
    let hash = hash_key(secret.ok_or(Refused)?);
    let mut found = None;
    for (id, credential) in &admin.credentials {
        if same(&credential.sha256, &hash) {
            found = Some((id, credential));
        }
    }
    let (id, credential) = found.ok_or(Refused)?;
    if credential.expires_ms.is_some_and(|at| now_ms >= at) {
        return Err(Refused);
    }
    Ok(Principal {
        id: id.clone(),
        scopes: credential.scopes.clone(),
        expires: credential.expires_utc(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AdminCredential;

    fn config(expires_ms: Option<i64>) -> (AdminConfig, String, String) {
        let ((gary, gary_hash), (sam, sam_hash)) = (
            crate::auth::generate_key().unwrap(),
            crate::auth::generate_key().unwrap(),
        );
        let parse = |hash: &str| -> [u8; 32] {
            hex::decode(hash.trim_start_matches("sha256:"))
                .unwrap()
                .try_into()
                .unwrap()
        };
        let admin = AdminConfig {
            listen: None,
            credentials: [
                (
                    "gary".to_string(),
                    AdminCredential {
                        sha256: parse(&gary_hash),
                        scopes: [AdminScope::Read, AdminScope::Overrides].into(),
                        expires_ms,
                    },
                ),
                (
                    "sam".to_string(),
                    AdminCredential {
                        sha256: parse(&sam_hash),
                        scopes: [AdminScope::Read].into(),
                        expires_ms: None,
                    },
                ),
            ]
            .into(),
        };
        (admin, gary, sam)
    }

    #[test]
    fn a_configured_secret_identifies_its_principal() {
        let (admin, gary, sam) = config(None);
        let who = authenticate(&admin, Some(&gary), 0).unwrap();
        assert_eq!(who.id, "gary");
        assert!(who.may(AdminScope::Overrides) && !who.may(AdminScope::Credentials));
        assert_eq!(who.audit_name(), "admin:gary");
        assert_eq!(authenticate(&admin, Some(&sam), 0).unwrap().id, "sam");
    }

    #[test]
    fn every_failure_has_the_same_shape() {
        let (admin, gary, _) = config(Some(1_000));
        assert_eq!(authenticate(&admin, None, 0), Err(Refused));
        assert_eq!(
            authenticate(&admin, Some("not-a-real-credential"), 0),
            Err(Refused)
        );
        assert_eq!(authenticate(&admin, Some(""), 0), Err(Refused));
        // Expired: the very moment of expiry already refuses.
        assert!(authenticate(&admin, Some(&gary), 999).is_ok());
        assert_eq!(authenticate(&admin, Some(&gary), 1_000), Err(Refused));
        assert_eq!(authenticate(&admin, Some(&gary), 5_000), Err(Refused));
    }

    #[test]
    fn a_client_key_is_not_an_admin_credential() {
        // The admin authenticator sees only admin credentials: a secret that is not one of them is refused
        // however valid it is for clients.
        let (admin, _, _) = config(None);
        let (client_key, _) = crate::auth::generate_key().unwrap();
        assert_eq!(authenticate(&admin, Some(&client_key), 0), Err(Refused));
    }

    #[test]
    fn an_empty_admin_section_refuses_everything() {
        let (_, gary, _) = config(None);
        assert_eq!(
            authenticate(&AdminConfig::default(), Some(&gary), 0),
            Err(Refused)
        );
    }
}
