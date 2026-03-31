# Technical Design — Usage Collector


<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
  - [3.8 Observability](#38-observability)
- [4. Additional context](#4-additional-context)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

Usage Collector follows the ModKit Gateway + Plugins pattern. The gateway module (`usage-collector`) is the single centralized service that receives usage records, enforces tenant isolation, and delegates all storage operations to the active plugin. Backend-specific persistence and query logic for ClickHouse or TimescaleDB is encapsulated in storage plugins that register via the GTS type system and are selected by operator configuration. The gateway contains no backend-specific logic.

The SDK crate (`usage-collector-sdk`) defines two trait boundaries: `UsageCollectorClientV1` for usage sources and consumers, and `UsageCollectorPluginClientV1` for storage backend implementations. When a usage source emits a record, the SDK persists it to the source's local database within the same transaction as the caller's domain operation (transactional outbox pattern). The SDK initializes and owns an `OutboxHandle` whose background pipeline automatically delivers enqueued records to the collector gateway, providing at-least-once delivery even when the collector is temporarily unavailable.

Once records arrive at the gateway, they are persisted via the active storage plugin and become available for aggregation and raw queries. All query operations are scoped to the authenticated tenant derived from the caller's SecurityContext. For ingest, tenant attribution is PDP-authorized at emit time: standard sources emit for their own tenant (derived from SecurityContext); sources explicitly authorized by the PDP to act on behalf of multiple tenants supply the target tenant in the record, which is validated against PDP-returned constraints before the outbox INSERT and re-validated at gateway ingest as a defense-in-depth check.

### 1.2 Architecture Drivers

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-usage-collector-fr-ingestion` | SDK `emit()` persists record to local outbox within the caller's transaction; outbox library background pipeline delivers to gateway |
| `cpt-cf-usage-collector-fr-idempotency` | Counter records require a non-null idempotency key; `emit()` rejects counter records without one (`EmitError::MissingIdempotencyKey`) before any outbox INSERT; gauge records accept a null key; idempotency key is carried in the outbox payload; storage plugin performs idempotent upsert on non-null keys at delivery |
| `cpt-cf-usage-collector-fr-delivery-guarantee` | Transactional outbox in source's local DB; outbox library retries with exponential backoff; messages that exhaust retry budget are moved to the dead-letter store and surfaced via monitoring |
| `cpt-cf-usage-collector-fr-counter-semantics` | `ScopedUsageCollectorClientV1.emit()` rejects counter records with a negative value or a missing idempotency key before inserting the outbox row; delta records are stored per-event; the persistent total for a (tenant, metric) pair is the SUM of all active delta records — see §3.7 Counter Accumulation for the full strategy including plugin-level acceleration options |
| `cpt-cf-usage-collector-fr-gauge-semantics` | Gauge records carry `metric_kind = gauge`; `emit()` applies no monotonicity validation; storage plugin stores values as-is |
| `cpt-cf-usage-collector-fr-tenant-attribution` | `emit()` uses `record.tenant_id` as supplied by the caller and validates it against PDP constraints in `EmitAuthorization`: if the PDP returned a tenant `In` constraint, `record.tenant_id` must be a member of the allowed set; if no tenant constraint was returned, `record.tenant_id` must equal `ctx.subject_tenant_id()` (preserving standard single-tenant behavior). The validated `tenant_id` is stamped into the outbox row. Gateway validates tenant attribution on ingest: when `record.tenant_id ≠ ctx.subject_tenant_id()`, the gateway calls the PDP with `context_tenant_id(record.tenant_id)` and action `"emit_on_behalf"` to re-authorize before delegating to the plugin. |
| `cpt-cf-usage-collector-fr-resource-attribution` | `UsageRecord` carries optional `resource_id` and `resource_type` fields; persisted in outbox payload and storage backend; gateway and plugin pass them through without interpretation |
| `cpt-cf-usage-collector-fr-subject-attribution` | `ScopedUsageCollectorClientV1.emit()` captures `subject_id` and `subject_type` from the authenticated SecurityContext and includes them in the outbox payload; never accepted from request payload |
| `cpt-cf-usage-collector-fr-tenant-isolation` | Gateway enforces tenant scoping on all read and write operations; plugins filter by tenant ID; system fails closed on authorization failure |
| `cpt-cf-usage-collector-fr-ingestion-authorization` | `ScopedUsageCollectorClientV1.authorize_emit()` calls the platform PDP before any transaction opens; returned `EmitAuthorization` carries PDP constraints (including an optional tenant `In` constraint for sources authorized to emit for multiple tenants), source-level rate limit quota snapshot, and the registered usage type schema — all evaluated in-memory by `emit()` before the outbox INSERT; denied or constraint-violating emissions are rejected before any record is persisted |
| `cpt-cf-usage-collector-fr-pluggable-storage` | Gateway resolves active plugin via GTS; plugin implements write and read traits; operator selects backend via configuration |
| `cpt-cf-usage-collector-fr-query-aggregation` | Gateway enforces PDP decision + constraint on each query; exposes aggregation query API with optional filters (usage type, subject, resource, source) and configurable GROUP BY dimensions (time bucket, usage type, subject, resource, source); delegates to plugin, which pushes aggregation (SUM, COUNT, MIN, MAX, AVG) and grouping down to the storage engine |
| `cpt-cf-usage-collector-fr-query-raw` | Gateway enforces PDP decision + constraint on each query; exposes raw query API with optional filters (usage type, subject, resource) and cursor-based pagination; delegates to plugin |
| `cpt-cf-usage-collector-fr-record-metadata` | Gateway enforces configurable 8 KB metadata size limit at ingest; plugin stores metadata as-is in a dedicated payload column; metadata is returned in query results without interpretation |
| `cpt-cf-usage-collector-fr-retention-policies` | Gateway manages retention policy configuration (global, per-tenant, per-usage-type); plugin enforces retention via storage-native TTL or scheduled deletion |
| `cpt-cf-usage-collector-fr-backfill-api` | Gateway accepts backfill requests and bulk-inserts historical records via the plugin; gateway calls the platform PDP to authorize the caller for the specified `tenant_id` before accepting the request — callers whose SecurityContext tenant differs from the requested `tenant_id` require a dedicated cross-tenant backfill PDP permission and the PDP must return a constraint that includes the target tenant; a PDP denial or constraint violation returns `403 PERMISSION_DENIED` immediately; existing records in the range are not modified; backfill path operates with independent rate limits; gateway emits `WriteAuditEvent` to `audit_service` on completion |
| `cpt-cf-usage-collector-fr-event-amendment` | Gateway exposes amendment and deactivation endpoints for individual events; plugin updates record status fields; gateway emits `WriteAuditEvent` to `audit_service` on each operation |
| `cpt-cf-usage-collector-fr-audit` | Gateway emits a structured `WriteAuditEvent` to platform `audit_service` on every operator-initiated write (backfill, amendment, deactivation); event includes common envelope (operation, actor_id, tenant_id, timestamp, justification) and operation-specific context; no audit data is stored locally |
| `cpt-cf-usage-collector-fr-backfill-boundaries` | Gateway enforces configurable maximum backfill window (default 90 days) and future timestamp tolerance (default 5 minutes); requests beyond the maximum window require elevated authorization verified before any plugin operation |
| `cpt-cf-usage-collector-fr-metadata-exposure` | Gateway exposes a watermark API endpoint; plugin queries per-source and per-tenant event counts and latest ingested timestamps to populate the response |
| `cpt-cf-usage-collector-fr-type-validation` | `authorize_emit()` fetches the registered usage type schema from types-registry; `emit()` validates the record against it in-memory before the outbox INSERT, rejecting invalid records immediately; gateway retains schema validation as a defense-in-depth check on delivered records before delegating to the plugin |
| `cpt-cf-usage-collector-fr-custom-units` | Gateway exposes a usage type registration endpoint that delegates to types-registry; operator configures authorization policies per usage type after registration |
| `cpt-cf-usage-collector-fr-rate-limiting` | `authorize_emit()` fetches the current source-level emission quota and window snapshot for the source module before any transaction opens; `emit()` evaluates the quota in-memory and rejects the emission before the outbox INSERT if the source quota is exhausted; gateway enforces per-(source, tenant) quota on ingest when `record.tenant_id` is known; rejections are surfaced via operational monitoring; rate limit configuration is managed by the platform operator |

#### NFR Allocation

| NFR ID | NFR Summary | Allocated To | Design Response | Verification Approach |
|--------|-------------|--------------|-----------------|----------------------|
| `cpt-cf-usage-collector-nfr-query-latency` | Aggregation queries over 30-day range complete within 500ms at p95 | Storage Plugin | Aggregation pushed down to storage engine; plugins SHOULD maintain pre-aggregated acceleration structures (ClickHouse `AggregatingMergeTree` view, TimescaleDB continuous aggregate) to meet this threshold at production record volumes — see §3.7 Counter Accumulation for the full strategy and consistency model | Benchmark test with 30-day synthetic dataset at target production record volume at p95 |
| `cpt-cf-usage-collector-nfr-availability` | 99.95% monthly availability for ingestion endpoints | Gateway, SDK | Stateless gateway enables horizontal replication; the SDK's outbox pipeline absorbs temporary gateway unavailability, preserving record capture continuity; liveness probes and graceful shutdown ensure fast instance recovery | SLO tracking on gateway uptime; synthetic availability probes on ingestion endpoint |
| `cpt-cf-usage-collector-nfr-throughput` | ≥ 10,000 records/sec sustained ingestion | Gateway, Storage Plugin | Gateway dispatches records to the storage plugin for batched idempotent upsert; plugin pushes bulk INSERTs to append-optimized storage (ClickHouse column store, TimescaleDB hypertable); stateless gateway scales horizontally | Sustained load test at 10,000 records/sec for 10 minutes; verify no records lost and no latency degradation |
| `cpt-cf-usage-collector-nfr-ingestion-latency` | Ingestion completes within 200ms at p95 | SDK | `emit()` is a local DB INSERT within the caller's transaction — no network I/O on the critical emission path; p95 latency is bounded by local DB write speed, well within the 200ms threshold | Benchmark `emit()` p95 latency under representative concurrent load |
| `cpt-cf-usage-collector-nfr-workload-isolation` | Ingestion p95 ≤ 200ms during concurrent query and retention workloads | Gateway, Storage Plugin | Query and retention workloads run on separate handler paths from the ingest handler; retention enforcement runs as a scheduled background task with lower priority; plugins leverage storage-native query prioritization (ClickHouse query priority classes, TimescaleDB resource groups) to prevent analytical workloads from starving ingest writes | Measure ingest p95 under concurrent aggregation queries and retention enforcement; verify it remains within `cpt-cf-usage-collector-nfr-ingestion-latency` threshold |
| `cpt-cf-usage-collector-nfr-authentication` | Zero unauthenticated API access | Gateway | All gateway endpoints require a valid authenticated SecurityContext; unauthenticated requests are rejected by the ModKit request pipeline before any handler or plugin is invoked | Integration tests verifying all endpoints return a rejection for requests without valid authentication credentials |
| `cpt-cf-usage-collector-nfr-authorization` | Zero unauthorized data access or write | Gateway, SDK | SDK `authorize_emit()` contacts the platform PDP before any transaction opens; gateway enforces PDP authorization on all query, backfill, amendment, and deactivation endpoints and applies returned constraints as additional query filters; system fails closed on any authorization failure (`cpt-cf-usage-collector-principle-fail-closed`) | Integration tests verifying unauthorized ingestion, query, backfill, and amendment requests are rejected with no data exposed or modified |
| `cpt-cf-usage-collector-nfr-scalability` | Linear throughput scaling with added instances | Gateway | Gateway is stateless — all per-request state is carried in SecurityContext and request payload; horizontal scaling is achieved by adding gateway instances behind a load balancer with no coordination required | Load test demonstrating linear throughput increase as gateway instance count is increased from 1 to N |
| `cpt-cf-usage-collector-nfr-fault-tolerance` | Zero data loss for durably captured records during storage backend failures | SDK, Gateway | The SDK's outbox pipeline retries failed gateway deliveries with exponential backoff; gateway retries plugin persist calls on transient storage errors; records durably captured in the source outbox are guaranteed to eventually reach the storage backend; messages that exhaust retry budget are moved to the dead-letter store and surfaced via operational monitoring | Chaos test: take storage backend offline, verify outbox rows survive and are delivered on recovery with zero data loss |
| `cpt-cf-usage-collector-nfr-recovery` | RTO ≤ 15 minutes from storage backend recovery | SDK, Gateway | The outbox library resumes delivery automatically once the gateway becomes reachable again; gateway re-establishes plugin connection on storage recovery without restart; the RTO bound is determined by the outbox retry schedule — `backoff_max` MUST be configured below 15 minutes to meet this threshold | Chaos test: restore storage backend after outage, measure elapsed time from backend availability to full outbox drain and query availability; verify elapsed time ≤ 15 minutes |
| `cpt-cf-usage-collector-nfr-retention` | Configurable retention from 7 days to 7 years | Storage Plugin | Retention policy configuration is managed by the gateway; plugin enforces retention via storage-native TTL expressions (ClickHouse) or scheduled deletion (TimescaleDB); policies apply at global, per-tenant, and per-usage-type scope | Test retention enforcement for each plugin: records older than the configured duration are removed within the enforcement window |
| `cpt-cf-usage-collector-nfr-graceful-degradation` | Zero ingestion failures due to downstream consumer unavailability | SDK | `emit()` writes to the source's local database within the caller's transaction — no runtime dependency on the collector or any downstream consumer; the outbox pipeline delivers asynchronously, fully decoupling the source's emission path from collector and consumer uptime | Integration test: take the collector gateway offline; verify sources continue emitting without errors and records are delivered on gateway recovery |
| `cpt-cf-usage-collector-nfr-rpo` | RPO = 0 for all records for which `emit()` returned `Ok` | SDK | `emit()` calls `Outbox::enqueue()` within the caller's DB transaction — a record is durable the moment that transaction commits; no committed record can be lost by a subsequent gateway restart, dispatcher crash, or storage outage, because the outbox row persists independently in the source's local DB until confirmed delivered | Test: call `emit()` successfully, kill the gateway process immediately after commit, restart gateway; verify the record is delivered and queryable with no data loss |

#### Key ADRs

| ADR ID | Decision Summary |
|--------|-----------------|
| `cpt-cf-usage-collector-adr-scoped-emit-source` | `UsageCollectorClientV1.for_module()` returns a `ScopedUsageCollectorClientV1` bound to the source module's authoritative name; scoped client stamps source identity on every emit |
| `cpt-cf-usage-collector-adr-two-phase-emit-authz` | `authorize_emit()` calls the PDP before any DB transaction; `emit()` evaluates returned constraints in-memory inside the transaction — no network I/O on the critical emission path |

### 1.3 Architecture Layers

```
┌────────────────────────────────────────────────────────────────┐
│                    Usage Sources                               │
│            (LLM Gateway, Compute Service, etc.)                │
├────────────────────────────────────────────────────────────────┤
│  usage-collector-sdk  │  emit() + outbox enqueue + delivery    │
├────────────────────────────────────────────────────────────────┤
│  usage-collector      │  Gateway: ingest, query, tenant iso.   │
├────────────────────────────────────────────────────────────────┤
│  Plugins              │  Backend-specific storage adapters     │
│  ┌──────────────────────┐  ┌───────────────────────────────┐   │
│  │ clickhouse-plugin    │  │ timescaledb-plugin            │   │
│  └──────────────────────┘  └───────────────────────────────┘   │
├────────────────────────────────────────────────────────────────┤
│  External              │  ClickHouse, TimescaleDB              │
└────────────────────────────────────────────────────────────────┘
```

| Layer | Responsibility | Technology |
|-------|---------------|------------|
| SDK | Emit API; outbox enqueue; owns `OutboxHandle` whose background pipeline automatically delivers records to the gateway | Rust crate (`usage-collector-sdk`), modkit-db outbox |
| Gateway | Ingest, query, aggregation API; tenant isolation; plugin resolution | Rust crate (`usage-collector`), Axum |
| Plugins | Backend-specific record persistence and aggregation queries | Rust crates, ClickHouse / TimescaleDB drivers |
| External | Durable time-series storage and query execution | ClickHouse or TimescaleDB |

## 2. Principles & Constraints

### 2.1 Design Principles

#### Source-Side Persistence

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-principle-source-side-persistence`

Usage records are persisted in the source's local database at emit time, within the same transaction as the caller's domain operation. This guarantees no record is lost between the source producing it and the collector receiving it. The source never blocks on collector availability.

#### Plugin-Based Storage

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-principle-pluggable-storage`

All storage operations — write, aggregation query, and raw query — are delegated to the active storage plugin. The gateway contains no backend-specific logic. New backends are added by implementing the plugin trait and registering via GTS, without changes to the gateway.

#### PDP-Authorized Tenant Attribution

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-principle-tenant-from-ctx`

Tenant identity for usage attribution is always PDP-authorized — never freely accepted from request payloads. For standard sources, the SDK derives the tenant from `SecurityContext.subject_tenant_id()` and validates that `record.tenant_id` matches. For sources explicitly authorized by the PDP to emit on behalf of multiple tenants, the PDP returns a tenant `In` constraint; `emit()` validates `record.tenant_id` against that constraint before the outbox INSERT. The gateway re-validates tenant attribution independently on every ingest delivery. Queries are always scoped to the authenticated tenant from SecurityContext.

#### Fail-Closed Security

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-principle-fail-closed`

**ADRs**: `cpt-cf-usage-collector-adr-two-phase-emit-authz`

On any authorization failure, the system rejects the request. No fallback to permissive behavior. Queries without a valid authenticated SecurityContext are rejected before reaching the storage plugin.

#### Scoped Source Attribution

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-principle-scoped-source-attribution`

**ADRs**: `cpt-cf-usage-collector-adr-scoped-emit-source`

Each consuming module obtains a `ScopedUsageCollectorClientV1` by calling `for_module()` once at module initialization, passing its compile-time `MODULE_NAME` constant. The scoped client stamps every outbox row with the source module identity. Which metrics a source is permitted to emit is governed by authz policy. Source identity is never supplied per-call.

#### Two-Phase Emit

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-principle-two-phase-authz`

**ADRs**: `cpt-cf-usage-collector-adr-two-phase-emit-authz`

Any logic that requires communication with external modules is consolidated in phase 1 (`authorize_emit()`), called before any DB transaction opens. Phase 1 accumulates all constraints and state into the returned `EmitAuthorization` token. Phase 2 (`emit()`) applies every accumulated constraint in-memory inside the caller's DB transaction — no external calls on the critical emission path.

`EmitAuthorization` acts as a pre-fetched, in-memory contract for the emission. It currently carries:
- **PDP authorization constraints**: the platform PDP decision and any returned `Constraint` predicates, evaluated against the `UsageRecord` fields; for sources authorized to emit for multiple tenants, the PDP returns a tenant `In` constraint listing the allowed target tenant IDs
- **Rate limit state**: the source-level emission quota and current window snapshot for the source module
- **Usage type schema**: the registered schema for the metric being emitted, used for in-memory record validation

Any future requirement that depends on external state at emit time MUST be resolved in phase 1 and included in `EmitAuthorization`, not deferred to phase 2. A denial, quota exhaustion, or validation failure from `authorize_emit()` surfaces immediately to the caller as an error before any domain operation is committed.

### 2.2 Constraints

#### Outbox Infrastructure Required

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-constraint-outbox-infra`

The SDK requires the modkit-db outbox schema to be present in the source's local database. The schema is installed by running `outbox_migrations()` from `modkit_db::outbox` as part of the source module's migration set. Usage sources that do not have a local database managed by modkit-db cannot use the SDK directly.

#### Single Active Plugin

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-constraint-single-plugin`

Only one storage backend plugin is active at a time, selected by operator configuration. Simultaneous dual-backend writes and online backend migration are not supported in this version.

#### ModKit Architecture Compliance

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-constraint-modkit`

The usage-collector module follows the ModKit module specification: the gateway is a standard ModKit module with its own scoped `ClientHub`, uses `SecureConn` for all storage plugin database access, propagates `SecurityContext` on all inter-module and plugin calls, and follows the `NEW_MODULE` guideline for bootstrapping. Storage plugins register via the GTS type system and are resolved at runtime via the gateway's `ClientHub`. No module-level static globals; no direct database connections outside `SecureConn`.

#### SecurityContext Propagation

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-constraint-security-context`

`SecurityContext` must be propagated on all operations: ingest, query (aggregated and raw), backfill, amendment, deactivation, metadata watermarks, retention configuration, and unit registration. Subject identity is always derived from `SecurityContext`. Tenant ID for usage attribution is always PDP-authorized (see `cpt-cf-usage-collector-principle-tenant-from-ctx`); it is never accepted from request payloads without prior PDP authorization and is never passed as an unvalidated parameter between internal components.

#### Types Registry Delegation

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-constraint-types-registry`

All usage type schema definition and validation is delegated to types-registry. The Usage Collector does not own or define metric type schemas. Custom unit registration routes through the gateway's unit registration endpoint, which delegates entirely to types-registry. Storage plugin discovery uses GTS schemas registered by each plugin implementation.

#### No Business Logic

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-constraint-no-business-logic`

The Usage Collector does not implement pricing, rating, billing rules, invoice generation, quota enforcement, or business-purpose aggregation. Record metadata is stored as opaque JSON without indexing or interpretation. All business decisions are the responsibility of downstream consumers.

#### Encrypted Transport and Storage

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-constraint-encryption`

All plugin connections to ClickHouse and TimescaleDB MUST use TLS; connection parameters are managed via `SecureConn`. Storage backends MUST be configured to reject unencrypted connections. Encryption at rest is mandatory and governed by platform infrastructure policy (ClickHouse native AES-128 encryption or filesystem-level encryption; TimescaleDB encrypted tablespace or OS-level encryption). Encryption key management is the responsibility of the platform secret management system. Record deletion via retention enforcement (`cpt-cf-usage-collector-fr-retention-policies`) constitutes secure disposal; no per-module key rotation is required for non-PII usage data.

## 3. Technical Architecture

### 3.1 Domain Model

**Technology**: Rust structs

**Location**: [`modules/system/usage-collector-sdk/src/types.rs`](../../usage-collector-sdk/src/types.rs)

**Core Entities**:

| Entity | Description | Schema |
|--------|-------------|--------|
| `UsageRecord` | Single measurement of resource consumption: tenant ID, source module, metric kind (counter/gauge), metric name, numeric value, timestamp; idempotency key (required for counter metrics, optional for gauge metrics), resource ID/type, subject ID/type, metadata (opaque JSON); status (`active` \| `inactive`) | [`usage-collector-sdk/src/types.rs`](../../usage-collector-sdk/src/types.rs) |
| `AggregationQuery` | Query parameters: tenant ID (derived from SecurityContext), time range (mandatory), aggregation function (SUM/COUNT/MIN/MAX/AVG); optional filters: usage type, subject (subject_id, subject_type), resource (resource_id, resource_type), source; optional GROUP BY dimensions: time bucket granularity, usage type, subject, resource, source | [`usage-collector-sdk/src/types.rs`](../../usage-collector-sdk/src/types.rs) |
| `AggregationResult` | Result row: aggregation function applied, aggregated numeric value; dimension values present for each active GROUP BY dimension: time bucket start timestamp (when grouped by time), usage type (when grouped by usage type), subject ID + subject type (when grouped by subject), resource ID + resource type (when grouped by resource), source module (when grouped by source); absent dimensions are null | [`usage-collector-sdk/src/types.rs`](../../usage-collector-sdk/src/types.rs) |
| `RawQuery` | Raw record query parameters: tenant ID (derived from SecurityContext), time range (mandatory), optional pagination cursor; optional filters: usage type, subject (subject_id, subject_type), resource (resource_id, resource_type) | [`usage-collector-sdk/src/types.rs`](../../usage-collector-sdk/src/types.rs) |
| `EmitAuthorization` | Opaque token returned by `authorize_emit()`; carries all constraints evaluated in-memory by `emit()`: PDP authorization constraints (`Vec<Constraint>`, may include a tenant `In` constraint for sources authorized to emit for multiple tenants), source-level rate limit quota snapshot (emission count, window bounds, and configured limit for the source module), the registered usage type schema for the metric being emitted, and a monotonic issuance timestamp with a random nonce. `emit()` verifies the token has not exceeded its maximum age before evaluating any other constraint. | [`usage-collector-sdk/src/types.rs`](../../usage-collector-sdk/src/types.rs) |
| `RetentionPolicy` | Retention rule: scope (`global` \| `tenant` \| `usage-type`), target identifier, retention duration. The global policy is mandatory and cannot be deleted; it applies when no more-specific policy matches. Precedence: per-usage-type > per-tenant > global. | [`usage-collector-sdk/src/types.rs`](../../usage-collector-sdk/src/types.rs) |
| `BackfillOperation` | Backfill request: operator identity, target `tenant_id` (PDP-authorized: gateway verifies the caller is permitted to backfill for the specified tenant; cross-tenant operations require a dedicated PDP permission and the returned constraint must include the target tenant), usage type, time range, historical records to insert | [`usage-collector-sdk/src/types.rs`](../../usage-collector-sdk/src/types.rs) |
| `WriteAuditEvent` | Structured event emitted to platform `audit_service` for each operator-initiated write: operation type (`backfill` \| `amend` \| `deactivate`), actor_id, tenant_id, timestamp, justification, and a `WriteAuditContext` variant with operation-specific context — backfill: (usage type, time range, records added); amendment: (record ID, changed fields before/after); deactivation: (record ID). Not stored locally. | [`usage-collector-sdk/src/types.rs`](../../usage-collector-sdk/src/types.rs) |
| `UsageType` | Registered usage type schema: type identifier, metric kind, unit label, validation rules | types-registry |

**Relationships**:
- `UsageRecord` belongs to exactly one tenant via `tenant_id`
- `UsageRecord` optionally belongs to one resource via `resource_id`/`resource_type`
- `UsageRecord` optionally belongs to one subject via `subject_id`/`subject_type`, always derived from SecurityContext
- `UsageRecord` carries optional `metadata` (opaque JSON); persisted and returned as-is without interpretation
- `UsageRecord` status is `active` on creation; transitions to `inactive` on operator-initiated deactivation or amendment
- `AggregationResult` carries dimension values only for dimensions active in the originating `AggregationQuery`; a query with no GROUP BY returns a single row with all dimension fields null
- `AggregationQuery` always scopes to exactly one tenant; tenant ID is never supplied by the caller
- `RawQuery` always scopes to exactly one tenant; tenant ID is never supplied by the caller
- `RetentionPolicy` scopes to global, a specific tenant, or a specific usage type; plugin permanently deletes (hard delete) records beyond the retention duration; precedence per-usage-type > per-tenant > global; global policy is mandatory
- `WriteAuditEvent` is emitted to platform `audit_service` for every operator-initiated write (backfill, amendment, deactivation); not stored locally

### 3.2 Component Model

```mermaid
graph TD
    subgraph Source["Usage Source Process"]
        App[Application Code]
        SDK[usage-collector-sdk]
        LDB[(Local DB)]
    end

    subgraph Collector["Usage Collector"]
        GW[Gateway]
        Plugin[Storage Plugin]
    end

    subgraph Storage["Storage Backend"]
        DB[(ClickHouse / TimescaleDB)]
    end

    Consumer[Usage Consumer]

    App -->|emit| SDK
    SDK -->|enqueue within caller tx| LDB
    SDK -->|"deliver (outbox pipeline)"| GW
    GW -->|write| Plugin
    Plugin -->|persist| DB
    Consumer -->|query| GW
    GW -->|read| Plugin
    Plugin -->|aggregate / paginate| DB
```

#### Usage Collector SDK

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-component-sdk`


##### Why this component exists

Provides the public API for usage sources to emit records. Handles transactional outbox persistence in the source's local database, decoupling sources from collector availability.

##### Responsibility scope

- At SDK initialization: register the `"usage-records"` outbox queue (idempotent, 4 partitions), start the decoupled outbox pipeline with the SDK's internal delivery handler, and retain the resulting `OutboxHandle` for the process lifetime — no explicit shutdown coordination is required
- Expose `for_module(name) -> ScopedUsageCollectorClientV1` on `UsageCollectorClientV1`; each consuming module calls this once at initialization with its compile-time `MODULE_NAME` constant
- `ScopedUsageCollectorClientV1.authorize_emit(ctx, metric_name)` — called before any DB transaction opens; calls the platform PDP (authz constraints for the given metric, which may include a tenant `In` constraint for sources authorized to emit for multiple tenants), fetches the current source-level rate limit quota snapshot, and retrieves the registered usage type schema for `metric_name` from types-registry; returns an opaque `EmitAuthorization` token on success or an error on denial / quota exhaustion
- `ScopedUsageCollectorClientV1.emit(ctx, record, &EmitAuthorization)` — called inside the caller's DB transaction; first verifies the token has not exceeded its maximum age (`EmitError::AuthorizationExpired` on expiry); then validates metric semantics (rejects counter records with a negative value or a missing idempotency key), captures subject ID and type from SecurityContext, stamps source module identity, then evaluates all `EmitAuthorization` constraints in-memory in order: usage type schema validation, source-level rate limit quota check, PDP constraint satisfaction (including tenant validation: if a tenant `In` constraint is present, `record.tenant_id` must be a member of the allowed set; if no tenant constraint is present, `record.tenant_id` must equal `ctx.subject_tenant_id()`); rejects before the outbox enqueue on any failure; on success stamps the outbox row with `record.tenant_id` and calls `Outbox::enqueue()` within the caller's transaction
- Serialize the usage record (tenant ID, source module, metric kind, idempotency key, resource attribution, subject attribution) to bytes as the outbox payload with `payload_type = "usage-collector.record.v1"`
- Pass optional metadata field through to the payload without interpretation; enforce the configurable size limit (default 8 KB) before enqueuing
- Internally implement a `MessageHandler` that the outbox library calls for each ready message: deserialize the payload, assemble a gateway ingest request, call `persist()`, and return `HandlerResult::Success` on confirmation, `HandlerResult::Retry` on transient failure, or `HandlerResult::Reject` on permanent non-retriable failure (e.g. deserialization error, gateway 400); `backoff_max` MUST be configured below 15 minutes to satisfy `cpt-cf-usage-collector-nfr-recovery`

##### Responsibility boundaries

- Does NOT expose the `MessageHandler` implementation as a public API or a separate component
- Does NOT call the PDP inside an open DB transaction
- Does NOT interact with the storage backend directly

##### Related components (by ID)

- `cpt-cf-usage-collector-component-gateway` — delivery target for outbox messages

#### Gateway

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-component-gateway`

##### Why this component exists

The centralized entry point for receiving delivered records and serving aggregation and raw queries. Enforces tenant isolation on all operations and delegates all storage work to the active plugin.

##### Responsibility scope

- Accept ingest requests from the SDK's outbox delivery pipeline
- Derive and enforce tenant ID from SecurityContext on all query operations; on ingest, validate tenant attribution: when `record.tenant_id == ctx.subject_tenant_id()`, accept without additional check; when `record.tenant_id ≠ ctx.subject_tenant_id()`, call the PDP with `context_tenant_id(record.tenant_id)` and action `"emit_on_behalf"` — reject the record if the PDP denies (fail-closed); enforce per-(source, tenant) rate limits on ingest once the record's target tenant is known
- Resolve the active storage plugin via GTS
- Expose aggregation and raw query API to usage consumers
- Authorize each query via the platform PDP: verify the caller is permitted to query; apply PDP-returned constraints as additional query filters before delegating to the plugin
- Delegate all storage reads and writes to the resolved plugin
- Fail closed on authorization failures — no permissive fallback
- Enforce configurable per-call timeout for all plugin operations (default 5 s); if a plugin call does not complete within the timeout, the gateway returns an error and does not retry within the same request — for ingest, retry is handled by the outbox library on the SDK side
- Apply a circuit breaker per storage plugin instance: open the circuit after 5 consecutive plugin call failures within a 10-second window; return `503 Service Unavailable` while the circuit is open; probe with a single call after a configurable half-open interval (default 30 s)
- Enforce configurable metadata size limit (default 8 KB) at ingest; reject oversized records before delegating to plugin
- Validate each inbound record against the schema registered in types-registry before delegating to plugin (defense-in-depth; primary validation occurs at SDK `emit()` time before the outbox INSERT)
- Accept operator backfill requests; call the platform PDP to verify the caller is authorized to backfill for the specified `tenant_id` before accepting the request — a PDP denial or a `tenant_id` that violates the returned constraint returns `403 PERMISSION_DENIED` immediately; callers whose SecurityContext tenant differs from the requested `tenant_id` require a dedicated cross-tenant backfill PDP permission; validate time boundaries and authorization; enqueue each backfill record as a separate outbox message via `Outbox::enqueue_batch()` within the handler transaction; emit `WriteAuditEvent` to `audit_service` once all records are committed to the outbox; respond `202 Accepted` to the caller
- Internally implement a `MessageHandler` for the backfill outbox that the outbox library calls for each individual backfill record: deserialize the payload into a single `UsageRecord`, call `plugin.backfill_ingest()`, and return `HandlerResult::Success` on confirmation, `HandlerResult::Retry` on transient plugin failure, or `HandlerResult::Reject` on permanent failure; `backoff_max` bounds retry duration; delivery throughput to the plugin is naturally rate-limited by the outbox partition count and `msg_batch_size`
- Expose amendment and deactivation endpoints for individual usage records; emit `WriteAuditEvent` to `audit_service` on each operation
- Enforce configurable backfill time boundaries (max window, future tolerance); require elevated authorization for requests beyond the max window
- Manage retention policy configuration (global, per-tenant, per-usage-type) and trigger plugin enforcement
- Expose watermark API endpoint returning per-source and per-tenant event counts and latest timestamps
- Expose unit registration endpoint delegating to types-registry

##### Responsibility boundaries

- Does NOT contain backend-specific storage logic
- Does NOT implement aggregation algorithms — computation is pushed to the plugin and storage engine
- Does NOT persist regular ingest records locally — ingest records arrive pre-persisted from the source's outbox; backfill records are buffered in a gateway-local outbox before plugin delivery

##### Related components (by ID)

- `cpt-cf-usage-collector-component-sdk` — inbound delivery via the SDK's outbox pipeline
- `cpt-cf-usage-collector-component-storage-plugin` — delegates all storage operations to plugin

#### Storage Plugin

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-component-storage-plugin`

##### Why this component exists

Provides a backend-specific implementation for persisting and querying usage records. Decouples the gateway from any particular storage technology.

##### Responsibility scope

- Persist usage records with idempotent upsert keyed on idempotency key
- Execute aggregation queries (SUM, COUNT, MIN, MAX, AVG) grouped by time bucket within the storage engine
- Execute raw record queries with cursor-based pagination, filtered by tenant ID and time range
- Enforce tenant ID filtering on all queries
- Persist metadata field as-is alongside the usage record; include in query results without modification
- Bulk-insert historical records from backfill operations with idempotent upsert on `idempotency_key`
- Update individual record status for amendment (property update) and deactivation
- Enforce retention policies via storage-native TTL or scheduled deletion
- Return per-source and per-tenant event counts and latest ingested timestamps (watermarks)
- Filter active-only records by default; expose inactive records only when explicitly requested

##### Responsibility boundaries

- Does NOT enforce authorization — that is the gateway's responsibility
- Does NOT implement delivery logic — records arrive pre-delivered by the SDK's outbox pipeline
- Does NOT contain business logic (pricing, billing, quota decisions)

##### Related components (by ID)

- `cpt-cf-usage-collector-component-gateway` — called by gateway for all storage operations

### 3.3 API Contracts

#### SDK Trait (`UsageCollectorClientV1`)

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-interface-sdk-trait`

**Technology**: Rust trait in `usage-collector-sdk` crate
**Data Format**: Rust types (`UsageRecord`, `AggregationQuery`, `AggregationResult`, `RawQuery`, `PagedResult`)

**Operations**:

| Operation | Caller | Description |
|-----------|--------|-------------|
| `for_module(name)` | Usage source at init | Returns a `ScopedUsageCollectorClientV1` bound to the source module's authoritative name |
| `persist(records)` | SDK (outbox delivery handler) | Delivers a batch of records to the collector gateway; idempotent on `idempotency_key` |
| `query_aggregated(ctx, query)` | Usage consumer | Returns aggregated usage data scoped to the authenticated tenant |
| `query_raw(ctx, query)` | Usage consumer | Returns paginated raw usage records scoped to the authenticated tenant |

#### Scoped Emit Client (`ScopedUsageCollectorClientV1`)

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-interface-scoped-client`

**Technology**: Rust struct in `usage-collector-sdk` crate; obtained exclusively via `UsageCollectorClientV1::for_module()`
**Data Format**: Rust types shared with SDK trait

**Operations**:

| Operation | Caller | Description |
|-----------|--------|-------------|
| `authorize_emit(ctx, metric_name)` | Usage source (before transaction) | Calls the platform PDP for the given metric (PDP may return a tenant `In` constraint for sources authorized to emit for multiple tenants), fetches the source-level rate limit quota snapshot, and fetches the registered usage type schema for `metric_name` from types-registry; returns `EmitAuthorization` on success, or `EmitError::Denied` / `EmitError::RateLimitExceeded` on failure; never called inside an open DB transaction |
| `emit(ctx, record, &EmitAuthorization)` | Usage source (within transaction) | Validates metric semantics (rejects counter records with a negative value or a missing idempotency key), captures subject from SecurityContext, then evaluates all `EmitAuthorization` constraints in-memory: schema validation, source-level rate limit quota, PDP constraint satisfaction including tenant validation (tenant `In` constraint present → `record.tenant_id ∈ allowed set`; no tenant constraint → `record.tenant_id == ctx.subject_tenant_id()`); stamps outbox row with `record.tenant_id` on success; returns `EmitError::MissingIdempotencyKey`, `EmitError::SchemaViolation`, `EmitError::RateLimitExceeded`, or `EmitError::ConstraintViolation` on failure — no outbox INSERT on any error |

#### Plugin Trait

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-interface-plugin-trait`

**Technology**: Rust trait in `usage-collector-sdk` crate
**Data Format**: Rust types shared with SDK trait

**Operations**:

| Operation | Description |
|-----------|-------------|
| `persist(records)` | Persists usage records with idempotent upsert on `idempotency_key`; for counter metrics, each record's `value` is a non-negative delta — the record is stored as-is alongside other deltas; the persistent total for any `(tenant_id, metric)` pair is the SUM of all active delta records for that key (see §3.7 Counter Accumulation); for gauge metrics, values are stored as-is |
| `query_aggregated(ctx, query)` | Executes aggregation query within the storage engine; applies optional filters (usage type, subject, resource, source) and GROUP BY dimensions from `AggregationQuery`; pushes aggregation and grouping down to the storage engine |
| `query_raw(ctx, query)` | Executes raw record query with cursor-based pagination; applies optional filters (usage type, subject, resource) from `RawQuery`; scoped to tenant |
| `backfill_ingest(ctx, tenant, usage_type, records)` | Bulk-inserts historical records with idempotent upsert on `idempotency_key`; does not modify existing records |
| `amend_record(ctx, record_id, updates)` | Updates mutable fields on an individual active record |
| `deactivate_record(ctx, record_id)` | Sets `status = inactive` on an individual record; retains record for audit |
| `enforce_retention(ctx, policy)` | Permanently deletes (hard delete) records beyond the configured retention duration for the given policy scope |
| `get_watermarks(ctx, tenant)` | Returns per-source event counts and latest ingested timestamps for the tenant |

#### Gateway REST API

- [ ] `p1` - **ID**: `cpt-cf-usage-collector-interface-gateway-rest`

**Technology**: REST/OpenAPI via Axum
**Location**: `modules/system/usage-collector/src/api/`

**Endpoints Overview**:

| Method  | Path                                | Description                                                                                 | Stability |
| ------- | ----------------------------------- | ------------------------------------------------------------------------------------------- | --------- |
| `GET`   | `/v1/usage/aggregated`              | Query aggregated usage data for the authenticated tenant                                    | stable    |
| `GET`   | `/v1/usage/raw`                     | Query raw usage records with cursor-based pagination for the authenticated tenant           | stable    |
| `POST`  | `/v1/usage/backfill`                | Operator-initiated bulk insert of historical records for a tenant and usage type time range | stable    |
| `PATCH` | `/v1/usage/records/{id}`            | Amend mutable fields on an individual active record                                         | stable    |
| `POST`  | `/v1/usage/records/{id}/deactivate` | Deactivate an individual record (marks inactive, retains for audit)                         | stable    |
| `GET`   | `/v1/usage/metadata/watermarks`     | Per-source and per-tenant event counts and latest timestamps                                | stable    |
| `POST`  | `/v1/usage/types`                   | Register a custom usage type (name, metric kind, unit label); delegates to types-registry   | stable    |
| `GET`   | `/v1/usage/retention`               | List configured retention policies                                                          | stable    |
| `PUT`   | `/v1/usage/retention/{scope}`       | Create or update a retention policy for a scope (global / tenant / usage-type)              | stable    |
| `DELETE` | `/v1/usage/retention/{scope}`      | Delete a non-global retention policy for a scope (tenant / usage-type); the global default policy cannot be deleted | stable    |

#### Error Handling

| HTTP | gRPC | Category | Description |
|------|------|----------|-------------|
| 400 | `INVALID_ARGUMENT` | Validation failure | Metadata size limit exceeded (primary gateway check); defense-in-depth schema or metric semantics rejection — should not occur in normal operation, as records must pass these checks at SDK `emit()` time before the outbox INSERT |
| 401 | `UNAUTHENTICATED` | Authentication failure | Request carries no valid SecurityContext; rejected by the ModKit request pipeline before any handler executes |
| 403 | `PERMISSION_DENIED` | Authorization failure | PDP denied the operation, or the caller is not permitted to access the target tenant, usage type, or record |
| 409 | `ALREADY_EXISTS` | Deduplication | Idempotency key already processed for this tenant; response body includes the original record ID |
| 422 | `INVALID_ARGUMENT` | Temporal violation | Backfill range exceeds the configured maximum window without elevated authorization |
| 500 | `INTERNAL` | Storage error | Plugin persistence or query failure not recoverable at the request scope |

#### API Versioning

All endpoints are **stable** (prefixed `/v1/`). Backward compatibility is guaranteed within the v1 major version. Breaking changes require a new major version and a minimum 90-day deprecation window, following the platform-wide API lifecycle convention.

### 3.4 Internal Dependencies

| Dependency Module | Interface Used | Purpose |
|-------------------|----------------|---------|
| modkit-db | `modkit_db::outbox` — `Outbox::enqueue()`, `OutboxHandle`, `outbox_migrations()`, dead-letter API | Durable outbox persistence, automatic background delivery pipeline, and dead-letter management for at-least-once delivery |
| types-registry | GTS type system — plugin schema registration and instance resolution; types-registry SDK — usage type schema retrieval | Storage plugin discovery and instantiation at runtime; usage type schema fetching for in-memory validation at emit time (called by `ScopedUsageCollectorClientV1.authorize_emit()`) |
| SecurityContext | Platform security primitive | Tenant identity, subject identity derivation and authorization enforcement on all operations |
| authz-resolver | `authz-resolver-sdk` — `PolicyEnforcer` / `AuthZResolverClient` | Platform PDP for ingestion authorization; called by `ScopedUsageCollectorClientV1.authorize_emit()` to verify the source module is permitted to emit the target metrics |

**Dependency Rules** (per project conventions):
- No circular dependencies
- Always use SDK modules for inter-module communication
- No cross-category sideways deps except through contracts
- Only integration/adapter modules talk to external systems
- `SecurityContext` must be propagated across all in-process calls

### 3.5 External Dependencies

#### audit_service

| Dependency Module | Interface Used | Purpose |
|-------------------|----------------|---------|
| audit_service (platform) | Event emitter | Receive structured `WriteAuditEvent` payloads for operator-initiated write operations (backfill, amendment, deactivation) |

**Dependency Rules**:
- Gateway emits audit events after each completed operator-initiated write; emission is best-effort and MUST NOT block or roll back the primary operation on failure
- Audit event emission MUST use a network timeout of ≤ 2 s; if `audit_service` does not respond within the timeout, the gateway logs the failure, increments `usage_audit_emit_total{result="failed"}`, and returns a successful response to the caller — the primary operation is not rolled back
- Emission failures MUST be surfaced via operational monitoring
- No audit data is stored locally by the Usage Collector

#### ClickHouse

- **Contract**: `cpt-cf-usage-collector-contract-storage-plugin`

| Dependency Module | Interface Used | Purpose |
|-------------------|---------------|---------|
| clickhouse-plugin | ClickHouse native protocol via Rust driver | Append-optimized time-series storage and native column-store aggregation |

**Dependency Rules** (per project conventions):
- Plugin is versioned with the module major version
- Plugin encapsulates all ClickHouse-specific query syntax and driver usage

#### TimescaleDB

- **Contract**: `cpt-cf-usage-collector-contract-storage-plugin`

| Dependency Module | Interface Used | Purpose |
|-------------------|---------------|---------|
| timescaledb-plugin | PostgreSQL protocol via SeaORM / sqlx | Time-series storage with hypertable partitioning and continuous aggregate support |

**Dependency Rules** (per project conventions):
- Plugin is versioned with the module major version
- Plugin encapsulates all TimescaleDB-specific query syntax and driver usage

### 3.6 Interactions & Sequences

#### Emit Usage Record

**ID**: `cpt-cf-usage-collector-seq-emit`

**Use cases**: `cpt-cf-usage-collector-usecase-emit`

**Actors**: `cpt-cf-usage-collector-actor-usage-source`

```mermaid
sequenceDiagram
    participant App as Usage Source
    participant SC as ScopedUsageCollectorClientV1
    participant PDP as authz-resolver (PDP)
    participant TR as types-registry
    participant LDB as Local DB
    participant OB as Outbox Pipeline (SDK background)
    participant GW as UC Gateway
    participant Plugin as Storage Plugin
    participant DB as ClickHouse / TimescaleDB

    Note over App,SC: Module init (once)
    App->>SC: for_module(MODULE_NAME)

    Note over App,LDB: Per-request, before transaction
    App->>SC: authorize_emit(ctx, metric_name)
    SC->>PDP: evaluate policy (source_module, metric_name)
    SC->>TR: fetch schema for metric_name
    SC->>SC: fetch source-level quota snapshot
    PDP-->>SC: constraints (incl. optional tenant In constraint) | Denied
    TR-->>SC: schema
    SC-->>App: Ok(EmitAuthorization{constraints, schema, quota}) | Err(Denied | RateLimitExceeded)

    Note over App,LDB: Inside caller's DB transaction
    App->>SC: emit(ctx, record{tenant_id=T}, &auth)
    SC->>SC: validate metric semantics (counter ≥ 0, gauge any)
    SC->>SC: capture subject_id / subject_type from SecurityContext
    SC->>SC: validate record against schema
    SC->>SC: check source-level quota (emission count within window limit)
    SC->>SC: evaluate PDP constraints incl. tenant: In constraint → record.tenant_id ∈ allowed set; no constraint → record.tenant_id == ctx.subject_tenant_id()
    SC->>LDB: Outbox::enqueue(tenant_id=record.tenant_id) within caller's tx
    LDB-->>SC: ok
    SC-->>App: Ok | Err(SchemaViolation | RateLimitExceeded | ConstraintViolation)

    Note over OB: Outbox library background pipeline
    OB->>GW: MessageHandler::handle() → persist(records)
    GW->>GW: validate SecurityContext
    alt record.tenant_id == ctx.subject_tenant_id()
        GW->>GW: tenant attribution valid (standard path)
    else record.tenant_id ≠ ctx.subject_tenant_id()
        GW->>PDP: emit_on_behalf? (source, record.tenant_id)
        PDP-->>GW: permit | deny
    end
    GW->>GW: enforce per-(source, tenant) rate limit
    GW->>Plugin: persist(records)
    Plugin->>DB: INSERT (idempotent upsert on idempotency_key)
    DB-->>Plugin: ok
    Plugin-->>GW: ok
    GW-->>OB: ok → HandlerResult::Success (outbox advances cursor)
    Note over OB: On transient failure: HandlerResult::Retry → exponential backoff<br/>On permanent failure: HandlerResult::Reject → dead-letter store
```

**Description**: At module initialization, the usage source calls `for_module()` to obtain a `ScopedUsageCollectorClientV1` bound to its platform identity. For each request that will emit usage, it calls `authorize_emit()` before opening any DB transaction — the scoped client contacts the PDP, fetches the registered usage type schema from types-registry, and fetches the current source-level rate limit quota snapshot; it returns an `EmitAuthorization` token bundling all three (including any tenant `In` constraint from the PDP for sources authorized to emit for multiple tenants), or an immediate `Denied` or `RateLimitExceeded` error. Inside the caller's transaction, `emit()` validates metric semantics, captures subject identity, evaluates all constraints in-memory including tenant validation (`record.tenant_id` must satisfy the PDP tenant constraint or equal `ctx.subject_tenant_id()` if no constraint was returned), and on success stamps the outbox row with `record.tenant_id` and calls `Outbox::enqueue()`. The outbox library's background pipeline then automatically picks up enqueued messages and invokes the SDK's internal `MessageHandler`. The gateway validates tenant attribution on delivery: standard records (where `record.tenant_id == ctx.subject_tenant_id()`) are accepted directly; cross-tenant records trigger a PDP `emit_on_behalf` check per unique target tenant in the batch before plugin delivery. The gateway also enforces per-(source, tenant) rate limits at this point. On `HandlerResult::Success` the outbox advances the partition cursor; on `HandlerResult::Retry` it applies exponential backoff and re-delivers; on `HandlerResult::Reject` it moves the message to the dead-letter store.

#### Query Aggregated Usage

**ID**: `cpt-cf-usage-collector-seq-query-aggregated`

**Use cases**: `cpt-cf-usage-collector-usecase-query-aggregated`

**Actors**: `cpt-cf-usage-collector-actor-usage-consumer`, `cpt-cf-usage-collector-actor-tenant-admin`

```mermaid
sequenceDiagram
    participant Consumer as Usage Consumer
    participant GW as UC Gateway
    participant PDP as authz-resolver (PDP)
    participant Plugin as Storage Plugin
    participant DB as ClickHouse / TimescaleDB

    Consumer->>GW: GET /usage/aggregated (ctx, query params)
    GW->>GW: derive tenant_id from SecurityContext
    GW->>PDP: authorize query (ctx, tenant, usage_type?)
    PDP-->>GW: decision + constraints
    GW->>GW: apply PDP constraints as additional filters
    GW->>Plugin: query_aggregated(ctx, query)
    Plugin->>DB: SELECT aggregate(...) GROUP BY [dimensions] WHERE tenant_id = ? [AND filters]
    DB-->>Plugin: result rows
    Plugin-->>GW: AggregationResult[]
    GW-->>Consumer: 200 OK, AggregationResult[]
```

**Description**: Usage consumer requests aggregated data. The gateway derives tenant ID from SecurityContext, then authorizes the query via the platform PDP — the PDP decision determines whether the caller is permitted, and any returned constraints are applied as additional query filters. The gateway delegates to the plugin with the full query including optional filters and GROUP BY dimensions. The plugin pushes aggregation and grouping down to the storage engine. An empty time range returns an empty result, not an error.

#### Query Raw Usage

**ID**: `cpt-cf-usage-collector-seq-query-raw`

**Use cases**: Not defined separately in PRD; see `cpt-cf-usage-collector-fr-query-raw`

**Actors**: `cpt-cf-usage-collector-actor-usage-consumer`, `cpt-cf-usage-collector-actor-tenant-admin`

```mermaid
sequenceDiagram
    participant Consumer as Usage Consumer
    participant GW as UC Gateway
    participant PDP as authz-resolver (PDP)
    participant Plugin as Storage Plugin
    participant DB as ClickHouse / TimescaleDB

    Consumer->>GW: GET /usage/raw (ctx, time range, filters, cursor)
    GW->>GW: derive tenant_id from SecurityContext
    GW->>PDP: authorize query (ctx, tenant, usage_type?)
    PDP-->>GW: decision + constraints
    GW->>GW: apply PDP constraints as additional filters
    GW->>Plugin: query_raw(ctx, raw_query)
    Plugin->>DB: SELECT * WHERE tenant_id = ? AND timestamp BETWEEN ? [AND filters] ORDER BY timestamp LIMIT ? AFTER cursor
    DB-->>Plugin: record page + next cursor
    Plugin-->>GW: PagedResult<UsageRecord>
    GW-->>Consumer: 200 OK, PagedResult<UsageRecord>
```

**Description**: Usage consumer requests a page of raw usage records for auditing or detailed analysis. The gateway derives tenant ID from SecurityContext, then authorizes the query via the platform PDP — the PDP decision determines whether the caller is permitted, and any returned constraints are applied as additional query filters. The gateway delegates to the plugin with the full query including optional filters (usage type, subject, resource). The cursor is opaque to the consumer; omitting the cursor returns the first page. An exhausted cursor returns an empty page.

#### Backfill Operation

**ID**: `cpt-cf-usage-collector-seq-backfill`

**Use cases**: See `cpt-cf-usage-collector-fr-backfill-api`

**Actors**: `cpt-cf-usage-collector-actor-platform-operator`

```mermaid
sequenceDiagram
    participant Op as Platform Operator
    participant GW as UC Gateway
    participant PDP as authz-resolver
    participant OB as Backfill Outbox (gateway-local)
    participant Plugin as Storage Plugin
    participant DB as ClickHouse / TimescaleDB
    participant AS as audit_service

    Op->>GW: POST /usage/backfill (tenant_id, usage_type, range, records)
    GW->>PDP: authorize backfill for tenant_id (actor from SecurityContext)
    alt PDP denies or tenant_id violates returned constraint
        PDP-->>GW: deny
        GW-->>Op: 403 PERMISSION_DENIED
    else PDP permits and constraint satisfied
        PDP-->>GW: allow
        GW->>GW: enforce time boundaries (max window, future tolerance)
        GW->>GW: verify elevated authz if range exceeds max window
        GW->>OB: Outbox::enqueue_batch() — one message per record (within handler transaction)
        alt outbox write fails
            OB-->>GW: error
            GW-->>Op: 500
        else outbox write succeeds
            OB-->>GW: ok
            GW->>AS: emit WriteAuditEvent
            GW-->>Op: 202 Accepted
        end
    end

    Note over OB: Outbox library background pipeline (gateway-local)
    OB->>GW: MessageHandler::handle() → single backfill record
    GW->>Plugin: backfill_ingest(tenant_id, usage_type, record)
    Plugin->>DB: INSERT records (idempotent upsert on idempotency_key)
    DB-->>Plugin: ok
    Plugin-->>GW: ok
    GW-->>OB: ok → HandlerResult::Success (outbox advances cursor)
    Note over OB: On transient failure: HandlerResult::Retry → exponential backoff<br/>On permanent failure: HandlerResult::Reject → dead-letter store
```

**Description**: Platform operator submits a backfill request specifying `tenant_id`, usage type, time range, and historical records to insert. The gateway first calls the platform PDP to verify the caller is authorized to backfill for the specified `tenant_id`; a PDP denial or a `tenant_id` that violates the returned constraint returns `403 PERMISSION_DENIED` immediately without touching the outbox. Callers whose SecurityContext tenant differs from the requested `tenant_id` require a dedicated cross-tenant backfill PDP permission; the PDP must return a constraint that includes the target tenant. After PDP authorization succeeds, the gateway enforces time boundaries and requires elevated authorization for windows exceeding the configured maximum. On successful validation, the gateway enqueues all backfill records to a gateway-local backfill outbox in a single transaction, then emits a `WriteAuditEvent` to `audit_service` and returns `202 Accepted` to the caller. The outbox library's background pipeline then automatically calls the gateway's internal `MessageHandler` for each individual record. The plugin inserts each record using idempotent upsert — existing records are not modified. Real-time ingestion continues uninterrupted throughout the operation.

#### Amend Individual Record

**ID**: `cpt-cf-usage-collector-seq-amend`

**Use cases**: See `cpt-cf-usage-collector-fr-event-amendment`

**Actors**: `cpt-cf-usage-collector-actor-platform-operator`

```mermaid
sequenceDiagram
    participant Op as Platform Operator
    participant GW as UC Gateway
    participant PDP as authz-resolver (PDP)
    participant Plugin as Storage Plugin
    participant DB as ClickHouse / TimescaleDB
    participant AS as audit_service

    Op->>GW: PATCH /usage/records/{id} (ctx, justification, updated fields)
    GW->>GW: derive tenant_id from SecurityContext
    GW->>PDP: authorize amendment (ctx, tenant, record_id)
    PDP-->>GW: permit | deny
    alt authorization denied
        GW-->>Op: 403 Forbidden
    else authorized
        GW->>Plugin: amend_record(ctx, record_id, updates, expected_version)
        alt record not found or wrong tenant
            Plugin-->>GW: not found
            GW-->>Op: 404 Not Found
        else optimistic concurrency conflict
            Plugin-->>GW: version mismatch
            GW-->>Op: 409 Conflict
        else record is inactive
            Plugin-->>GW: precondition failed
            GW-->>Op: 422 Unprocessable Entity
        else amendment succeeds
            Plugin->>DB: UPDATE record SET ... WHERE id = ? AND version = ?
            DB-->>Plugin: ok
            Plugin-->>GW: ok
            GW->>AS: emit WriteAuditEvent (amend, actor_id, tenant_id, record_id, before/after)
            GW-->>Op: 200 OK
        end
    end
```

**Description**: Platform operator submits an amendment for an individual active record, providing a justification and the fields to update. The gateway authorizes via the platform PDP, then delegates to the plugin using optimistic concurrency — the plugin applies the update only if the record's current version matches the caller-supplied expected version. Inactive records cannot be amended. On success, the gateway emits a `WriteAuditEvent` to `audit_service` with the before/after field values and the operator's justification.

#### Deactivate Individual Record

**ID**: `cpt-cf-usage-collector-seq-deactivate`

**Use cases**: See `cpt-cf-usage-collector-fr-event-amendment`

**Actors**: `cpt-cf-usage-collector-actor-platform-operator`

```mermaid
sequenceDiagram
    participant Op as Platform Operator
    participant GW as UC Gateway
    participant PDP as authz-resolver (PDP)
    participant Plugin as Storage Plugin
    participant DB as ClickHouse / TimescaleDB
    participant AS as audit_service

    Op->>GW: POST /usage/records/{id}/deactivate (ctx, justification)
    GW->>GW: derive tenant_id from SecurityContext
    GW->>PDP: authorize deactivation (ctx, tenant, record_id)
    PDP-->>GW: permit | deny
    alt authorization denied
        GW-->>Op: 403 Forbidden
    else authorized
        GW->>Plugin: deactivate_record(ctx, record_id)
        alt record not found or wrong tenant
            Plugin-->>GW: not found
            GW-->>Op: 404 Not Found
        else record already inactive
            Plugin-->>GW: ok (idempotent)
            GW->>AS: emit WriteAuditEvent (deactivate, actor_id, tenant_id, record_id)
            GW-->>Op: 200 OK
        else deactivation succeeds
            Plugin->>DB: UPDATE record SET status = 'inactive', inactive_at = NOW()
            DB-->>Plugin: ok
            Plugin-->>GW: ok
            GW->>AS: emit WriteAuditEvent (deactivate, actor_id, tenant_id, record_id)
            GW-->>Op: 200 OK
        end
    end
```

**Description**: Platform operator deactivates an individual record, providing a justification. The record is retained for audit but excluded from active-only queries. The gateway authorizes via the platform PDP, then delegates to the plugin. Deactivation is idempotent — calling it on an already-inactive record returns success and still emits the audit event. The gateway emits a `WriteAuditEvent` to `audit_service` on every successful deactivation (including idempotent re-deactivations).

### 3.7 Database schemas & tables

#### Outbox Queue (source-local)

**ID**: `cpt-cf-usage-collector-dbtable-outbox`

The outbox schema is owned by the `modkit-db` shared infrastructure and installed via `outbox_migrations()` from `modkit_db::outbox` as part of the source module's migration set. The SDK interacts with the outbox exclusively through the public API: `Outbox::enqueue()` to persist a message and `Outbox` dead-letter methods for operational management. Internal table structure is an implementation detail of the outbox library and is not referenced here.

**Queue**: The SDK registers a single queue named `"usage-records"` with `Partitions::of(4)` (configurable). Queue registration is idempotent and runs at SDK initialization time via `Outbox::builder(db).queue("usage-records", Partitions::of(4)).decoupled(delivery_handler).start()`.

**Payload**: Each enqueued message carries `payload_type = "usage-collector.record.v1"` and a binary payload containing the serialized usage record fields:

| Field | Type | Description |
|-------|------|-------------|
| `tenant_id` | UUID | Tenant owning this usage record |
| `source_module` | TEXT | Source module name from `ScopedUsageCollectorClientV1` |
| `metric_kind` | TEXT | `"counter"` or `"gauge"` |
| `metric` | TEXT | Name of the measured resource metric |
| `value` | NUMERIC | Numeric measurement value |
| `timestamp` | TIMESTAMPTZ | When the measurement occurred |
| `idempotency_key` | TEXT | Client-provided deduplication key; non-null for counter records (enforced by `emit()` before enqueue); nullable for gauge records |
| `resource_id` | UUID (nullable) | Resource instance this record is attributed to |
| `resource_type` | TEXT (nullable) | Resource type corresponding to `resource_id` |
| `subject_id` | UUID (nullable) | Subject (user/service account) from SecurityContext |
| `subject_type` | TEXT (nullable) | Subject type corresponding to `subject_id` |
| `metadata` | JSON object (nullable) | Optional opaque metadata; size validated by SDK before enqueue (default 8 KB limit) |

**Payload invariants** (enforced by SDK before `Outbox::enqueue()` is called): `tenant_id`, `source_module`, `metric_kind`, `metric`, `value`, `timestamp` are all non-null. `idempotency_key` is non-null for counter records.

**Dead letters**: Messages permanently rejected by the SDK's delivery handler (`HandlerResult::Reject`) are moved to the outbox dead-letter store by the library. Dead-letter management (replay, resolve, discard) is available via the `Outbox` dead-letter API.

#### Storage Backend Tables (plugin-owned)

Storage table schemas are defined and owned by each plugin implementation. The gateway does not define, access, or migrate storage tables directly. Each plugin is responsible for its own schema migration lifecycle.

**Expected columns** (plugin-specific naming and types):

| Column | Purpose |
|--------|---------|
| `tenant_id` | Tenant scoping and isolation; required filter on all queries |
| `source_module` | Source module identity; enables per-module metric auditing |
| `metric_kind` | `"counter"` or `"gauge"`; governs aggregation semantics |
| `metric` | Name of the measured resource metric |
| `value` | Measured numeric value |
| `timestamp` | Measurement time; primary partitioning and ordering dimension |
| `idempotency_key` | Deduplication key; NOT NULL for counter records, nullable for gauge records; unique constraint or upsert target per plugin for non-null values |
| `resource_id` | Resource instance attribution; nullable |
| `resource_type` | Resource type attribution; nullable |
| `subject_id` | Subject attribution from SecurityContext; nullable |
| `subject_type` | Subject type attribution; nullable |
| `ingested_at` | When the collector received and persisted the record |
| `metadata` | Opaque JSON object; persisted as-is without indexing or interpretation; nullable |
| `status` | Record lifecycle state: `active` (default), `inactive` (deactivated by operator-initiated amendment or deactivation) |
| `inactive_at` | Timestamp when the record was deactivated; null for active records |
| `version` | Integer incremented on each mutable field update; used for optimistic concurrency control on `amend_record` — the caller supplies expected version; conflict returned on mismatch; inactive records cannot be amended |

**Required indexes** (plugin implementations MUST provide these; failure to do so violates `cpt-cf-usage-collector-nfr-query-latency`):

| Index | Columns | Purpose |
|-------|---------|---------|
| Primary time-series index | `(tenant_id, timestamp)` | Mandatory filter on all queries; drives time-range scans |
| Metric filter index | `(tenant_id, metric, timestamp)` | Supports `usage_type` filter in aggregation and raw queries |
| Subject filter index | `(tenant_id, subject_id, timestamp)` | Supports `subject` filter in aggregation and raw queries |
| Resource filter index | `(tenant_id, resource_id, timestamp)` | Supports `resource` filter in aggregation and raw queries |
| Idempotency deduplication | `(tenant_id, idempotency_key)` — unique or upsert target | Required for idempotent `persist()` and `backfill_ingest()` |

Storage-native index types (ClickHouse sorting key, TimescaleDB btree/brin) are at the plugin's discretion, provided they satisfy the query latency NFR.

##### Counter Accumulation

**ID**: `cpt-cf-usage-collector-dbtable-counter-accumulation`

Counter semantics (`cpt-cf-usage-collector-fr-counter-semantics`) define the total for a `(tenant_id, metric)` pair as a monotonically increasing value that grows as non-negative deltas are ingested.

**Source of truth — delta records as the persistent store**: The plugin-owned usage records table is the authoritative record of all submitted deltas. There is no separate totals table in the baseline schema. The persistent total for any `(tenant_id, metric)` pair is computed as `SUM(value)` over all active records matching that key for the desired time range. Because all delta values are non-negative (enforced at emit time by `ScopedUsageCollectorClientV1.emit()`), the cumulative SUM is guaranteed to be monotonically increasing.

**Accumulation key vs. full attribution tuple**: `cpt-cf-usage-collector-fr-counter-semantics` scopes the total to `(tenant_id, metric)`. The full record carries additional attribution dimensions — `source_module`, `resource_id`, `resource_type`, `subject_id`, `subject_type` — that enable GROUP BY breakdowns within `query_aggregated`. The counter total per `(tenant_id, metric)` is the SUM across all attribution dimensions for that pair; narrower totals (e.g., per `resource_id`) are query-time GROUP BY variants of the same underlying delta records.

**Query-time aggregation**: `query_aggregated` satisfies the total-computation requirement by pushing `SUM(value)` down to the storage engine. The 500ms p95 latency NFR (`cpt-cf-usage-collector-nfr-query-latency`) for 30-day ranges is the binding performance constraint; the aggregation strategy chosen by each plugin must satisfy this NFR.

**Plugin-level pre-aggregation (expected optimization)**: A raw `SUM(value) WHERE status = 'active'` scan over millions of delta records per `(tenant_id, metric)` pair becomes expensive as data grows. To meet the query latency NFR at production record volumes, plugins SHOULD maintain pre-aggregated acceleration structures in addition to the delta records table. This is an implementation detail owned entirely by the plugin; the gateway and SDK are unaware of them and the plugin trait boundary does not change. Each plugin that omits pre-aggregation MUST benchmark its storage engine's native aggregation against the 500ms p95 threshold and only omit the acceleration structure if benchmarks confirm the threshold is achievable without it.

Expected acceleration techniques by backend:

| Plugin | Acceleration technique |
|--------|------------------------|
| TimescaleDB | Continuous aggregate (`CREATE MATERIALIZED VIEW ... WITH (timescaledb.continuous)`) over the usage records hypertable, refreshed on a configurable schedule |
| ClickHouse | `AggregatingMergeTree`-backed materialized view or a pre-computed SUM rollup view by time bucket |

**Consistency model**: The delta records table is the authoritative source of truth and provides **strong consistency** — a query served directly from it always reflects the current state of all ingested, non-deactivated records. Pre-aggregated acceleration structures provide **eventual consistency**: their totals may lag behind the latest ingested deltas by up to one refresh cycle. Plugins MUST document the maximum refresh lag as an operational parameter (for example, `continuous_aggregate_refresh_interval` for TimescaleDB). The plugin MAY serve `query_aggregated` requests from the acceleration structure by default and SHOULD fall back to the delta records table when the acceleration structure is unavailable.

Plugins using acceleration structures MUST ensure:

- The acceleration structure excludes inactive records (`status = inactive`) from aggregated totals
- Backfill-inserted records are reflected in the acceleration structure within the next scheduled refresh cycle
- Amendment operations that change the `value` field result in accurate aggregate values after the next refresh cycle (or are applied directly to the acceleration structure if the plugin supports incremental updates)
- The maximum refresh lag is surfaced as a queryable operational parameter and included in plugin documentation

**Backfill and deactivation effects**:

- `backfill_ingest` inserts historical delta records at their original timestamps. The SUM for any time range that includes those timestamps increases accordingly. Plugins with materialized aggregates must reflect backfilled records within the next refresh cycle.
- `deactivate_record` sets `status = inactive` on a record. Inactive records are excluded from all aggregation queries by default. This is the only mechanism by which the effective total for a time range can decrease.
- There is no counter reset primitive. A counter total can only decrease (for a given time range) if records within that range are deactivated.

**Additional plugin-owned tables** (schemas defined and managed by each plugin):

##### Write audit events

Audit events for operator-initiated writes (backfill, amendment, deactivation) are emitted to the platform `audit_service` and are not stored locally. See `cpt-cf-usage-collector-fr-audit` and the `WriteAuditEvent` type in the domain model for the event schema.

##### Retention policy

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID | Surrogate primary key |
| `scope` | TEXT | `"global"`, `"tenant"`, or `"usage_type"` |
| `target_id` | TEXT (nullable) | Tenant ID or usage type identifier for non-global scopes |
| `retention_duration` | INTERVAL | How long records are retained before deletion or expiry |
| `updated_at` | TIMESTAMPTZ | Last modification timestamp |

### 3.8 Observability

#### Key Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `usage_ingestion_total` | Counter | `tenant_id`, `usage_type`, `status` | Total ingest attempts at gateway; `status` values: `success`, `validation_error`, `dedup`, `auth_denied` |
| `usage_ingestion_latency_ms` | Histogram | `tenant_id` | Gateway ingest handler latency at p50, p95, p99 |
| `usage_ingestion_batch_size` | Histogram | `source_module` | Records per batch delivered by the SDK outbox pipeline |
| `outbox_pending_rows` | Gauge | `source_module` | Unprocessed outbox messages pending delivery in the source's local DB |
| `outbox_dead_letters_total` | Counter | `source_module` | Messages moved to the dead-letter table after permanent delivery failure (`HandlerResult::Reject`) |
| `outbox_delivery_attempts_total` | Counter | `source_module`, `status` | SDK delivery handler invocations by the outbox pipeline; `status` values: `success`, `failure` |
| `outbox_delivery_latency_ms` | Histogram | `source_module` | Time from outbox row insertion to confirmed gateway delivery |
| `usage_dedup_total` | Counter | `tenant_id` | Records deduplicated by idempotent upsert on `idempotency_key` |
| `usage_schema_validation_errors_total` | Counter | `tenant_id`, `usage_type` | Records rejected by types-registry schema validation at gateway |
| `usage_query_latency_ms` | Histogram | `tenant_id`, `query_type` | Query handler latency; `query_type` values: `aggregated`, `raw` |
| `usage_backfill_operations_total` | Counter | `tenant_id`, `status` | Backfill operations completed; `status` values: `success`, `error` |
| `usage_retention_records_deleted_total` | Counter | `tenant_id`, `usage_type` | Records removed by retention enforcement |
| `usage_audit_emit_total` | Counter | `operation`, `result` | Audit events emitted to `audit_service`; `result` values: `ok`, `failed` |
| `storage_health_status` | Gauge | `plugin` | Storage backend reachability: `1` = healthy, `0` = unreachable |

#### Structured Logging

All log entries carry: correlation ID, `tenant_id`, operation type, and handler latency.

- **Ingest**: batch size, dedup count, validation error count, `source_module`
- **SDK delivery**: outbox message sequence, partition, delivery attempt number, backoff delay
- **Backfill**: time range, records inserted
- **Authorization**: operation type, PDP decision; no permission details in structured fields

Never logged: record metric values, metadata contents, subject or resource identifiers.

#### Health Checks

| Endpoint | Type | Failure Behaviour |
|----------|------|-------------------|
| `/health/live` | Liveness — process is running and accepting connections | Container restart |
| `/health/ready` | Readiness — storage plugin healthy, types-registry reachable, authz-resolver reachable | Load balancer stops routing traffic to the instance |

#### Alert Definitions

| Alert | Condition | Severity |
|-------|-----------|----------|
| Storage backend unreachable | `storage_health_status == 0` for > 30 seconds | Critical |
| Outbox dead letters accumulating | `outbox_dead_letters_total` increasing for any `source_module` | Critical |
| Outbox backlog growing | `outbox_pending_rows` above threshold sustained for 10 minutes | High |
| Ingestion latency degraded | `usage_ingestion_latency_ms` p95 > 200ms sustained for 5 minutes | High |
| Audit emit failures | `usage_audit_emit_total{result="failed"}` > 0 sustained for 5 minutes | High |
| Retention job stalled | Retention enforcement not completed within expected window | Medium |

## 4. Additional context

The outbox pattern implementation relies on the modkit-db outbox infrastructure. Key properties:

- `Outbox::enqueue()` is called within the caller's DB transaction, ensuring no usage record is produced without a durably persisted outbox message
- The outbox library drives the delivery background pipeline automatically once `OutboxHandle` is started; no separate polling loop or lifecycle task is needed in the SDK
- No explicit shutdown coordination is required: the SDK and its outbox pipeline share the process lifetime; when the process exits, tokio drops all background tasks; delivery continuity is guaranteed through the persistent outbox and lease re-claiming on the next startup, not through graceful drain
- The outbox library uses per-partition lease-based claiming (decoupled mode) to prevent concurrent delivery across multiple SDK instances for the same partition; the cancellation token fires at 80% of the lease duration to allow graceful exit before lease expiry
- Retry uses exponential backoff (configurable `backoff_base` and `backoff_max`; default 1s–60s); `backoff_max` MUST be configured below 15 minutes to satisfy `cpt-cf-usage-collector-nfr-recovery`
- Messages permanently rejected by the SDK's delivery handler (`HandlerResult::Reject`) are moved to the dead-letter store by the outbox library; surfaced via `outbox_dead_letters_total` monitoring to satisfy `cpt-cf-usage-collector-fr-delivery-guarantee`
- Idempotent upsert at the storage plugin layer handles duplicate deliveries that arise from at-least-once retry semantics

**Not applicable — UX architecture**: Usage Collector is a headless backend module with no user-facing interface.

**Not applicable — Compliance/Privacy architecture**: No PII is stored. Usage records contain only numeric values, metric names, timestamps, and tenant-scoped identifiers. No consent management, data subject rights, or cross-border transfer controls are required.

#### Security Threat Model

| Threat (STRIDE) | Attack Vector | Mitigation |
|-----------------|---------------|------------|
| **Spoofing** | Unauthenticated request to any endpoint | Rejected by the ModKit request pipeline before any handler executes; `SecurityContext` is mandatory on all operations |
| **Tampering** | Overwriting existing records on delivery | Idempotent upsert prevents silent overwrite; `version` field enforces optimistic concurrency on amendments; all operator-initiated writes produce a `WriteAuditEvent` to `audit_service` |
| **Repudiation** | Operator denies issuing a backfill, amendment, or deactivation | `WriteAuditEvent` emitted to `audit_service` for every operator-initiated write; events are not stored locally to prevent local tampering (`cpt-cf-usage-collector-fr-audit`) |
| **Information Disclosure** | Cross-tenant data access via query API | All queries scoped to the authenticated tenant from `SecurityContext`; PDP constraints applied as additional query filters; system fails closed on any authorization failure (`cpt-cf-usage-collector-principle-fail-closed`) |
| **Denial of Service** | Outbox flooding or backfill saturation by a single source | Rate limiting enforced by `authorize_emit()` before any transaction (`cpt-cf-usage-collector-fr-rate-limiting`); independent rate limits on the backfill outbox pipeline prevent storage saturation; dead-letter state surfaces unhealthy sources via monitoring |
| **Elevation of Privilege** | Accessing oversized backfill windows without elevated authorization | Elevated authorization required for requests exceeding the configured maximum backfill window; all authorization failures result in immediate rejection with no partial operation (`cpt-cf-usage-collector-principle-fail-closed`) |

**Not applicable — Recovery architecture**: Backup strategy, point-in-time recovery, and disaster recovery for ClickHouse and TimescaleDB storage backends are governed by platform infrastructure policy and managed by the platform SRE team; this module's design does not define storage backup topology. The transactional outbox ensures that records durably captured in source outboxes can be re-delivered if storage data is restored from a backup.

**Not applicable — Capacity and cost budgets**: Capacity planning, cost estimation, and budget allocation for storage backends are the responsibility of the platform infrastructure team; they are not defined at the module design level.

**Not applicable — Infrastructure as Code**: Kubernetes manifests, Helm charts, and deployment configuration are maintained in the platform infrastructure repository; they are not defined at the module design level.

**Not applicable — Technical debt management**: This is a new module with no known technical debt at initial design.

**Not applicable — Vendor and licensing constraints**: ClickHouse and TimescaleDB Community Edition are licensed under Apache License 2.0 and impose no licensing constraints on this module's design.

**Not applicable — Data residency and resource constraints**: Data residency requirements for tenant data are enforced at the platform infrastructure level, not at the module design level. Development timeline and team size are tracked in project management tooling, not in design documents.

**Not applicable — Data governance (catalog, lineage, master data)**: Data ownership, data lineage, and data catalog integration are defined at the platform level in PRD §6.2 (Data Governance); no additional module-level data governance architecture is required.

**Not applicable — Deployment topology**: Container strategy, orchestration, and environment promotion follow platform-wide conventions managed in the platform infrastructure repository; no module-specific deployment architecture decisions are required.

**Not applicable — Documentation strategy**: API documentation, runbooks, and knowledge base follow platform-wide conventions defined in `guidelines/NEW_MODULE.md`; no module-specific documentation architecture decisions are required.

**Not applicable — Testing architecture**: Test doubles for storage plugins (injected via plugin trait), `authz-resolver`, and `types-registry` (injected via SDK traits) follow the platform-wide testability pattern defined in `guidelines/NEW_MODULE.md#step-12-testing`; no module-specific testability architecture decisions are required.

## 5. Traceability

- **PRD**: [PRD.md](./PRD.md)
- **ADRs**: [ADR/](./ADR/)
- **Features**: [features/](./features/)
