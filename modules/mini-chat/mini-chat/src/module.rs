use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use authz_resolver_sdk::AuthZResolverClient;
use mini_chat_sdk::{MiniChatAuditPluginSpecV1, MiniChatModelPolicyPluginSpecV1};
use modkit::api::OpenApiRegistry;
use modkit::{DatabaseCapability, Module, ModuleCtx, RestApiCapability};
use oagw_sdk::ServiceGatewayClientV1;
use sea_orm_migration::MigrationTrait;
use tracing::info;
use types_registry_sdk::{RegisterResult, TypesRegistryClient};

use crate::api::rest::routes;
use crate::domain::service::{AppServices as GenericAppServices, Repositories};

pub(crate) type AppServices = GenericAppServices<
    TurnRepository,
    MessageRepository,
    QuotaUsageRepository,
    ReactionRepository,
    ChatRepository,
>;
use crate::infra::db::repo::attachment_repo::AttachmentRepository;
use crate::infra::db::repo::chat_repo::ChatRepository;
use crate::infra::db::repo::message_repo::MessageRepository;
use crate::infra::db::repo::quota_usage_repo::QuotaUsageRepository;
use crate::infra::db::repo::reaction_repo::ReactionRepository;
use crate::infra::db::repo::thread_summary_repo::ThreadSummaryRepository;
use crate::infra::db::repo::turn_repo::TurnRepository;
use crate::infra::db::repo::vector_store_repo::VectorStoreRepository;
use crate::infra::llm::provider_resolver::ProviderResolver;
use crate::infra::model_policy::ModelPolicyGateway;

/// Default URL prefix for all mini-chat REST routes.
pub const DEFAULT_URL_PREFIX: &str = "/mini-chat";

/// The mini-chat module: multi-tenant AI chat with SSE streaming.
#[modkit::module(
    name = "mini-chat",
    deps = ["types-registry", "authz-resolver", "oagw"],
    capabilities = [db, rest],
)]
pub struct MiniChatModule {
    service: OnceLock<Arc<AppServices>>,
    url_prefix: OnceLock<String>,
}

impl Default for MiniChatModule {
    fn default() -> Self {
        Self {
            service: OnceLock::new(),
            url_prefix: OnceLock::new(),
        }
    }
}

#[async_trait]
impl Module for MiniChatModule {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        info!("Initializing {} module", Self::MODULE_NAME);

        let cfg: crate::config::MiniChatConfig = ctx.config_expanded()?;
        cfg.streaming
            .validate()
            .map_err(|e| anyhow::anyhow!("streaming config: {e}"))?;
        cfg.estimation_budgets
            .validate()
            .map_err(|e| anyhow::anyhow!("estimation_budgets config: {e}"))?;
        cfg.quota
            .validate()
            .map_err(|e| anyhow::anyhow!("quota config: {e}"))?;
        cfg.outbox
            .validate()
            .map_err(|e| anyhow::anyhow!("outbox config: {e}"))?;
        for (id, entry) in &cfg.providers {
            entry
                .validate(id)
                .map_err(|e| anyhow::anyhow!("providers config: {e}"))?;
        }

        let vendor = cfg.vendor.trim().to_owned();
        if vendor.is_empty() {
            return Err(anyhow::anyhow!(
                "{}: vendor must be a non-empty string",
                Self::MODULE_NAME
            ));
        }

        let registry = ctx.client_hub().get::<dyn TypesRegistryClient>()?;
        register_plugin_schemas(
            &*registry,
            &[
                (
                    MiniChatModelPolicyPluginSpecV1::gts_schema_with_refs_as_string(),
                    MiniChatModelPolicyPluginSpecV1::gts_schema_id(),
                    "model-policy",
                ),
                (
                    MiniChatAuditPluginSpecV1::gts_schema_with_refs_as_string(),
                    MiniChatAuditPluginSpecV1::gts_schema_id(),
                    "audit",
                ),
            ],
        )
        .await?;

        self.url_prefix
            .set(cfg.url_prefix)
            .map_err(|_| anyhow::anyhow!("{} url_prefix already set", Self::MODULE_NAME))?;

        let db = Arc::new(ctx.db_required()?);

        let authz = ctx
            .client_hub()
            .get::<dyn AuthZResolverClient>()
            .map_err(|e| anyhow::anyhow!("failed to get AuthZ resolver: {e}"))?;

        let gateway = ctx
            .client_hub()
            .get::<dyn ServiceGatewayClientV1>()
            .map_err(|e| anyhow::anyhow!("failed to get OAGW gateway: {e}"))?;

        // Register OAGW upstreams for each configured provider.
        crate::infra::oagw_provisioning::register_oagw_upstreams(&gateway, &cfg.providers).await?;

        let provider_resolver = Arc::new(ProviderResolver::new(&gateway, cfg.providers));

        let repos = Repositories {
            chat: Arc::new(ChatRepository::new(modkit_db::odata::LimitCfg {
                default: 20,
                max: 100,
            })),
            attachment: Arc::new(AttachmentRepository),
            message: Arc::new(MessageRepository::new(modkit_db::odata::LimitCfg {
                default: 20,
                max: 100,
            })),
            quota: Arc::new(QuotaUsageRepository),
            turn: Arc::new(TurnRepository),
            reaction: Arc::new(ReactionRepository),
            thread_summary: Arc::new(ThreadSummaryRepository),
            vector_store: Arc::new(VectorStoreRepository),
        };

        let model_policy_gw = Arc::new(ModelPolicyGateway::new(ctx.client_hub(), vendor));
        let services = Arc::new(AppServices::new(
            &repos,
            db,
            authz,
            &(model_policy_gw.clone() as Arc<dyn crate::domain::repos::ModelResolver>),
            provider_resolver,
            cfg.streaming,
            model_policy_gw.clone() as Arc<dyn crate::domain::repos::PolicySnapshotProvider>,
            model_policy_gw as Arc<dyn crate::domain::repos::UserLimitsProvider>,
            cfg.estimation_budgets,
            cfg.quota,
        ));

        self.service
            .set(services)
            .map_err(|_| anyhow::anyhow!("{} module already initialized", Self::MODULE_NAME))?;

        info!("{} module initialized successfully", Self::MODULE_NAME);
        Ok(())
    }
}

impl DatabaseCapability for MiniChatModule {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        use sea_orm_migration::MigratorTrait;
        info!("Providing mini-chat database migrations");
        crate::infra::db::migrations::Migrator::migrations()
    }
}

impl RestApiCapability for MiniChatModule {
    fn register_rest(
        &self,
        _ctx: &ModuleCtx,
        router: axum::Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let services = self
            .service
            .get()
            .ok_or_else(|| anyhow::anyhow!("{} not initialized", Self::MODULE_NAME))?;

        info!("Registering mini-chat REST routes");
        let prefix = self
            .url_prefix
            .get()
            .ok_or_else(|| anyhow::anyhow!("{} not initialized (url_prefix)", Self::MODULE_NAME))?;

        let router = routes::register_routes(router, openapi, Arc::clone(services), prefix);
        info!("Mini-chat REST routes registered successfully");
        Ok(router)
    }
}

async fn register_plugin_schemas(
    registry: &dyn TypesRegistryClient,
    schemas: &[(String, &str, &str)],
) -> anyhow::Result<()> {
    let mut payload = Vec::with_capacity(schemas.len());
    for (schema_str, schema_id, _label) in schemas {
        let mut schema_json: serde_json::Value = serde_json::from_str(schema_str)?;
        let obj = schema_json
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("schema {schema_id} is not a JSON object"))?;
        obj.insert(
            "additionalProperties".to_owned(),
            serde_json::Value::Bool(false),
        );
        payload.push(schema_json);
    }
    let results = registry.register(payload).await?;
    RegisterResult::ensure_all_ok(&results)?;
    for (_schema_str, schema_id, label) in schemas {
        info!(schema_id = %schema_id, "Registered {label} plugin schema in types-registry");
    }
    Ok(())
}
