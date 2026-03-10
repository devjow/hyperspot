use std::sync::Arc;

use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use tokio::sync::{Notify, Semaphore};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use super::dialect::{Dialect, ReaperSql};
use super::handler::HandlerResult;
use super::strategy::{ProcessContext, ProcessingStrategy};
use super::types::{OutboxError, QueueConfig};
use crate::Db;

/// Max body IDs per batched DELETE in the reaper.
const REAPER_CHUNK_SIZE: usize = 100;

/// Per-partition adaptive batch sizing state machine.
///
/// Degrades to single-message mode on failure, ramps back up on consecutive
/// successes. Analogous to TCP slow start.
#[derive(Debug, Clone)]
pub enum PartitionMode {
    /// Normal operation — use configured `msg_batch_size`.
    Normal,
    /// Degraded after failure — process fewer messages at a time.
    /// Ramps back up (doubling) on consecutive successes until reaching
    /// the configured `msg_batch_size`, then transitions back to `Normal`.
    Degraded {
        effective_size: u32,
        consecutive_successes: u32,
    },
}

impl PartitionMode {
    /// Returns the effective batch size for this mode.
    pub(crate) fn effective_batch_size(&self, configured: u32) -> u32 {
        match self {
            Self::Normal => configured,
            Self::Degraded { effective_size, .. } => *effective_size,
        }
    }

    /// Transition after a handler result.
    ///
    /// `processed_count`: how many messages were successfully processed before
    /// the batch ended. `Some` for `PerMessageAdapter` handlers, `None` for batch
    /// handlers. On Retry/Reject, degradation uses `max(pc, 1)` when known,
    /// or falls back to 1 when `None` (batch handler — we can't know where
    /// the failure occurred).
    pub(crate) fn transition(
        &mut self,
        result: &HandlerResult,
        configured_batch_size: u32,
        processed_count: Option<u32>,
    ) {
        match result {
            HandlerResult::Success => match self {
                Self::Normal => {}
                Self::Degraded {
                    effective_size,
                    consecutive_successes,
                } => {
                    *consecutive_successes += 1;
                    // Double the effective size on each consecutive success
                    let next = effective_size.saturating_mul(2).min(configured_batch_size);
                    if next >= configured_batch_size {
                        *self = Self::Normal;
                    } else {
                        *effective_size = next;
                    }
                }
            },
            HandlerResult::Retry { .. } | HandlerResult::Reject { .. } => {
                // Degrade: use max(processed_count, 1) as the new effective
                // size. If the handler processed some messages before failing,
                // we know the failure is at position pc+1, so we degrade to
                // max(pc, 1) to isolate the poison message. For batch handlers
                // (None), fall back to 1 (most conservative).
                let degrade_to = processed_count.map_or(1, |pc| pc.max(1));
                *self = Self::Degraded {
                    effective_size: degrade_to,
                    consecutive_successes: 0,
                };
            }
        }
    }
}

/// A per-partition processor parameterized by its processing strategy.
///
/// Each instance owns exactly one `partition_id` and runs as a long-lived
/// tokio task. The strategy (`TransactionalStrategy` or `DecoupledStrategy`)
/// is baked in at compile time via monomorphization.
pub struct PartitionProcessor<S: ProcessingStrategy> {
    strategy: S,
    partition_id: i64,
    config: QueueConfig,
    notify: Arc<Notify>,
    semaphore: Arc<Semaphore>,
    backoff_until: Option<Instant>,
    partition_mode: PartitionMode,
}

impl<S: ProcessingStrategy> PartitionProcessor<S> {
    pub fn new(
        strategy: S,
        partition_id: i64,
        config: QueueConfig,
        notify: Arc<Notify>,
        semaphore: Arc<Semaphore>,
    ) -> Self {
        Self {
            strategy,
            partition_id,
            config,
            notify,
            semaphore,
            backoff_until: None,
            partition_mode: PartitionMode::Normal,
        }
    }

    /// Main event loop. Runs until `cancel` fires.
    pub async fn run(mut self, db: &Db, cancel: CancellationToken) -> Result<(), OutboxError> {
        let sea_conn = db.sea_internal();
        let backend = sea_conn.get_database_backend();
        let dialect = Dialect::from(backend);
        drop(sea_conn);

        let mut has_more = false;
        loop {
            // Respect backoff — sleep precisely for the remaining duration
            // instead of waking every poll_interval to re-check.
            if let Some(until) = self.backoff_until.take() {
                let now = Instant::now();
                if now < until {
                    let remaining = until - now;
                    tokio::select! {
                        () = cancel.cancelled() => break,
                        () = tokio::time::sleep(remaining) => {},
                    }
                    if cancel.is_cancelled() {
                        break;
                    }
                }
            }

            if !has_more {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    () = self.notify.notified() => {},
                    () = tokio::time::sleep(self.config.poll_interval) => {},
                }
            }
            if cancel.is_cancelled() {
                break;
            }

            // Acquire semaphore permit (bounded concurrency per queue)
            let effective_size = self
                .partition_mode
                .effective_batch_size(self.config.msg_batch_size);

            let result = {
                let Ok(_permit) = self.semaphore.acquire().await else {
                    break; // semaphore closed — shut down
                };

                let ctx = ProcessContext {
                    db,
                    backend,
                    dialect,
                    partition_id: self.partition_id,
                };

                // Use effective batch size from partition mode
                let mut effective_config = self.config.clone();
                effective_config.msg_batch_size = effective_size;

                let child_cancel = cancel.child_token();
                self.strategy
                    .process(&ctx, &effective_config, child_cancel)
                    .await?
            }; // permit dropped here

            if let Some(pr) = result {
                has_more = pr.count >= effective_size;
                // Clamp processed_count to batch count as defensive bound
                let clamped_pc = pr.processed_count.map(|pc| pc.min(pr.count));
                self.partition_mode.transition(
                    &pr.handler_result,
                    self.config.msg_batch_size,
                    clamped_pc,
                );
                self.update_backoff(&pr.handler_result, pr.attempts_before);
                if pr.count > 0 {
                    debug!(
                        partition_id = self.partition_id,
                        count = pr.count,
                        mode = ?self.partition_mode,
                        "partition batch complete"
                    );
                }
            } else {
                has_more = false;
                // Reaper: clean up processed outgoing + body rows when idle
                self.reap(db, backend, &dialect).await?;
            }
        }
        Ok(())
    }

    /// Reaper: bulk-delete processed outgoing rows and their body rows.
    async fn reap(
        &self,
        db: &Db,
        backend: DbBackend,
        dialect: &Dialect,
    ) -> Result<(), OutboxError> {
        let conn = db.sea_internal();
        let row = conn
            .query_one(Statement::from_sql_and_values(
                backend,
                dialect.read_processor(),
                [self.partition_id.into()],
            ))
            .await?;
        drop(conn);

        let Some(row) = row else {
            return Ok(());
        };
        let processed_seq: i64 = row.try_get_by_index(0).map_err(|e| {
            OutboxError::Database(sea_orm::DbErr::Custom(format!("processed_seq column: {e}")))
        })?;
        if processed_seq == 0 {
            return Ok(());
        }

        let sea_conn = db.sea_internal();
        let txn = sea_conn.begin().await?;

        match dialect.reaper_cleanup() {
            ReaperSql::Cte(sql) => {
                txn.execute(Statement::from_sql_and_values(
                    backend,
                    sql,
                    [self.partition_id.into(), processed_seq.into()],
                ))
                .await?;
            }
            ReaperSql::TwoStep {
                select_body_ids,
                delete_outgoing,
            } => {
                let rows = txn
                    .query_all(Statement::from_sql_and_values(
                        backend,
                        select_body_ids,
                        [self.partition_id.into(), processed_seq.into()],
                    ))
                    .await?;

                let body_ids: Vec<i64> = rows
                    .iter()
                    .filter_map(|r| r.try_get_by_index::<i64>(0).ok())
                    .collect();

                txn.execute(Statement::from_sql_and_values(
                    backend,
                    delete_outgoing,
                    [self.partition_id.into(), processed_seq.into()],
                ))
                .await?;

                // Batch body deletes — chunk to avoid exceeding DB parameter limits
                for chunk in body_ids.chunks(REAPER_CHUNK_SIZE) {
                    let delete_sql = dialect.build_delete_body_batch(chunk.len());
                    let values: Vec<sea_orm::Value> = chunk.iter().map(|&id| id.into()).collect();
                    txn.execute(Statement::from_sql_and_values(backend, &delete_sql, values))
                        .await?;
                }
            }
        }

        txn.commit().await?;
        Ok(())
    }

    fn update_backoff(&mut self, result: &HandlerResult, current_attempts: i16) {
        match result {
            HandlerResult::Retry { .. } => {
                let attempts = current_attempts + 1;
                #[allow(clippy::cast_possible_truncation)]
                let base_ms = self.config.backoff_base.as_millis() as u64;
                #[allow(clippy::cast_possible_truncation)]
                let max_ms = self.config.backoff_max.as_millis() as u64;

                #[allow(clippy::cast_sign_loss)]
                let exp = (attempts as u32).saturating_sub(1).min(30);
                let delay_ms = base_ms.saturating_mul(1u64 << exp).min(max_ms);

                #[allow(clippy::integer_division)]
                let jitter_ms = if delay_ms > 0 {
                    rand_jitter(delay_ms / 4)
                } else {
                    0
                };
                let total_ms = delay_ms.saturating_add(jitter_ms);

                self.backoff_until =
                    Some(Instant::now() + std::time::Duration::from_millis(total_ms));
            }
            HandlerResult::Success | HandlerResult::Reject { .. } => {
                self.backoff_until = None;
            }
        }
    }
}

/// Simple deterministic-ish jitter without pulling in a PRNG crate.
fn rand_jitter(max: u64) -> u64 {
    if max == 0 {
        return 0;
    }
    let nanos = u64::from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos(),
    );
    nanos % max
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::super::strategy::{ProcessContext, ProcessResult, ProcessingStrategy};
    use super::*;
    use std::time::Duration;

    fn make_config(base: Duration, max: Duration) -> QueueConfig {
        QueueConfig {
            backoff_base: base,
            backoff_max: max,
            ..Default::default()
        }
    }

    fn make_processor_for_backoff(
        base: Duration,
        max: Duration,
    ) -> PartitionProcessor<StubStrategy> {
        PartitionProcessor {
            strategy: StubStrategy,
            partition_id: 1,
            config: make_config(base, max),
            notify: Arc::new(Notify::new()),
            semaphore: Arc::new(Semaphore::new(1)),
            backoff_until: None,
            partition_mode: PartitionMode::Normal,
        }
    }

    /// Stub strategy that is never called — only used for backoff unit tests.
    struct StubStrategy;

    impl ProcessingStrategy for StubStrategy {
        async fn process(
            &self,
            _ctx: &ProcessContext<'_>,
            _config: &QueueConfig,
            _cancel: CancellationToken,
        ) -> Result<Option<ProcessResult>, OutboxError> {
            unimplemented!("stub")
        }
    }

    #[test]
    fn rand_jitter_zero_returns_zero() {
        assert_eq!(rand_jitter(0), 0);
    }

    #[test]
    fn rand_jitter_bounded() {
        for _ in 0..100 {
            let j = rand_jitter(1000);
            assert!(j < 1000, "jitter {j} should be < 1000");
        }
    }

    #[test]
    fn backoff_on_retry_sets_deadline() {
        let mut p = make_processor_for_backoff(Duration::from_millis(100), Duration::from_secs(10));
        assert!(p.backoff_until.is_none());

        p.update_backoff(
            &HandlerResult::Retry {
                reason: "fail".into(),
            },
            0,
        );
        assert!(p.backoff_until.is_some());
    }

    #[test]
    fn backoff_on_success_clears_deadline() {
        let mut p = make_processor_for_backoff(Duration::from_millis(100), Duration::from_secs(10));
        // Set a backoff first
        p.update_backoff(
            &HandlerResult::Retry {
                reason: "fail".into(),
            },
            0,
        );
        assert!(p.backoff_until.is_some());

        // Success clears it
        p.update_backoff(&HandlerResult::Success, 1);
        assert!(p.backoff_until.is_none());
    }

    #[test]
    fn backoff_on_reject_clears_deadline() {
        let mut p = make_processor_for_backoff(Duration::from_millis(100), Duration::from_secs(10));
        p.update_backoff(
            &HandlerResult::Retry {
                reason: "fail".into(),
            },
            0,
        );
        assert!(p.backoff_until.is_some());

        p.update_backoff(
            &HandlerResult::Reject {
                reason: "bad".into(),
            },
            1,
        );
        assert!(p.backoff_until.is_none());
    }

    #[test]
    fn backoff_exponential_growth_capped_by_max() {
        let mut p =
            make_processor_for_backoff(Duration::from_millis(100), Duration::from_millis(5000));

        // First retry: ~100ms base
        p.update_backoff(
            &HandlerResult::Retry { reason: "x".into() },
            0, // current_attempts=0, so attempts=1, exp=0, delay=100ms
        );
        let d1 = p.backoff_until.unwrap();

        // Fifth retry: ~1600ms base (100 * 2^4), still under 5000ms max
        p.update_backoff(
            &HandlerResult::Retry { reason: "x".into() },
            4, // current_attempts=4, so attempts=5, exp=4, delay=1600ms
        );
        let d5 = p.backoff_until.unwrap();
        assert!(d5 > d1, "higher attempts should produce later deadline");

        // Very high attempt: capped at max (5000ms)
        p.update_backoff(&HandlerResult::Retry { reason: "x".into() }, 20);
        // Just verify it doesn't panic and sets a deadline
        assert!(p.backoff_until.is_some());
    }

    // ---- PartitionMode state machine tests ----

    #[test]
    fn partition_mode_normal_uses_configured_size() {
        let mode = PartitionMode::Normal;
        assert_eq!(mode.effective_batch_size(50), 50);
    }

    #[test]
    fn partition_mode_degraded_uses_effective_size() {
        let mode = PartitionMode::Degraded {
            effective_size: 4,
            consecutive_successes: 2,
        };
        assert_eq!(mode.effective_batch_size(50), 4);
    }

    #[test]
    fn partition_mode_retry_degrades_to_one() {
        let mut mode = PartitionMode::Normal;
        mode.transition(
            &HandlerResult::Retry {
                reason: "fail".into(),
            },
            50,
            None, // batch handler
        );
        assert!(matches!(
            mode,
            PartitionMode::Degraded {
                effective_size: 1,
                consecutive_successes: 0,
            }
        ));
    }

    #[test]
    fn partition_mode_success_ramps_up() {
        let mut mode = PartitionMode::Degraded {
            effective_size: 1,
            consecutive_successes: 0,
        };
        // 1 → 2
        mode.transition(&HandlerResult::Success, 50, None);
        assert!(matches!(
            mode,
            PartitionMode::Degraded {
                effective_size: 2,
                consecutive_successes: 1,
            }
        ));
        // 2 → 4
        mode.transition(&HandlerResult::Success, 50, None);
        assert!(matches!(
            mode,
            PartitionMode::Degraded {
                effective_size: 4,
                ..
            }
        ));
        // 4 → 8
        mode.transition(&HandlerResult::Success, 50, None);
        assert!(matches!(
            mode,
            PartitionMode::Degraded {
                effective_size: 8,
                ..
            }
        ));
    }

    #[test]
    fn partition_mode_ramps_up_to_normal() {
        let mut mode = PartitionMode::Degraded {
            effective_size: 16,
            consecutive_successes: 4,
        };
        // 16 → 32
        mode.transition(&HandlerResult::Success, 32, None);
        // Should transition back to Normal since 32 >= configured(32)
        assert!(matches!(mode, PartitionMode::Normal));
    }

    #[test]
    fn partition_mode_reject_in_normal_degrades() {
        let mut mode = PartitionMode::Normal;
        mode.transition(
            &HandlerResult::Reject {
                reason: "bad".into(),
            },
            50,
            None, // batch handler — falls back to 1
        );
        assert!(matches!(
            mode,
            PartitionMode::Degraded {
                effective_size: 1,
                consecutive_successes: 0,
            }
        ));
    }

    #[test]
    fn partition_mode_reject_with_processed_count() {
        // PerMessageAdapter handler processed 3 msgs before poison at index 3
        let mut mode = PartitionMode::Normal;
        mode.transition(
            &HandlerResult::Reject {
                reason: "bad".into(),
            },
            50,
            Some(3), // PerMessageAdapter processed 3 successfully
        );
        assert!(matches!(
            mode,
            PartitionMode::Degraded {
                effective_size: 3,
                consecutive_successes: 0,
            }
        ));
    }

    #[test]
    fn partition_mode_retry_with_processed_count_zero() {
        // PerMessageAdapter failed at the very first message
        let mut mode = PartitionMode::Normal;
        mode.transition(
            &HandlerResult::Retry {
                reason: "fail".into(),
            },
            50,
            Some(0), // failed at first message
        );
        // max(0, 1) = 1
        assert!(matches!(
            mode,
            PartitionMode::Degraded {
                effective_size: 1,
                consecutive_successes: 0,
            }
        ));
    }

    #[test]
    fn partition_mode_success_in_normal_stays_normal() {
        let mut mode = PartitionMode::Normal;
        mode.transition(&HandlerResult::Success, 50, None);
        assert!(matches!(mode, PartitionMode::Normal));
    }

    #[test]
    fn partition_mode_full_recovery_cycle() {
        let mut mode = PartitionMode::Normal;

        // Retry → Degraded(1)
        mode.transition(&HandlerResult::Retry { reason: "x".into() }, 8, None);
        assert_eq!(mode.effective_batch_size(8), 1);

        // Success: 1→2→4→8→Normal
        mode.transition(&HandlerResult::Success, 8, None);
        assert_eq!(mode.effective_batch_size(8), 2);
        mode.transition(&HandlerResult::Success, 8, None);
        assert_eq!(mode.effective_batch_size(8), 4);
        mode.transition(&HandlerResult::Success, 8, None);
        assert!(matches!(mode, PartitionMode::Normal));
        assert_eq!(mode.effective_batch_size(8), 8);
    }
}
