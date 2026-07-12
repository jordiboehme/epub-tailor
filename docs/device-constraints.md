# Device Constraints - Ground Truth

Every transform in the pipeline exists because of a line in this file.

**The bulk of this document describes the Xteink readers running CrossPoint firmware**, whose renderer is a microcontroller-class HTML/CSS subset engine. That is what `x4` and `x3` target, and it is why the pipeline has transforms as aggressive as table linearization and CSS subsetting. Sources: CrossPoint firmware source (develop @ `4b34a576eb`, 2026-07-10) - full citations in [`research/crosspoint-reader-epub-support.md`](../research/crosspoint-reader-epub-support.md).

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
| Anything > 2048×1536 px | ❌ decode aborts |

Converter obligations: transcode everything to baseline grayscale JPEG (photos) or PNG (line art); rasterize SVG; pre-fit to screen; strip `<img width/height>` attributes.

## CSS - the entire supported grammar

- **Selectors**: `tag`, `.class`, `tag.class`, comma groups. Anything containing `+ > [ : # ~ *` or a space is rejected.
- **@-rules**: none. `@font-face`, `@media`, `@import` all skipped structurally.
- **Properties**: `text-align`, `font-style`, `font-weight` (binary: ≥700 = bold), `text-decoration(-line)` (underline/line-through only), `text-indent`, `margin*`/`padding*` (horizontal clamped to 2em), `width`/`height` (on `img` only), `display: none`, `direction`, `vertical-align: super|sub`.
- **Everything else is a no-op**: font-size, font-family, color, background, line-height, borders, float, position, list-style, text-transform, white-space, …
- `<style>` in `<head>` is never read (`<head>` is skipped entirely). External `.css` files ARE read - via OPF manifest **and** a raw zip scan.
- Caps: 128KB per CSS file, 1,500 rules per book. Inline `style=""` is parsed.

## HTML - supported tags and the traps

- Supported: `h1-h6` (centered by default), `p div blockquote br`, `b strong i em u ins del s strike sup sub`, `hr` (real rule), `img`, `li`, internal `<a href>`.
- **`<li>` always renders as "•"** - ordered lists lose numbering. Bake numbers into text.
- **Tables are flattened** to "Tab Row N, Cell M:" paragraphs; the device drops nested tables outright. Linearize before the device does. Opt-in: `--tables image` rasterizes tables complex enough that flattening would hurt to a line-art PNG, rendering a nested table as an inner grid inside the parent's image (one nesting level; a table nested deeper collapses to text).
- **`<pre>`/`<code>` whitespace collapses**; no monospace exists. Use `<br/>` + `&nbsp;`.
- Footnotes are href-based: any internal `<a href="#x">` becomes a footnote entry. `epub:type` ignored; `javascript:` hrefs unparsed. Targets must be `id`s on block elements (span ids dropped; 1,024 anchors/chapter cap).
- Words hard-cut at 200 bytes. NBSP honored as non-breaking.

## Text & packaging

- UTF-8 only - the device does not transcode. NFC-normalize (no combining-mark positioning).
- Built-in fonts cover Latin/Cyrillic/Vietnamese, sizes 8–18pt. Embedded fonts never load - strip them.
- DRM (`META-INF/encryption.xml`) crashes the device - refuse with a clear error.
- No ZIP64. `mimetype` first, STORED; everything else DEFLATE.
- TOC: emit both EPUB3 nav and NCX; every TOC entry must target a spine-file start.
- Spine files over ~200KB become 1,000-page sections that stall indexing - split at heading boundaries.
