---
status: proposed
date: 2026-03-18
---

# ADR-0006: Use Responses API with Custom Extensions for Image Generation

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Dedicated Image Generation API (based on OpenAI `/images/generations`)](#dedicated-image-generation-api-based-on-openai-imagesgenerations)
  - [Responses API with custom CyberFabric extensions](#responses-api-with-custom-cyberfabric-extensions)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-cf-llm-gateway-adr-image-generation-api`

## Context and Problem Statement

LLM Gateway must support image generation as required by the product requirements (`cpt-cf-llm-gateway-fr-image-generation-v1`). The Gateway has adopted the Open Responses protocol as its public API (`cpt-cf-llm-gateway-adr-open-responses-protocol`). However, the Open Responses specification does not define standard output items for binary data such as generated images, nor does it define built-in tools for controlling image generation parameters. The Gateway needs to decide how to expose image generation through its public API.

## Decision Drivers

* API consistency — minimize the number of distinct API paradigms consumers must learn; the Responses API is already the canonical interface per ADR-0005
* Multimodal input/output — the approach should handle both text-to-image and image editing (image+text → image) without requiring separate endpoints
* Async job support — image generation can be long-running and needs `background: true` support without a separate async mechanism
* Open Responses alignment — maintain a single public API surface; avoid fragmenting the interface with legacy endpoint patterns
* Provider coverage — design must accommodate differences across providers (OpenAI DALL-E/gpt-image, Google Gemini/Imagen, Replicate Flux, etc.) while exposing a common set of generation parameters
* Future extensibility — the same extension pattern should apply to other binary outputs (audio synthesis, video generation) without introducing new endpoints each time
* Client compatibility — existing OpenAI SDK clients using the Responses API should be able to trigger image generation with minimal additional configuration

## Considered Options

* Dedicated Image Generation API (based on OpenAI `/images/generations`)
* Responses API with custom CyberFabric extensions

## Decision Outcome

Chosen option: "Responses API with custom CyberFabric extensions", because it maintains the Responses API as the single public interface, natively supports multimodal input/output that fits well with latest OpenAI and Google models, includes async job support via `background: true`, and aligns with the industry trajectory of newer models handling image generation as part of their conversational response flow rather than through a separate endpoint.

### Consequences

* Gateway must define a custom built-in tool type `cyberfabric:image_generation` following the Open Responses hosted tool extension format, covering common generation parameters supported by the majority of providers: aspect ratio, resolution, quality, output format, output compression, and response format
* Gateway must define a custom output item type `cyberfabric:data` following the Open Responses item extension format, representing binary output data with fields for type, status, id, MIME type, and either base64 data or a file storage link
* Gateway must define custom streaming events prefixed with `cyberfabric:` per the Open Responses extension specification: `cyberfabric:response.data.in_progress` when the binary output item begins processing and `cyberfabric:response.data.done` when the item completes with final data
* SDK schemas (`llm-gateway-sdk/schemas/`) must be extended with the new tool type, output item type, and streaming event definitions
* Provider adapters must translate between the `cyberfabric:image_generation` tool parameters and each provider's native image generation API (e.g., OpenAI Images API, Google Gemini, Replicate), and map provider responses back to `cyberfabric:data` output items
* The `cyberfabric:data` output item is intentionally generic (not image-specific) so that audio synthesis, video generation, and other binary outputs can reuse the same item type and streaming events in the future

### Confirmation

Implementation verified via:

* Code review confirming the `cyberfabric:image_generation` tool schema, `cyberfabric:data` output item schema, and `cyberfabric:response.data.*` streaming event schemas are defined in `llm-gateway-sdk/schemas/`
* Integration tests with at least one image generation provider (OpenAI DALL-E or Google Gemini) confirming end-to-end flow through `POST /responses`
* Streaming tests confirming `cyberfabric:response.data.in_progress` and `cyberfabric:response.data.done` events are correctly emitted and contain expected fields (id, status, MIME type, data/link)
* Verification that the `cyberfabric:data` output item can be returned both as base64 inline data and as a file storage URL

## Pros and Cons of the Options

### Dedicated Image Generation API (based on OpenAI `/images/generations`)

A separate REST endpoint modeled on the OpenAI Images API: `POST /images/generations` with prompt, model, size, quality, and response_format parameters. Returns an array of image URLs or base64 data.

* Good, because straightforward API — prompt as input, image as output; simple mental model for consumers
* Good, because supported by existing OpenAI client libraries that implement the Images API
* Good, because well-defined parameter set (size, quality, style) with no ambiguity
* Bad, because legacy API pattern — newer OpenAI and Google models are operating primarily with responses-like multimodal input/output interface
* Bad, because lacks multimodal input/output support — editing images (image+text → image) would require a separate `/images/edits` endpoint, fragmenting the API surface further; no support for multi-turn conversation
* Bad, because no async job support in the standard spec — long-running generation requires a separate polling mechanism outside the Images API
* Bad, because introduces a second API paradigm alongside the Responses API, increasing consumer learning curve and SDK surface area
* Bad, because each new binary modality (audio, video) would require yet another dedicated endpoint

### Responses API with custom CyberFabric extensions

Extend the existing `POST /responses` endpoint with a custom built-in tool (`cyberfabric:image_generation`) that the model or consumer can invoke, and a custom output item (`cyberfabric:data`) that carries the generated binary data. Custom streaming events (`cyberfabric:response.data.*`) signal progress and completion of binary output items.

* Good, because maintains a single public API surface — consumers use `POST /responses` for all operations including image generation
* Good, because full multimodal input/output support — image editing (image+text → image) works naturally by including both `input_image` content parts and the image generation tool in the same request
* Good, because async job support is included via the existing `background: true` mechanism, with no additional API design needed
* Good, because the `cyberfabric:data` output item is reusable for audio, video, and other binary outputs, providing a consistent extension pattern
* Good, because aligns with the direction of major providers — OpenAI's GPT-4o generates images through the Responses API, and Google's Gemini produces images as part of conversational responses
* Good, because follows the Open Responses extension specification for custom types, ensuring interoperability and clear namespacing
* Bad, because the Open Responses specification does not define standard image output items, so the Gateway must introduce custom `cyberfabric:` extensions that clients need to understand
* Bad, because requires custom streaming events (`cyberfabric:response.data.in_progress`, `cyberfabric:response.data.done`) not present in the base Open Responses streaming contract, adding complexity to stream consumers

## More Information

The `cyberfabric:image_generation` tool follows the Open Responses hosted tool extension format. Its parameters cover the common denominator of image generation settings across providers:

| Parameter | Type | Values | Description |
|-----------|------|--------|-------------|
| `aspect_ratio` | string | `1:1`, `2:3`, `3:2`, `3:4`, `4:3`, `9:16`, `16:9` | Output image aspect ratio |
| `resolution` | string | `0.5`, `1`, `2`, `4` | Resolution in megapixels |
| `quality` | string | `low`, `medium`, `high`, `auto` | Generation quality level |
| `output_format` | string | `png`, `jpeg`, `webp` | Image file format |
| `output_compression` | integer | 0–100 | Compression level (applicable to jpeg) |
| `response_format` | string | `base64`, `url` | How the generated image is returned |

The `cyberfabric:data` output item carries binary output with the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | `cyberfabric:data` |
| `id` | string | Unique item identifier |
| `status` | string | `in_progress` or `completed` |
| `mime_type` | string | MIME type of the data (e.g., `image/png`) |
| `base64` | string or null | Base64-encoded data (when `response_format` is `base64`) |
| `url` | string or null | File storage URL (when `response_format` is `url`) |

This decision aligns with the Gateway's pass-through design principle (`cpt-cf-llm-gateway-adr-pass-through`): provider adapters translate between the `cyberfabric:image_generation` tool and each provider's native image generation API, while the API layer treats the tool and output items as opaque typed items.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **Related ADR**: [ADR-0005](./0005-cpt-cf-llm-gateway-adr-open-responses-protocol.md) — Open Responses protocol selection (this ADR extends the protocol with image generation support)

This decision directly addresses the following requirements or design elements:

* `cpt-cf-llm-gateway-fr-image-generation-v1` — Defines how image generation is exposed through the public API using the Responses API with custom extensions
* `cpt-cf-llm-gateway-usecase-image-generation-v1` — The use case flow uses `POST /responses` with the `cyberfabric:image_generation` tool instead of a dedicated endpoint
* `cpt-cf-llm-gateway-adr-open-responses-protocol` — Extends the Open Responses protocol with custom CyberFabric items, tools, and streaming events for binary output
* `cpt-cf-llm-gateway-component-api-layer` — API layer handles the new tool type and output item type within the existing Responses endpoint
* `cpt-cf-llm-gateway-component-provider-adapters` — Adapters translate between `cyberfabric:image_generation` tool parameters and each provider's native image generation API
