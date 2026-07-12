//! CSS handling: filter every stylesheet down to the subset the CrossPoint
//! firmware parses ([`subset`]) and enforce the device's byte and rule caps
//! ([`caps`]).
//!
//! The device's CSS grammar is tiny (see `docs/device-constraints.md`), and
//! `<style>` in `<head>` is never read while external `.css` files are, so the
//! pipeline filters every stylesheet in place, relocates head/inline styles into
//! an external sheet, and drops everything outside the supported subset.

pub mod subset;

pub(crate) mod caps;
pub(crate) mod scope;

pub use subset::{FilteredCss, FilteredRule, filter_css, filter_css_rules, filter_inline_style};
