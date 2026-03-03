use serde::Deserialize;
use uuid::Uuid;

use credstore_sdk::SharingMode;

/// Plugin configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StaticCredStorePluginConfig {
    /// Vendor name for GTS instance registration.
    pub vendor: String,

    /// Plugin priority (lower = higher priority).
    pub priority: i16,

    /// Static secrets served by this plugin.
    pub secrets: Vec<SecretConfig>,
}

impl Default for StaticCredStorePluginConfig {
    fn default() -> Self {
        Self {
            vendor: "hyperspot".to_owned(),
            priority: 100,
            secrets: Vec::new(),
        }
    }
}

/// A single secret entry in the plugin configuration.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretConfig {
    /// Tenant that owns this secret.
    pub tenant_id: Uuid,

    /// Owner (subject) of this secret.
    pub owner_id: Uuid,

    /// Secret reference key (validated as `SecretRef` at init).
    pub key: String,

    /// Secret value (plaintext string, converted to bytes at init).
    pub value: String,

    /// Sharing mode for this secret.
    #[serde(default)]
    pub sharing: SharingMode,
}

impl core::fmt::Debug for SecretConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SecretConfig")
            .field("tenant_id", &self.tenant_id)
            .field("owner_id", &self.owner_id)
            .field("key", &self.key)
            .field("value", &"<redacted>")
            .field("sharing", &self.sharing)
            .finish()
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_are_applied() {
        let yaml = r#"
secrets:
  - tenant_id: "00000000-0000-0000-0000-000000000001"
    owner_id: "00000000-0000-0000-0000-000000000002"
    key: "openai_api_key"
    value: "sk-test-123"
"#;

        let parsed: Result<StaticCredStorePluginConfig, _> = serde_saphyr::from_str(yaml);
        assert!(parsed.is_ok());

        let cfg = match parsed {
            Ok(cfg) => cfg,
            Err(e) => panic!("failed to parse config: {e}"),
        };

        assert_eq!(cfg.vendor, "hyperspot");
        assert_eq!(cfg.priority, 100);
        assert_eq!(cfg.secrets.len(), 1);
        assert_eq!(cfg.secrets[0].sharing, SharingMode::Tenant);
    }

    #[test]
    fn config_rejects_unknown_fields() {
        let yaml = r#"
vendor: "hyperspot"
priority: 100
unexpected: true
"#;

        let parsed: Result<StaticCredStorePluginConfig, _> = serde_saphyr::from_str(yaml);
        assert!(parsed.is_err());
    }

    #[test]
    fn config_allows_empty_secrets() {
        let parsed: Result<StaticCredStorePluginConfig, _> = serde_saphyr::from_str("{}");
        assert!(parsed.is_ok());

        let cfg = match parsed {
            Ok(cfg) => cfg,
            Err(e) => panic!("failed to parse config: {e}"),
        };
        assert!(cfg.secrets.is_empty());
        assert_eq!(cfg.vendor, "hyperspot");
        assert_eq!(cfg.priority, 100);
    }
}
