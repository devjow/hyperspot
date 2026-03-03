use async_trait::async_trait;
use modkit_security::SecurityContext;
use uuid::Uuid;

use crate::error::MiniChatError;
use crate::models::{Chat, ChatPatch, NewChat};

/// Object-safe client for inter-module consumption (`ClientHub` registered) (Version 1).
///
/// This trait is registered in `ClientHub`:
/// ```ignore
/// let mini_chat = hub.get::<dyn MiniChatClientV1>()?;
/// ```
#[async_trait]
pub trait MiniChatClientV1: Send + Sync {
    /// Create a new chat.
    async fn create_chat(
        &self,
        ctx: &SecurityContext,
        new_chat: NewChat,
    ) -> Result<Chat, MiniChatError>;

    /// Get a chat by ID.
    async fn get_chat(&self, ctx: &SecurityContext, id: Uuid) -> Result<Chat, MiniChatError>;

    /// Update a chat.
    async fn update_chat(
        &self,
        ctx: &SecurityContext,
        id: Uuid,
        patch: ChatPatch,
    ) -> Result<Chat, MiniChatError>;

    /// Delete a chat (soft delete).
    async fn delete_chat(&self, ctx: &SecurityContext, id: Uuid) -> Result<(), MiniChatError>;
}
