---
status: proposed
date: 2026-03-26
---

# ADR-0007: Do Not Support Stored Responses or Server-Side Conversation State

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Full support for stored responses](#full-support-for-stored-responses)
  - [Partial support (OpenAI pass-through only)](#partial-support-openai-pass-through-only)
  - [No support for stored responses](#no-support-for-stored-responses)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-cf-llm-gateway-adr-no-stored-responses`

## Context and Problem Statement

The Open Responses protocol (adopted in ADR-0005) inherits stateful features from the OpenAI Responses API: `store` (server-side response persistence, defaulting to `true` in OpenAI) and `previous_response_id` (conversation continuation from a stored response). OpenAI uses stored responses to support multi-turn conversations where the consumer sends only a new message and the server appends it to the existing history. On top of this, OpenAI provides server-side conversation compacting, a prompts library, and integration between stored responses and background jobs.

The LLM Gateway must decide whether to support these stateful features given that: (a) cross-provider parity is a core design goal, (b) storing user request/response content raises data retention and access control obligations, and (c) the Gateway's architecture is fundamentally stateless (ADR-0001).

## Decision Drivers

* Cross-provider parity — OpenAI is essentially the only provider supporting server-side stored responses and `previous_response_id` conversation continuation in this format; xAI may support a subset, but this is unverified; all other providers require full message history per request
* Data retention compliance — storing request/response content would require retention policies, access controls, PII management, and tenant data isolation mechanisms that the Gateway currently avoids by being stateless
* Implementation complexity — cross-provider support would require the Gateway to persist all requests and responses in its own database, reconstruct full conversation context from stored history, implement context-window management (compacting with access to text only, not embeddings), and manage conversation lifecycle and cleanup
* Stateless architecture alignment — ADR-0001 established that Gateway does not store conversation history; introducing stored responses would fundamentally contradict this architectural decision
* Minimal viable scope — the Gateway can deliver full value for all supported modalities without server-side conversation state; consumers already manage their own context

## Considered Options

* Full support for stored responses
* Partial support (OpenAI pass-through only)
* No support for stored responses

## Decision Outcome

Chosen option: "No support for stored responses", because it preserves the Gateway's stateless architecture, avoids data retention complexity, and maintains cross-provider parity. Consumers provide full conversation context with each request. Background job results (async mode) remain stored temporarily per the existing data retention NFR.

### Consequences

* Gateway does not need conversation storage infrastructure, context management logic, or server-side compacting — the stateless architecture from ADR-0001 remains intact
* `store` parameter is accepted but forced to `false` regardless of the value provided by the consumer; the response always reflects `store: false`
* `previous_response_id` is not supported; requests that include a non-null `previous_response_id` are rejected with a `capability_not_supported` error
* Consumers building multi-turn conversations must manage their own conversation history and pass all input items with each request
* Background mode (`background: true`) continues to work — job results are stored temporarily per `cpt-cf-llm-gateway-nfr-data-retention-v1` and are not affected by this decision
* OpenAI-specific features that depend on stored responses (conversation compacting, prompts library) are not available through the Gateway

### Confirmation

Implementation verified via:

* Code review confirming `store` is forced to `false` in the API layer before forwarding to provider adapters
* Code review confirming `previous_response_id` is rejected with `capability_not_supported` when non-null
* Integration tests verifying that responses always return `store: false`
* Integration tests verifying that requests with `previous_response_id` return an error
* Verification that background mode continues to function independently of stored responses

## Pros and Cons of the Options

### Full support for stored responses

Implement `store=true` and `previous_response_id` with Gateway-managed conversation storage across all providers.

* Good, because full compatibility with OpenAI Responses API semantics — consumers using OpenAI SDKs get the same behavior
* Good, because simplified consumer experience for multi-turn conversations — consumers send only the new message
* Bad, because requires Gateway to store all request and response content in its own database, introducing data retention, PII management, and tenant isolation requirements
* Bad, because cross-provider support requires sending full reconstructed message history to non-OpenAI providers, which means the Gateway must implement context-window management and conversation compacting with access to text only (no embeddings)
* Bad, because fundamentally contradicts ADR-0001 (stateless design) — Gateway becomes a stateful service with conversation storage, requiring distributed state synchronization for horizontal scaling
* Bad, because significant implementation complexity with limited benefit — most consumers already manage conversation context client-side

### Partial support (OpenAI pass-through only)

Support `store=true` and `previous_response_id` only when the resolved provider is OpenAI, passing through to OpenAI's native stored responses. Reject for all other providers.

* Good, because leverages OpenAI's existing infrastructure for conversation storage — no Gateway-side storage needed for OpenAI requests
* Good, because consumers targeting OpenAI specifically get full feature support
* Bad, because creates provider-dependent behavior — the same API call succeeds or fails depending on provider routing, which is non-obvious to consumers
* Bad, because breaks the Gateway's abstraction — consumers must know which provider they are targeting to use these features
* Bad, because provider fallback becomes impossible for conversations using `previous_response_id` — the stored response exists only on OpenAI's servers

### No support for stored responses

Gateway rejects `previous_response_id` and forces `store=false` for all providers. Consumers provide full context with every request.

* Good, because preserves stateless architecture — no conversation storage, no state synchronization, horizontal scaling remains trivial
* Good, because no data retention obligations for conversation content — Gateway continues to store only temporary async job results
* Good, because consistent behavior across all providers — no provider-dependent feature availability
* Good, because aligns with existing consumer patterns — consumers already provide full context per the stateless design
* Bad, because consumers cannot use server-side conversation continuation even when targeting OpenAI
* Bad, because consumers must implement their own conversation management, including context-window budgeting for long conversations

## More Information

This decision reinforces and extends ADR-0001 (Stateless Gateway Design) in the context of the Open Responses protocol adoption (ADR-0005). While the Open Responses specification includes `store` and `previous_response_id` as protocol-level features, the Gateway implements a subset of the protocol that excludes server-side state. This is consistent with the Gateway's role as a translation and routing layer, not a conversation management platform.

The `store` parameter is accepted (not rejected) to maintain request format compatibility with OpenAI SDKs and the Open Responses specification — but its value is forced to `false`. This avoids breaking clients that send `store: true` by default (as the OpenAI SDK does) while making the Gateway's behavior explicit in the response.

Consumers requiring multi-turn conversation support should manage conversation history at the application layer and pass the full input item array with each request. This pattern is already required for all non-OpenAI providers and is well-supported by the Open Responses protocol's items-based input model.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses the following requirements or design elements:

* `cpt-cf-llm-gateway-adr-stateless` — Reinforces the stateless design by explicitly excluding conversation storage features from the Open Responses protocol implementation
* `cpt-cf-llm-gateway-adr-open-responses-protocol` — Defines the subset of the Open Responses protocol supported by the Gateway, excluding stateful features
* `cpt-cf-llm-gateway-principle-stateless` — Preserves the stateless principle by rejecting features that would require server-side conversation state
* `cpt-cf-llm-gateway-nfr-data-retention-v1` — Avoids introducing new data retention obligations beyond the existing async job result retention
* `cpt-cf-llm-gateway-nfr-scalability-v1` — Maintains horizontal scalability by avoiding distributed conversation state
* `cpt-cf-llm-gateway-constraint-no-stored-responses` — This ADR is the rationale for the no-stored-responses constraint defined in DESIGN.md
