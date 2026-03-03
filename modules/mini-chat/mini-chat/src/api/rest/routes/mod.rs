mod attachments;
mod chats;
mod messages;
mod models;
mod reactions;
mod turns;

use std::sync::Arc;

use axum::Router;
use modkit::api::OpenApiRegistry;
use modkit::api::operation_builder::LicenseFeature;

use crate::domain::service::AppServices;

/// License feature required by all mini-chat endpoints.
///
/// DESIGN constraint `cpt-cf-mini-chat-constraint-license-gate`:
/// access requires the `ai_chat` feature on the tenant license.
pub(crate) struct AiChatLicense;

impl AsRef<str> for AiChatLicense {
    fn as_ref(&self) -> &'static str {
        "ai_chat"
    }
}

impl LicenseFeature for AiChatLicense {}

/// Register all mini-chat REST routes.
pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    services: Arc<AppServices>,
    prefix: &str,
) -> Router {
    let router = chats::register_chat_routes(router, openapi, prefix);
    let router = messages::register_message_routes(router, openapi, prefix);
    let router = attachments::register_attachment_routes(router, openapi, prefix);
    let router = turns::register_turn_routes(router, openapi, prefix);
    let router = models::register_model_routes(router, openapi, prefix);
    let router = reactions::register_reaction_routes(router, openapi, prefix);

    router.layer(axum::Extension(services))
}
