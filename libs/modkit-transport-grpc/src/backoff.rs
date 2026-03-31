//! Shared exponential-backoff helper used by both [`crate::client`] and [`crate::rpc_retry`].

use std::time::Duration;

/// Compute exponential backoff with jitter, clamped to `max_backoff`.
///
/// Formula: `base * 2^(attempt-1)`, capped at `max_backoff`, then `jitter_factor * raw` is
/// added and the result is clamped to `max_backoff` again so that `max_backoff` is always a
/// strict upper bound even after jitter.
///
/// The `jitter_factor` parameter (typically in `[0.0, 0.25]`) is passed in so the function
/// is pure and can be tested deterministically without touching an RNG.
pub fn compute_backoff(
    base: Duration,
    max_backoff: Duration,
    attempt: u32,
    jitter_factor: f64,
) -> Duration {
    let exp = i32::try_from(attempt.saturating_sub(1)).unwrap_or(i32::MAX);
    let factor = 2_f64.powi(exp);
    let raw = if factor.is_finite() {
        base.mul_f64(factor).min(max_backoff)
    } else {
        max_backoff
    };
    (raw + raw.mul_f64(jitter_factor)).min(max_backoff)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn test_compute_backoff_first_attempt_no_jitter() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(5);
        // attempt=1: base * 2^0 = 100ms
        assert_eq!(
            compute_backoff(base, max, 1, 0.0),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn test_compute_backoff_exponential_growth() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(5);
        // attempt=2: 100ms * 2^1 = 200ms
        assert_eq!(
            compute_backoff(base, max, 2, 0.0),
            Duration::from_millis(200)
        );
        // attempt=3: 100ms * 2^2 = 400ms
        assert_eq!(
            compute_backoff(base, max, 3, 0.0),
            Duration::from_millis(400)
        );
    }

    #[test]
    fn test_compute_backoff_capped_at_max() {
        let base = Duration::from_millis(100);
        let max = Duration::from_millis(150);
        // attempt=2 gives 200ms without cap; expect 150ms
        assert_eq!(
            compute_backoff(base, max, 2, 0.0),
            Duration::from_millis(150)
        );
    }

    #[test]
    fn test_compute_backoff_jitter_does_not_exceed_max() {
        let base = Duration::from_millis(100);
        let max = Duration::from_millis(100);
        // With max jitter (25%), raw = 100ms; 100ms + 25ms would be 125ms but must be capped
        assert_eq!(
            compute_backoff(base, max, 1, 0.25),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn test_compute_backoff_jitter_applied() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(5);
        // With 10% jitter: 100ms + 10ms = 110ms
        assert_eq!(
            compute_backoff(base, max, 1, 0.10),
            Duration::from_millis(110)
        );
    }

    #[test]
    fn test_compute_backoff_huge_attempt_does_not_overflow() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(5);
        // Large attempt → exp clamped to i32::MAX, exponential saturates to f64::INFINITY,
        // then .min(max_backoff) clamps the result
        assert_eq!(compute_backoff(base, max, u32::MAX, 0.0), max);
    }
}
