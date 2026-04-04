extern crate modkit_canonical_errors;

use modkit_canonical_errors::resource_error;
use modkit_canonical_errors::{CanonicalError, Problem};

#[resource_error("gts.cf.core.users.user.v1~")]
struct R;

#[resource_error("gts.cf.core.test.resource.v1~")]
struct TestR;

#[test]
fn problem_from_not_found_has_correct_fields() {
    let err = R::not_found("Resource not found")
        .with_resource("user-123")
        .create();
    let problem = Problem::from(err);
    assert_eq!(
        problem.problem_type,
        "gts://gts.cf.core.errors.err.v1~cf.core.err.not_found.v1~"
    );
    assert_eq!(problem.title, "Not Found");
    assert_eq!(problem.status, 404);
    assert_eq!(problem.detail, "Resource not found");
    assert_eq!(
        problem.context["resource_type"],
        "gts.cf.core.users.user.v1~"
    );
    assert_eq!(problem.context["resource_name"], "user-123");
}

#[test]
fn problem_json_excludes_none_fields() {
    let err = CanonicalError::service_unavailable()
        .with_retry_after_seconds(30)
        .create();
    let problem = Problem::from(err);
    let json = serde_json::to_value(&problem).unwrap();
    assert!(json.get("trace_id").is_none());
}

#[test]
fn direct_constructor_has_no_resource_type() {
    let err = CanonicalError::service_unavailable()
        .with_retry_after_seconds(30)
        .create();
    assert_eq!(err.resource_type(), None);
    let _problem = Problem::from(err);
}

#[test]
fn problem_json_excludes_resource_type_when_none() {
    let err = CanonicalError::internal("some error").create();
    let problem = Problem::from(err);
    let json = serde_json::to_value(&problem).unwrap();
    assert!(json["context"].get("resource_type").is_none());
}

// =========================================================================
// diagnostic() accessor
// =========================================================================

#[test]
fn diagnostic_returns_description_for_internal() {
    let err = CanonicalError::internal("db pool exhausted").create();
    assert_eq!(err.diagnostic(), Some("db pool exhausted"));
}

#[test]
fn diagnostic_returns_description_for_unknown() {
    let err = TestR::unknown("unexpected upstream response").create();
    assert_eq!(err.diagnostic(), Some("unexpected upstream response"));
}

#[test]
fn diagnostic_returns_none_for_other_categories() {
    let err = TestR::not_found("gone").with_resource("x").create();
    assert_eq!(err.diagnostic(), None);
}

// =========================================================================
// from_error_debug() — non-production path
// =========================================================================

#[test]
fn from_error_debug_includes_description_for_internal() {
    let err = CanonicalError::internal("db pool exhausted").create();
    let problem = Problem::from_error_debug(&err).unwrap();
    assert_eq!(problem.context["description"], "db pool exhausted");
}

#[test]
fn from_error_debug_includes_description_for_unknown() {
    let err = TestR::unknown("unexpected upstream response").create();
    let problem = Problem::from_error_debug(&err).unwrap();
    assert_eq!(
        problem.context["description"],
        "unexpected upstream response"
    );
}

#[test]
fn from_error_does_not_include_description_for_internal() {
    let err = CanonicalError::internal("db pool exhausted").create();
    let problem = Problem::from_error(&err).unwrap();
    assert!(problem.context.get("description").is_none());
}

#[test]
fn from_error_does_not_include_description_for_unknown() {
    let err = TestR::unknown("unexpected upstream response").create();
    let problem = Problem::from_error(&err).unwrap();
    assert!(problem.context.get("description").is_none());
}

#[test]
fn from_error_debug_no_op_for_other_categories() {
    let err = TestR::not_found("gone").with_resource("x").create();
    let normal = Problem::from_error(&err).unwrap();
    let debug = Problem::from_error_debug(&err).unwrap();
    assert_eq!(
        serde_json::to_value(&normal).unwrap(),
        serde_json::to_value(&debug).unwrap(),
    );
}
