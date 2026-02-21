use std::sync::OnceLock;

use crate::telemetry::TracingConfig;

#[cfg(feature = "otel")]
use {
    crate::telemetry::config::ExporterKind,
    anyhow::Context,
    opentelemetry_otlp::{Protocol, WithExportConfig, WithHttpConfig, WithTonicConfig},
    opentelemetry_sdk::metrics::SdkMeterProvider,
};

#[cfg(feature = "otel")]
static METRICS_INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// Build a [`SdkMeterProvider`] from [`MetricsConfig::exporter`] settings and
/// register it as the global meter provider via [`init_metrics`].
///
/// The OTLP endpoint, headers, and timeout are taken from
/// `TracingConfig::metrics.exporter`, allowing metrics to use a different
/// collector endpoint than traces.
///
/// This function is guarded by [`OnceLock`] — the provider is built and
/// registered at most once; subsequent calls return the cached result.
///
/// Returns `Ok(())` when metrics are successfully initialized, or an `Err` when
/// metrics configuration is missing/disabled or the exporter fails to build.
///
/// # Errors
///
/// - Metrics `enabled == false` (propagated from `init_metrics`).
/// - The OTLP metric exporter cannot be constructed.
#[cfg(feature = "otel")]
pub fn init_metrics_provider(cfg: &TracingConfig) -> anyhow::Result<()> {
    METRICS_INIT
        .get_or_init(|| do_init_metrics_provider(cfg).map_err(|e| e.to_string()))
        .clone()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(feature = "otel")]
fn do_init_metrics_provider(cfg: &TracingConfig) -> anyhow::Result<()> {
    let metrics_cfg = &cfg.metrics;

    let (kind, endpoint, timeout) = {
        let (k, ep) = metrics_cfg.exporter.as_ref().map_or_else(
            || (ExporterKind::OtlpGrpc, "http://127.0.0.1:4317".to_owned()),
            |e| {
                (
                    e.kind,
                    e.endpoint
                        .clone()
                        .unwrap_or_else(|| "http://127.0.0.1:4317".to_owned()),
                )
            },
        );
        let t = metrics_cfg
            .exporter
            .as_ref()
            .and_then(|e| e.timeout_ms)
            .map(std::time::Duration::from_millis);
        (k, ep, t)
    };

    // Build OTLP metric exporter matching the configured transport
    let exporter = if matches!(kind, ExporterKind::OtlpHttp) {
        let mut b = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(&endpoint);
        if let Some(t) = timeout {
            b = b.with_timeout(t);
        }
        if let Some(headers) =
            crate::telemetry::init::build_headers_from_cfg_and_env(metrics_cfg.exporter.as_ref())
        {
            b = b.with_headers(headers);
        }
        b.build().context("build OTLP HTTP metric exporter")?
    } else {
        let mut b = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint);
        if let Some(t) = timeout {
            b = b.with_timeout(t);
        }
        if let Some(md) =
            crate::telemetry::init::build_metadata_from_cfg_and_env(metrics_cfg.exporter.as_ref())
        {
            b = b.with_metadata(md);
        }
        b.build().context("build OTLP gRPC metric exporter")?
    };

    // Build resource with service name (reuse tracing helper)
    let resource = crate::telemetry::init::build_resource(cfg);

    // Build the SdkMeterProvider with periodic exporter
    let mut builder = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(resource);

    // Apply a global cardinality limit when configured
    if let Some(limit) = metrics_cfg.cardinality_limit {
        builder = builder.with_view(move |_: &opentelemetry_sdk::metrics::Instrument| {
            opentelemetry_sdk::metrics::Stream::builder()
                .with_cardinality_limit(limit)
                .build()
                .ok()
        });
    }

    let provider = builder.build();

    // Delegate to init_metrics which validates config and sets global provider
    crate::telemetry::init::init_metrics(metrics_cfg, provider)?;

    Ok(())
}

/// No-op when the `otel` feature is disabled.
///
/// # Errors
/// Always returns an error indicating the feature is disabled.
#[cfg(not(feature = "otel"))]
pub fn init_metrics_provider(_cfg: &TracingConfig) -> anyhow::Result<()> {
    Err(anyhow::anyhow!("otel feature is disabled"))
}
