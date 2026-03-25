extern crate self as modkit_canonical_errors;

pub mod builder;
pub mod context;
pub mod error;
mod gts_validation;
pub mod problem;

#[doc(hidden)]
pub use gts_validation::validate_gts_resource_type;

pub use builder::{ResourceErrorBuilder, ServiceUnavailableBuilder};
pub use context::{
    Aborted, AbortedV1, AlreadyExists, AlreadyExistsV1, Cancelled, CancelledV1, DataLoss,
    DataLossV1, DeadlineExceeded, DeadlineExceededV1, FailedPrecondition, FailedPreconditionV1,
    FieldViolation, FieldViolationV1, Internal, InternalV1, InvalidArgument, InvalidArgumentV1,
    NotFound, NotFoundV1, OutOfRange, OutOfRangeV1, PermissionDenied, PermissionDeniedV1,
    PreconditionViolation, PreconditionViolationV1, QuotaViolation, QuotaViolationV1,
    ResourceExhausted, ResourceExhaustedV1, ServiceUnavailable, ServiceUnavailableV1,
    Unauthenticated, UnauthenticatedV1, Unimplemented, UnimplementedV1, Unknown, UnknownV1,
};
pub use error::CanonicalError;
pub use problem::Problem;

/// Generates a resource error type with builder-returning constructors for the 13 canonical
/// error categories that carry a `resource_type`.
///
/// Generated constructors either accept a detail string or are zero-argument
/// (using a default message). Each returns a `ResourceErrorBuilder` with
/// typestate enforcement — required fields must be set via builder methods
/// (e.g. `.with_resource(...)`, `.with_reason(...)`) before `.create()`
/// compiles.
///
/// Categories where `resource_type` is absent (`internal`,
/// `service_unavailable`, `unauthenticated`) are **not** generated — use
/// `CanonicalError::*()` directly for those.
///
/// The GTS type literal is validated at compile time.
///
/// # Example
///
/// ```ignore
/// resource_error!(TenantResourceError, "gts.cf.core.tenants.tenant.v1~");
///
/// let err = TenantResourceError::not_found("tenant not found")
///     .with_resource("tenant-123")
///     .create();
/// assert_eq!(err.resource_type(), Some("gts.cf.core.tenants.tenant.v1~"));
/// ```
#[macro_export]
macro_rules! resource_error {
    ($vis:vis $name:ident, $gts_type:literal) => {
        const _: () = $crate::validate_gts_resource_type($gts_type);

        $vis struct $name;

        impl $name {
            // --- resource_name required ---

            $vis fn not_found(detail: impl Into<String>)
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceMissing,
                    $crate::builder::NoContext,
                >
            {
                $crate::ResourceErrorBuilder::__not_found($gts_type, detail)
            }

            $vis fn already_exists(detail: impl Into<String>)
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceMissing,
                    $crate::builder::NoContext,
                >
            {
                $crate::ResourceErrorBuilder::__already_exists($gts_type, detail)
            }

            $vis fn data_loss(detail: impl Into<String>)
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceMissing,
                    $crate::builder::NoContext,
                >
            {
                $crate::ResourceErrorBuilder::__data_loss($gts_type, detail)
            }

            // --- resource_name optional ---

            $vis fn aborted(detail: impl Into<String>)
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceOptional,
                    $crate::builder::NeedsReason,
                >
            {
                $crate::ResourceErrorBuilder::__aborted($gts_type, detail)
            }

            $vis fn unknown(detail: impl Into<String>)
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceOptional,
                    $crate::builder::NoContext,
                >
            {
                $crate::ResourceErrorBuilder::__unknown($gts_type, detail)
            }

            $vis fn deadline_exceeded(detail: impl Into<String>)
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceOptional,
                    $crate::builder::NoContext,
                >
            {
                $crate::ResourceErrorBuilder::__deadline_exceeded($gts_type, detail)
            }

            // --- resource_name absent ---

            $vis fn permission_denied()
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceAbsent,
                    $crate::builder::NeedsReason,
                >
            {
                $crate::ResourceErrorBuilder::__permission_denied($gts_type, "You do not have permission to perform this operation")
            }

            $vis fn unimplemented(detail: impl Into<String>)
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceOptional,
                    $crate::builder::NoContext,
                >
            {
                $crate::ResourceErrorBuilder::__unimplemented($gts_type, detail)
            }

            $vis fn cancelled()
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceAbsent,
                    $crate::builder::NoContext,
                >
            {
                $crate::ResourceErrorBuilder::__cancelled($gts_type, "Operation cancelled by the client")
            }

            // --- resource_name optional, needs field violations ---

            $vis fn invalid_argument()
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceOptional,
                    $crate::builder::NeedsFieldViolation,
                >
            {
                $crate::ResourceErrorBuilder::__invalid_argument($gts_type, "Request validation failed")
            }

            $vis fn out_of_range(detail: impl Into<String>)
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceOptional,
                    $crate::builder::NeedsFieldViolation,
                >
            {
                $crate::ResourceErrorBuilder::__out_of_range($gts_type, detail)
            }

            // --- resource_name optional, needs quota violations ---

            $vis fn resource_exhausted(detail: impl Into<String>)
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceOptional,
                    $crate::builder::NeedsQuotaViolation,
                >
            {
                $crate::ResourceErrorBuilder::__resource_exhausted($gts_type, detail)
            }

            // --- resource_name optional, needs precondition violations ---

            $vis fn failed_precondition()
                -> $crate::ResourceErrorBuilder<
                    $crate::builder::ResourceOptional,
                    $crate::builder::NeedsPreconditionViolation,
                >
            {
                $crate::ResourceErrorBuilder::__failed_precondition($gts_type, "Operation precondition not met")
            }
        }
    };
}
