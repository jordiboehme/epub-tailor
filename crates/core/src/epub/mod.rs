//! EPUB reading: parsing an `.epub` archive into the shared [`model::Book`]
//! representation. Writing/transforming land in later milestones.

pub mod model;
pub mod read;
pub mod write;

pub use model::{Book, Metadata, Resource, TocEntry};
pub use read::{ReadEpub, read_epub};
pub use write::{relative_href, write_epub};
