use std::sync::Arc;

use axum::Extension;
use axum::extract::Path;
use modkit::api::prelude::*;
use modkit_security::SecurityContext;

use crate::domain::service::AppServices;

use super::not_implemented;

/// GET /mini-chat/v1/chats/{id}/messages
pub(crate) async fn list_messages(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path(_chat_id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// POST /mini-chat/v1/chats/{id}/messages/stream
pub(crate) async fn stream_message(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path(_chat_id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}
