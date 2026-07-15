//! EPUB reading: parsing an `.epub` archive into the shared [`model::Book`]
//! representation. Writing/transforming land in later milestones.

pub mod model;
pub mod read;
pub mod stamp;
pub mod write;

pub use model::{Book, Creator, Identifier, Metadata, Resource, Series, TocEntry};
pub use read::{ReadEpub, read_epub};
pub use stamp::{StampInfo, read_stamp, read_stamp_info};
pub use write::{relative_href, write_epub};
