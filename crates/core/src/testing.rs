//! Test-support helpers shared by proptest suites, both this crate's own
//! `src/` unit tests and its `tests/` integration tests. `#[doc(hidden)]`:
//! this is not part of the crate's real API, it exists only because
//! `crates/core/tests/serialize_roundtrip.rs` cannot reach a `pub(crate)`
//! item (it is compiled as a separate crate against the public surface),
//! and duplicating this three-line helper there would be worse than the one
//! extra `pub` item here.
#![doc(hidden)]

/// The proptest case count to run for a suite whose full count is
/// `default_ci`: that count when `CI` is set (so CI keeps running every
/// case), otherwise the parsed `PROPTEST_CASES` env var, otherwise 32. These
/// are property tests over unoptimized numeric code, so the full counts are
/// slow enough to matter in a local edit/test loop but cheap enough to run
/// unconditionally in CI.
pub fn proptest_cases(default_ci: u32) -> u32 {
    if std::env::var_os("CI").is_some() {
        return default_ci;
    }
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32)
}
