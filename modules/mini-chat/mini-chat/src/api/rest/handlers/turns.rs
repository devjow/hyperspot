use std::sync::Arc;

use axum::Extension;
use axum::extract::Path;
use modkit::api::prelude::*;
use modkit_security::SecurityContext;

use crate::domain::service::AppServices;

use super::not_implemented;

/// GET /mini-chat/v1/chats/{id}/turns/{request_id}
pub(crate) async fn get_turn(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path((_chat_id, _request_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// POST /mini-chat/v1/chats/{id}/turns/{request_id}/retry
pub(crate) async fn retry_turn(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path((_chat_id, _request_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// PATCH /mini-chat/v1/chats/{id}/turns/{request_id}
pub(crate) async fn edit_turn(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path((_chat_id, _request_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// DELETE /mini-chat/v1/chats/{id}/turns/{request_id}
pub(crate) async fn delete_turn(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path((_chat_id, _request_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}
