use std::time::Duration;

use thiserror::Error;

/// Default batch size for the sequencer (rows per cycle).
pub const DEFAULT_SEQUENCER_BATCH_SIZE: u32 = 100;

/// Default poll interval (safety net fallback for sequencer and processors).
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Default message batch size (messages per handler call per partition).
pub const DEFAULT_MSG_BATCH_SIZE: u32 = 1;

/// Default backoff base delay.
pub const DEFAULT_BACKOFF_BASE: Duration = Duration::from_secs(1);

/// Default backoff maximum delay.
pub const DEFAULT_BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Default lease duration for decoupled mode partition locks.
pub const DEFAULT_LEASE_DURATION: Duration = Duration::from_secs(30);

/// Number of partitions for a queue. Must be a power of 2 in `1..=64`.
///
/// ```
/// use modkit_db::outbox::Partitions;
/// let p = Partitions::of(4);
/// assert_eq!(p.count(), 4);
/// ```
///
/// Invalid values panic at compile time when used as a const:
/// ```compile_fail
/// use modkit_db::outbox::Partitions;
/// const BAD: Partitions = Partitions::of(3); // not a power of 2
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Partitions(u16);

impl Partitions {
    /// Create a partition count.
    ///
    /// # Panics
    ///
    /// Panics if `n` is not a power of 2 in `1..=64`.
    #[must_use]
    pub const fn of(n: u16) -> Self {
        assert!(
            n >= 1 && n <= 64 && n.is_power_of_two(),
            "partition count must be a power of 2 between 1 and 64"
        );
        Self(n)
    }

    /// Returns the numeric partition count.
    #[must_use]
    pub const fn count(self) -> u16 {
        self.0
    }
}

/// Identifier for an enqueued outbox message (the `modkit_outbox_incoming.id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutboxMessageId(pub i64);

/// A single message to enqueue in the outbox.
///
/// Used with [`Outbox::enqueue_batch`] to enqueue multiple messages in one call.
/// Each message specifies its target partition, the message payload (owned), and
/// a payload type string (borrowed — typically a static string like a MIME type
/// or schema identifier).
#[derive(Debug)]
pub struct EnqueueMessage<'a> {
    /// Target partition index (0-based, within the queue's partition count).
    pub partition: u32,
    /// Message payload bytes. Ownership is transferred to the outbox.
    pub payload: Vec<u8>,
    /// Type tag for the payload (e.g. `"application/json"`, schema name).
    pub payload_type: &'a str,
}

/// Errors from the outbox subsystem.
#[derive(Debug, Error)]
pub enum OutboxError {
    #[error("queue '{0}' is not registered")]
    QueueNotRegistered(String),

    #[error("partition {partition} is out of range for queue '{queue}' (max {max})")]
    PartitionOutOfRange {
        queue: String,
        partition: u32,
        max: u32,
    },

    #[error("payload size {size} exceeds maximum {max}")]
    PayloadTooLarge { size: usize, max: usize },

    #[error("partition count mismatch for queue '{queue}': expected {expected}, found {found}")]
    PartitionCountMismatch {
        queue: String,
        expected: u16,
        found: usize,
    },

    #[error(transparent)]
    Database(#[from] sea_orm::DbErr),
}

/// Configuration for the outbox subsystem.
#[derive(Debug, Clone, Default)]
pub struct OutboxConfig {
    pub sequencer: SequencerConfig,
}

/// Configuration for the sequencer background task.
#[derive(Debug, Clone)]
pub struct SequencerConfig {
    pub batch_size: u32,
    pub poll_interval: Duration,
}

impl Default for SequencerConfig {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_SEQUENCER_BATCH_SIZE,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

/// Per-queue configuration with sensible defaults.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Lease duration for decoupled mode partition locks.
    /// Ignored for transactional mode. Default: 30s.
    pub lease_duration: Duration,
    /// Max partitions processed concurrently within this queue.
    /// Default: `usize::MAX` (all partitions).
    pub max_concurrent_partitions: usize,
    /// Messages per handler call per partition. Default: 1.
    pub msg_batch_size: u32,
    /// Safety net fallback poll interval per partition. Default: 1s.
    /// Not exposed in `QueueBuilder` — always uses the default. Override
    /// only internally (e.g. integration tests).
    pub(crate) poll_interval: Duration,
    /// Base delay for exponential backoff on retry. Default: 1s.
    pub backoff_base: Duration,
    /// Maximum delay for exponential backoff on retry. Default: 60s.
    pub backoff_max: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            lease_duration: DEFAULT_LEASE_DURATION,
            max_concurrent_partitions: usize::MAX,
            msg_batch_size: DEFAULT_MSG_BATCH_SIZE,
            poll_interval: DEFAULT_POLL_INTERVAL,
            backoff_base: DEFAULT_BACKOFF_BASE,
            backoff_max: DEFAULT_BACKOFF_MAX,
        }
    }
}
