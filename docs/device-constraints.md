# Device Constraints - Ground Truth

Every transform in the pipeline exists because of a line in this file.

**The bulk of this document describes the Xteink readers running CrossPoint firmware**, whose renderer is a microcontroller-class HTML/CSS subset engine. That is what `x4` and `x3` target, and it is why the pipeline has transforms as aggressive as table linearization and CSS subsetting. Sources: CrossPoint firmware source (release 2.0.0 @ `42d58daa`, 2026-11-01; line numbers below are at that tag unless marked) - full citations in [`research/crosspoint-reader-epub-support.md`](../research/crosspoint-reader-epub-support.md).

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
| GIF | ⚠️ since 2.0: static, first frame only; an interlaced GIF or one wider than 3,072 px fails (`GifToFramebufferConverter.cpp:249-259`) |
| WebP / TIFF / SVG | ❌ `[Image]` placeholder or nothing |
| Over 8,388,608 px in area, or over 32,767 px on a side | ❌ decode aborts (`ImageToFramebufferDecoder.h:56-57`; since 1.6.0, 2048×1536 before) |
| PNG wider than 8,191 px (8-bit gray) | ❌ row buffer overflow, placeholder |

Converter obligations: transcode everything to baseline grayscale JPEG (photos) or PNG (line art) - GIF included, because the interlaced and wide ones still fail; rasterize SVG; pre-fit to screen; strip `<img width/height>` attributes; strip EXIF/XMP (the firmware probes the first ~1 KB for the JPEG SOF or PNG IHDR before extracting lazily). An `<image href>` inside `<svg>` renders its raster since 1.6.0; vector SVG still does not.

## CSS - the entire supported grammar

Firmware 2.0 widened the grammar a lot (PR #3891). Everything below is `lib/Epub/Epub/css/CssParser.cpp` unless named otherwise.

- **Selectors**: a compound is an element or `*`, at most one `#id` and up to 4 classes (`p.a.b`, `p#x.a`); compounds chain with descendant (` `) and child (` > `) combinators (`div.poem p`, `blockquote > p`); a trailing `::first-letter` or `:first-letter` is allowed; comma groups; specificity orders the rules (`CssParser.h:19-37`; `:111-199, 1164-1242`). Still rejected: `+`, `~`, `[attr]` and every other pseudo. A rule whose last compound names no tag, id or class (a bare `*`) is dropped (`:1196`). The device follows ancestors to a depth of 16 and has no limit on the chain length.
- **@-rules**: none. `@font-face`, `@media`, `@import` all skipped structurally (`:1273-1293`).
- **Properties** (`:938-1144`): `text-align`, `font-style`, `font-weight` (binary: ≥700 = bold), `text-decoration(-line)` (underline/line-through only), `text-indent` (but see the paragraph indent below), `margin*`/`padding*` (horizontal clamped to 2em), `width`/`height` (on `img` only), `display: none`, `direction`, `vertical-align: super|sub`, and new in 2.0:
  - `font-size` in keywords, px, pt, em, rem, %, ex or ch, turned into a scale, clamped to 0.5-3.0 and snapped to the installed sizes;
  - `font-variant(-caps)`: `small-caps` / `all-small-caps`;
  - `page-break-before/after` and `break-before/after`;
  - `border`, `border-width`, `border-style` and the per-side forms, 1-16 px, color ignored;
  - `background` / `background-color`: a box is shaded when the color's luma `(299R + 587G + 114B) / 1000` is in 128..239 and left plain otherwise (`:392-397`);
  - `float: left` and `initial-letter` for drop caps;
  - `white-space`: `pre`, `pre-wrap`, `break-spaces` and `normal` (`:949-955`);
  - `list-style-type`: only `none` versus everything else; the `list-style` shorthand is **not** parsed.
- **Everything else is a no-op**: font-family, `color` (`CssStyle` has no color field), line-height, letter/word-spacing, position, text-transform, …
- **What `filter_css` keeps**: exactly that grammar, with a few choices of our own. A bare `*` subject is dropped like the device would. At most 8 compounds per selector is our cap, not the firmware's. Every `list-style` shorthand becomes `list-style-type`, because the device reads only the long form. When the gray remap is active (`remap_colors` on a grayscale panel), `background` becomes `background-color` (`#0000`, "not shaded", when it has no color). `float` and `list-style-type` keep any value, because the device takes every value as an override of an earlier rule. `white-space` keeps only the four values above.
- **Shaded boxes on `gray4`**: `remap_colors` sends a light background (firmware luma 128..239) to gray 170, which is inside the shade band, so the firmware still shades the box. Every other background, dark ones included, becomes white.
- **Default sizes without CSS**: h1-h6 scale by 2.0, 1.5, 1.17, 1.0, 0.83 and 0.67, `<small>` by 0.83 and `<big>` by 1.2 (`ChapterHtmlSlimParser.cpp:3015-3031`). The built-in font has only 12/14/16/18 pt, so on a 16 pt body an h1 lands on 18 pt.
- `!important` is stripped from every value and the value then parsed as usual. Rules with no supported property are dropped before storage; byte-identical stylesheets are read once. Beyond the 1,500-rule cap, selector text is capped at 32 KB and unique declaration bodies at 256 per book (1.6.0).
- `<style>` in `<head>` is never read (`<head>` is skipped entirely). External `.css` files ARE read - via OPF manifest **and** a raw zip scan.
- Caps: 128KB per CSS file, 1,500 rules per book. Inline `style=""` is parsed.

## HTML - supported tags and the traps

- Supported: `h1-h6` (centered by default), `p div blockquote br`, `b strong i em u ins del s strike sup sub`, `hr` (real rule), `img`, `li`, internal `<a href>`.
- **Ordered lists are numbered natively since 1.6.5, decimal only**: each `<ol>` item gets "1.", "2.", ... from a per-list counter, nested lists restart, and `list-style-type: none` removes the marker. There is no `start`, `value`, `reversed`, `type`, no letters or roman numerals and no compound labels such as "2.1." (`ChapterHtmlSlimParser.cpp:2139-2165`). Up to 1.6.0 every `<li>` was a "•". We bake the numbers into paragraphs, which keeps all of that and never leaves an `<ol>` for the device to number twice.
- **Every paragraph gets a first-line indent since 2.0**: a reader setting (Off or 1-5 space widths, default 2) indents each justified or left-aligned paragraph, and it overrides a missing, zero or positive CSS `text-indent`. Only a negative `text-indent`, a hanging indent, is kept (`ParsedText.cpp:759-766`; `src/CrossPointSettings.h:290`). So every paragraph we generate (baked list items, linearized table cells, code lines) carries a negative `text-indent`: list numbers hang in the margin, and the other blocks use `-1px` so they line up.
- **Tables**: a simple table renders as a real grid - at most 4 columns, no `colspan`/`rowspan`, no links, and since 2.0 at most 160 words and 2,048 non-whitespace bytes per row, summed over its cells (1.5.0 to 1.6.5 capped each cell at 32 words and 512 bytes) (`ChapterHtmlSlimParser.h:152-175`). A block tag in a cell (`p li div br blockquote ul ol pre figure h1-h6`), an `hr` or an image makes the row "stacked" (`ChapterHtmlSlimParser.cpp:56, 1639-1644`): every cell becomes an unlabeled paragraph and a nested table flattens into its cell. Since 2.0 a stacked cell shows its images as images; borders and cell padding are drawn when the CSS asks for them. A `<br>` or `<li>` in a cell therefore stacks the row too, so we linearize the table instead. Keep the simple ones, linearize the rest before the device does. Opt-in: `--tables image` rasterizes tables complex enough that flattening would hurt to a line-art PNG, rendering a nested table as an inner grid inside the parent's image (one nesting level; a table nested deeper collapses to text).
- **`<pre>` keeps its whitespace since 2.0** (`white-space: pre` by default; every `\n` breaks the line, a tab becomes 4 spaces), but the text is still set in the proportional reader font - there is no monospace font unless the reader installs one as the whole-book SD font (`ChapterHtmlSlimParser.cpp:1456-1459, 2424-2442`). Up to 1.6.5 the whitespace collapsed. We still rebuild code with `<br/>` + `&nbsp;`, which renders the same on every version. Since 1.5.0 a `<br/>` that leaves its block empty (two in a row, or one alone between blocks) costs a full line height, which is what a blank code line should look like; a `<br/>` after text is a plain line break.
- Footnotes are href-based: any internal `<a href="#x">` becomes a footnote entry. `epub:type` ignored; `javascript:` hrefs unparsed. Targets must be `id`s on block elements (span ids dropped; 1,024 anchors/chapter cap). The `hidden` attribute is honoured as `display: none` since 1.6.5; do not strip it.
- Words hard-cut at 200 bytes. NBSP honored as non-breaking.

## Text & packaging

- UTF-8 only - the device does not transcode. NFC-normalize (no combining-mark positioning).
- The built-in reader font is Noto Sans at 12/14/16/18 pt; since 2.0 Noto Serif is an optional SD-card font, and picking it without the file falls back to Noto Sans. The built-in font covers Latin, Latin Extended-A, Vietnamese, Cyrillic, punctuation and currency; no Greek, CJK, Arabic or Hebrew glyphs are built in (SD-card fonts can add them). Embedded fonts never load - strip them. `dc:language` picks the hyphenation dictionary (de, en, es, fi, fr, it, pl, pt, ru, sv, uk) and the SD font, so emit the right one.
- `<ruby>`/`<rt>` render natively (`<rp>` is skipped); never flatten ruby.
- DRM (`META-INF/encryption.xml`): up to 1.6.0 such a book hung the device. Since 2.0 an AES-encrypted book opens only with a key a vendor plugin wrote for that device, and otherwise the firmware fails the open with a "protected content unavailable" error (`lib/Epub/Epub.cpp:446-466`); Adobe ADEPT and LCP are not supported. We cannot decrypt it either, so we refuse it with a clear error.
- No ZIP64. `mimetype` first, STORED; everything else DEFLATE.
- TOC: emit both EPUB3 nav and NCX (nav is read first). A TOC entry may target a fragment (`file.xhtml#id`): the firmware forces a page break at the anchor and jumps to it. Only `<nav epub:type="toc">` is read; a `guide` reference is honoured only when its `type` is `start`.
- Big spine files index in the background since 1.5.0 and there is no hard size limit, but the firmware still pops a warning above 96 KB of HTML and page counts stay estimates until indexing ends - we split chapters over ~200KB at heading boundaries as a quality preference.
