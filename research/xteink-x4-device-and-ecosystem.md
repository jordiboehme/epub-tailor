# Xteink X4 & CrossPoint Ecosystem

## Device specs (Xteink X4)
| Spec | Value |
|---|---|
| Screen | 4.3" E Ink, 480×800 portrait, ~219-220 PPI |
| Grayscale | Panel driven 1-bit + grayscale planes; CrossPoint quantizes images to 4 levels; native XTC=1-bit, XTCH=2-bit |
| CPU / RAM | ESP32-C3 (single-core RISC-V), ~380-400 KB usable SRAM (Xteink publicly corrected a "128MB" listing error, Apr 2026) |
| Storage | microSD (ships 16GB, up to 256-512GB), exFAT recommended |
| USB-C | Charging + firmware flashing ONLY - book transfer via Wi-Fi web UI / WebDAV / Calibre plugin / OPDS / SD card |
| Refresh | Partial per page; full refresh every 1/5/10/15/30 pages (configurable) |
| Battery | 650 mAh, ~2 weeks at 1-3h/day |
| Input | Buttons only, no touch, no frontlight |
| Size | 114×69×5.9mm, ~75g, MagSafe-mountable |

## CrossPoint user-facing capabilities
EPUB 2/3 + txt + xtc/xtch + bmp; embedded-style toggle; footnote navigation w/ return; bookmarks; go-to-percent; auto page turn; 4 orientations; Focus Reading; KOReader kosync progress sync; Wi-Fi file transfer web UI incl. built-in browser-side EPUB Optimizer; WebDAV; Calibre wireless plugin (with optimize-on-send); OPDS client; OTA updates. Fonts: NotoSerif/NotoSans built-in (Latin/Cyrillic/Vietnamese only); custom `.cpfont` SD fonts for CJK/Hebrew/Arabic/Greek etc. Docs state GIF + progressive JPEG unsupported → `[Image]` placeholder; large covers ~10s conversion.

## User-reported EPUB pain points
Stock firmware layout "pretty much nonexistent" (MobileRead); even on CrossPoint reviewers report spotty bold/italic styling in some books, hit-or-miss TOC navigation, images not appearing (older versions); DRM books unsupported everywhere (crash risk); pre-converted .xtc ignores reader font settings.

## Related org repos
crosspoint-reader (firmware) · crosspoint-tools (site, flasher, font builder) · escape-hatch (recovery) · crosspoint-simulator (desktop sim) · crosspoint-fonts (SD font packs) · community-sdk / freeink-sdk (HAL) · calibre-plugins (wireless driver + EPUB optimizer port by @zgredex) · Murphy (other-device RE) · JPEGDEC fork. Community forks: papyrix-reader (FB2/MD/HTML, Arabic/Thai shaping, Knuth-Plass), CrossInk, crosspoint-reader-cjk, inx, CrossMux.

## Prior art / competing converters
- **bigbag/epub-to-xtc-converter** (JS/Node): EPUB→XTC/XTCH via CREngine-WASM (X4/X3 presets, fonts, hyphenation 42 langs, dithering) + an Optimizer mode (strip floats/flex/grid/fixed CSS, strip embedded fonts, grayscale/resize/flatten alpha, drop tiny decorative images, baseline JPEG re-encode, remove SVG/WebP/TIFF, inject e-paper CSS, batch+ZIP).
- **papyrix xteink-epub-optimizer** (CLI): CSS sanitization, font stripping, resize-to-480px, grayscale.
- **CrazyCoder/cr2xt**: desktop EPUB/RTF/DOC/HTML/MOBI/TXT→XTC; XTC format spec published as Gist.
- **x4converter.rho.sh**, **epub2xtc.xteink.cn** (official), **Xlibre**, **XTCJS.app** (CBZ/PDF→XTC).
- **zgredex/baseline_jpg_converter**: fixes progressive-JPEG covers; logic ported into firmware + Calibre plugin.
- No Calibre output profile exists for X4; plugin-based instead.

## Community EPUB prep recommendations
DRM-free EPUB2/3; baseline JPEG/PNG only; full-page images 480×800 <100KB, inline 480px wide <100KB, covers 480×800 <127KB pre-scaled; grayscale+dither; strip complex CSS (float/flex/grid/fixed) + embedded fonts; UTF-8; exFAT SD; run an optimizer when formatting breaks; convert to .xtc only when reflow can be sacrificed.

## Sources
crosspointreader.com/docs.html · github.com/crosspoint-reader/* · github.com/bigbag/epub-to-xtc-converter · github.com/bigbag/papyrix-reader · xteink.com/products/xteink-x4 · Wirecutter X3/X4 review · Lifehacker X4 review · MobileRead thread 370369 · joshualowcock.com X4 guide + memory-dispute article · CrazyCoder XTC gist. (Reddit not directly verified - sign-in wall.)
