use std::sync::Arc;

use authz_resolver_sdk::PolicyEnforcer;
use modkit_macros::domain_model;

use crate::domain::repos::{ChatRepository, MessageRepository, ReactionRepository};

use super::DbProvider;

/// Service handling message reaction operations.
#[domain_model]
pub struct ReactionService {
    _db: Arc<DbProvider>,
    _reaction_repo: Arc<dyn ReactionRepository>,
    _message_repo: Arc<dyn MessageRepository>,
    _chat_repo: Arc<dyn ChatRepository>,
    _enforcer: PolicyEnforcer,
}

impl ReactionService {
    pub(crate) fn new(
        db: Arc<DbProvider>,
        reaction_repo: Arc<dyn ReactionRepository>,
        message_repo: Arc<dyn MessageRepository>,
        chat_repo: Arc<dyn ChatRepository>,
        enforcer: PolicyEnforcer,
    ) -> Self {
        Self {
            _db: db,
            _reaction_repo: reaction_repo,
            _message_repo: message_repo,
            _chat_repo: chat_repo,
            _enforcer: enforcer,
        }
    }
}
