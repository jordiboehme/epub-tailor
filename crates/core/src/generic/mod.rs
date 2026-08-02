//! The de-marking passes behind the `generic` profile: removing what varies
//! between two copies of the same edition, so they converge byte for byte.
//!
//! The convergence criterion is what keeps these passes conservative. We never
//! judge whether a field identifies the buyer, only whether it can differ
//! between copies; anything shared by every copy is left alone.

pub(crate) mod identity;
pub mod invisible;
pub(crate) mod media;
