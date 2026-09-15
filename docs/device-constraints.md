# Device Constraints - Ground Truth

Every transform in the pipeline exists because of a line in this file.

**The bulk of this document describes the Xteink readers running CrossPoint firmware**, whose renderer is a microcontroller-class HTML/CSS subset engine. That is what `x4` and `x3` target, and it is why the pipeline has transforms as aggressive as table linearization and CSS subsetting. Sources: CrossPoint firmware source (release 1.6.0 @ `54337e6d`, 2026-09-05, with 1.6.5rc @ `a1ceb633` noted where it differs) - full citations in [`research/crosspoint-reader-epub-support.md`](../research/crosspoint-reader-epub-support.md).

**Every other device we ship a profile for is far more capable**, and turning the CrossPoint transforms on for them would damage the book. Their constraints, and the evidence behind each profile's switches, live in:

- [`research/supernote-a6x2-nomad.md`](../research/supernote-a6x2-nomad.md) - Supernote Nomad (`nomad`)
- [`research/kindle-epub-ingestion.md`](../research/kindle-epub-ingestion.md) - the Kindle family (`kindle-*`); note a Kindle cannot open an EPUB at all, so the profile targets Amazon's Send-to-Kindle converter
- [`research/kobo-readers.md`](../research/kobo-readers.md) - the Kobo family (`kobo-*`); a plain `.epub` sideload renders through Adobe RMSDK
- [`research/pocketbook-readers.md`](../research/pocketbook-readers.md) - the PocketBook family (`pocketbook-*`); same Adobe engine on the EPUB2 path
- [`research/boox-readers.md`](../research/boox-readers.md) - the Onyx Boox family (`boox-*`); the weakest-evidence target, and the doc says why
- [`research/tolino-readers.md`](../research/tolino-readers.md) - the tolino family (`tolino-*`); the current models run Kobo firmware, the epos 3 does not
- [`research/ereader-market.md`](../research/ereader-market.md) - why these devices and not others, and why we cite no market-share percentages

**Adobe RMSDK is the one constraint that spans families.** It is the engine behind a plain `.epub` on a Kobo, PocketBook's EPUB2 path and tolino's RMSDK mode. Its CSS parser has no fault tolerance: a single `calc()`, `var()` or `clamp()` and it discards the whole stylesheet, or refuses the book. That is what `sanitize_css` exists for.

## Device profiles

| | X4 | X3 |
|---|---|---|
| Screen (portrait) | 480×800 | 528×792 |
| PPI | ~220 | ~220 |
| Panel | `gray4` (2-bit, Bayer-dithered) | `gray4` |
| CPU / RAM | ESP32-C3, ~380-400KB usable SRAM | same |

Inline image target: fit 480×730 (X4 usable reading area), never upscale. Cover: 480×800. Community size budgets: inline <100KB, cover <127KB.

## Images - what the firmware decodes

| Format | Result on device |
|---|---|
| Baseline JPEG | ✅ renders |
| PNG (8-bit) | ✅ renders (alpha flattened onto white) |
| Progressive JPEG | ⚠️ DC-only 1/8-resolution blur |
| GIF / WebP / TIFF / SVG | ❌ `[Image]` placeholder or nothing |
| Over 8,388,608 px in area, or over 32,767 px on a side | ❌ decode aborts (firmware 1.6.0; 2048×1536 before) |
| PNG wider than 8,191 px (8-bit gray) | ❌ row buffer overflow, placeholder |

Converter obligations: transcode everything to baseline grayscale JPEG (photos) or PNG (line art); rasterize SVG; pre-fit to screen; strip `<img width/height>` attributes; strip EXIF/XMP (the firmware probes the first ~1 KB for the JPEG SOF or PNG IHDR before extracting lazily). An `<image href>` inside `<svg>` renders its raster since 1.6.0; vector SVG still does not.

## CSS - the entire supported grammar

- **Selectors**: `tag`, `.class`, `tag.class`, comma groups. Anything containing `+ > [ : # ~ *` or a space is rejected.
- **@-rules**: none. `@font-face`, `@media`, `@import` all skipped structurally.
- **Properties**: `text-align`, `font-style`, `font-weight` (binary: ≥700 = bold), `text-decoration(-line)` (underline/line-through only), `text-indent`, `margin*`/`padding*` (horizontal clamped to 2em), `width`/`height` (on `img` only), `display: none`, `direction`, `vertical-align: super|sub`.
- **Everything else is a no-op**: font-size, font-family, color, background, line-height, borders, float, position, list-style, text-transform, white-space, …
- `!important` is stripped from every value and the value then parsed as usual. Rules with no supported property are dropped before storage; byte-identical stylesheets are read once. Beyond the 1,500-rule cap, selector text is capped at 32 KB and unique declaration bodies at 256 per book (1.6.0).
- `<style>` in `<head>` is never read (`<head>` is skipped entirely). External `.css` files ARE read - via OPF manifest **and** a raw zip scan.
- Caps: 128KB per CSS file, 1,500 rules per book. Inline `style=""` is parsed.

## HTML - supported tags and the traps

- Supported: `h1-h6` (centered by default), `p div blockquote br`, `b strong i em u ins del s strike sup sub`, `hr` (real rule), `img`, `li`, internal `<a href>`.
- **`<li>` renders as "•"** up to firmware 1.6.0, so ordered lists lose numbering; 1.6.5 (rc as of 2026-09-14) numbers `<ol>` items natively and honours `list-style-type: none`. We bake numbers into paragraphs, which renders the same on both.
- **Tables**: since firmware 1.5.0 a simple table renders as a real grid - at most 4 columns, no `colspan`/`rowspan`, no links, cells of at most 32 words and 512 bytes. Anything else is "stacked": every cell becomes an unlabeled paragraph, a nested table flattens into its cell, an `hr` is dropped and an image becomes its alt text. Keep the simple ones, linearize the rest before the device does. Opt-in: `--tables image` rasterizes tables complex enough that flattening would hurt to a line-art PNG, rendering a nested table as an inner grid inside the parent's image (one nesting level; a table nested deeper collapses to text).
- **`<pre>`/`<code>` whitespace collapses**; no monospace exists. Use `<br/>` + `&nbsp;`. Since 1.5.0 a `<br/>` that leaves its block empty (two in a row, or one alone between blocks) costs a full line height, which is what a blank code line should look like; a `<br/>` after text is a plain line break.
- Footnotes are href-based: any internal `<a href="#x">` becomes a footnote entry. `epub:type` ignored; `javascript:` hrefs unparsed. Targets must be `id`s on block elements (span ids dropped; 1,024 anchors/chapter cap). The `hidden` attribute is honoured as `display: none` from 1.6.5rc; do not strip it.
- Words hard-cut at 200 bytes. NBSP honored as non-breaking.

## Text & packaging

- UTF-8 only - the device does not transcode. NFC-normalize (no combining-mark positioning).
- Built-in reader fonts (Noto Serif and Noto Sans, 12/14/16/18 pt) cover Latin, Latin Extended-A, Vietnamese, Cyrillic, punctuation and currency; no Greek, CJK, Arabic or Hebrew glyphs are built in (SD-card fonts can add them). Embedded fonts never load - strip them. `dc:language` picks the hyphenation dictionary and the SD font, so emit the right one.
- `<ruby>`/`<rt>` render natively (`<rp>` is skipped); never flatten ruby.
- DRM (`META-INF/encryption.xml`) crashes the device - refuse with a clear error.
- No ZIP64. `mimetype` first, STORED; everything else DEFLATE.
- TOC: emit both EPUB3 nav and NCX (nav is read first). A TOC entry may target a fragment (`file.xhtml#id`): the firmware forces a page break at the anchor and jumps to it. Only `<nav epub:type="toc">` is read; a `guide` reference is honoured only when its `type` is `start`.
- Spine files over ~200KB become 1,000-page sections that stall indexing - split at heading boundaries.
