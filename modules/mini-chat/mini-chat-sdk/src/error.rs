use thiserror::Error;

/// Errors that can be returned by the `MiniChatClient`.
#[derive(Error, Debug, Clone)]
pub enum MiniChatError {
    /// Chat with the specified ID was not found.
    #[error("Chat not found: {id}")]
    ChatNotFound { id: uuid::Uuid },

    /// The requested model is invalid or unavailable.
    #[error("Invalid model: {name}")]
    InvalidModel { name: String },

    /// Validation error with the provided data.
    #[error("Validation error: {message}")]
    Validation { message: String },

    /// Access denied (authorization failure).
    #[error("Access denied")]
    Forbidden,

    /// An internal error occurred.
    #[error("Internal error")]
    Internal,
}
