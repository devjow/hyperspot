use std::collections::HashMap;
use std::sync::Arc;

use sea_orm::{ConnectionTrait, DbBackend, FromQueryResult, Statement, TransactionTrait};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use super::Outbox;
use super::dialect::{AllocSql, Dialect};
use super::types::{OutboxError, SequencerConfig};
use crate::Db;

/// Per-partition notify map shared between sequencer and processors.
type PartitionNotifyMap = Arc<HashMap<i64, Arc<Notify>>>;

/// Background sequencer that consumes from incoming, assigns per-partition
/// sequence numbers, and writes to outgoing.
///
/// Processes partitions in deterministic PK order within a single transaction.
/// Uses `FOR UPDATE SKIP LOCKED` on Postgres/MySQL to allow concurrent
/// sequencers without deadlocks.
pub struct Sequencer {
    config: SequencerConfig,
    outbox: Arc<Outbox>,
    notify: Arc<Notify>,
    /// Per-partition notify map for direct signaling.
    partition_notify: std::sync::RwLock<Option<PartitionNotifyMap>>,
}

/// Result of a single `sequence_batch` invocation.
pub struct SequenceBatchResult {
    /// Whether any single partition returned a full batch (hit the LIMIT).
    pub any_partition_saturated: bool,
}

#[derive(Debug, FromQueryResult)]
struct ClaimedIncoming {
    id: i64,
    body_id: i64,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl Sequencer {
    /// Create a new sequencer.
    #[must_use]
    pub fn new(config: SequencerConfig, outbox: Arc<Outbox>, notify: Arc<Notify>) -> Self {
        Self {
            config,
            outbox,
            notify,
            partition_notify: std::sync::RwLock::new(None),
        }
    }

    /// Set the per-partition notify map. Called by the manager before spawning.
    ///
    /// # Panics
    ///
    /// Panics if the internal `RwLock` is poisoned (another thread panicked
    /// while holding it).
    pub fn set_partition_notify(&self, map: PartitionNotifyMap) {
        let mut guard = self
            .partition_notify
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some(map);
    }

    /// Run the sequencer loop until cancellation.
    ///
    /// # Errors
    ///
    /// Returns an error if a database operation fails.
    pub async fn run(&self, db: &Db, cancel: CancellationToken) -> Result<(), OutboxError> {
        let mut has_more = false;
        loop {
            if !has_more {
                tokio::select! {
                    () = cancel.cancelled() => break,
                    () = self.notify.notified() => {}
                    () = tokio::time::sleep(self.config.poll_interval) => {}
                }
            }
            if cancel.is_cancelled() {
                break;
            }
            let result = self.sequence_batch(db).await?;
            has_more = result.any_partition_saturated;
        }
        Ok(())
    }

    /// Execute one sequencing cycle: iterate all partitions in PK order,
    /// claim from incoming per-partition, assign sequences, write to outgoing.
    /// Returns total number of items sequenced across all partitions.
    ///
    /// # Errors
    ///
    /// Returns an error if a database operation fails.
    pub async fn sequence_batch(&self, db: &Db) -> Result<SequenceBatchResult, OutboxError> {
        let partition_ids = self.outbox.all_partition_ids();
        if partition_ids.is_empty() {
            return Ok(SequenceBatchResult {
                any_partition_saturated: false,
            });
        }

        let conn = db.sea_internal();
        let backend = conn.get_database_backend();
        let dialect = Dialect::from(backend);
        let txn = conn.begin().await?;

        let mut total = 0u32;
        let mut any_saturated = false;
        let mut affected_partitions = Vec::new();

        for partition_id in &partition_ids {
            // Try to acquire a row lock; skip if held by another sequencer.
            // Postgres/MySQL use FOR UPDATE SKIP LOCKED; SQLite returns None (no row locking).
            if let Some(lock_sql) = dialect.lock_partition()
                && !self
                    .try_lock_partition(&txn, backend, *partition_id, lock_sql)
                    .await?
            {
                continue;
            }

            // Claim incoming for this partition
            let claimed = self
                .claim_incoming_for_partition(&txn, backend, &dialect, *partition_id)
                .await?;
            if claimed.is_empty() {
                continue;
            }

            #[allow(clippy::cast_possible_wrap)]
            let item_count = claimed.len() as i64;

            #[allow(clippy::cast_possible_truncation)]
            let count = claimed.len() as u32;
            if count >= self.config.batch_size {
                any_saturated = true;
            }

            // Allocate sequences
            let start_seq = self
                .allocate_sequences(&txn, backend, &dialect, *partition_id, item_count)
                .await?;

            // Insert outgoing rows (batched)
            let outgoing_sql = dialect.build_insert_outgoing_batch(claimed.len());
            let mut values: Vec<sea_orm::Value> = Vec::with_capacity(claimed.len() * 4);
            for (i, item) in claimed.iter().enumerate() {
                #[allow(clippy::cast_possible_wrap)]
                let seq = start_seq + 1 + i as i64;
                values.push((*partition_id).into());
                values.push(item.body_id.into());
                values.push(seq.into());
                values.push(item.created_at.into());
            }
            txn.execute(Statement::from_sql_and_values(
                backend,
                &outgoing_sql,
                values,
            ))
            .await?;

            total += count;
            affected_partitions.push(*partition_id);
        }

        txn.commit().await?;

        // Notify per-partition processors after commit
        if let Some(map) = self
            .partition_notify
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            for pid in &affected_partitions {
                if let Some(notify) = map.get(pid) {
                    notify.notify_one();
                }
            }
        }

        if total > 0 {
            debug!(
                total,
                partitions = affected_partitions.len(),
                "sequenced batch"
            );
        }

        Ok(SequenceBatchResult {
            any_partition_saturated: any_saturated,
        })
    }

    /// Try to acquire a row-level lock on the partition row.
    /// Returns `true` if the lock was acquired, `false` if skipped.
    async fn try_lock_partition(
        &self,
        txn: &impl ConnectionTrait,
        backend: DbBackend,
        partition_id: i64,
        sql: &str,
    ) -> Result<bool, OutboxError> {
        let row = txn
            .query_one(Statement::from_sql_and_values(
                backend,
                sql,
                [partition_id.into()],
            ))
            .await?;
        Ok(row.is_some())
    }

    /// Claim incoming items for a single partition.
    ///
    /// Uses SELECT-then-DELETE on all backends to guarantee FIFO order:
    /// the SELECT returns rows ordered by `id`, and the caller assigns
    /// sequences in that iteration order.
    async fn claim_incoming_for_partition(
        &self,
        txn: &impl ConnectionTrait,
        backend: DbBackend,
        dialect: &Dialect,
        partition_id: i64,
    ) -> Result<Vec<ClaimedIncoming>, OutboxError> {
        let claim = dialect.claim_incoming(self.config.batch_size);

        // SELECT id, body_id, created_at ... ORDER BY id
        let rows = ClaimedIncoming::find_by_statement(Statement::from_sql_and_values(
            backend,
            &claim.select,
            [partition_id.into()],
        ))
        .all(txn)
        .await?;

        if rows.is_empty() {
            return Ok(rows);
        }

        // DELETE the selected rows by id
        let delete_sql = dialect.delete_incoming_batch(rows.len());
        let values: Vec<sea_orm::Value> = rows.iter().map(|r| r.id.into()).collect();
        txn.execute(Statement::from_sql_and_values(backend, &delete_sql, values))
            .await?;

        Ok(rows)
    }

    /// Atomically allocate sequence numbers for a partition.
    /// Returns the `start_seq` (items get `start_seq` + 1, `start_seq` + 2, etc.).
    async fn allocate_sequences(
        &self,
        txn: &impl ConnectionTrait,
        backend: DbBackend,
        dialect: &Dialect,
        partition_id: i64,
        count: i64,
    ) -> Result<i64, OutboxError> {
        match dialect.allocate_sequences() {
            AllocSql::UpdateReturning(sql) => {
                // Pg/SQLite: UPDATE ... RETURNING — $1 = partition_id, $2 = count
                let row = txn
                    .query_one(Statement::from_sql_and_values(
                        backend,
                        sql,
                        [partition_id.into(), count.into()],
                    ))
                    .await?
                    .ok_or_else(|| {
                        OutboxError::Database(sea_orm::DbErr::Custom(
                            "UPDATE RETURNING returned no row for sequence allocation".to_owned(),
                        ))
                    })?;
                let start_seq: i64 = row.try_get_by_index(0).map_err(|e| {
                    OutboxError::Database(sea_orm::DbErr::Custom(format!("start_seq column: {e}")))
                })?;
                Ok(start_seq)
            }
            AllocSql::UpdateThenSelect { update, select } => {
                // MySQL: UPDATE then SELECT
                // ? order: (count, partition_id) matching SQL occurrence
                txn.execute(Statement::from_sql_and_values(
                    backend,
                    update,
                    [count.into(), partition_id.into()],
                ))
                .await?;
                let row = txn
                    .query_one(Statement::from_sql_and_values(
                        backend,
                        select,
                        [count.into(), partition_id.into()],
                    ))
                    .await?
                    .ok_or_else(|| {
                        OutboxError::Database(sea_orm::DbErr::Custom(
                            "SELECT returned no row for sequence allocation".to_owned(),
                        ))
                    })?;
                let start_seq: i64 = row.try_get_by_index(0).map_err(|e| {
                    OutboxError::Database(sea_orm::DbErr::Custom(format!("start_seq column: {e}")))
                })?;
                Ok(start_seq)
            }
        }
    }
}
