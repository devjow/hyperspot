use async_trait::async_trait;
use credstore_sdk::{
    CredStoreError, CredStorePluginClientV1, SecretMetadata, SecretRef, SecretValue,
};
use modkit_security::SecurityContext;

use super::service::Service;

#[async_trait]
impl CredStorePluginClientV1 for Service {
    async fn get(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
    ) -> Result<Option<SecretMetadata>, CredStoreError> {
        let tenant_id = ctx.subject_tenant_id();

        let Some(entry) = self.get(tenant_id, key) else {
            return Ok(None);
        };

        Ok(Some(SecretMetadata {
            value: SecretValue::new(entry.value.as_bytes().to_vec()),
            owner_id: entry.owner_id,
            sharing: entry.sharing,
            owner_tenant_id: entry.owner_tenant_id,
        }))
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::config::{SecretConfig, StaticCredStorePluginConfig};
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

    fn ctx_for_tenant(tenant_id: Uuid) -> SecurityContext {
        SecurityContext::builder()
            .subject_id(owner())
            .subject_tenant_id(tenant_id)
            .build()
            .unwrap()
    }

    fn service_with_single_secret() -> Service {
        let cfg = StaticCredStorePluginConfig {
            secrets: vec![SecretConfig {
                tenant_id: tenant_a(),
                owner_id: owner(),
                key: "openai_api_key".to_owned(),
                value: "sk-test-123".to_owned(),
                sharing: credstore_sdk::SharingMode::Tenant,
            }],
            ..StaticCredStorePluginConfig::default()
        };

        Service::from_config(&cfg).unwrap()
    }

    #[tokio::test]
    async fn get_returns_metadata_for_matching_tenant_and_key() {
        let service = service_with_single_secret();
        let plugin: &dyn CredStorePluginClientV1 = &service;
        let key = SecretRef::new("openai_api_key").unwrap();

        let result = plugin.get(&ctx_for_tenant(tenant_a()), &key).await;
        assert!(result.is_ok());

        let metadata = result.unwrap();
        assert!(metadata.is_some());
        let metadata = metadata.unwrap();
        assert_eq!(metadata.value.as_bytes(), b"sk-test-123");
        assert_eq!(metadata.owner_id, owner());
        assert_eq!(metadata.owner_tenant_id, tenant_a());
    }

    #[tokio::test]
    async fn get_returns_none_for_other_tenant() {
        let service = service_with_single_secret();
        let plugin: &dyn CredStorePluginClientV1 = &service;
        let key = SecretRef::new("openai_api_key").unwrap();

        let result = plugin.get(&ctx_for_tenant(tenant_b()), &key).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_returns_none_for_missing_key() {
        let service = service_with_single_secret();
        let plugin: &dyn CredStorePluginClientV1 = &service;
        let key = SecretRef::new("missing").unwrap();

        let result = plugin.get(&ctx_for_tenant(tenant_a()), &key).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_returns_none_when_no_secrets_configured() {
        let service = Service::from_config(&StaticCredStorePluginConfig::default()).unwrap();
        let plugin: &dyn CredStorePluginClientV1 = &service;
        let key = SecretRef::new("openai_api_key").unwrap();

        let result = plugin.get(&ctx_for_tenant(tenant_a()), &key).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
