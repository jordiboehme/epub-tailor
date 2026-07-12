//! CSS handling. Two very different passes, for two very different renderers:
//!
//! - [`subset`] filters a stylesheet down to the dozen properties the
//!   CrossPoint firmware parses (see `docs/device-constraints.md`). It is a
//!   demolition job, and it is only ever right for the Xteink readers.
//! - [`sanitize`] keeps a stylesheet whole and removes only the handful of
//!   modern constructs that make Adobe RMSDK - the engine behind a plain
//!   `.epub` on Kobo, PocketBook's EPUB2 path and tolino's RMSDK mode - throw
//!   the *entire* stylesheet away.
//!
//! [`caps`] enforces the device's byte and rule caps on whatever survives.

pub mod sanitize;
pub mod subset;

pub(crate) mod caps;
pub(crate) mod scope;

pub use sanitize::{SanitizedCss, sanitize_css};
pub use subset::{FilteredCss, FilteredRule, filter_css, filter_css_rules, filter_inline_style};
