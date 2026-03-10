use std::sync::Arc;

use authz_resolver_sdk::PolicyEnforcer;
use modkit_macros::domain_model;

use crate::domain::repos::{AttachmentRepository, ChatRepository, VectorStoreRepository};

use super::DbProvider;

/// Service handling file attachment operations.
#[domain_model]
pub struct AttachmentService<CR: ChatRepository> {
    _db: Arc<DbProvider>,
    _attachment_repo: Arc<dyn AttachmentRepository>,
    _chat_repo: Arc<CR>,
    _vector_store_repo: Arc<dyn VectorStoreRepository>,
    _enforcer: PolicyEnforcer,
}

impl<CR: ChatRepository> AttachmentService<CR> {
    pub(crate) fn new(
        db: Arc<DbProvider>,
        attachment_repo: Arc<dyn AttachmentRepository>,
        chat_repo: Arc<CR>,
        vector_store_repo: Arc<dyn VectorStoreRepository>,
        enforcer: PolicyEnforcer,
    ) -> Self {
        Self {
            _db: db,
            _attachment_repo: attachment_repo,
            _chat_repo: chat_repo,
            _vector_store_repo: vector_store_repo,
            _enforcer: enforcer,
        }
    }
}
