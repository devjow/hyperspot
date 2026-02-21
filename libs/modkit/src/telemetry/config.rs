//! OpenTelemetry tracing and metrics configuration types
//!
//! These types define the configuration structure for OpenTelemetry distributed
//! tracing and metrics.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Tracing configuration for OpenTelemetry distributed tracing
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TracingConfig {
    pub enabled: bool,
    pub service_name: Option<String>,
    pub exporter: Option<Exporter>,
    pub sampler: Option<Sampler>,
    pub propagation: Option<Propagation>,
    pub resource: Option<HashMap<String, String>>,
    pub http: Option<HttpOpts>,
    pub logs_correlation: Option<LogsCorrelation>,
    #[serde(default)]
    pub metrics: MetricsConfig,
}

/// Metrics configuration for OpenTelemetry metrics collection
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub exporter: Option<Exporter>,
    /// Maximum number of distinct attribute combinations per instrument.
    /// When the limit is reached, new combinations are folded into an
    /// overflow data point.  `None` means the SDK default is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality_limit: Option<usize>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ExporterKind {
    #[default]
    OtlpGrpc,
    OtlpHttp,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Exporter {
    pub kind: ExporterKind,
    pub endpoint: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Sampler {
    ParentBasedAlwaysOn {},
    ParentBasedRatio {
        #[serde(skip_serializing_if = "Option::is_none")]
        ratio: Option<f64>,
    },
    AlwaysOn {},
    AlwaysOff {},
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Propagation {
    pub w3c_trace_context: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HttpOpts {
    pub inject_request_id_header: Option<String>,
    pub record_headers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogsCorrelation {
    pub inject_trace_ids_into_logs: Option<bool>,
}
