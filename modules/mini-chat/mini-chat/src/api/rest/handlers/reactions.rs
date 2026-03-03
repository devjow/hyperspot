use std::sync::Arc;

use axum::Extension;
use axum::extract::Path;
use modkit::api::prelude::*;
use modkit_security::SecurityContext;

use crate::domain::service::AppServices;

use super::not_implemented;

/// PUT /mini-chat/v1/chats/{id}/messages/{msg_id}/reaction
pub(crate) async fn put_reaction(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path((_chat_id, _msg_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// DELETE /mini-chat/v1/chats/{id}/messages/{msg_id}/reaction
pub(crate) async fn delete_reaction(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path((_chat_id, _msg_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}
