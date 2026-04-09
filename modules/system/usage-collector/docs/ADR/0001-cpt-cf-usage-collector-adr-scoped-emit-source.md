<!-- Updated: 2026-04-07 by Constructor Tech -->

# ADR-0001: Use `for_module()` Scoped Client for Metric Source Attribution


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Per-call source declaration on `emit()` or `UsageRecord`](#per-call-source-declaration-on-emit-or-usagerecord)
  - [`for_module()` scoped client bound once at module init](#for_module-scoped-client-bound-once-at-module-init)
  - [Convention-only, no SDK-level source tracking](#convention-only-no-sdk-level-source-tracking)
- [More Information](#more-information)
- [Review Cadence](#review-cadence)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-cf-usage-collector-adr-scoped-emit-source`

## Context and Problem Statement

The `UsageCollectorClientV1` SDK client is registered as a shared singleton in ClientHub by the usage-collector module and retrieved by any consuming module via `hub.get::<dyn UsageCollectorClientV1>()`. Because one client instance is shared across all consuming modules, there is no per-module initialization opportunity to bind a source identifier at client construction time. Without explicit source attribution, the system cannot determine which platform module produced a given metric record, and the SDK cannot enforce that metric names are consistent with the emitting module's declared namespace (e.g., that only the LLM Gateway emits `llm.*` metrics).

## Decision Drivers

* SDK client is a shared singleton from ClientHub — source cannot be bound at client instantiation time
* Module identity is available at compile time via `Self::MODULE_NAME`, a constant injected by the `#[modkit::module]` macro and used as the module's authoritative name throughout the platform
* Call sites must remain clean — every `emit()` invocation should not carry boilerplate source declaration parameters
* Source attribution must be code-review-auditable — the binding must appear in module initialization code where it is clearly visible and reviewable by peers
* Threat model is accidental misuse prevention, not adversarial compromise — cryptographic binding is not required; SDK-level convention enforcement suffices

## Considered Options

* Per-call source declaration on `emit()` or `UsageRecord`
* `for_module()` scoped client bound once at module init
* Convention-only, no SDK-level source tracking

## Decision Outcome

Chosen option: "`for_module()` scoped client bound once at module init", because it is the only option that achieves clean call sites without per-call boilerplate while keeping the source attribution binding code-review-auditable and tied to the module's authoritative `MODULE_NAME` constant.

### Consequences

* Good, because call sites remain clean — `emit(ctx, record)` carries no source boilerplate
* Good, because the `for_module()` call in module init is the single, auditable point where source is declared, using the compile-time `MODULE_NAME` constant rather than a free-form string
* Good, because the SDK can enforce metric prefix consistency at `emit()` time and fail fast in the source process before the outbox is written
* Good, because the module name used for attribution is the same authoritative name registered in the module orchestrator, not a separately maintained string
* Bad, because consuming modules must store a `ScopedUsageCollectorClientV1` wrapper rather than the raw `UsageCollectorClientV1` trait object — a minor change to client storage conventions
* Bad, because source attribution remains self-asserted — a developer can deliberately pass any module name to `for_module()`; this is acceptable given the internal threat model but would not withstand adversarial bypass

### Confirmation

* Code review: verify each consuming module calls `for_module()` with its own `MODULE_NAME` constant (e.g., `LlmGatewayModule::MODULE_NAME`), not another module's constant
* SDK unit tests: verify `ScopedUsageCollectorClientV1::emit()` returns an error when the metric name does not start with the declared source module prefix
* Integration test: verify that emitting a metric with a mismatched prefix (e.g., a file-storage scoped client emitting `llm-gateway.tokens.input`) is rejected at the SDK level before reaching the outbox

## Pros and Cons of the Options

### Per-call source declaration on `emit()` or `UsageRecord`

Pass `source_module` as an explicit parameter to `emit()` or as a required field on `UsageRecord` on every call.

* Good, because source attribution is explicit and visible at every call site
* Good, because no additional wrapper type is required
* Bad, because every `emit()` call site carries a repetitive boilerplate parameter
* Bad, because there is no single auditable binding point — the correct module name must be passed correctly across every emit call, increasing the risk of copy-paste errors
* Bad, because there is no natural place to derive the value from `Self::MODULE_NAME`; the call site must supply it manually each time

### `for_module()` scoped client bound once at module init

The SDK exposes `for_module(name: &str) -> ScopedUsageCollectorClientV1`. Each consuming module calls this once during initialization with `Self::MODULE_NAME` and stores the scoped client. The `ScopedUsageCollectorClientV1` stamps the declared source on every `emit()` and validates the metric prefix.

* Good, because source attribution is declared once, from the authoritative compile-time constant
* Good, because call sites are clean — `emit(ctx, record)` with no source parameter
* Good, because the binding is code-review-auditable in one location (module init)
* Good, because metric prefix enforcement happens automatically on every emit without call-site involvement
* Bad, because consuming modules store a `ScopedUsageCollectorClientV1` instead of the raw trait object

### Convention-only, no SDK-level source tracking

Rely on metric naming convention (`<module-name>.<metric>`) without any runtime tracking or validation. No `source_module` field, no scoped client, no SDK enforcement.

* Good, because no changes to the SDK API or call sites
* Good, because no additional wrapper type or initialization step
* Bad, because there is no mechanism to detect or prevent a module from emitting metrics using another module's namespace prefix
* Bad, because the gateway and storage backend have no source attribution metadata, limiting auditability and future policy enforcement

## More Information

The `#[modkit::module(name = "...")]` macro injects `pub const MODULE_NAME: &'static str` into the module struct at compile time. This constant is the same value used for module registration in the module orchestrator and gRPC hub, making it the platform-canonical identifier for the module. Using it as the source attribution value for `for_module()` ensures consistency with the rest of the platform's module identity model.

## Review Cadence

This decision is stable for the initial release. Revisit if:

- The platform adopts a module identity mechanism that supersedes `MODULE_NAME` (e.g., a cryptographically bound identity token), which could enable stronger source attribution guarantees
- Usage patterns reveal systematic module name mismatches that the SDK-level prefix convention alone cannot prevent, warranting a stricter binding mechanism
- The platform threat model shifts to adversarial internal callers, requiring cryptographic binding instead of convention-based enforcement

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses the following requirements or design elements:

* `cpt-cf-usage-collector-component-sdk` — adds `for_module()` to the SDK component's responsibility scope, returning a `ScopedUsageCollectorClientV1` that binds source attribution and enforces metric prefix at emit time
* `cpt-cf-usage-collector-interface-sdk-trait` — `for_module()` extends the SDK public interface, returning a `ScopedUsageCollectorClientV1`
* `cpt-cf-usage-collector-interface-scoped-client` — `ScopedUsageCollectorClientV1` is the type consuming modules store and use for emission; it binds `MODULE_NAME` and validates metric prefix at `emit()` time
* `cpt-cf-usage-collector-principle-fail-closed` — the SDK fails closed on metric prefix mismatch, rejecting the emit before the outbox is written
* `cpt-cf-usage-collector-fr-ingestion` — source attribution is part of the ingestion record, enabling per-module metric auditing at the gateway and storage layers
