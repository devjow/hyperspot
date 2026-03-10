use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

use super::handler::{
    Handler, MessageHandler, PerMessageAdapter, TransactionalHandler, TransactionalMessageHandler,
};
use super::manager::{DeferredSpawnFactory, OutboxBuilder, QueueDeclaration};
use super::processor::PartitionProcessor;
use super::strategy::{DecoupledStrategy, TransactionalStrategy, generate_worker_id};
use super::types::{Partitions, QueueConfig};
use crate::Db;

/// Builder for registering a queue with per-queue configuration.
///
/// Obtained via [`OutboxBuilder::queue`]. Terminal methods (`transactional`,
/// `decoupled`, `batch_transactional`, `batch_decoupled`) register the queue
/// and return the parent [`OutboxBuilder`] for chaining.
pub struct QueueBuilder {
    builder: OutboxBuilder,
    name: String,
    partitions: Partitions,
    config: QueueConfig,
}

impl QueueBuilder {
    pub(crate) fn new(builder: OutboxBuilder, name: String, partitions: Partitions) -> Self {
        Self {
            builder,
            name,
            partitions,
            config: QueueConfig::default(),
        }
    }

    /// Lease duration for decoupled mode partition locks. Ignored for
    /// transactional mode.
    #[must_use]
    pub fn lease_duration(mut self, d: Duration) -> Self {
        self.config.lease_duration = d;
        self
    }

    /// Max partitions processed concurrently within this queue.
    #[must_use]
    pub fn max_concurrent_partitions(mut self, n: usize) -> Self {
        self.config.max_concurrent_partitions = n;
        self
    }

    /// Messages per handler call per partition.
    #[must_use]
    pub fn msg_batch_size(mut self, n: u32) -> Self {
        self.config.msg_batch_size = n;
        self
    }

    /// Base delay for exponential backoff on retry.
    #[must_use]
    pub fn backoff_base(mut self, d: Duration) -> Self {
        self.config.backoff_base = d;
        self
    }

    /// Maximum delay for exponential backoff on retry.
    #[must_use]
    pub fn backoff_max(mut self, d: Duration) -> Self {
        self.config.backoff_max = d;
        self
    }

    /// Register a single-message transactional handler (common case).
    ///
    /// Forces `msg_batch_size = 1` — the `PerMessageAdapter` adapter processes
    /// one message at a time, so fetching larger batches would be wasteful.
    #[must_use]
    pub fn transactional(
        mut self,
        handler: impl TransactionalMessageHandler + 'static,
    ) -> OutboxBuilder {
        self.config.msg_batch_size = 1;
        self.batch_transactional(PerMessageAdapter::new(handler))
    }

    /// Register a single-message decoupled handler (common case).
    ///
    /// Forces `msg_batch_size = 1` — the `PerMessageAdapter` adapter processes
    /// one message at a time, so fetching larger batches would be wasteful.
    #[must_use]
    pub fn decoupled(mut self, handler: impl MessageHandler + 'static) -> OutboxBuilder {
        self.config.msg_batch_size = 1;
        self.batch_decoupled(PerMessageAdapter::new(handler))
    }

    /// Register a batch transactional handler (advanced).
    #[must_use]
    pub fn batch_transactional(
        self,
        handler: impl TransactionalHandler + 'static,
    ) -> OutboxBuilder {
        let handler = Arc::new(handler);
        let config = self.config.clone();

        let make_spawn_fn: DeferredSpawnFactory = Box::new(move |pid, _outbox| {
            let strategy =
                TransactionalStrategy::new(Box::new(ArcTransactionalHandler(Arc::clone(&handler))));
            let config = config.clone();

            Box::new(
                move |db: Db,
                      cancel: CancellationToken,
                      notify: Arc<Notify>,
                      sem: Arc<Semaphore>| {
                    let processor = PartitionProcessor::new(strategy, pid, config, notify, sem);
                    tokio::spawn(async move {
                        if let Err(e) = processor.run(&db, cancel).await {
                            tracing::error!(error = %e, partition_id = pid, "partition processor exited with error");
                        }
                    })
                },
            )
        });

        let mut builder = self.builder;
        builder.queue_declarations.push(QueueDeclaration {
            name: self.name,
            partitions: self.partitions,
            config: self.config,
            make_spawn_fn,
        });
        builder
    }

    /// Register a batch decoupled handler (advanced).
    #[must_use]
    pub fn batch_decoupled(self, handler: impl Handler + 'static) -> OutboxBuilder {
        let handler = Arc::new(handler);
        let config = self.config.clone();
        let queue_name = self.name.clone();

        let make_spawn_fn: DeferredSpawnFactory = Box::new(move |pid, _outbox| {
            let worker_id = generate_worker_id(&queue_name);
            let strategy =
                DecoupledStrategy::new(Box::new(ArcHandler(Arc::clone(&handler))), worker_id);
            let config = config.clone();

            Box::new(
                move |db: Db,
                      cancel: CancellationToken,
                      notify: Arc<Notify>,
                      sem: Arc<Semaphore>| {
                    let processor = PartitionProcessor::new(strategy, pid, config, notify, sem);
                    tokio::spawn(async move {
                        if let Err(e) = processor.run(&db, cancel).await {
                            tracing::error!(error = %e, partition_id = pid, "partition processor exited with error");
                        }
                    })
                },
            )
        });

        let mut builder = self.builder;
        builder.queue_declarations.push(QueueDeclaration {
            name: self.name,
            partitions: self.partitions,
            config: self.config,
            make_spawn_fn,
        });
        builder
    }
}

// Arc wrappers to share handler across partitions

struct ArcTransactionalHandler<H: TransactionalHandler>(Arc<H>);

#[async_trait::async_trait]
impl<H: TransactionalHandler> TransactionalHandler for ArcTransactionalHandler<H> {
    async fn handle(
        &self,
        txn: &dyn sea_orm::ConnectionTrait,
        msgs: &[super::handler::OutboxMessage],
        cancel: CancellationToken,
    ) -> super::handler::HandlerResult {
        self.0.handle(txn, msgs, cancel).await
    }

    fn processed_count(&self) -> Option<usize> {
        self.0.processed_count()
    }
}

struct ArcHandler<H: Handler>(Arc<H>);

#[async_trait::async_trait]
impl<H: Handler> Handler for ArcHandler<H> {
    async fn handle(
        &self,
        msgs: &[super::handler::OutboxMessage],
        cancel: CancellationToken,
    ) -> super::handler::HandlerResult {
        self.0.handle(msgs, cancel).await
    }

    fn processed_count(&self) -> Option<usize> {
        self.0.processed_count()
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::outbox::types::DEFAULT_LEASE_DURATION;

    #[test]
    fn queue_config_defaults() {
        let config = QueueConfig::default();
        assert_eq!(config.lease_duration, DEFAULT_LEASE_DURATION);
        assert_eq!(config.max_concurrent_partitions, usize::MAX);
        assert_eq!(config.msg_batch_size, 1);
    }

    #[test]
    fn partitions_count() {
        assert_eq!(Partitions::of(1).count(), 1);
        assert_eq!(Partitions::of(2).count(), 2);
        assert_eq!(Partitions::of(4).count(), 4);
        assert_eq!(Partitions::of(8).count(), 8);
        assert_eq!(Partitions::of(16).count(), 16);
        assert_eq!(Partitions::of(32).count(), 32);
        assert_eq!(Partitions::of(64).count(), 64);
    }
}
