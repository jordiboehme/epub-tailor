# CrossPoint Reader - EPUB Rendering & Feature Research Report

Repository: `https://github.com/crosspoint-reader/crosspoint-reader` (default branch: **`develop`**). Sections 1-6 cite commit `4b34a576eb` (2026-07-10); section 7 records what changed up to release 1.6.0 (`54337e6d`, 2026-09-05) and pre-release 1.6.5rc (`a1ceb633`, 2026-09-14).

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

*Sourced read-only from the develop branch via GitHub REST API + raw.githubusercontent.com and the public issue tracker.*


