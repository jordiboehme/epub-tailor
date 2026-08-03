//! The de-marking passes behind the `generic` profile: removing what varies
//! between two copies of the same edition, so they converge byte for byte.
//!
//! The convergence criterion is what keeps these passes conservative. We never
//! judge whether a field identifies the buyer, only whether it can differ
//! between copies; anything shared by every copy is left alone.

pub(crate) mod identity;
pub mod invisible;
pub(crate) mod media;
pub(crate) mod reachable;

/// Normalize a declared media type for matching: trimmed, lowercased, with
/// any `;`-separated parameter (`; charset=utf-8`) stripped. A manifest is
/// free to declare `IMAGE/JPEG` or `text/css; charset=utf-8` just as validly
/// as the canonical spelling, and matching the raw string verbatim would
/// silently fall through to "not recognized" - shared by [`reachable`]'s
/// reference walk (where that means deleting everything the resource points
/// at) and [`media`]'s metadata strip (where it means shipping the buyer's
/// watermark untouched with no warning). This normalizes casing and
/// parameters only; a non-canonical alias like `image/jpg` is still a
/// different string from `image/jpeg` after normalizing, so a caller that
/// cares about that alias handles it separately.
pub(crate) fn normalize_media_type(media_type: &str) -> String {
    media_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}
