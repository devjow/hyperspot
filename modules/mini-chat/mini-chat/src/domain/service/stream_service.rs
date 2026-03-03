use std::sync::Arc;

use authz_resolver_sdk::PolicyEnforcer;
use modkit_macros::domain_model;

use crate::domain::repos::{ChatRepository, MessageRepository, TurnRepository};

use super::DbProvider;

/// Service handling SSE streaming, turn orchestration, and turn mutations.
#[domain_model]
pub struct StreamService {
    _db: Arc<DbProvider>,
    _turn_repo: Arc<dyn TurnRepository>,
    _message_repo: Arc<dyn MessageRepository>,
    _chat_repo: Arc<dyn ChatRepository>,
    _enforcer: PolicyEnforcer,
}

impl StreamService {
    pub(crate) fn new(
        db: Arc<DbProvider>,
        turn_repo: Arc<dyn TurnRepository>,
        message_repo: Arc<dyn MessageRepository>,
        chat_repo: Arc<dyn ChatRepository>,
        enforcer: PolicyEnforcer,
    ) -> Self {
        Self {
            _db: db,
            _turn_repo: turn_repo,
            _message_repo: message_repo,
            _chat_repo: chat_repo,
            _enforcer: enforcer,
        }
    }
}
