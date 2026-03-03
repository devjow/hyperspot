use std::sync::Arc;

use axum::Extension;
use axum::extract::Path;
use modkit::api::prelude::*;
use modkit_security::SecurityContext;

use crate::domain::service::AppServices;

use super::not_implemented;

/// POST /mini-chat/v1/chats
pub(crate) async fn create_chat(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// GET /mini-chat/v1/chats
pub(crate) async fn list_chats(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// GET /mini-chat/v1/chats/{id}
pub(crate) async fn get_chat(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path(_id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// PATCH /mini-chat/v1/chats/{id}
pub(crate) async fn update_chat(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path(_id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// DELETE /mini-chat/v1/chats/{id}
pub(crate) async fn delete_chat(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path(_id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}
