//! Telemetry utilities for OpenTelemetry integration
//!
//! This module provides utilities for setting up and configuring
//! OpenTelemetry tracing layers for distributed tracing.

pub mod config;
pub mod init;
pub mod throttled_log;

pub use config::{
    Exporter, HttpOpts, LogsCorrelation, MetricsConfig, Propagation, Sampler, TracingConfig,
};
#[cfg(feature = "otel")]
pub use init::init_metrics;
pub use init::{init_tracing, shutdown_tracing};
pub use throttled_log::ThrottledLog;
