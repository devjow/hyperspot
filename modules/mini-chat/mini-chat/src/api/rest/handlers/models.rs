use std::sync::Arc;

use axum::Extension;
use axum::extract::Path;
use modkit::api::prelude::*;
use modkit_security::SecurityContext;

use crate::domain::service::AppServices;

use super::not_implemented;

/// GET /mini-chat/v1/models
pub(crate) async fn list_models(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}

/// GET /mini-chat/v1/models/{id}
pub(crate) async fn get_model(
    Extension(_ctx): Extension<SecurityContext>,
    Extension(_svc): Extension<Arc<AppServices>>,
    Path(_id): Path<String>,
) -> ApiResult<StatusCode> {
    Err(not_implemented())
}
