# Competitive Architectural Evaluation: Chat Engine (PR #484) vs Mini Chat (PR #626)

## Executive Comparison Summary

**Design B (Mini Chat, PR #626)** is structurally stronger for the stated evaluation criteria — billing correctness, crash safety, tenant isolation, and P1 production-readiness.

The core reason is architectural scope: Design B owns the full domain lifecycle from user request to billing settlement, with explicit invariants for every failure mode. Design A deliberately pushes billing, quotas, model selection, RAG, and content processing to unspecified webhook backends, making it impossible to evaluate end-to-end correctness from the artifacts provided.

Design A is a cleaner abstraction for a *generic multi-backend chat infrastructure*. Design B is a more complete *production AI chat system*. These are fundamentally different architectural goals. Evaluated against the stated criteria (billing correctness critical, tenant isolation critical, multi-pod K8s, P1 readiness), Design B wins decisively.

---

## Design A (Chat Engine, PR #484) — Strengths

**1. Clean separation of concerns.** The "zero business logic in routing" principle (ADR-0004) is architecturally pure. Chat Engine routes, persists, and manages the message tree. Nothing else. This makes the routing layer trivially correct — it cannot have billing bugs because it has no billing.

**2. Immutable message tree (ADR-0001).** Messages form a DAG via `parent_message_id` with variant branching and `is_active` flags. This is structurally richer than Design B's linear model and enables conversation forking, A/B testing, and exploration — capabilities Design B cannot provide without a redesign.

**3. Stateless horizontal scaling (ADR-0010).** All state in PostgreSQL, zero in-memory session state, any pod handles any request. This is correct by construction for the routing layer.

**4. Exhaustive decision documentation.** 25 ADRs covering immutable trees, webhook authority, streaming, zero business logic, file storage, sync webhooks, circuit breakers, backpressure, cancellation, soft delete, reactions, search, and more. The decision trail is exceptional.

**5. Schema-first API design.** ~90 JSON schemas with GTS `$id` identifiers, separate HTTP and webhook protocol specs, comprehensive README with examples. The API surface is well-specified and versioned.

**6. Webhook extensibility.** Backends define their own capabilities, enabling multiple AI providers, processing strategies, and conversation patterns without Chat Engine changes. Session type switching mid-conversation is natively supported.

**7. Circuit breaker per backend (ADR-0011).** Per-`session_type_id` circuit breaker with 5-failure threshold and 60-second timeout provides backend isolation.

**8. Content-agnostic design.** Chat Engine treats message content as opaque JSONB. This future-proofs against content format changes and eliminates a class of parsing/validation bugs.

---

## Design A (Chat Engine, PR #484) — Weaknesses / Risks

**W-A1. No billing model whatsoever.** Section 5 (Intentional Exclusions) explicitly states: "Rate Limiting — Throttling algorithms, quota management — Handled at API gateway layer upstream of Chat Engine." For a system where "billing correctness is critical," this is a structural gap. The billing responsibility is not delegated to a *specified* component — it's delegated to an unnamed "API gateway layer" with no contract. **Verdict: Unacceptable for P1 if billing correctness is a stated requirement.**

**W-A2. No turn concept, no finalization semantics.** There is no equivalent of `chat_turns` with state machines, CAS guards, or settlement transactions. If a webhook backend crashes mid-stream, Chat Engine saves the partial response with `is_complete = false` — but there is no mechanism to reconcile billing, release reserves, or detect orphaned streams. Who settles the bill? The webhook backend? The "API gateway"? Not specified.

**W-A3. No crash recovery model for in-flight operations.** The streaming cancellation sequence (S10) shows connection close -> cancel request -> save partial response -> close webhook connection. But what if the Chat Engine pod crashes between "cancel request" and "save partial response"? The message exists in an indeterminate state. There is no watchdog, no timeout, no recovery mechanism.

**W-A4. No quota enforcement at any layer.** Intentionally excluded. A malicious user can send unbounded requests, triggering unbounded LLM costs at the webhook backend. The "API gateway" rate limiting is not specified, not contracted, and not architecturally bound to the user/tenant/model context needed for meaningful quota enforcement.

**W-A5. Webhook backend security gap.** Section 3.5: "Chat Engine does not add authentication headers to webhook requests. Webhook endpoint security is the responsibility of the session type administrator." Section 4 (Context: Security): "Chat Engine does not validate webhook responses beyond HTTP status codes. Malicious webhook backends can return arbitrary content." This is an explicit acknowledgment of a trust boundary violation — the webhook response is streamed directly to the user. A compromised backend can inject arbitrary content.

**W-A6. No provider ID sanitization.** Design B explicitly prohibits exposing `provider_file_id`, `vector_store_id` in API responses. Design A stores `file_ids` (UUIDs) in messages and forwards them to webhook backends. While the UUID indirection helps, there is no stated invariant preventing webhook backends from returning provider-specific identifiers in their response content.

**W-A7. Authorization model is thin.** JWT `client_id` ownership checks. No PDP/PEP pattern, no `AccessScope` compilation, no per-operation authorization matrix. Session sharing uses a single `share_token` VARCHAR column with no documented expiry, revocation, or brute-force protection.

**W-A8. No observability contract for cost governance.** 5 metrics (request_duration, webhook_duration, circuit_breaker_state, active_streams, session_operations_total). No token usage metrics, no cost tracking, no quota metrics, no billing reconciliation visibility. For a system where billing correctness is critical, this is insufficient.

---

## Design B (Mini Chat, PR #626) — Strengths

**1. Exhaustive billing invariants.** Sections 5.1-5.6 (600+ lines) define: reserve/commit two-phase quota counting, transactional outbox pattern with `(turn_id, request_id)` uniqueness, deterministic charged token formula for aborted streams, terminal error reconciliation, first-terminal-wins CAS race resolution, billing event completeness invariant, pre-provider failure handling, and the reserve uncommitted invariant. This is the most thorough billing specification in the comparison by an unbridgeable margin.

**2. Explicit turn state machine with CAS finalization.** `chat_turns.state`: `pending -> running -> completed|failed|cancelled`. State transitions enforced via `UPDATE ... WHERE state = 'running'` CAS guard. Terminal states are immutable. Every turn reaches exactly one terminal billing state. The `(turn_id, request_id)` outbox unique constraint serves as a secondary defense. This is textbook exactly-once settlement.

**3. Orphan turn watchdog.** Turns stuck in `running` beyond the configurable timeout (default: 5 min) are finalized with `error_code = 'orphan_timeout'`, `outcome = "aborted"`, using the same deterministic formula. The watchdog uses the identical CAS guard, preventing double-settlement. This handles pod crashes, network partitions, and hung provider connections.

**4. Deterministic charged token formula.** `charged_tokens = min(reserve_tokens, estimated_input_tokens + minimal_generation_floor)`. The `minimal_generation_floor` (default: 50 tokens) prevents zero-charge exploitation via immediate disconnect. Configuration validation at startup rejects invalid values. Actual usage is preferred when available.

**5. Per-chat vector store isolation.** Each chat gets a dedicated provider vector store. Physical and logical isolation. No cross-chat document leakage. `provider_file_id` and `vector_store_id` are never exposed in any API response — client-visible identifiers are internal UUIDs only.

**6. PEP/PDP authorization with AccessScope.** AuthZ Resolver evaluates every data-access operation. Constraints compiled to `AccessScope` objects. Applied via Secure ORM (`#[derive(Scopable)]`). Per-operation authorization matrix covering all 15+ endpoints. Fail-closed behavior on PDP errors. This is a formalized constraint model, not ad-hoc ownership checks.

**7. Two-tier rate limiting with downgrade cascade.** Premium -> standard tier cascade with per-tier, per-period (daily, monthly) token limits. Preflight resolves `effective_model` before any outbound call. Emergency kill switches (`disable_premium_tier`, `force_standard_tier`, `disable_file_search`, `disable_web_search`). All decisions made before OAGW sees the request.

**8. Context plan assembly with truncation algorithm.** Priority-ordered truncation: retrieval excerpts first, then doc summaries, then old messages. Thread summary and system prompt never truncated. Budget computed after model resolution (downgraded model may have smaller context window).

**9. 80+ Prometheus metrics.** Covering: streaming/UX health, cancellation, quota/cost control, tools/retrieval, RAG quality, thread summary, turn mutations, provider/OAGW, uploads/attachments, image usage, cleanup/drift, audit emission, DB health. Plus alert definitions and SLO thresholds.

**10. PM Comparison Report.** An honest gap analysis between product requirements and the technical design, identifying 14 action items. This shows architectural self-awareness and alignment discipline.

---

## Design B (Mini Chat, PR #626) — Weaknesses / Risks

**W-B1. Linear conversation model limits extensibility.** "P1 does not support branching, history forks, or rewriting arbitrary historical messages. Only the most recent turn may be mutated." This is simpler but means conversation exploration, A/B testing, and non-linear workflows require a redesign, not an extension.

**W-B2. Provider coupling.** Tightly coupled to OpenAI-compatible Responses API (OpenAI / Azure OpenAI). Files stored in provider storage. Vector stores are provider-managed. Multi-provider support (Anthropic, Google) is explicitly deferred. A provider API change or outage has blast radius across the entire system.

**W-B3. No DB failover specification.** The CAS guards, transactional outbox, and atomic billing settlements all depend on PostgreSQL transaction integrity. There is no discussion of what happens during a PostgreSQL failover, connection pool exhaustion, or split-brain scenario. The billing invariants are correct *assuming the database works correctly*, but database failure modes are not addressed.

**W-B4. `minimal_generation_floor` exploitation vector.** A malicious user could rapidly send and immediately cancel many turns, each incurring the 50-token floor charge against the *provider's* resources while the user's quota absorbs only 50 tokens per turn. If the per-turn overhead to the provider exceeds 50 tokens of actual compute, this creates a cost amplification vector. **Acceptable P1 trade-off** — the floor prevents zero-cost abuse, and rate limiting bounds the attack rate.

**W-B5. Outbox dispatcher failure mode.** Section 5.2 documents the outbox dispatcher with SELECT FOR UPDATE SKIP LOCKED row claiming. If the dispatcher fails permanently, outbox rows accumulate indefinitely. The operational guidance says "monitor `mini_chat_usage_outbox_pending` gauge" but there's no automated circuit breaker or dead-letter mechanism. **Acceptable P1 trade-off** — operational monitoring with manual intervention is standard for outbox patterns.

**W-B6. Thread summary quality gate is underspecified.** The sequence diagram shows a "quality gate" on thread summary updates, but the criteria (what constitutes acceptable quality, how hallucination is detected, what happens on quality failure) are not formalized. The summary is LLM-generated and could introduce factual errors into the context window.

**W-B7. Single-provider vector store dependency.** If OpenAI / Azure OpenAI vector store service has an outage, file search becomes unavailable for all chats. There's no fallback, no local index, no graceful degradation. The `disable_file_search` kill switch helps but is a blunt instrument.

**W-B8. Complexity budget is high.** 3,102 lines of DESIGN.md. 8 DB tables. 600+ lines of billing invariants. 80+ metrics. This is a lot of specification surface to implement correctly. The risk is implementation drift — the code may not perfectly match the spec, and the spec is so detailed that partial implementation creates false confidence.

---

## Head-to-Head Comparison

### Billing Correctness

| Criterion | Design A | Design B |
|-----------|----------|----------|
| Reserve/commit semantics | Not specified | Two-phase with preflight reserve and terminal commit |
| Idempotency guarantees | Not addressed | `(turn_id, request_id)` CAS + outbox unique constraint |
| Double-settlement risk | Cannot evaluate (no billing) | Eliminated by CAS guard + first-terminal-wins rule |
| Stuck-reserve risk | Cannot evaluate (no billing) | Orphan watchdog with bounded 5-min timeout |
| Drift between tokens and credits | Not applicable | Token-based internally; credit conversion deferred to presentation layer |
| Deterministic finalization | Not applicable | Explicit formula for all 3 terminal states |

**Winner: Design B.** Design A does not compete — billing is intentionally absent.

### Crash Safety

| Criterion | Design A | Design B |
|-----------|----------|----------|
| Pod crash during streaming | Partial response saved (if pod survives to save it) | Orphan watchdog detects stuck `running` turns, finalizes with CAS |
| Pod crash during billing | Not applicable (no billing) | Atomic DB transaction: state + quota + outbox committed together or not at all |
| Orphan detection | None | Watchdog scans `running` turns older than timeout |
| Recovery mechanism | Stateless restart (routing layer resumes) | CAS guard ensures no double-action on recovery |

**Winner: Design B.** Design A's statelessness means the routing layer recovers, but in-flight operations are lost with no reconciliation.

### Tenant Isolation

| Criterion | Design A | Design B |
|-----------|----------|----------|
| DB query scoping | `tenant_id` WHERE clause | `AccessScope` from PDP compiled to SQL via Secure ORM |
| Provider ID exposure | UUIDs in messages (not provider IDs directly, but webhook can return anything) | Explicit invariant: no `provider_file_id`, `vector_store_id` in any API response |
| RAG/vector store isolation | Not applicable (no RAG in Chat Engine) | Per-chat dedicated vector store (physical + logical isolation) |
| Authorization model | JWT `client_id` ownership checks | PEP/PDP with per-operation matrix, fail-closed |
| Cross-tenant data path | Possible via malicious webhook backend returning another tenant's data | Eliminated by design — domain service scopes all queries |

**Winner: Design B.** Both have tenant_id scoping, but Design B has formalized authorization, vector store isolation, and provider ID sanitization.

### Invariant Clarity

| Criterion | Design A | Design B |
|-----------|----------|----------|
| Stated invariants | Immutable tree, zero business logic, external file storage, sync webhooks, single DB | 30+ explicitly named invariants with `cpt-cf-mini-chat-*` IDs |
| Billing invariants | None | Reserve uncommitted invariant, billing event completeness invariant, first-terminal-wins, CAS guard |
| State machine invariants | Implicit (message `is_complete` flag) | Explicit turn state machine with terminal state immutability |
| Testable invariants | ADR principles (not machine-verifiable) | Machine-verifiable (CAS returns rows_affected, outbox unique constraint) |

**Winner: Design B.** The invariants in Design B are testable, enforceable, and cross-referenced. Design A's invariants are architectural principles, not enforcement mechanisms.

### SSE Correctness

| Criterion | Design A | Design B |
|-----------|----------|----------|
| Event types | 4 (start, chunk, complete, error) | 6 (delta, tool, citations, done, error, ping) |
| Ordering guarantees | Implicit (NDJSON stream order) | Explicit event ordering rules documented |
| Finalization semantics | `complete` event with optional usage metadata | `done` event with usage, effective_model, quota warnings (P2) |
| Replay/reconnect | Not specified | Idempotency via `request_id`, replay invariant, reconnect rule, Turn Status API |
| Exactly-once settlement | Not applicable (no billing) | CAS + outbox = exactly-once per turn |
| Provider error translation | Not applicable (backend controls events) | Provider error normative mapping table with HTTP status -> SSE error code |

**Winner: Design B.** Richer event semantics, explicit recovery mechanisms, and settlement guarantees.

---

## Which Design Is More Production-Ready (P1) and Why

**Design B (Mini Chat) is more production-ready for P1** under the stated constraints.

### Adversarial Analysis

**Assume malicious users:**
- Design A: A malicious user can send unlimited requests (no rate limiting). Can probe webhook backends through Chat Engine (no content validation). Can brute-force share tokens (no protection specified). Cost exposure is unbounded.
- Design B: Rate-limited per user per tier per period. Kill switches for emergency cost control. Preflight quota check rejects before any provider call. `minimal_generation_floor` prevents zero-cost abuse on cancellation.

**Assume pod crashes:**
- Design A: In-flight streaming is lost. Partial response may or may not be saved (race between crash and save). No orphan detection. No billing reconciliation (because no billing). Users see incomplete messages with no recovery path.
- Design B: Orphan watchdog detects stuck turns within 5 minutes. CAS guard ensures exactly-one finalization. Turn Status API provides authoritative state for client recovery. Billing is reconciled regardless of crash timing.

**Assume retries:**
- Design A: No idempotency mechanism. Client retries create duplicate messages in the tree. No `request_id` correlation for streaming turns.
- Design B: `request_id` idempotency key. Parallel turn guard returns 409. Replay invariant: if turn already completed, returns the existing result. Turn Status API lets client check before retrying.

**Assume partial failures:**
- Design A: Webhook times out -> circuit breaker opens for future requests. Current request gets an error. No billing impact (no billing). No cleanup of partial state.
- Design B: Provider error -> post-stream terminal error reconciliation. Reserve settled atomically. Outbox event emitted. Quota adjusted. Turn marked failed. Client gets structured error with `error_code`.

### P1 Verdict

Design B addresses the **hard problems**: billing correctness under failure, crash recovery, malicious user defense, and tenant isolation with formalized authorization. These are the problems that cause production incidents, financial loss, and security breaches.

Design A addresses **extensibility and architectural purity** — valuable qualities, but not the ones that prevent 3 AM pages.

### Caveats

1. Design A may be the right choice if the webhook backends are independently specified and collectively provide the billing/quota/isolation guarantees that Chat Engine intentionally omits. Without those backend specs, the system is incomplete.

2. Design B's complexity (3,102 lines of DESIGN.md) creates implementation risk. If the implementation doesn't match the spec — particularly the CAS guards, outbox atomicity, and formula consistency — the stated guarantees collapse.

3. Design A's simpler data model (4 tables vs 8) means fewer migration risks, fewer join paths, and lower operational complexity *for the routing layer*. The total system complexity is similar — it's just distributed differently.

4. Design A's immutable message tree is architecturally superior for conversation management. If Design B ever needs branching, it will require a significant redesign. **Acceptable P1 trade-off** — linear conversations are sufficient for P1.

---

## Artifact Summary

| Dimension | Design A (Chat Engine) | Design B (Mini Chat) |
|-----------|----------------------|---------------------|
| DESIGN.md | 1,226 lines | 3,102 lines |
| PRD | 1,189 lines | 1,142 lines |
| ADRs | 25 | 3 |
| API Spec | OpenAPI 3.0.3 (1,913 lines) + Webhook (658 lines) | OpenAPI 3.1.0 (1,616 lines) |
| JSON Schemas | ~90 files | Inline in OpenAPI |
| DB Tables | 4 | 8 |
| Billing Spec | 0 lines | 600+ lines |
| Metrics | 5 | 80+ |
| PM Gap Analysis | None | 224 lines |
| Total Files | 127 | 6 |
| Total Additions | 10,011 | 6,166 |
