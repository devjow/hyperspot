use std::sync::Arc;

use axum::Extension;
use axum::extract::Path;
use modkit::api::prelude::*;
use modkit_security::SecurityContext;

use crate::module::AppServices;

use super::not_implemented;

/// POST /mini-chat/v1/chats/{id}/attachments
pub(crate) async fn upload_attachment(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path(_chat_id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// GET /mini-chat/v1/chats/{id}/attachments/{attachment_id}
pub(crate) async fn get_attachment(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path((_chat_id, _attachment_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}
