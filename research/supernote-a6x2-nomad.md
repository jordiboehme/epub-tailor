# Supernote A6X2 Nomad (Ratta) - device and EPUB reader

Researched 2026-07-12 against firmware **Chauvet 3.29.42** (2026-06-15, the current stable for Manta and Nomad). Unlike CrossPoint, this firmware is closed source, so everything below is either vendor documentation or reproduced community reports, labelled as such. Where nothing is published, this file says so rather than guessing - the profile leaves those knobs alone.

## Device

| Spec | Value | Source |
|---|---|---|
| Screen | 7.8" glass E Ink, **1404x1872**, **300 PPI** | supernote.com/products/supernote-nomad; manual X2 V3.28.42 p.9 |
| Color | Monochrome, no frontlight (both deliberate, per Ratta FAQ) | supernote.com/pages/supernote-nomad |
| Gray levels | **Not published by Ratta.** The pen UI exposes 4 shades; secondary sources claim 16 for the drawing app | - |
| SoC / RAM | Rockchip RK3566 quad-core 1.8 GHz, 4 GB RAM, 32 GB storage, microSD to 2 TB | supernote.com product page |
| OS | Chauvet, **Android 11-based**. Sideloading is an official setting (Settings > Security & Privacy) | product page; changelog 3.16.27 |
| Formats | .note, PDF, EPUB, Word, TXT, PNG, JPG, BMP, WebP, CBZ, FB2, XPS (+ Kindle app) | supernote.com |

Reader chrome is user-hideable (full-screen mode, hideable page-number bar), and Ratta publishes **no reserved viewport**, so the profile fits images to the full 1404x1872 panel rather than inventing a safe area.

## The EPUB reader

The single most important structural finding: **no EPUB rendering-engine change appears anywhere in the changelog from 3.14.27 (Dec 2023) through 3.29.42 (Jun 2026).** The only EPUB-touching entries are multi-level TOC expand/collapse, the text-selection pen, and Bluetooth page-turner support. The community reports below are mostly 2022-2024, but they describe an engine that has not been reworked since.

### It is not a WebView

A user bisected a table bug **on the A6X2 Nomad itself** ([r/Supernote 19b9ccv](https://www.reddit.com/r/Supernote/comments/19b9ccv/tables_in_epub_not_displayed_correctly/), flaired "Bug: Received" by Ratta):

- `<table><tr><td>` with no section wrappers renders correctly.
- Adding `<thead>` / `<tbody>` collapses the columns into separate rows.
- `rowspan` is not recognized.

Any WebView renders `thead`/`tbody` fine. This is a custom HTML/CSS subset renderer, and it is the reason the profile does not assume modern CSS or EPUB3 semantics.

### Two display modes, and only one of them is usable

The reader offers "Document default setting" and "User-defined". Ratta staff confirmed on the record that switching to User-defined **strips the book's formatting**: *"after change the format, it will remove all the format so it will not keep italics on that"* ([r/Supernote y7aih5](https://www.reddit.com/r/Supernote/comments/y7aih5/formatting_loss_in_epub_files_when_changing_font/)). Font, size, row spacing and margin are only adjustable in that destructive mode.

Consequence for us: **the publisher stylesheet is honored in default mode**, so `filter_css` (which reduces CSS to CrossPoint's ~12-property grammar) must stay off. The community's proven lever is editing the book's own CSS - `@page { margin: 0 }` plus a `font-size` on `html, body` - which is a content edit, not a device transform, and out of scope for a device profile.

### Confirmed working

Inline images render; TOC and internal links work, including EPUB3 nav documents; bookmarks and annotations work. Loading is fast.

### Hard constraints (primary, from the manual)

- Once an EPUB carries a handwritten annotation, **display settings can no longer be changed** (manual p.87).
- **No pinch-zoom in EPUB** (PDF, CBZ, XPS and DOC only, manual p.88) - it is a reflow-only reader.

### Genuinely unknown - do not invent numbers

No source, primary or community, exists for any of these, so the profile does not encode them:

- **`@font-face` / embedded fonts.** Every font discussion concerns *device*-installed fonts in `Document/Fonts`, which force the destructive User-defined mode. Whether the reader loads fonts from inside the container is unknown. The profile therefore leaves `strip_fonts` off: keeping them is lossless either way.
- **SVG.** No Supernote-specific report exists at all. `rasterize_svg` is on because a rasterized image always renders, while an unsupported SVG would be missing content.
- **Max image dimensions, file size, or downscale behavior.** Not documented. `max_source_px` is set to a generous 4096x4096 and is only consulted by `check` to warn.
- **Stylesheet size/complexity cap, large-chapter behavior, EPUB size limits.** Not documented (one anecdote of a ~150 MB EPUB making the UI sluggish). `chapter_split` is therefore off.
- **EPUB2 vs EPUB3.** Directly conflicting community reports; Ratta's answer to a direct query was "no EPUB3", yet users open EPUB3 books from Project Gutenberg successfully. Working hypothesis: containers open and basic content renders, EPUB3-specific features do not.

### What the profile does not fix

The `thead`/`tbody` column collapse is a real, reproduced bug with a known-good markup shape (hoist `<tr>` directly into `<table>`, expand `rowspan`). `epub-tailor` has no transform for that today - `linearize_tables` would flatten the table to labelled paragraphs, which is worse than what the device already renders for simple tables. Left unaddressed on purpose; `--tables image-all` is the available escape hatch, since a rasterized table sidesteps the bug entirely.

## Sources

supernote.com/products/supernote-nomad · support.supernote.com (Manta & Nomad changelog, EPUB and PDF Documents, custom fonts FAQ) · manual X2 V3.28.42 (ib.supernote.com) · r/Supernote threads 19b9ccv (tables, reproduced on Nomad), y7aih5 (Ratta staff on formatting loss), 199acp1 (detailed EPUB experiences), 15mb232, 18hngpg (CSS font-size recipe) · eWritable Nomad review.
