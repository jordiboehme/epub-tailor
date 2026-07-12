use thiserror::Error;

/// Errors that can occur while processing a book.
///
/// Messages are written to be shown directly to the end user: each one explains
/// what went wrong and, where possible, why.
#[derive(Error, Debug)]
pub enum ConvertError {
    /// The input EPUB is DRM-protected. Encrypted content cannot be read, let
    /// alone transformed, so this is a hard stop.
    #[error(
        "this book is protected with DRM (found META-INF/encryption.xml); encrypted \
         content cannot be read or transformed. Remove the DRM protection first \
         (e.g. with Calibre + a DRM removal plugin) and try again"
    )]
    DrmProtected,

    /// The input EPUB is a ZIP64 archive, which many e-readers cannot open.
    #[error(
        "this EPUB is a ZIP64 archive, which many e-readers cannot open; \
         re-save it as a standard (non-ZIP64) ZIP archive and try again"
    )]
    Zip64Unsupported,

    /// The input EPUB is structurally invalid or missing required parts.
    #[error("invalid EPUB: {0}")]
    InvalidEpub(String),

    /// The book's spine has no readable content (no itemrefs, or none that
    /// resolve to a manifest item), so there is nothing to convert.
    #[error("the spine has no readable content - nothing to convert")]
    EmptySpine,

    /// The input Markdown could not be parsed or is missing required structure.
    #[error("invalid Markdown: {0}")]
    InvalidMarkdown(String),

    /// The input file is not a format this tool can convert.
    #[error("unsupported input: {0}")]
    UnsupportedInput(String),

    /// An image could not be decoded, transcoded or resized.
    #[error("image processing failed: {0}")]
    Image(String),

    /// An underlying I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl ConvertError {
    /// A stable, machine-matchable code for this failure.
    ///
    /// The message is written for a person and may be reworded at any time; this
    /// is what a GUI or a script should branch on. Same contract as
    /// [`crate::validate::LintFinding::code`].
    pub fn code(&self) -> &'static str {
        match self {
            ConvertError::DrmProtected => "drm-protected",
            ConvertError::Zip64Unsupported => "zip64-unsupported",
            ConvertError::InvalidEpub(_) => "invalid-epub",
            ConvertError::EmptySpine => "empty-spine",
            ConvertError::InvalidMarkdown(_) => "invalid-markdown",
            ConvertError::UnsupportedInput(_) => "unsupported-input",
            ConvertError::Image(_) => "image-failed",
            ConvertError::Io(_) => "io-error",
        }
    }
}
