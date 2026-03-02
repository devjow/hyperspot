use std::sync::Arc;

use authz_resolver_sdk::PolicyEnforcer;
use modkit_macros::domain_model;

use crate::domain::repos::{ChatRepository, MessageRepository, ThreadSummaryRepository};

use super::DbProvider;

/// Service handling chat CRUD and message listing operations.
#[domain_model]
pub struct ChatService {
    _db: Arc<DbProvider>,
    _chat_repo: Arc<dyn ChatRepository>,
    _message_repo: Arc<dyn MessageRepository>,
    _thread_summary_repo: Arc<dyn ThreadSummaryRepository>,
    _enforcer: PolicyEnforcer,
}

impl ChatService {
    pub(crate) fn new(
        db: Arc<DbProvider>,
        chat_repo: Arc<dyn ChatRepository>,
        message_repo: Arc<dyn MessageRepository>,
        thread_summary_repo: Arc<dyn ThreadSummaryRepository>,
        enforcer: PolicyEnforcer,
    ) -> Self {
        Self {
            _db: db,
            _chat_repo: chat_repo,
            _message_repo: message_repo,
            _thread_summary_repo: thread_summary_repo,
            _enforcer: enforcer,
        }
    }
}
