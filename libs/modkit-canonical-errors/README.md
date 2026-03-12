# ModKit Canonical Errors

Canonical error types for CyberFabric modules, based on the [Google AIP-193](https://google.aip.dev/193) error model.

## Overview

The `cf-modkit-canonical-errors` crate provides:

- `CanonicalError` – a structured error type with category, message, and rich context
- `resource_error!` – a macro for declaring resource-scoped error types with generated constructors
- Typed error-context structs for every canonical category (`InvalidArgument`, `NotFound`, `PermissionDenied`, `Internal`, etc.)
- `Problem` – RFC-9457 problem detail representation for HTTP responses

## Usage

### Resource-scoped errors (via macro)

```rust
use cf_modkit_errors::CanonicalError;

cf_modkit_errors::resource_error!(WidgetResourceError, "gts.cf.mymod.widgets.widget.v1~");

// Not-found with a resource identifier
let err = WidgetResourceError::not_found("Widget not found")
    .with_resource("widget-42")
    .create();

// Invalid-argument with field violations
let err = WidgetResourceError::invalid_argument()
    .with_field_violation("name", "must not be empty", "REQUIRED")
    .create();
```

### System-level errors (direct constructors)

```rust
use cf_modkit_errors::CanonicalError;

let err = CanonicalError::unauthenticated()
    .with_reason("TOKEN_EXPIRED")
    .create();

let err = CanonicalError::internal("An internal error occurred. Please retry later.")
    .create();
```

## License

Licensed under Apache-2.0.
