// End-user copy for the CLI's stable error codes (see `ConvertError::code`
// in crates/core/src/error.rs, and the string literals in
// crates/cli/src/batch.rs and crates/cli/src/main.rs for the codes that are
// not tied to a `ConvertError` variant). The raw code stays available to
// the caller for a details view; this only supplies the friendly line.

const COPY: Record<string, string> = {
  "drm-protected":
    "This book is locked with DRM. We do not pick locks - and neither should your converter. Strip the DRM first (Calibre and a DRM removal plugin will do it) and try again.",
  "zip64-unsupported":
    "This EPUB is packaged as a ZIP64 archive, a format most e-readers cannot open either. Re-save it as a standard ZIP and try again.",
  "invalid-epub":
    "This file is not a valid EPUB, or is too damaged to read. Open it in another reader first to check it is not the file that is broken.",
  "empty-spine":
    "This EPUB has no readable chapters, so there is nothing to tailor.",
  "invalid-markdown":
    "This Markdown file could not be parsed. Check the front matter and heading structure and try again.",
  "unsupported-input":
    "This is not a file type we convert. Point us at an EPUB or a Markdown file instead.",
  "image-failed":
    "An image inside this book could not be decoded or resized, so it was left out.",
  "malformed-output":
    "The converted file failed our own sanity check, so nothing was written. This should not happen - please report it.",
  "io-error":
    "A filesystem error interrupted the conversion. Check that the disk has room and that nothing else has the file open.",
  "read-failed":
    "This file could not be read. It may have been moved or deleted, or you may not have permission to open it.",
  "write-failed":
    "The output could not be written. Check that the destination folder exists and is not read-only.",
  "output-collision":
    "A file already sits where this output would land, so nothing was written and nothing was overwritten. Move that file, or pick a different destination or name, and try again.",
  metadata:
    "The metadata you supplied could not be read. Check the document or the fields you typed and try again.",
};

/**
 * Friendly copy for a CLI error `code`, falling back to the CLI's own
 * `message` for a code this app does not (yet) recognize.
 */
export function friendlyError(code: string, message: string): string {
  return COPY[code] ?? message;
}
