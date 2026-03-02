use axum::Router;
use modkit::api::OpenApiRegistry;
use modkit::api::operation_builder::OperationBuilder;

use super::AiChatLicense;
use crate::api::rest::handlers;

pub(super) fn register_chat_routes(
    mut router: Router,
    openapi: &dyn OpenApiRegistry,
    prefix: &str,
) -> Router {
    // POST {prefix}/v1/chats
    router = OperationBuilder::post(format!("{prefix}/v1/chats"))
        .operation_id("mini_chat.create_chat")
        .summary("Create a new chat")
        .tag("chats")
        .authenticated()
        .require_license_features([&AiChatLicense])
        .handler(handlers::chats::create_chat)
        .json_response(http::StatusCode::CREATED, "Created chat")
        .standard_errors(openapi)
        .register(router, openapi);

    // GET {prefix}/v1/chats
    router = OperationBuilder::get(format!("{prefix}/v1/chats"))
        .operation_id("mini_chat.list_chats")
        .summary("List chats for the current user")
        .tag("chats")
        .authenticated()
        .require_license_features([&AiChatLicense])
        .handler(handlers::chats::list_chats)
        .json_response(http::StatusCode::OK, "List of chats")
        .standard_errors(openapi)
        .register(router, openapi);

    // GET {prefix}/v1/chats/{id}
    router = OperationBuilder::get(format!("{prefix}/v1/chats/{{id}}"))
        .operation_id("mini_chat.get_chat")
        .summary("Get a chat by ID")
        .tag("chats")
        .authenticated()
        .require_license_features([&AiChatLicense])
        .path_param("id", "Chat UUID")
        .handler(handlers::chats::get_chat)
        .json_response(http::StatusCode::OK, "Chat found")
        .standard_errors(openapi)
        .register(router, openapi);

    // PATCH {prefix}/v1/chats/{id}
    router = OperationBuilder::patch(format!("{prefix}/v1/chats/{{id}}"))
        .operation_id("mini_chat.update_chat")
        .summary("Update a chat")
        .tag("chats")
        .authenticated()
        .require_license_features([&AiChatLicense])
        .path_param("id", "Chat UUID")
        .handler(handlers::chats::update_chat)
        .json_response(http::StatusCode::OK, "Updated chat")
        .standard_errors(openapi)
        .register(router, openapi);

    // DELETE {prefix}/v1/chats/{id}
    router = OperationBuilder::delete(format!("{prefix}/v1/chats/{{id}}"))
        .operation_id("mini_chat.delete_chat")
        .summary("Delete a chat")
        .tag("chats")
        .authenticated()
        .require_license_features([&AiChatLicense])
        .path_param("id", "Chat UUID")
        .handler(handlers::chats::delete_chat)
        .json_response(http::StatusCode::NO_CONTENT, "Chat deleted")
        .standard_errors(openapi)
        .register(router, openapi);

    router
}
