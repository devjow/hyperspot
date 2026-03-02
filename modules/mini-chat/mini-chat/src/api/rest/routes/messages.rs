use axum::Router;
use modkit::api::OpenApiRegistry;
use modkit::api::operation_builder::OperationBuilder;

use super::AiChatLicense;
use crate::api::rest::{dto, handlers};

pub(super) fn register_message_routes(
    mut router: Router,
    openapi: &dyn OpenApiRegistry,
    prefix: &str,
) -> Router {
    // GET {prefix}/v1/chats/{id}/messages
    router = OperationBuilder::get(format!("{prefix}/v1/chats/{{id}}/messages"))
        .operation_id("mini_chat.list_messages")
        .summary("List messages in a chat")
        .tag("messages")
        .authenticated()
        .require_license_features([&AiChatLicense])
        .path_param("id", "Chat UUID")
        .handler(handlers::messages::list_messages)
        .json_response(http::StatusCode::OK, "List of messages")
        .standard_errors(openapi)
        .register(router, openapi);

    // TODO: DESIGN.md specifies Google-style custom method `messages:stream`, but Axum's
    // matchit router doesn't support mixed param+literal segments. Consider adding a
    // rewrite middleware in api-gateway to map `:verb` → `/verb` so clients can use the
    // colon syntax externally while Axum routes via `/stream` internally.
    // POST {prefix}/v1/chats/{id}/messages/stream
    router = OperationBuilder::post(format!("{prefix}/v1/chats/{{id}}/messages/stream"))
        .operation_id("mini_chat.stream_message")
        .summary("Send a message and stream the response via SSE")
        .tag("messages")
        .authenticated()
        .require_license_features([&AiChatLicense])
        .path_param("id", "Chat UUID")
        .handler(handlers::messages::stream_message)
        .sse_json::<dto::StreamEvent>(openapi, "SSE stream of chat response events")
        .standard_errors(openapi)
        .register(router, openapi);

    router
}
