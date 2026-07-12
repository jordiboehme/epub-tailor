# Kindle - devices and the EPUB ingestion path

Researched 2026-07-12. Primary source of record is Amazon's **Kindle Publishing Guidelines 2026.2** (`https://kindlegen.s3.amazonaws.com/AmazonKindlePublishingGuidelines.pdf`), which is far more specific than the HTML help pages. Section numbers below refer to it.

## The thing to understand first

**A Kindle cannot open an .epub.** Copying one over USB puts the file on the device where it is never indexed. EPUB reaches a Kindle through **Send to Kindle** (email, web, or app), which converts it server-side to Amazon's own format - community-established as KFX, though *Amazon never writes "KFX"* in either the Publishing Guidelines or the Send to Kindle help pages.

So a Kindle profile does not tailor an EPUB for a renderer. It tailors an EPUB **to survive Amazon's converter**. That is a different target, and it is why these profiles disable nearly every device transform: the converter, not the panel, is the consumer.

Amazon's USB help page also notes that Scribe and 2024-and-later Kindles **require a separate app for USB transfer** - they no longer mount as plain mass storage.

### Size limits (primary)

| Path | Limit |
|---|---|
| Send to Kindle, web/app | 200 MB per document |
| Send to Kindle, email | 50 MB total, 25 attachments |
| EPUB source (KDP) | 650 MB (§11.4.3) |
| **Per XHTML file** | **under 30 MB** |
| **HTML files per book** | **under 300** |

The 30 MB per-file cap is the one hard limit `chapter_split` can enforce; `max_chapter_kb` is set to 20480 (20 MB) to stay clear of it without shattering a book into the 300-file cap.

## What the converter honors

This is why `filter_css` is **off** for every Kindle profile. Amazon publishes an explicit KF8 CSS support table, and it is large: `@font-face`, `@import`, all `background-*`, all `border-*` (including `border-radius`), `float`, `position`, `opacity`, `text-shadow`, `line-height`, `letter-spacing`, `z-index`, `white-space`, and selectors `E`, `E.class`, `E#id`, `E:link`. Our `filter_css` reduces CSS to CrossPoint's ~12 properties and would strip nearly all of that.

Explicitly **not** supported, and the actionable list for a future Kindle-specific CSS pass: `counter-increment`/`counter-reset`, `max-width`/`max-height` (though `min-width`/`min-height` work), `outline*` except `outline-offset`, sibling combinators (`E + F`, `E ~ F`), all pseudo-elements, and nearly all structural pseudo-classes (`:first-child`, `:nth-child`, …).

Other primary facts:

- **Embedded fonts**: supported, OTF/TTF only. Type 1 is silently replaced with Kindle fonts. The reader can toggle publisher fonts off. `strip_fonts` therefore stays off.
- **Tables**: KF8 supports nested tables and merged cells; Enhanced Typesetting does not support nesting. Keep tables under ~100 rows and 10-11 columns - a large table silently demotes the whole book out of Enhanced Typesetting. `linearize_tables` stays off: Kindle renders real tables.
- **`<pre>` / `<code>` / `<samp>` / `<kbd>` / `<tt>`**: rendered in a real monospaced font (§11.3.8). `preserve_code_blocks` (which rebuilds code as `<br/>` + `&nbsp;`) stays off.
- **Footnotes**: Amazon *requires* bidirectional links for popup footnotes, and warns that non-footnote links must **not** be bidirectional or they produce spurious popups. `normalize_footnotes` stays off rather than risk disturbing that.
- **TOC**: EPUB3 `nav` and NCX are both accepted; **max two levels of nesting**; an in-content HTML TOC near the front is recommended.

## Images - the numbers that matter

§11.4.1, primary:

> "Kindle devices and reading applications do not support TIFF, multi-frame GIFs, or any image with a transparency."
> "Kindle supports JPEG and PNG. SVG images aren't supported, but SVG rasterization is partially supported."

That is exactly what `transcode_images` delivers, and it is why it stays **on** for Kindle: it flattens alpha onto white (transparency is unsupported outright), and it normalizes GIF/WebP/TIFF/BMP into baseline JPEG or 8-bit PNG.

- **Images below 72 ppi cause the book to FAIL conversion** (§11.4.5). Not degrade - fail.
- Full-page canonical size is 1200x1800; minimum 1200 px wide for a 100%-width image. Fitting to the device panel (1072x1448 / 1264x1680 / 1980x2640, all at 300 PPI) satisfies this.
- **Not published**: any maximum image file size, any maximum pixel dimension, and the threshold at which Amazon downsamples. Amazon confirms it *does* recompress ("Amazon performs automatic image conversions to optimize the content for Kindle") without giving numbers. Our byte budgets are therefore chosen for the Send-to-Kindle size limits, not from an Amazon figure.
- SVG support is real but narrow (integer `viewBox` starting `0 0`, no `<text>`, no namespaces). `rasterize_svg` is on: not worth the gamble.

## The devices

**Amazon publishes no pixel resolution, no RAM and no gray-level count for any Kindle** - only PPI. PCMag states this explicitly. Every pixel dimension below is therefore a **secondary source** (Wikipedia, Good e-Reader), cross-checked against the published diagonal and 300 PPI.

| Profile | Device | Screen | Panel |
|---|---|---|---|
| `kindle` | Kindle 11th gen (2024), 6" | 1072x1448, 300 PPI | mono |
| `kindle-paperwhite` | Paperwhite 12th gen / Signature (2024), 7" | 1264x1680, 300 PPI | mono |
| `kindle-colorsoft` | Colorsoft / Signature, 7" | 1264x1680, 300 PPI mono / **150 PPI color** | **Kaleido 3** |
| `kindle-scribe` | Scribe 3rd gen (2025), 11" | 1980x2640, 300 PPI | mono |
| `kindle-scribe-colorsoft` | Scribe Colorsoft (2025), 11" | 1980x2640, 300 PPI mono / 150 PPI color | **Kaleido 3** |

Geometry check: every figure is exactly the diagonal at 300 PPI (6" -> 1072x1448, 7" -> 1264x1680, 11" -> 1980x2640), which is why the 11" Scribe shares the Colorsoft Scribe's reported resolution even though Amazon publishes neither.

Amazon never says "Kaleido" - it says "our custom Colorsoft display". The Kaleido 3 identification (4096 colors, 16 gray levels, color at half linear density) is from Good e-Reader and E Ink's own platform spec.

Colorsoft **does** show color from a sideloaded/converted EPUB (unlike sideloaded PDFs, which render gray) - so preserving color through the pipeline is the whole point of `"panel": "color"` on those two profiles.

Discontinued: **Kindle Oasis** (2024, Amazon will not restock). The 10.2" 2nd-gen Scribe is superseded and absent from Amazon's 2026 lineup.

### A caveat worth knowing

The converted book Amazon produces syncs across all your Kindles. The mono profiles set `"panel": "gray16"`, which grayscales images - correct and smaller for the device named, but if you also read that same book on a Colorsoft, use `kindle-colorsoft` instead so the color survives.

## Sources

Kindle Publishing Guidelines 2026.2 (PDF) · kdp.amazon.com KF8 CSS/HTML support tables, Table Guidelines · amazon.com Send to Kindle help (nodeId G5WYD9SAF7PGXRNA, G7NECT4B4ZWHQ8WV) and USB transfer help · aboutamazon.com device announcements (2024-10-16, 2025-07-24, Scribe Colorsoft) · PCMag Colorsoft review (Amazon publishes no resolution) · Good e-Reader, the-ebook-reader (resolutions, RAM, Kaleido 3) · MobileRead (KFX rollout, AZW3 sideload failures) · E Ink Kaleido platform page.
