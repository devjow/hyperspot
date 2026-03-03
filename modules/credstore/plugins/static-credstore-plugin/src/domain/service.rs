use std::collections::HashMap;

use credstore_sdk::{OwnerId, SecretRef, SecretValue, SharingMode, TenantId};
use modkit_macros::domain_model;

use crate::config::StaticCredStorePluginConfig;

/// Pre-built secret entry for O(1) lookup.
#[domain_model]
pub struct SecretEntry {
    pub value: SecretValue,
    pub owner_id: OwnerId,
    pub sharing: SharingMode,
    pub owner_tenant_id: TenantId,
}

/// Static credstore service.
///
/// Stores secrets in a two-level `HashMap<TenantId, HashMap<SecretRef, SecretEntry>>`
/// built at init from YAML configuration.
#[domain_model]
pub struct Service {
    secrets: HashMap<TenantId, HashMap<SecretRef, SecretEntry>>,
}

impl Service {
    /// Create a service from plugin configuration.
    ///
    /// Validates each secret key via `SecretRef::new` and builds the lookup map.
    ///
    /// # Errors
    ///
    /// Returns an error if any configured key fails `SecretRef` validation.
    pub fn from_config(cfg: &StaticCredStorePluginConfig) -> anyhow::Result<Self> {
        let mut secrets: HashMap<TenantId, HashMap<SecretRef, SecretEntry>> = HashMap::new();

        for entry in &cfg.secrets {
            let key = SecretRef::new(&entry.key)?;
            let secret_entry = SecretEntry {
                value: SecretValue::from(entry.value.as_str()),
                owner_id: entry.owner_id,
                sharing: entry.sharing,
                owner_tenant_id: entry.tenant_id,
            };
            let tenant_map = secrets.entry(entry.tenant_id).or_default();
            if tenant_map.contains_key(&key) {
                anyhow::bail!(
                    "duplicate secret key '{}' for tenant {}",
                    entry.key,
                    entry.tenant_id
                );
            }
            tenant_map.insert(key, secret_entry);
        }

        Ok(Self { secrets })
    }

    /// Look up a secret by tenant ID and key.
    #[must_use]
    pub fn get(&self, tenant_id: TenantId, key: &SecretRef) -> Option<&SecretEntry> {
        self.secrets.get(&tenant_id)?.get(key)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::config::SecretConfig;
    use uuid::Uuid;

    fn tenant_a() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
    }

    fn tenant_b() -> Uuid {
        Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
    }

    fn owner() -> Uuid {
        Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap()
    }

    fn cfg_with_single_secret() -> StaticCredStorePluginConfig {
        StaticCredStorePluginConfig {
            secrets: vec![SecretConfig {
                tenant_id: tenant_a(),
                owner_id: owner(),
                key: "openai_api_key".to_owned(),
                value: "sk-test-123".to_owned(),
                sharing: SharingMode::Tenant,
            }],
            ..StaticCredStorePluginConfig::default()
        }
    }

    #[test]
    fn from_config_rejects_invalid_secret_ref() {
        let cfg = StaticCredStorePluginConfig {
            secrets: vec![SecretConfig {
                tenant_id: tenant_a(),
                owner_id: owner(),
                key: "invalid:key".to_owned(),
                value: "value".to_owned(),
                sharing: SharingMode::Tenant,
            }],
            ..StaticCredStorePluginConfig::default()
        };

        let result = Service::from_config(&cfg);
        assert!(result.is_err());
    }

    #[test]
    fn get_returns_secret_for_matching_tenant_and_key() {
        let service = Service::from_config(&cfg_with_single_secret()).unwrap();
        let key = SecretRef::new("openai_api_key").unwrap();

        let entry = service.get(tenant_a(), &key);
        assert!(entry.is_some());

        let entry = entry.unwrap();
        assert_eq!(entry.value.as_bytes(), b"sk-test-123");
        assert_eq!(entry.owner_id, owner());
        assert_eq!(entry.owner_tenant_id, tenant_a());
        assert_eq!(entry.sharing, SharingMode::Tenant);
    }

    #[test]
    fn get_returns_none_for_different_tenant() {
        let service = Service::from_config(&cfg_with_single_secret()).unwrap();
        let key = SecretRef::new("openai_api_key").unwrap();

        let entry = service.get(tenant_b(), &key);
        assert!(entry.is_none());
    }

    #[test]
    fn get_returns_none_for_missing_key() {
        let service = Service::from_config(&cfg_with_single_secret()).unwrap();
        let key = SecretRef::new("missing").unwrap();

        let entry = service.get(tenant_a(), &key);
        assert!(entry.is_none());
    }

    #[test]
    fn from_config_rejects_duplicate_key_for_same_tenant() {
        let secret = SecretConfig {
            tenant_id: tenant_a(),
            owner_id: owner(),
            key: "openai_api_key".to_owned(),
            value: "sk-first".to_owned(),
            sharing: SharingMode::Tenant,
        };
        let cfg = StaticCredStorePluginConfig {
            secrets: vec![
                secret.clone(),
                SecretConfig {
                    value: "sk-second".to_owned(),
                    ..secret
                },
            ],
            ..StaticCredStorePluginConfig::default()
        };

        match Service::from_config(&cfg) {
            Ok(_) => panic!("expected error for duplicate key"),
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("duplicate"), "expected 'duplicate' in: {msg}");
                assert!(
                    msg.contains("openai_api_key"),
                    "expected key name in: {msg}"
                );
                assert!(
                    msg.contains(&tenant_a().to_string()),
                    "expected tenant id in: {msg}"
                );
            }
        }
    }

    #[test]
    fn from_config_with_empty_secrets_initializes() {
        let cfg = StaticCredStorePluginConfig::default();
        let service = Service::from_config(&cfg);
        assert!(service.is_ok());
    }
}
