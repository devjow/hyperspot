//! HTTP DTOs (serde/utoipa) — REST-only request and response types.
//!
//! All REST DTOs live here; SDK `models.rs` stays transport-agnostic.
//! Provide `From` conversions between SDK models and DTOs in this file.

/// Server-sent event envelope for the `messages:stream` endpoint.
///
/// Placeholder — will be expanded with concrete event variants
/// (delta, `tool_call`, done, error, etc.) during Phase 1 implementation.
#[derive(Debug, Clone)]
#[modkit_macros::api_dto(response)]
pub struct StreamEvent {
    /// Event type discriminator (e.g. "delta", "done", "error").
    pub event: String,
    /// JSON-encoded event payload.
    pub data: String,
}
