# Security policy

## Report a vulnerability

Please don't open a public issue for a security problem. Use GitHub's private vulnerability reporting instead:

1. Open the [Security tab](https://github.com/jordiboehme/epub-tailor/security) of this repository.
2. Click "Report a vulnerability".
3. Describe the problem and how to reproduce it. A sample book helps a lot.

Only the maintainer can see your report.

## Supported versions

Only the latest release gets security fixes. Please check that the problem still exists there before you report it.

## How epub-tailor treats books

epub-tailor reads books from unknown sources, so it treats every book as untrusted. It never runs code from a book: scripts in a chapter are data to epub-tailor, nothing more.

A book that makes epub-tailor or the desktop app do something it should not (run code, write outside the output folder, read files it was not given) is a security problem. Please report it as described above.
