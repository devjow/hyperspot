use async_trait::async_trait;
use uuid::Uuid;

use crate::audit_models::{
    TurnAuditEvent, TurnDeleteAuditEvent, TurnEditAuditEvent, TurnRetryAuditEvent,
};
use crate::error::{MiniChatAuditPluginError, MiniChatModelPolicyPluginError, PublishError};
use crate::models::{PolicySnapshot, PolicyVersionInfo, UsageEvent, UserLimits};

/// Plugin API trait for mini-chat model policy implementations.
///
/// Plugins implement this trait to provide model catalog and policy data.
/// The mini-chat module discovers plugins via GTS types-registry and
/// delegates policy queries to the selected plugin.
#[async_trait]
pub trait MiniChatModelPolicyPluginClientV1: Send + Sync {
    /// Get the current policy version for a user.
    async fn get_current_policy_version(
        &self,
        user_id: Uuid,
    ) -> Result<PolicyVersionInfo, MiniChatModelPolicyPluginError>;

    /// Get the full policy snapshot for a given version, including
    /// model catalog and kill switches.
    async fn get_policy_snapshot(
        &self,
        user_id: Uuid,
        policy_version: u64,
    ) -> Result<PolicySnapshot, MiniChatModelPolicyPluginError>;

    /// Get per-user credit limits for a specific policy version.
    async fn get_user_limits(
        &self,
        user_id: Uuid,
        policy_version: u64,
    ) -> Result<UserLimits, MiniChatModelPolicyPluginError>;

    /// Publish a usage event after turn finalization.
    ///
    /// Called by the outbox processor after the finalization transaction
    /// commits. Plugins can forward the event to external billing systems.
    async fn publish_usage(&self, payload: UsageEvent) -> Result<(), PublishError>;
}

/// Plugin API trait for mini-chat audit event publishing.
///
/// Plugins implement this trait to receive audit events from the mini-chat
/// module. The mini-chat module discovers plugins via GTS types-registry and
/// dispatches audit events to all registered implementations.
///
/// # Caller contract
///
/// The **caller** (mini-chat domain service) MUST redact secret patterns and
/// truncate string content (max 8 KiB per field) *before* invoking any method
/// on this trait. Plugins MUST assume all content fields are already sanitized.
/// See DESIGN.md "Audit content handling (P1)" for the full redaction rule table.
///
/// # Delivery semantics
///
/// Audit emission is best-effort (fire-and-forget after DB commit). There is no
/// transactional outbox for audit events. If the process crashes between DB
/// commit and audit emission, the event is lost. Callers SHOULD track emission
/// outcomes via `mini_chat_audit_emit_total{result}` metrics.
///
/// # Independence
///
/// When multiple audit plugin instances are registered, each MUST be
/// independent. A failure in one plugin MUST NOT prevent delivery to others.
#[async_trait]
pub trait MiniChatAuditPluginClientV1: Send + Sync {
    /// Emit a turn audit event (turn completed or failed).
    async fn emit_turn_audit(&self, event: TurnAuditEvent) -> Result<(), MiniChatAuditPluginError>;

    /// Emit a turn-retry audit event.
    async fn emit_turn_retry_audit(
        &self,
        event: TurnRetryAuditEvent,
    ) -> Result<(), MiniChatAuditPluginError>;

    /// Emit a turn-edit audit event.
    async fn emit_turn_edit_audit(
        &self,
        event: TurnEditAuditEvent,
    ) -> Result<(), MiniChatAuditPluginError>;

    /// Emit a turn-delete audit event.
    async fn emit_turn_delete_audit(
        &self,
        event: TurnDeleteAuditEvent,
    ) -> Result<(), MiniChatAuditPluginError>;
}
