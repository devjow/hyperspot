use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::builder::QueueBuilder;
use super::core::Outbox;
use super::sequencer::Sequencer;
use super::types::{OutboxConfig, OutboxError, Partitions, QueueConfig, SequencerConfig};
use crate::Db;

/// Type-erased spawn function captured by the builder.
/// Called once at `start()` to spawn a per-partition processor task.
pub type SpawnFn =
    Box<dyn FnOnce(Db, CancellationToken, Arc<Notify>, Arc<Semaphore>) -> JoinHandle<()> + Send>;

/// Deferred spawn factory — creates `SpawnFn`s at `start()` time when
/// `Arc<Outbox>` is available.
pub type DeferredSpawnFactory = Box<dyn FnMut(i64, Arc<Outbox>) -> SpawnFn + Send>;

/// Deferred queue declaration — config + spawn factory, resolved at `start()`.
pub struct QueueDeclaration {
    pub(crate) name: String,
    pub(crate) partitions: Partitions,
    pub(crate) config: QueueConfig,
    pub(crate) make_spawn_fn: DeferredSpawnFactory,
}

/// Fluent builder for the outbox pipeline.
///
/// Entry point: [`Outbox::builder(db)`](Outbox::builder). Configure global
/// settings and register queues with handlers, then call
/// [`start()`](Self::start) to spawn background tasks.
///
/// ```ignore
/// let handle = Outbox::builder(db)
///     .poll_interval(Duration::from_millis(100))
///     .queue("orders", Partitions::of(4))
///         .decoupled(my_handler)
///     .start().await?;
/// // enqueue via handle.outbox()
/// handle.stop().await;
/// ```
pub struct OutboxBuilder {
    db: Db,
    sequencer_batch_size: u32,
    poll_interval: Duration,
    pub(crate) queue_declarations: Vec<QueueDeclaration>,
}

impl OutboxBuilder {
    pub(crate) fn new(db: Db) -> Self {
        Self {
            db,
            sequencer_batch_size: super::types::DEFAULT_SEQUENCER_BATCH_SIZE,
            poll_interval: super::types::DEFAULT_POLL_INTERVAL,
            queue_declarations: Vec::new(),
        }
    }

    /// Sequencer batch size (rows per cycle). Default: 100.
    #[must_use]
    pub fn sequencer_batch_size(mut self, n: u32) -> Self {
        self.sequencer_batch_size = n;
        self
    }

    /// Safety net fallback poll interval for both the sequencer and
    /// per-partition processors. Default: 1s.
    #[must_use]
    pub fn poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = d;
        self
    }

    /// Begin building a queue registration.
    #[must_use]
    pub fn queue(self, name: &str, partitions: Partitions) -> QueueBuilder {
        QueueBuilder::new(self, name.to_owned(), partitions)
    }

    /// Spawn background tasks and return a handle to the running pipeline.
    ///
    /// Registers all queues in the database, creates the sequencer and
    /// per-partition processors, then starts them as background tasks.
    ///
    /// # Errors
    ///
    /// Returns an error if queue registration fails (DB operation).
    pub async fn start(mut self) -> Result<OutboxHandle, OutboxError> {
        let sequencer_notify = Arc::new(Notify::new());
        let config = OutboxConfig {
            sequencer: SequencerConfig {
                batch_size: self.sequencer_batch_size,
                poll_interval: self.poll_interval,
            },
        };
        let outbox = Arc::new(Outbox::new(config, Arc::clone(&sequencer_notify)));

        let cancel = CancellationToken::new();
        let mut handles = Vec::new();
        let partition_notify: DashMap<i64, Arc<Notify>> = DashMap::new();

        // Register queues and create spawn closures
        for decl in &mut self.queue_declarations {
            // Apply global poll_interval to each queue
            decl.config.poll_interval = self.poll_interval;

            outbox
                .register_queue(&self.db, &decl.name, decl.partitions.count())
                .await?;

            let partition_ids = outbox.partition_ids_for_queue(&decl.name);
            let sem = Arc::new(Semaphore::new(
                decl.config
                    .max_concurrent_partitions
                    .min(Semaphore::MAX_PERMITS),
            ));

            for &pid in &partition_ids {
                let notify = Arc::new(Notify::new());
                partition_notify.insert(pid, Arc::clone(&notify));
                let spawn_fn = (decl.make_spawn_fn)(pid, Arc::clone(&outbox));
                let handle = spawn_fn(self.db.clone(), cancel.clone(), notify, Arc::clone(&sem));
                handles.push(handle);
            }
        }

        // Collect per-partition notify map for the sequencer
        let mut notify_map: HashMap<i64, Arc<Notify>> = HashMap::new();
        for entry in &partition_notify {
            notify_map.insert(*entry.key(), Arc::clone(entry.value()));
        }
        let notify_map = Arc::new(notify_map);

        // Spawn sequencer
        let sequencer = Sequencer::new(
            outbox.config().sequencer.clone(),
            Arc::clone(&outbox),
            Arc::clone(&sequencer_notify),
        );
        sequencer.set_partition_notify(Arc::clone(&notify_map));
        let seq_cancel = cancel.clone();
        let seq_db = self.db.clone();

        let seq_handle = tokio::spawn(async move {
            if let Err(e) = sequencer.run(&seq_db, seq_cancel).await {
                tracing::error!(error = %e, "sequencer exited with error");
            }
        });
        handles.push(seq_handle);

        Ok(OutboxHandle {
            outbox,
            cancel,
            handles,
        })
    }
}

/// A running outbox pipeline. Obtained by calling [`OutboxBuilder::start()`].
///
/// Provides access to the [`Outbox`] for enqueue operations and a
/// [`stop()`](Self::stop) method for graceful shutdown.
pub struct OutboxHandle {
    outbox: Arc<Outbox>,
    cancel: CancellationToken,
    handles: Vec<JoinHandle<()>>,
}

impl OutboxHandle {
    /// Returns the outbox for enqueue operations.
    #[must_use]
    pub fn outbox(&self) -> &Arc<Outbox> {
        &self.outbox
    }

    /// Cancel background tasks and join all handles. Consumes self.
    pub async fn stop(self) {
        self.cancel.cancel();
        for handle in self.handles {
            drop(handle.await);
        }
    }

    /// Access the cancellation token (for composing with external shutdown).
    #[must_use]
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
}
