# CrossPoint Reader - EPUB Rendering & Feature Research Report

Repository: `https://github.com/crosspoint-reader/crosspoint-reader` (default branch: **`develop`**). Sections 1-6 cite commit `4b34a576eb` (2026-07-10); section 7 records what changed up to release 1.6.0 (`54337e6d`, 2026-09-05) and pre-release 1.6.5rc (`a1ceb633`, 2026-09-14); section 8 records what changed up to release **2.0.0** (`42d58daa`, 2026-11-01), the firmware the `x4`/`x3` profiles target now. Where sections 1-7 and section 8 disagree, section 8 is current.

## 1. Repo Overview

| Item | Value |
|---|---|
| Name / org | `crosspoint-reader/crosspoint-reader` |
| Description | "Firmware for the Xteink X3 and X4 e-readers" |
| Primary language | C/C++ (Arduino/PlatformIO) |
| Target hardware | **ESP32-C3** (`board = esp32-c3-devkitm-1`), Xteink X4 and X3. `docs/contributing/architecture.md` lines 3-5 |
| Display | 480×800 portrait logical on X4 (528×792 on X3), grayscale e-ink. `lib/GfxRenderer/GfxRenderer.h:33-38`, `lib/Xtc/README:14`, `src/network/html/FilesPage.html:3272-3274` (`DEVICE_PROFILES = { X4: 480×800, X3: 528×792 }`) |
| Build | PlatformIO; deps: ArduinoJson, bitbank2/PNGdec, bitbank2/JPEGDEC (patched), bundled expat, uzlib/InflateReader, custom MiniBidi, freeink-sdk HAL |
| License | MIT |
| Latest release | **v1.4.1** (2026-06-26); v1.4.0 (06-24), v1.3.0 (05-15), v1.2.0 (04-03), v1.0.0 (02-09), first pre-release v0.2.1 (2025-12-03) |
| Activity | 5,889 stars, 1,116 forks, 388 open issues, 100+ contributors, 86 commits/30 days |
| README claims | "EPUB 2/3 rendering with embedded-style option, image handling, hyphenation, kerning, chapter navigation, footnotes, bookmarks, go-to-percent, auto page turn, orientation control, focus reading, KOReader progress sync"; native `.epub`, `.xtc/.xtch`, `.txt`, `.bmp` |

## 2. Rendering Pipeline

1. **Container/OPF discovery** - `lib/Epub/Epub.cpp::findContentOpfFile()` (16-46) reads `META-INF/container.xml` via expat-based `ContainerParser` (`lib/Epub/Epub/parsers/ContainerParser.cpp:52-83`).
2. **OPF parsing** - `Epub::parseContentOpf()` (48-145) drives `ContentOpfParser` (SAX): dc:title/creator/language, manifest, cover detection (meta name="cover", EPUB3 properties="cover-image", guide/HTML-scrape fallback), NCX path, EPUB3 nav path, CSS hrefs. Manifests ≥400 items get hash-sorted binary-search index (`LARGE_SPINE_THRESHOLD`, ContentOpfParser.cpp:141-147).
3. **Spine/TOC caching** - `BookMetadataCache` persists spine + TOC to `book.bin` (format v7, `docs/file-formats.md`). EPUB3 nav (`TocNavParser`) tried first, NCX (`TocNcxParser`) fallback (`Epub.cpp:438-462`).
4. **CSS collection & parsing** - manifest CSS + raw ZIP scan for `.css` (`Epub::discoverCssFilesFromZip()`, 261-283), parsed by `CssParser`, cached to `css_rules.cache`. Guardrails: >128KB CSS files skipped (`MAX_CSS_FILE_SIZE`, Epub.cpp:288), skipped if free heap <64KB (line 290), 1,500-rule cap (`MAX_RULES`, CssParser.cpp:42).
5. **Chapter HTML parse + layout** - `ChapterHtmlSlimParser` (~1,500 lines) streams XHTML through expat in one SAX pass → `ParsedText`/`TextBlock`/`ImageBlock`, applies CSS + inline styles, extracts images to SD cache, paginates into `Page` objects. Resumable (`beginParse()/parseStep()/finishParse()`).
6. **Line breaking** - `ParsedText::layoutAndExtractLines()`: greedy line-fill w/ justification, first-line indent, opt-in Liang hyphenation (`hyphenation/Hyphenator.cpp`), BiDi reordering (`lib/MiniBidi/BidiUtils.cpp`).
7. **Pagination cache** - pages serialized to `sections/*.bin` (v29). Cache key: font, viewport, alignment, hyphenation, embedded-CSS toggle, image mode, focus reading.
8. **Render** - `GfxRenderer` draws to 1-bit + grayscale-plane framebuffer → e-ink via HAL.

## 3. Feature Support Matrix

### 3.1 Container / packaging
| Feature | Supported? | Details / source |
|---|---|---|
| ZIP container | Yes | Central-directory driven, streaming (`lib/ZipFile/ZipFile.h:9-129`) |
| ZIP64 | **No** | All offsets uint32 (`ZipFile.h:11-22`) |
| DRM (ADEPT/LCP) | **No** | No `encryption.xml` handling anywhere; DRM books crash/hang (issue #565) |
| mimetype entry | Generic read | Own Optimizer writes mimetype first, STORE (`FilesPage.html:4741-4745`) |
| Multiple renditions | No | First `application/oebps-package+xml` rootfile only (`ContainerParser.cpp:79`) |

### 3.2 OPF / metadata / TOC
| Feature | Supported? | Details / source |
|---|---|---|
| dc:title/creator/language | Yes | First title only; multiple creators joined ", " (`ContentOpfParser.cpp:108-124,342-348`) |
| Cover detection | Yes | meta name="cover" (image types), properties="cover-image", guide-scrape fallback regexing `xlink:href`/`src` (JPG/PNG only, GIF skipped) (`ContentOpfParser.cpp:161-177,234-249`; `Epub.cpp:84-127`) |
| EPUB3 nav | Yes, preferred | `TocNavParser`; NCX fallback only if nav fails (`Epub.cpp:438-462`) |
| nav landmarks `hidden` | Buggy | Not filtered; pollutes TOC (issue #2181) |
| Guide type=text/start | Yes | Initial reading position (`ContentOpfParser.cpp:311-330`) |
| Nested TOC | Yes | `TocEntry.level` (`BookMetadataCache.h:29-43`) |
| TOC vs spine misalignment | Broken | Issue #383 open |

### 3.3 XHTML tags (`ChapterHtmlSlimParser.cpp:32-39`, handlers 328-1292)
| Tag(s) | Behavior |
|---|---|
| h1-h6 | Block, centered by default; no auto size change (font-size unsupported) (lines 32, 848-860) |
| p, div, blockquote, br | Generic blocks; blockquote unstyled unless CSS margins (33, 861-886) |
| li | Always "•" (U+2022) prefix - **ordered lists never numbered** (883-885); ul/ol containers not specially handled (issue #291) |
| b/strong; i/em | Bold; italic (34-35, 901-940) |
| u/ins | Underline, word-by-word discontinuous (issue #1398) (36, 887-893) |
| del/s/strike | Strikethrough (37, 894-900) |
| sup/sub | Yes, font-style bits (941-956) |
| span | CSS-only effect (957-989) |
| img | JPEG/PNG only, see 3.6 (491-760) |
| a (internal href) | Footnote/link system: non-http(s)/mailto/ftp/tel/javascript hrefs → underline + `FootnoteEntry` (number + href) in Footnotes menu. External links inert. `javascript:` data-attr footnotes NOT parsed (TODO in source) (68-76, 781-823) |
| epub:type noteref | Ignored (all internal a identical); issue #1313 |
| role/epub:type pagebreak | Subtree skipped (769-779) |
| table/tr/td/th | **Flattened**: each cell = paragraph "Tab Row N, Cell M:" (italic); no grid; nested tables dropped (82-84, 422-489, 1193-1199, 1243-1256; issue #876) |
| hr | Real horizontal rule, 25% width centered (266-326, 829-846) |
| head | Skipped entirely - inline `<style>` in head IGNORED (39, 762-767) |
| pre/code | Not handled; whitespace collapses, no monospace (issue #1519) |
| nav/aside/figure/figcaption/audio/video/iframe/dl | Generic passthrough, no special handling |
| SVG inline | Not rendered (no SVG decoder); only cover-scrape (`Epub.cpp:101-127`) |
| Entities | Custom table + expat expansion (`htmlEntities.cpp`) |
| NBSP U+00A0/U+202F | Visible space, non-breaking (1049-1101) |
| Anchors (id=) | Jump targets, capped 1,024/chapter, span IDs excluded unless TOC targets (25-30, 344-375) |

### 3.4 CSS (`CssParser.h:21-32` states subset explicitly)
- Selectors: element, `.class`, `tag.class`, grouped (comma). **Rejected**: anything with `+ > [ : # ~ *` or space (CssParser.cpp:455-468).
- All `@`-rules structurally skipped (506-524): no @font-face, @media, @import.
- Inline `style=""` parsed and applied over rules.
- Properties supported: text-align (l/r/c/justify/start/end), font-style, font-weight (≥700→bold, binary), text-decoration(-line) (underline, line-through only), text-indent (px/em/rem/pt/%), margin*/padding* (1-4 value shorthand; horizontal clamped to 2em, `MAX_HORIZONTAL_INSET_EM`), width/height (img only, px/em/rem/%), display:none (only none modeled), direction (ltr/rtl), vertical-align super/sub.
- **Unsupported (zero effect)**: font-size, font-family, color, background*, line-height, letter/word-spacing, border*, float, position, list-style*, text-transform, white-space, overflow*, box-sizing (exhaustive check of parseDeclarationIntoStyle, CssParser.cpp:310-412).
- Caps: 1,500 rules; 128KB/file; heap bail-outs (<64KB parse, <48KB resolve).
- User toggle "Book's Style" (`embeddedStyle`, default ON) can disable all book CSS.

### 3.5 Typography
- 4 static styles per family (regular/bold/italic/bold-italic), sizes 8/10/12/14/16/18pt baked in. Families: NotoSerif, NotoSans (+ UI fonts).
- Kerning + ligatures in font data (uneven spacing: issue #1182).
- Hyphenation opt-in, Liang tries: de,en,es,fr,it,pl,ru,sv,uk.
- Greedy justification (not Knuth-Plass; issues #1777/#1161).
- User settings: FONT_SIZE S/M/L/XL, LINE_COMPRESSION tight/normal/wide, alignment justified/left/center/right/book-style, screenMargin (default 5).
- Custom SD fonts `.cpfont` via `fontconvert_sdcard.py` or crosspointreader.com/fonts - NOT EPUB-embedded fonts.
- Built-in coverage: Latin, Cyrillic, Vietnamese. NO CJK/Arabic/Greek/Hebrew/Thai/Devanagari (USER_GUIDE.md:577-583); CJK partially via SD fonts since v1.3.0.
- No Arabic contextual shaping (MiniBidi = reorder only, issue #1719). No combining-mark positioning → Hebrew nikud invisible (issue #2312, code comment Epub.cpp:77-79). Text NFC-composed on load.
- UTF-8 assumed; **no transcoding on device** (windows-1252 etc. only handled by browser-side Optimizer's `safeReadText`, FilesPage.html:3744-3771).

### 3.6 Images
| Feature | Details |
|---|---|
| JPEG baseline | Yes, JPEGDEC (patched), grayscale conversion in draw callback (`JpegToFramebufferConverter.cpp`) |
| JPEG progressive | **DC-only 1/8 resolution** blurry path (JpegToFramebufferConverter.cpp:425-459) |
| PNG | Yes, PNGdec; alpha blended onto white; non-8bpp warns (PngToFramebufferConverter.cpp:108-166, 348-350) |
| GIF/WebP/TIFF/SVG | **No** (`ImageDecoderFactory.cpp:14-42`); alt-text `[Image]` placeholder or skipped |
| Max source size | **2048×1536 px** (`MAX_SOURCE_PIXELS`, ImageToFramebufferDecoder.h:36); PNG scanline buffer sized ~2048px wide (`PNG_MAX_BUFFERED_PIXELS=16416`, platformio.ini) |
| Scaling | Downscale-only (never upscale), nearest-neighbor/Bresenham, to CSS size or container width × viewport height |
| Grayscale | Always; luminance (R×77+G×150+B×29)>>8 |
| Dithering | 4×4 ordered Bayer → **4 gray levels (2-bit)** (`DitherUtils.h:5-27`) |
| Modes | Display / Placeholder (alt text) / Suppress (`IMAGE_RENDERING`, CrossPointSettings.h:174) |
| Pixel cache | Decoded rows cached on SD (`PixelCache.h`) |
| Cover gen | JPG/PNG only; ~2000px covers take ~10s; stub-thumbnail bug #2136 |

### 3.7 Other formats
- `.txt`: plain-text reader; sidecar cover (BMP as-is, JPG converted, **PNG rejected**) (`lib/Txt/Txt.cpp:60-156`).
- `.md`: **treated as plain text** (`ReaderActivity.cpp:24`: "Treat .md as txt files (until we have a markdown reader)"; issue #444).
- `.xtc/.xtch`: native pre-rendered bitmap page format (1-bit XTG / 2-bit XTH) at 480×800; produced by external tools (`lib/Xtc/README`).
- `.bmp`: image viewer.

### 3.8 Display/limits
480×800 (X4) / 528×792 (X3); orientations swap to 800×480. Hardware bezel margins top 9px, others 3px (GfxRenderer.h:96-99). ~320KB RAM total; decoders heap-allocated ~20-44KB on demand. Word hard-cut at 200 bytes (`MAX_WORD_SIZE`). Giant single spine files used to become 1,000-page sections with slow indexing and crash reports (#1067, #1752, #2293, #1622, #2047); since 1.5.0 sections index incrementally in the background (8 pages per chunk, 2 chunks per tick, partial section files persist; `Section.h:92-135` at 1.6.0), the page count is a byte-based estimate until done, and a popup warns above 96 KB of HTML. There is no hard size limit in the source; keep chapters under ~200 KB as a quality preference.

## 4. Known limitations (issues)
#876 tables (open), #291 list prefixes (open), #1313 noteref (open), #1398 text-decoration (open), #1777/#1161 justification (open), #1182 kerning (open), #2181 landmarks-hidden (open), #383 TOC/spine misalignment (open), #1519 code blocks (open), #1369 embedded fonts (open), #1825 optimizer font extraction (open), #2312 nikud (open), #604/#1200/#2005 CJK, #1719 Arabic shaping, #1126 Thai, #1516 Devanagari, #565 DRM crash (open), #1067 giant chapters (open), #1645 layout edge case, #2347 image position (open), #1029 centering under justify, #2136 cover stub. Closed/fixed: #292 tables hidden, #756 chapter landing, #1011 large grayscale images, #993/#947 PNG/CSS crashes, #1431/#1289/#712 display:none/image toggle/CSS toggle.

## 5. CrossPoint's built-in browser-side "EPUB Optimizer" (reference implementation)
`src/network/html/FilesPage.html` (~5,700 lines JS + jszip), labeled "ported from EPUB Optimizer Pro" (line 3718):
- All images (PNG/GIF/WebP/BMP/JPEG) → **baseline JPEG**, quality slider default **85** (30/45/60/75/85/95), STORE in zip.
- `DEVICE_PROFILES = { X4: 480×800, X3: 528×792 }`; downscale to screen, never upscale.
- Grayscale default ON; opt-in auto-crop white borders (threshold 245).
- Tall-image splitting (comics): H/V split with 5/10/15% overlap, each part own `<div><img>`, OPF+XHTML updated (3890-3977, 4880-4979).
- SVG cover/image unwrapping to plain `<img>` (`fixSvgCover`, `fixSvgWrappedImages`, 3980-4110); removes `svg` from manifest properties.
- Strips `<img>` width/height attrs post-conversion (4858-4864).
- Injects defensive CSS into head (mostly no-op for CrossPoint's own engine since properties unsupported).
- Encoding: BOM strip, strict UTF-8, fallback charset sniff → windows-1252 → transcode to UTF-8.
- NCX/OPF sync: dtb:uid ↔ unique-identifier, media-type fixes, ensures cover meta.
- Does NOT: extract/convert embedded fonts (#1825), fix tables, split giant chapters, strip DRM.

## 6. Converter recommendations (derived)
**Images**: baseline grayscale JPEG q80-85; fit 480×800 (X4)/528×792 (X3), never upscale; cap 8,388,608 px area and 32,767 px per side (1.6.0; 2048×1536 before); rasterize SVG, convert GIF/WebP; split taller-than-screen images with overlap; strip img width/height attrs.
**HTML**: keep tables the firmware lays out as a grid (≤4 columns, no spans/links/images, cells ≤32 words and ≤512 bytes, 1.5.0+), linearize or render the rest; bake numbers into ordered-list items ("1. …"); pre-format code blocks with explicit `<br/>` + NBSP indentation; footnotes as plain internal `<a href="#…">` (not epub:type-only/javascript:); split spine files >~150-300 layout pages; refuse/flag DRM; block-level anchors only (span-ID cap).
**CSS**: flatten to single selectors (tag/.class/tag.class); strip unsupported properties (shrinks files); move inline head `<style>` into linked stylesheet; <128KB/file, <1500 rules; no @font-face.
**Text**: UTF-8 (transcode legacy inputs), NFC-normalize; avoid decomposed marks; feed logical-order unshaped RTL text.
**Packaging**: mimetype first STORE, rest DEFLATE; EPUB3 nav + NCX both; keep landmarks from polluting TOC; no ZIP64.

## 7. Delta up to 1.6.0 (and 1.6.5rc)

Diffed `4b34a576eb..1.6.0` (`54337e6d`, 2026-09-05) and `..1.6.5rc` (`a1ceb633`, 2026-09-14) on 2026-09-15. Line numbers at 1.6.0 unless marked.

### 7.1 Unchanged (checked)
Selector grammar and `MAX_SELECTOR_LENGTH = 256` (`lib/Epub/Epub/css/CssParser.cpp:663-679`); all @-rules skipped (`:723-733, 746-750`); property chain (`:540-631`, `CssStyle.h` byte-identical); `<style>` in `<head>` skipped (`lib/Epub/Epub/VisibleTextUtils.h:17-20`); 1,500 rules / 128 KB / 64 KB and 48 KB heap guards (`lib/Epub/Epub.cpp:244-246, 322-343`); embedded fonts never loaded; JPEG+PNG only by extension (`converters/ImageDecoderFactory.cpp:14-40`); progressive JPEG DC-only (`JpegToFramebufferConverter.cpp:423-426`); no upscaling, `<img width/height>` ignored (`ChapterHtmlSlimParser.cpp:921-930, 1004-1096`); no `<pre>`/`<code>` handling; headings centered and bold; footnotes href-based, `epub:type` ignored (`:1270-1272`); 1,024 anchors, span ids dropped unless TOC targets (`:41-46, 715-736`); `MAX_WORD_SIZE 200`; UTF-8 only; no ZIP64 (`lib/ZipFile/ZipFile.h:13-19`); no `encryption.xml` handling; X4 480×800 / X3 528×792 (`freeink-sdk/libs/display/FreeInkDisplay/include/FreeInkDisplay.h:76-81`).

### 7.2 Changed
| Area | Now | Evidence | Consequence for epub-tailor |
|---|---|---|---|
| Tables (1.5.0, PR #2654) | Simple rows laid out as a grid: ≤4 columns (`MAX_GRID_TABLE_COLUMNS`), ≤32 words and ≤512 bytes per cell, no colspan/rowspan >1, no internal links, cell width ≥3 line heights; `th` bold; 1-px rule between rows. Otherwise "stacked": each cell an unlabeled full-width paragraph. Nested tables flatten into the cell, `hr` dropped, block tags collapse to a word boundary, images → alt. The "Tab Row N, Cell M:" label is gone. | `ChapterHtmlSlimParser.h:92-103`; `.cpp:48-52, 354-371, 490-535, 556-678, 784-916` | `linearize_tables` keeps grid-fit tables (`native_grid_blocker`) |
| Image decode cap (1.6.0, PR #2959) | 8,388,608 px area + 32,767 px per side (was 2048×1536). PNG rows bounded by `PNG_MAX_BUFFERED_PIXELS = 16416`: max width 8,191 (8-bit gray/indexed), 2,730 (RGB), 2,047 (RGBA). 1/2/4-bit gray and indexed PNG decode; 16-bit fails to a placeholder. | `converters/ImageToFramebufferDecoder.h:49-57`; `platformio.ini:40`; `PngToFramebufferConverter.cpp:111-124, 410-434` | `max_source_px` + `max_source_area_px` |
| `<image href>` in `<svg>` (1.6.0) | The wrapped raster renders; vector SVG still does not. | `ChapterHtmlSlimParser.cpp:60, 918-935` | none, `rasterize_svg` stays |
| Image probing | JPEG SOF / PNG IHDR probed in the first ~1 KB, extraction lazy on first render, failures draw a placeholder. | `ChapterHtmlSlimParser.cpp:960-999`; `converters/ImageDimsProbe.h` | strip EXIF/XMP (already done) |
| Ordered lists (1.6.5rc, PR #3500) | `<ol>` items numbered "N." with per-list counters and nested restart; `list-style-type: none` suppresses the marker; `ul`/`ol` become block tags. 1.6.0 still renders "•". | 1.6.5rc `ChapterHtmlSlimParser.cpp:1390-1417`, `CssParser.cpp:637-641` | none: baked paragraphs render identically |
| `<br>` (1.5.0, PR #2548) | After text: a margin-stripped line break. A `<br>` whose block stays empty injects a full line-height gap. | `ChapterHtmlSlimParser.cpp:378-402, 1358-1380` | none; eyeball code blocks on a device |
| CSS store caps (1.6.0) | `SELECTOR_POOL_CAP = 32 KB` of selector text and `MAX_UNIQUE_STYLES = 256` deduplicated declaration bodies; rules with no supported property dropped before storage; `!important` stripped; byte-identical stylesheets deduplicated by (CRC32, size). | `CssParser.cpp:42-50, 117-138, 300-345, 651-654`; `Epub.cpp:257-318` | doc only |
| `hidden` attribute (1.6.5rc, PR #3390) | Treated as `display: none`. | 1.6.5rc parser diff | do not strip |
| Fonts | Reader fonts are NotoSerif + NotoSans at 12/14/16/18 pt; no Greek, CJK, Arabic or Hebrew built in. SD fonts via a manifest with script groups. Arabic contextual shaping added. Hyphenation adds Finnish; the language comes from `dc:language`. `<ruby>`/`<rt>` render natively, `<rp>` skipped. | `lib/EpdFont/scripts/convert-builtin-fonts.sh:8-10`; `fontconvert.py:44-136`; `sd-fonts.yaml:34-233`; `hyphenation/LanguageRegistry.cpp:35-44`; `ChapterHtmlSlimParser.cpp:1226-1249` | emit a correct `dc:language`; never flatten ruby |
| Footnotes | `FOOTNOTE_HREF_LEN` 96 → 256; label strips `[ ]`, 31 chars; links are tap targets and keep sup/sub; a link inside a grid cell forces the stacked layout. | `FootnoteEntry.h:5-9`; `ChapterHtmlSlimParser.cpp:1556-1582` | doc only |
| OPF / TOC | Namespace-prefix agnostic parsing (PR #3001); guide honours only `type="start"` (PR #2716); metadata fields clamped to 512 bytes (1.6.5rc); TOC fragment targets resolve (already true at base: `TocNavParser.cpp:127-141`, `Section.cpp:397-404`). | `ContentOpfParser.cpp`; `Epub.cpp:511-535` | doc correction |
| Big spine files (1.5.0) | Incremental background indexing replaces the stall; no hard limit; popup above 96 KB of HTML. | `Section.h:92-135`; `Section.cpp:288-293` | keep 200 KB as a preference |
| Devices | Seeed reTerminal Sticky (1.5.0), Xteink X4 Pro and M5Stack PaperMono (1.6.0), Xteink X4 Classic (1.6.5rc): all ESP32-S3 + 8 MB PSRAM, 800×480 B/W panels (PaperMono 3-level), same engine. | `freeink-sdk/README.md:123-135`; `docs/xteink-x4pro-support.md`; `docs/xteink-x4c-support.md` | `x4-pro`, `x4-classic` aliases |
| Cache versions | book.bin v10, sections v45, CSS v11 (v12 at 1.6.5rc); heap comment now ~380 KB. | `BookMetadataCache.cpp:14`; `Section.cpp:50`; `ChapterHtmlSlimParser.cpp:45` | none |

## 8. Delta up to 2.0.0

Researched 2026-11-10. Repo `crosspoint-reader/crosspoint-reader` (not moved). Tags after 1.6.0: `1.6.5rc` (`a1ceb633`, 2026-09-14), `1.6.5` (`93e98bb7`, 2026-09-27), `2.0.0rc` (`91ed3ef8`, 2026-10-19), **`2.0.0` (`42d58daa`, 2026-11-01)**. The real tag name is `2.0.0` (bare, no `v`). freeink-sdk submodule at 2.0.0: `5e94fe74` (1.6.0: `cb9167d5`).

Method: `git diff 1.6.0..2.0.0` on `lib/Epub`, `lib/ZipFile`, `lib/Txt` (51 files, +6505/-1322), then a read of the code at tag `2.0.0`. Line numbers are at `2.0.0` unless marked. `CHSP` = `lib/Epub/Epub/parsers/ChapterHtmlSlimParser`. **[code]** = verified in source; **[notes]** = only in release notes / PR text, not traced. "Landed in" was checked with `git merge-base --is-ancestor` against the tags.

Almost all EPUB rendering changes come from one PR: #3891 "Improve epub rendering" (2.0.0). Others: #3500 ordered lists, #3390 `hidden`, #2925 progressive JPEG (all 1.6.5); #3727 paragraph indentation, #2299 GIF, #4022 Noto Serif to SD (all 2.0.0).

### 8.1 Unchanged (checked) [code]

- **@-rules**: all skipped structurally, including `@media`, `@font-face` and `@import` (`lib/Epub/Epub/css/CssParser.cpp:1273-1293`). So hoisting `@media screen` content is still right.
- **`<style>` in `<head>`**: never read (`lib/Epub/Epub/VisibleTextUtils.h:17-20`). `relocate_styles` is still needed.
- **External CSS**: still found via the OPF and a raw ZIP scan (`lib/Epub/Epub.cpp:247, 517, 577`).
- **CSS caps** (`CssParser.cpp:44-50`; `Epub.cpp:274-276, 351-366`): 1,500 rules (`MAX_RULES`), 32 KB of selector text (`SELECTOR_POOL_CAP`), 256 unique declaration bodies (`MAX_UNIQUE_STYLES`), 256-byte selectors (`MAX_SELECTOR_LENGTH`), 128 KB per file, 64 KB free heap before parsing.
- **CSS parsing details**: `!important` is stripped and the value parsed as usual (`CssParser.cpp:399-420, 947`). Rules with no supported property are dropped (`:1166`). Inline `style=""` is applied over the rules (`CHSP.cpp:1430-1433`).
- **`color`**: still not modelled. `CssStyle` has no color field (`css/CssStyle.h:173-207`), so `color` is a no-op on the device.
- **`list-style-type`**: only `none` vs everything else (`CssStyle.h:75-76`; `CssParser.cpp:1082-1085`). The `list-style` shorthand is **not** parsed.
- **Horizontal margin/padding**: still clamped to 2em (`blocks/BlockStyle.h:16, 132-141`).
- **Embedded fonts**: never loaded. The CSS parser skips `@font-face`, and fonts come only from the SD card (`/fonts`, `.cpfont` / `.ttf` / `.otf`). Issue #1369 was closed as a duplicate of #3939 "Embedded TTF support", which is **open**.
- **Image formats**: JPEG and PNG, chosen by file extension (`converters/ImageDecoderFactory.cpp:29-38`), plus GIF now (see 8.2).
- **Progressive JPEG**: still decoded DC-only at 1/8 resolution (`converters/JpegToFramebufferConverter.cpp:442-474`). 1.6.5 #2925 only fixes progressive files with separated component scans.
- **Decode caps**: `MAX_SOURCE_PIXELS = 8388608` and `MAX_SOURCE_DIMENSION = INT16_MAX` (`converters/ImageToFramebufferDecoder.h:56-57`); `PNG_MAX_BUFFERED_PIXELS = 16416` (`platformio.ini:66`).
- **Image sizing**: never upscaled (`CHSP.cpp:1825-1836`). `<img width/height>` attributes are ignored; only `src`/`href`/`alt` are read (`:1650-1657`), so sizing comes from CSS `width`/`height` only. Images are always horizontally centered (`:1910`).
- **Image placement and gray levels**: images are still probed from the first 1 KB (`:1694-1697`). Still 4 gray levels with 4x4 Bayer dithering (`converters/DitherUtils.h:13-27`). The new "absolute" grayscale mode on S3 boards only changes how the two gray planes are encoded (`DirectPixelWriter.h:47, 173`) and adds no levels. `panel: gray4` stays correct.
- **SVG**: vector SVG is not rendered. `<image href|xlink:href>` is rendered (`CHSP.cpp:61, 1653`).
- **Footnotes and links**: href-based; `epub:type="noteref"` is ignored (#1313 is open). `javascript:` hrefs are skipped and the TODO is still there (`CHSP.cpp:2015-2061`; `isInternalEpubLink` `:132-140`). `FOOTNOTE_HREF_LEN = 256` (`FootnoteEntry.h:6`).
- **Anchors**: at most 1,024 per chapter. `span` ids are dropped unless they are TOC targets (`CHSP.cpp:47, 130, 1393-1407`).
- **Words**: `MAX_WORD_SIZE 200` (`CHSP.h:25`).
- **ZIP**: no ZIP64. All offsets and sizes are `uint32_t`, and the entry count is `uint16_t` (`lib/ZipFile/ZipFile.h:11-22`). The only diff is a lookup cache.
- **Encoding**: no transcoding layer was added. The XML parser is still created with the default encoding (`CHSP.cpp:2874`).
- **TOC**: `TocNavParser`, `TocNcxParser` and `ContainerParser` have **no diff** 1.6.0..2.0.0. Only `<nav epub:type="toc">` is read (`TocNavParser.cpp:71-74`). #2181 (landmarks `hidden`) and #383 (TOC/spine misalignment) are both **open**.
- **`hidden` attribute**: treated as `display:none` (`CHSP.cpp:1410, 1462-1465`). Landed in 1.6.5.
- **Ruby**: still rendered natively; `<rt>` is handled (`CHSP.cpp:1968-1990`). Two 2.0 fixes, #4162 and #4204, improve the layout.
- **Headings**: still centered by default (`CHSP.cpp:2094`).
- **Big-chapter popup**: still at 96 KB (`src/activities/reader/EpubReaderActivity.h:160`). There is still no hard spine-file size limit.
- **Hyphenation**: language still comes from `dc:language`.
- **Screen sizes**: X4 800x480 and X3 792x528 (freeink-sdk `5e94fe74` `libs/display/FreeInkDisplay/include/FreeInkDisplay.h:79-84`).

### 8.2 Changed

The last column says what epub-tailor does about it now.

| Area | Now | Evidence (2.0.0 unless marked) | epub-tailor |
|---|---|---|---|
| **First-line indent (2.0, #3727)** | A user setting, "paragraph indentation" (Off or 1-5 space widths, **default 2**), is applied to every natural-aligned paragraph (justify, or left in LTR). It **overrides a missing, zero or positive CSS `text-indent`**. Only a **negative** `text-indent` (a hanging indent) is kept. In 1.6.0, `text-indent:0` was honoured. | `ParsedText.cpp:759-766, 793-795`; `src/CrossPointSettings.h:290`; 1.6.0 `ParsedText.cpp:664-675`; PR #3727 text | Every paragraph class in `ET_STYLES` uses a negative `text-indent`: `.et-ol-item` hangs its number (`-1.5em` with a matching margin, the nested and continuation classes get their own), `.et-table-cell` and `.et-code` use `-1px`. Negative indents also take the same path on 1.6.0 (1.6.0 `ParsedText.cpp:668-670`). Drawing at a negative offset is not verified on a device |
| `<br>` and blank lines | Same as 1.5/1.6. A `<br>` on an empty block adds one full line height. A `<br>` after text starts a new block and suppresses its first-line indent. A second `<br>` reuses that empty block without resetting the suppression (`startNewTextBlock(style, false)`; the reset only runs when `paragraphEnd`), so the line after a blank line is **not** indented either. | `CHSP.cpp:765-773, 776-793, 2111-2116`; `CHSP.h:245`; `ParsedText.h:165-166` | The 1.6.0 question about tall blank lines in code blocks is closed: blank lines are one line height, with no extra indent. `preserve_code_blocks` pops only trailing blank lines, so a code block that starts with blank lines keeps them |
| `<pre>` / `white-space` (2.0, #3891) | `<pre>` defaults to `white-space: pre`. CSS `white-space: pre / pre-wrap / break-spaces / normal` is parsed. When whitespace is preserved: each `\n` is a line break, each space is kept, a tab becomes 4 spaces, justify falls back to left, and **every line suppresses the first-line indent**. There is still **no monospace font**: a mono font exists only as a whole-book SD font (`sd-fonts.yaml:251-265`). Issue #953 (pre) is still open even though the code landed. | `CHSP.cpp:1456-1459, 697-701, 772, 2424-2442`; `CssParser.cpp:949-955`; `CssStyle.h:205` | `preserve_code_blocks` still emits `<p class="et-code">` with `<br/>` and NBSP, which renders the same on every version. A native `<pre>` would keep inline markup, but it collapses on 1.6.x; expat passes the newline right after `<pre>` through, and a leading `\n` on an empty block adds a blank line (`:772`), so a native `<pre>` would need its leading and trailing newlines stripped. Not done |
| Ordered lists (1.6.5, #3500) | `<ol>` items get `"%d."` from a per-list counter (1, 2, 3...). Nested lists restart. `list-style-type:none` on the list removes the marker. **No** `start`, `value`, `reversed`, `type` or alpha/roman styles, and no compound labels ("2.1."). `<li>` outside a list gets "•". | `CHSP.cpp:2139-2165`; `CHSP.h:185-197`; #291 closed with the comment "start/value extensions remain a separate gap" | `bake_ordered_lists` stays: it honours `start`, `type`, `value` and compound labels, and no `<ol>`/`<li>` reaches the device, so nothing is numbered twice |
| Tables: grid limits (2.0, #3891) | Per-cell limits of 32 words / 512 bytes are **replaced by per-row limits of 160 words / 2,048 bytes** summed over all cells. Still at most 4 columns. Still no colspan/rowspan above 1, and still no internal links (a link forces stacked). Column widths are planned from a pre-scan of the first 32 KB of the table markup, like CSS auto layout. Planned widths **skip the "3 line heights" minimum**, which now applies only to unplanned rows. | `CHSP.h:152-175`; `CHSP.cpp:728-759, 982-1142, 1144-1181, 1576-1580, 2028-2032` | `native_grid_blocker` (`html/tables.rs`) judges per row: 160 words and 2,048 bytes without whitespace (matching `flushPartWordBuffer`), at most 4 columns |
| Tables: rich cells (2.0) | Any header or block tag inside a cell (`p li div br blockquote ul ol pre figure h1-h6`), any `hr` and any image **forces the row to stacked**. In 1.6.0, block tags collapsed to a word boundary, `hr` was dropped and images became alt text. Stacked cells now render images as real images. Nested tables still flatten into the enclosing cell (`CHSP.cpp:2664-2671`). | `CHSP.cpp:56, 142-144, 1639-1644`; 1.6.0 `CHSP.cpp:895-915` | `native_grid_blocker` also blocks `br` ("line breaks in cells") and `li` ("list items in cells"). On 1.6.0 a `<br>` was silently turned into a space, so blocking it is right on both versions. The link, image, `hr`, nested-table and caption blockers stay |
| Tables: borders, padding, caption (2.0) | `border` attribute > 0 or a CSS border on the table or a cell draws cell frames (up to 3 px). Cell `padding` is honoured (up to 1/4 of the viewport). `th` is bold unless CSS says otherwise. Text before the first `<tr>` (the comment says "typically a `<caption>`") is flushed as its own paragraph. | `CHSP.cpp:1536-1543, 1586-1633, 1232-1240, 1549-1553` | The caption path is only a code comment and was not traced, so the "caption" blocker stays. Borders show because `filter_css` now keeps `border*` |
| CSS selectors (2.0, #3891) | Compounds of an element or `*`, one `#id` and up to 4 classes (`p.a.b`, `p#x.a`). **Descendant and child combinators** (`div.poem p`, `blockquote > p`). `::first-letter` / `:first-letter`. Specificity ordering. Still rejected: `+ ~ [` and any other `:`. A rule whose subject compound is empty (a bare `*`) is dropped (`CssParser.cpp:1196`). Ancestors are tracked up to depth 16. At most 8 classes per element are considered and at most 16 matches per element. | `CssParser.h:19-37`; `CssParser.cpp:60-65, 111-199, 1164-1242, 1418-1500`; `CHSP.h:94-95` | `filter_css` keeps this grammar, drops a bare `*` subject and caps a chain at 8 compounds (our cap). `relocate_styles` scopes the new shapes through a `cpr{k}-r` class placed first on `<html>` only (a selector starting on `body` takes the `.cpr{k}-r` prefix like any other); the old `tag`/`.class`/`tag.class` shapes are scoped per element as before |
| CSS properties (2.0, #3891) | New: **`font-size`**: keywords, px, pt, em, rem, %, ex/ch; converted to a scale, clamped 0.5-3.0 and snapped to the installed sizes. **`font-variant(-caps): small-caps / all-small-caps`**. **`page-break-before/after` and `break-before/after`** (always, page, left, right, recto, verso). **`border`, `border-width`, `border-style`, `border-<side>[-width/-style]`** (color ignored, width 1-16 px). **`background` / `background-color`**: shaded only when luminance is 128-239. **`float`** and **`initial-letter`** (drop caps). **`white-space`**. | `CssParser.cpp:938-1144, 260-397, 885-934`; `CssStyle.h:88-205` | `filter_css` keeps all of these. `float` and `list-style-type` keep any value, because the device records every value as an override (`CssParser.cpp:1071-1073, 1082-1085`). Every `list-style` shorthand becomes `list-style-type`. Under `remap_colors`, `background` becomes `background-color` (`#0000` when it has no color). `white-space` keeps only `pre`, `pre-wrap`, `break-spaces` and `normal`. `font-size` is dropped on `html`/`body` and near one (0.85-1.15), and `rem` is rescaled by the book's root size, so body text follows the reader's size setting (`css/font_size.rs`). On `gray4`, the page and whole-chapter wrappers get an inline white background so a page tint does not shade every page (`color/page.rs`). More kept rules means more pressure on the caps: `css/caps.rs` warns for all three store caps, and the generated sheets come first in the manifest |
| Default heading sizes and page flow (2.0) | Without CSS, h1-h6 scale by 2.0, 1.5, 1.17, 1.0, 0.83 and 0.67 (browser defaults). `<small>` scales by 0.83 and `<big>` by 1.2. Headings keep with the next block, and widow/orphan lines carry to the next page. An inline span that opens a block with `font-size >= 1.5em`, or with `float:left`, becomes a drop cap. | `CHSP.cpp:3015-3031, 2100, 2252-2267, 485-494`; `CHSP.h:138-150` | Documented. Built-in sizes are 12/14/16/18 pt only (`lib/EpdFont/scripts/convert-builtin-fonts.sh:7-8`), and the built-in path snaps to the nearest size when growing and rounds down when shrinking (`src/ReaderFontSizes.h:24-45`), so an h1 at 2.0x on a 16 pt body becomes 18 pt |
| Fonts (2.0, #4022; 1.6.5, #3646) | **Noto Serif is no longer built in.** It is an optional SD font, and selecting it without the font falls back to Noto Sans. The built-in reader font is NotoSans 12/14/16/18 only. `.ttf`/`.otf` load from SD on PSRAM boards (1.6.5). Still no EPUB-embedded fonts. Hyphenation adds `pt` (1.6.5): de, en, es, fi, fr, it, pl, pt, ru, sv, uk. | `convert-builtin-fonts.sh:7-17`; `src/SdCardFontSystem.cpp:283-322`; `hyphenation/LanguageRegistry.cpp:33-47` | `strip_fonts` stays. Documented |
| GIF (2.0, #2299) | Static GIF renders, including covers. Only the first frame is drawn. **Interlaced GIFs fail.** Canvas width at most 3,072 (`MAX_WIDTH`). Normal area caps apply. | `converters/GifToFramebufferConverter.cpp:226-261, 367-369`; `ImageDecoderFactory.cpp:39-48`; `platformio.ini:52-53` | None. We transcode GIF anyway, which is safer because interlaced GIFs and size limits still fail |
| Image display options (2.0) | New `IMAGES_DISPLAY_BW` setting: images are drawn with B/W dithering, which avoids the full refresh. "Image Grayscale toggle" (#4160). The EEGO A4 skips image grayscale. | `src/CrossPointSettings.h:207-214`; `EpubReaderActivity.cpp:2074-2092` [code, UX only from notes] | None. It is a reader setting; we cannot detect it |
| Content protection (2.0, #3824, #4016) | If `META-INF/encryption.xml` names `aes128-cbc` or `aes256-cbc`, the book opens through a vendor content-protection SDK. That needs a device-wrapped `<book>.key` written when a **plugin** fulfils the book. Without it, the open fails with a "protected content unavailable" error instead of hanging. Font obfuscation entries are not AES and are ignored. ADEPT/LCP rights are not supported. #565 (DRM hang) was closed via #3724 ("doesn't freeze"). | `lib/Epub/Epub/parsers/EncryptionManifestProbe.h:36-45`; `lib/Epub/ContentProtection.cpp:100-140`; `lib/Epub/Epub.cpp:446-466` | We keep refusing DRM input: we cannot decrypt it, and it would not open on the device anyway |
| OPF metadata (2.0) | Series is read from `calibre:series` / `calibre:series_index` and from EPUB3 `belongs-to-collection` + `group-position`. ISBN/ASIN come from `dc:identifier` (scheme or prefix). Metadata text is clamped to 512 bytes. Guide handling: `book.bin` v10 "ignore ambiguous guide text references". | `parsers/ContentOpfParser.cpp:224-230, 297-303, 543-592`; `BookMetadataCache.cpp:16` | `templates/content.opf` already emits both series forms. `normalize_identity` (off for x4) drops vendor identifiers, which also removes the ISBN/ASIN that KOSync extended metadata uses [notes] |
| Book scale (2.0, #3733, #3784, #4040) | Books with thousands of spine items no longer abort the index build. Cumulative spine sizes live in `book.bin`. Pagination finishes in idle time. | release notes + commit titles [notes] | None. `max_chapter_kb: 200` stays as a quality preference |
| Line breaking (2.0, #3883, #4142, #3985) | Lines can break after hyphens and dashes. Justified spacing is fixed. | commit titles [notes] | None |
| Devices (1.6.5 to 2.0) | New in 2.0: **Metalio E-Ink 4** (800x480 B/W, SSD1677), **BOOX Picco / Tiles** (3.97" 800x480 B/W, SSD1677/SSD2677) and **EEGO Reader A4** (**768x552**, UC8279C, touch; standard and Lite). X4 Classic shipped in 1.6.5. All are ESP32-S3 with the same engine. | freeink-sdk `5e94fe74` `README.md:140, 143`; `platformio.ini:414-418, 625-633, 685-695` | No new profile or alias. Picco and Metalio share the x4 geometry. EEGO A4 is the only new geometry, and it is not Xteink, so it is recorded only; the README says which built-in profile fits each reader |
| Caches | CSS cache v19, section v73, `book.bin` v10. | `CssParser.h:61`; `Section.cpp:84`; `BookMetadataCache.cpp:16` | None |

### 8.3 Light backgrounds and `remap_colors` (resolved)

x4/x3 run `filter_css` with `remap_colors` on, so concrete `background-color` values survive filtering and are remapped. Background tones used to be confined to L* >= 80, and on `gray4` the panel levels are 0/85/170/255, so every remapped background became white and 2.0's shaded boxes could never appear (the firmware shades only luminance 128-239, `CssParser.cpp:392-397`). Now a `gray4` background whose firmware luma `(299R + 587G + 114B) / 1000` is in 128..239 maps to 170, which is inside the band, and every other background maps to 255 (`crosspoint_background` in `color/palette.rs`). This is computed from the code, not tested on a device.

### 8.4 Implemented in epub-tailor

The `x4`/`x3` baseline moved to 2.0.0 (one 2.0 binary ships for X3 and X4). Done:

- Hanging indents for every generated paragraph (`ET_STYLES`), see the first-line indent row.
- Per-row native grid limits and the `br`/`li` blockers in `html/tables.rs`.
- The 2.0 selector grammar and properties in `filter_css` (`css/subset.rs`), with the rewrites listed in the CSS properties row.
- Scoping of the new selector shapes in `relocate_styles` (`css/scope.rs`).
- Light backgrounds kept as the device shade (8.3).
- `docs/device-constraints.md`, `docs/profiles.md` and the README describe 2.0.

Not done: a native `<pre>` for code blocks (see the `<pre>` row). `max_source_px` / `max_source_area_px` stay at `[32767, 32767]` / `8388608`.

### 8.5 Issue status

- **Open**: #2181 landmarks `hidden`; #383 TOC/spine; #1313 noteref; #1777 justification; #2347 image position; #1825 optimizer fonts; #953 `<pre>` (open, but the 2.0 code supports it, see 8.2); #3939 embedded TTF.
- **Closed**: #291 lists (completed by #3500; `start`/`value` is still a gap); #876 tables; #1398 text-decoration; #1067 giant chapters (mitigated by #2452); #565 DRM hang (completed via #3724); #1369 embedded fonts (duplicate of #3939, still open); #1519 code blocks (duplicate of #953).

Not examined: the 2.0.0rc to 2.0.0 diff on its own (the final tag is the reference); body-text NFC on the device (titles are NFC-composed, `Epub.cpp:102-105`; our `unicode_hygiene` stays either way); the web "EPUB Optimizer" changes; OPDS, plugins, BLE and highlights (release notes only, not rendering).

*Sourced read-only from the develop branch via GitHub REST API + raw.githubusercontent.com and the public issue tracker.*


