//! Perceptual gray-tone remapping (the "palette solver").
//!
//! On a grayscale panel, distinct document colors can collapse into
//! near-identical grays. This module maps each collected text (CSS) and
//! diagram (SVG) color to a gray tone that matches the color's *apparent*
//! brightness ([`space`]) while staying perceptually distinct from the other
//! tones on the target panel ([`solve`]): document CSS colors get one global
//! solve per book, each SVG gets its own. See `docs/profiles.md` for the feature
//! flag (`remap_colors`) and the pipeline hooks in [`crate::convert`].

pub(crate) mod css;
pub(crate) mod palette;
pub(crate) mod solve;
pub(crate) mod space;
pub(crate) mod svg;
