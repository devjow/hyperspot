/// Validates a GTS resource-type literal at compile time.
///
/// Expected format: `gts.<vendor>.<package>.<namespace>.<type>.<version>~`
///
/// Rules enforced (mirrors `validate_gts_format` in the proc-macro crate):
///
/// 1. Must start with `gts.`
/// 2. Must end with `~`
/// 3. After `gts.` and before the trailing `~`, at least 4 dot-separated
///    segments must be present (`vendor.package.namespace.type.version`).
/// 4. Every segment must be non-empty and contain only lowercase ASCII
///    letters, digits, or underscores.
/// 5. The last segment must be a version token: `v` followed by one or more
///    ASCII digits.
///
/// # Panics
///
/// Panics with a descriptive message when the literal is malformed.
/// Inside a `const` context the panic becomes a **compile error**.
pub const fn validate_gts_resource_type(s: &str) {
    let b = s.as_bytes();
    let len = b.len();

    assert!(len > 0, "GTS resource type must not be empty");

    assert!(b[len - 1] == b'~', "GTS resource type must end with '~'");

    assert!(
        len >= 6 && b[0] == b'g' && b[1] == b't' && b[2] == b's' && b[3] == b'.',
        "GTS resource type must start with 'gts.'"
    );

    // Validate the content between "gts." (index 4) and the trailing "~" (index len-1).
    let start = 4;
    let end = len - 1;

    assert!(
        start < end,
        "GTS resource type must have segments after 'gts.' prefix"
    );

    // Walk bytes: count dots, validate characters, record last-dot position.
    let mut dot_count: usize = 0;
    let mut seg_len: usize = 0;
    let mut last_dot: usize = start; // position after last dot (or start)
    let mut i = start;
    while i < end {
        let c = b[i];
        if c == b'.' {
            assert!(seg_len > 0, "GTS resource type contains an empty segment");
            dot_count += 1;
            seg_len = 0;
            last_dot = i + 1;
        } else {
            assert!(
                (c >= b'a' && c <= b'z') || (c >= b'0' && c <= b'9') || c == b'_',
                "GTS resource type segments must contain only lowercase ASCII letters, digits, or underscores"
            );
            seg_len += 1;
        }
        i += 1;
    }
    // Trailing segment must be non-empty.
    assert!(seg_len > 0, "GTS resource type contains an empty segment");

    // Need ≥ 4 dots → 5 segments: vendor.package.namespace.type.version
    assert!(
        dot_count >= 4,
        "GTS resource type must have at least 5 segments after 'gts.': vendor.package.namespace.type.version"
    );

    // --- version segment validation ---
    // last_dot points to the first byte of the version segment.
    assert!(
        b[last_dot] == b'v',
        "GTS resource type must end with a version segment starting with 'v' (e.g. v1)"
    );
    assert!(
        last_dot + 1 < end,
        "GTS resource type version segment must have at least one digit after 'v'"
    );

    let mut k = last_dot + 1;
    while k < end {
        assert!(
            b[k] >= b'0' && b[k] <= b'9',
            "GTS resource type version segment after 'v' must contain only ASCII digits"
        );
        k += 1;
    }
}
