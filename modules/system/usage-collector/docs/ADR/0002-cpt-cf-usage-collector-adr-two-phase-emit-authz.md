<!-- Updated: 2026-04-07 by Constructor Tech -->

# ADR-0002: Two-Phase PDP Authorization for Usage Record Emission


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [PDP call synchronously at `emit()` time (inside or adjacent to DB transaction)](#pdp-call-synchronously-at-emit-time-inside-or-adjacent-to-db-transaction)
  - [Authorization deferred to gateway at delivery time (dispatcher → gateway)](#authorization-deferred-to-gateway-at-delivery-time-dispatcher--gateway)
  - [Pre-loaded static SDK policy (config-based metric allowlist, no PDP)](#pre-loaded-static-sdk-policy-config-based-metric-allowlist-no-pdp)
  - [Two-phase PDP: `authorize_emit()` before transaction + in-memory constraint evaluation at `emit()`](#two-phase-pdp-authorize_emit-before-transaction--in-memory-constraint-evaluation-at-emit)
- [More Information](#more-information)
  - [TOCTOU Window Analysis](#toctou-window-analysis)
  - [Performance Budget](#performance-budget)
- [Review Cadence](#review-cadence)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-cf-usage-collector-adr-two-phase-emit-authz`

## Context and Problem Statement

The SDK's `emit()` call must persist usage records to the local outbox within the source service's DB transaction, as required by the transactional outbox pattern. Performing PDP authorization synchronously at `emit()` time would require a network call inside or adjacent to an open DB transaction, holding database locks during network I/O and blocking the source service on external service availability. Deferring authorization to delivery time (dispatcher → gateway) avoids this but produces opaque late rejections: the source's original caller has already received a success response, making it impossible to surface authorization failures meaningfully. The system needs a mechanism to enforce authorization checks at request time with immediate failure feedback, without violating the quick injection principle.

## Decision Drivers

* `emit()` MUST NOT make network calls or block on external services — records must be enqueued into the local outbox as fast as a DB insert
* Authorization checks MUST NOT be performed inside an open DB transaction — network calls holding DB locks violate platform conventions and degrade source service throughput
* Authorization errors MUST be surfaced to the source service's caller at request time, before any domain operation is committed
* The system MUST fail closed on authorization failure (`cpt-cf-usage-collector-principle-fail-closed`)
* The `ScopedUsageCollectorClientV1` from ADR 0001 (`cpt-cf-usage-collector-adr-scoped-emit-source`) is already instantiated per source module and holds the source module identity — it is the natural owner of emit authorization logic
* The existing `authz-resolver-sdk` `PolicyEnforcer` / `AuthZResolverClient` must be reused without introducing new authorization primitives

## Considered Options

* PDP call synchronously at `emit()` time (inside or adjacent to DB transaction)
* Authorization deferred to gateway at delivery time (dispatcher → gateway)
* Pre-loaded static SDK policy (config-based metric allowlist, no PDP)
* Two-phase PDP: `authorize_emit()` before transaction returns `EmitAuthorization`; `emit()` evaluates constraints in-memory

## Decision Outcome

Chosen option: "Two-phase PDP: `authorize_emit()` before transaction + in-memory constraint evaluation at `emit()`", because it is the only option that satisfies all three constraints simultaneously: no network call inside a DB transaction, immediate failure feedback at request time, and reuse of the existing platform PDP infrastructure.

`ScopedUsageCollectorClientV1` gains two responsibilities:

1. `authorize_emit(ctx, metric_name: &str) -> Result<EmitAuthorization, EmitError>` — called before the DB transaction opens; contacts the PDP for the given metric, fetches the registered usage type schema for `metric_name` from types-registry, and returns an opaque `EmitAuthorization` token on success, or propagates a denial error immediately.
2. `emit(ctx, record, &EmitAuthorization)` — called inside the DB transaction; evaluates the pre-fetched constraints against the record in-memory (no I/O), then inserts the outbox row.

The `EmitAuthorization` type wraps the PDP response constraints (`Vec<Constraint>` from `authz-resolver-sdk`) and is opaque to the calling module. A denial from the PDP is surfaced as an `Err` from `authorize_emit()`, so `EmitAuthorization` is only constructed on success. In-memory constraint evaluation uses the existing `Constraint` / `Predicate` types (`Eq`, `In`) from `authz-resolver-sdk`, evaluated against `UsageRecord` fields without SQL compilation.

`EmitAuthorization` includes `issued_at: Instant` (set at construction time) and `nonce: u64` (randomly generated per call). `emit()` calls `EmitAuthorization::validate_freshness()` as the first step before any constraint evaluation or outbox INSERT; if `issued_at.elapsed() > MAX_AUTH_AGE`, `emit()` returns `EmitError::AuthorizationExpired`. `MAX_AUTH_AGE` is a compile-time constant set to 30 seconds — well above any normal request handler duration and well below the minimum operationally meaningful policy propagation delay. This provides runtime enforcement of the single-use, single-request constraint without relying solely on code review.

### Consequences

* Good, because the PDP call happens before any DB transaction opens — no network I/O holds DB locks
* Good, because authorization errors are surfaced at request time via `authorize_emit()` failure — the source caller receives an immediate, meaningful error before any domain operation commits
* Good, because `emit()` performs only in-memory constraint evaluation and an outbox INSERT — no external calls on the critical emission path
* Good, because the implementation reuses `authz-resolver-sdk` constraint types and `PolicyEnforcer::build_request_with()` without introducing new authorization primitives
* Good, because `ScopedUsageCollectorClientV1` (established in ADR 0001) is the natural and auditable owner of `authorize_emit()`, which already carries the source module identifier for PDP resource property attribution
* Good, because `EmitAuthorization` is opaque to consuming modules — no authz concepts leak into business code at call sites
* Bad, because source services must call `authorize_emit()` once per request that may emit usage — adds one PDP round-trip per request on the non-transaction path
* Bad, because `emit()` gains a required `&EmitAuthorization` argument, changing the SDK API surface relative to a naive `emit(ctx, record)` signature
* Bad, because in-memory constraint evaluation requires a new evaluator in the SDK (`EmitAuthorization::is_satisfied_by`) — the existing framework only compiles constraints to SQL via `AccessScope`
* Bad, because when the platform PDP (`authz-resolver`) is unavailable, `authorize_emit()` returns an error and all usage emission from the source fails for the duration of the outage — this is an intentional fail-closed behavior per `cpt-cf-usage-collector-principle-fail-closed`, but it couples usage metering availability to PDP availability; PDP uptime must be treated as a hard dependency for usage sources

### Confirmation

* Unit tests: `authorize_emit()` propagates PDP denial (`decision: false`) as `EmitError::Denied` without inserting any outbox row
* Unit tests: `emit()` with an `EmitAuthorization` whose constraints exclude the record's metric name returns `EmitError::ConstraintViolation` without inserting any outbox row
* Unit tests: `emit()` with empty constraints (unconstrained `EmitAuthorization`) succeeds and inserts the outbox row
* Unit tests: `emit()` with an `EmitAuthorization` whose `issued_at` is older than `MAX_AUTH_AGE` returns `EmitError::AuthorizationExpired` without inserting any outbox row
* Unit tests: two successive calls to `emit()` with the same `EmitAuthorization` instance both succeed when within `MAX_AUTH_AGE`; the nonce field is present but freshness enforcement is time-based, not single-use (replay within the window is bounded to one request lifetime by construction)
* Integration test: source service calls `authorize_emit()` before opening a DB transaction, then calls `emit()` inside the transaction — confirms no PDP or other network calls occur while the transaction is open

## Pros and Cons of the Options

### PDP call synchronously at `emit()` time (inside or adjacent to DB transaction)

Perform the PDP network call immediately when `emit()` is invoked, either inside the caller's DB transaction or in a preamble that still holds a DB connection.

* Good, because no additional call site ceremony — emit semantics remain simple
* Good, because authorization is tightly coupled to emission — no window for stale authorization
* Bad, because a network call inside a DB transaction holds database locks for the duration of the PDP round-trip, degrading throughput and risking lock contention under PDP latency spikes
* Bad, because PDP unavailability directly blocks all usage emission, coupling `emit()` reliability to PDP availability
* Bad, because violates the quick injection principle — `emit()` must complete within a local DB insert budget

### Authorization deferred to gateway at delivery time (dispatcher → gateway)

Validate authorization when the outbox dispatcher delivers records to the collector gateway — a `4xx` response dead-letters the record.

* Good, because `emit()` remains fast with no external calls
* Good, because no API change to `emit()` is required
* Bad, because the source caller has already received a success response before authorization is evaluated — the original request context is gone and the failure cannot be surfaced meaningfully
* Bad, because dead-lettered records are discovered via monitoring lag, not immediate feedback — identifying an authorization misconfiguration is delayed and operationally expensive
* Bad, because the source service has no actionable signal at the point where it should: the handler that produces the usage record

### Pre-loaded static SDK policy (config-based metric allowlist, no PDP)

Encode the allowed metric names for each source service in SDK initialization configuration (static list or file), evaluated at `emit()` time without any runtime network calls.

* Good, because `emit()` has no runtime external dependency — evaluation is purely local
* Good, because startup is fast if config is local (no network call at init time)
* Bad, because authorization policy is not centrally managed — operators must keep per-service configs in sync with PDP policies manually
* Bad, because policy changes require redeploying source services to take effect — no dynamic policy enforcement
* Bad, because deviates from the platform-wide PDP pattern used across all other CyberFabric modules

### Two-phase PDP: `authorize_emit()` before transaction + in-memory constraint evaluation at `emit()`

Separate the PDP call (network, async, before the transaction) from the constraint check (in-memory, synchronous, inside the transaction). See Decision Outcome for full description.

* Good, because PDP call happens outside the DB transaction — no lock contention
* Good, because `emit()` is fast — only in-memory evaluation and outbox INSERT
* Good, because authorization failures surface immediately at request time
* Good, because reuses existing `authz-resolver-sdk` infrastructure and constraint types
* Bad, because requires one PDP call per request that may emit usage
* Bad, because `emit()` API changes to accept `&EmitAuthorization`

## More Information

The constraint evaluation logic uses `authz-resolver-sdk`'s existing `Constraint` / `Predicate` types. The PDP request sent by `authorize_emit()` uses `PolicyEnforcer::build_request_with()` directly — bypassing `access_scope_with()` — to obtain raw `Vec<Constraint>` without SQL compilation, since evaluation targets an in-memory `UsageRecord` struct rather than a database query. The `source_module` identifier (from ADR 0001) is included as a resource property in the PDP evaluation request, enabling metric-namespace-scoped policies in the PDP.

Per-request PDP calls are chosen over request-level caching for simplicity and consistency with the platform pattern used in other modules (see `examples/modkit/users-info`). Caching can be introduced in a future ADR if PDP call volume proves to be a concern at scale.

### TOCTOU Window Analysis

A time-of-check/time-of-use (TOCTOU) window exists between `authorize_emit()` (PDP call) and `emit()` (constraint evaluation). If an authorization policy is revoked or tightened in this window, `emit()` will evaluate constraints from the now-stale `EmitAuthorization` and may permit an emission that would be denied under the updated policy.

This window is accepted under the following rationale:

- The window duration is bounded by the request handler execution time — typically well under 500ms in the absence of pathological application code — which is too short for routine policy changes to be operationally targeted at individual requests
- PDP policy changes are administrative operations with propagation delays of their own; expecting sub-second policy revocation enforcement across all active request contexts is beyond the operational model of the platform PDP
- The fail-closed principle applies to the next request: any subsequent `authorize_emit()` call will observe the updated policy and deny immediately
- Reusing `EmitAuthorization` across request boundaries is explicitly prohibited (see Confirmation); each request obtains a fresh token, bounding maximum staleness to one request lifetime

To minimize exposure, `EmitAuthorization` MUST NOT be cached or reused across request handlers. Runtime enforcement is provided by the freshness check in `emit()`: tokens older than `MAX_AUTH_AGE` (30 seconds) are rejected with `EmitError::AuthorizationExpired`. This bounds maximum staleness to `MAX_AUTH_AGE` regardless of call-site behavior, eliminating reliance on code review for TOCTOU window closure.

### Performance Budget

`authorize_emit()` introduces one synchronous PDP round-trip per request on the non-transaction path. To satisfy `cpt-cf-usage-collector-nfr-ingestion-latency` (p95 ≤ 200ms for the full emit path including the domain operation), the combined latency budget for `authorize_emit()` is bounded by the following heuristic:

- Local DB insert (`emit()`) plus domain operation: ~50ms typical
- PDP call budget: ≤ 100ms p95 to leave ≥ 50ms headroom against the 200ms threshold

`authorize_emit()` MUST use a network timeout of ≤ 150ms to ensure a slow or unresponsive PDP does not breach the ingestion latency NFR. This timeout MUST be configurable and MUST default to 150ms or lower. If the PDP does not respond within the timeout, `authorize_emit()` MUST return `EmitError::Denied` (fail-closed).

## Review Cadence

This decision is stable. Revisit if:

- PDP round-trip volume at scale justifies introducing request-level caching of `EmitAuthorization` tokens — this would require a new ADR defining cache TTL, invalidation, and stale-token risk
- The ingestion latency NFR tightens below 200ms such that the PDP latency budget must be re-evaluated
- A platform-wide change to the PDP infrastructure (e.g., local sidecar PDP) materially changes the round-trip cost, potentially eliminating the TOCTOU concern and the "Bad" PDP-unavailability consequence

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses the following requirements or design elements:

* `cpt-cf-usage-collector-fr-ingestion` — preserves quick ingestion by keeping the outbox INSERT on the critical path and moving the PDP call off it
* `cpt-cf-usage-collector-fr-tenant-attribution` — `authorize_emit()` passes the subject's tenant context to the PDP, ensuring tenant-aware authorization at the point of emission
* `cpt-cf-usage-collector-fr-tenant-isolation` — PDP constraints returned by `authorize_emit()` can enforce tenant-scoped metric restrictions, evaluated before any record enters the outbox
* `cpt-cf-usage-collector-principle-fail-closed` — denial from the PDP propagates as an `Err` from `authorize_emit()`; no record is enqueued on authorization failure
* `cpt-cf-usage-collector-principle-source-side-persistence` — the outbox INSERT in `emit()` remains within the caller's DB transaction; only in-memory evaluation is added inside the transaction boundary
* `cpt-cf-usage-collector-component-sdk` — `authorize_emit()` and `EmitAuthorization` extend the SDK component's responsibility scope
* `cpt-cf-usage-collector-interface-scoped-client` — `ScopedUsageCollectorClientV1` gains `authorize_emit(ctx, metric_name: &str) -> Result<EmitAuthorization, EmitError>` and the `EmitAuthorization` type; `emit()` gains a required `&EmitAuthorization` parameter
* `cpt-cf-usage-collector-adr-scoped-emit-source` — this decision builds on ADR 0001: `ScopedUsageCollectorClientV1` is the owner of `authorize_emit()`, using the `source_module` it already holds for PDP resource property attribution
