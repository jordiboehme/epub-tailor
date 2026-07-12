# PocketBook - devices and EPUB reader

Researched 2026-07-12. PRIMARY sources are PocketBook's own product pages (`pocketbook.ch`, `products.pocketbook.ch`) and the official firmware manuals on `support.pocketbook-int.com`.

## Why PocketBook matters more than its profile count suggests

Its users mostly **cannot** leave the stock reader. It is the only app on the device that opens their Adobe-DRM and Readium-LCP library books (Onleihe, Libby), so unlike Boox - where the community simply installs KOReader - a PocketBook owner is stuck with PB Reader. Tailoring for it actually lands.

And PB Reader's EPUB2 path is **Adobe RMSDK**. PocketBook says so themselves: the manual's compliance page reads

> "Contains Reader® Mobile technology by Adobe Systems Incorporated"

which is the strongest single fact here. It puts PocketBook on the same fragile CSS engine as a sideloaded `.epub` on a Kobo, and it is why every `pocketbook-*` profile turns on `sanitize_css`. (See `research/kobo-readers.md` for the sourcing caveat on that engine's fault-intolerance: it is community-established, not vendor-documented.)

## Devices

| Profile | Device | Screen | Panel |
|---|---|---|---|
| `pocketbook-verse` | Verse, 6" | 758x1024, **212 PPI** | Carta, mono |
| `pocketbook-verse-pro` | Verse Pro, 6" | 1072x1448, 300 PPI | Carta, mono |
| `pocketbook-era` | Era, 7" | 1264x1680, 300 PPI | Carta 1200, mono |
| `pocketbook-era-color` | Era Color, 7" | 1264x1680 grey / 632x840 colour, **300 / 150 PPI** | Kaleido 3, 4096 colours |
| `pocketbook-inkpad-4` | InkPad 4, 7.8" | 1404x1872, 300 PPI | Carta 1200, mono |
| `pocketbook-inkpad-color-3` | InkPad Color 3, 7.8" | 1404x1872 grey / 702x936 colour, **300 / 150 PPI** | Kaleido 3, 4096 colours |

Also current but not profiled: Verse Lite, Verse Pro Color, and the e-note line (InkPad One, InkPad Eo, Color Note). Discontinued: Basic Lux 4, the 2020 PocketBook Color, InkPad Color 2.

The readers run **Linux** (the Era Color manual states "Operating system: Linux Kernel 4.9.56"), not Android - the InkPad Eo e-note is the exception.

## Formats and DRM

The exact list, from the Era Color manual (firmware 6.8.3283):

> ACSM, AZW, AZW3, CBR, CBZ, CHM, DJVU, DOC, DOCX, EPUB(DRM), EPUB, FB2, FB2.ZIP, HTM, HTML, MOBI, PDF (DRM), PDF, PRC, RTF, TXT

Adobe DRM is fully supported (ACSM download, on-device Adobe account activation, ADE sideloading). Firmware 6.10.x added **Readium LCP** and a built-in Libby app.

## Two EPUB readers, and you can pick

PocketBook exposes the engine choice to the user. From the manual:

> "You can open EPUB files with **PB Reader (EPUB2/EPUB3)**… If you are not happy with the playback quality, long press on a book … to change the playback software."

Community reporting (MobileRead) says the split is **RMSDK for EPUB2 and a WebKit engine for EPUB3**, auto-selected but overridable, and that the EPUB2 reader renders **images noticeably smaller** than the EPUB3 one - opening the same file with "PB Reader (EPUB 3)" shows them much larger. Treat the WebKit attribution as community-level; the *behavior* (two selectable readers that render differently) is independently reported on e-ink hardware.

## Reader features (PRIMARY, from the manual)

- Display settings: line spacing, font size, margin width, hyphenation on/off, font style; contrast/brightness/gamma.
- **Multi-level TOC is supported** ("higher level entry will be marked with '+'").
- **Footnotes are links you jump to, not pop-ups**: "To follow a footnote, internal or external link, touch to enter links mode."

## Image numbers

PocketBook publishes **no device-side image limits at all**. The only published numbers come from **tolino media**-style ingestion guidance on their publishing side, which we do not have for PocketBook - so `max_source_px` is set to a conservative `[2000, 2000]` (~4M px) purely as a `check` warning threshold, matching what tolino publishes. It is not a PocketBook figure and the research says so rather than inventing one.

## Unknown / not documented

Nothing below is guessed; PocketBook simply does not publish it:

- The **CSS subset** honored by either reader. No compatibility table exists.
- Whether PB Reader honors **`@font-face`** embedded fonts. The manual's font tab only offers device fonts, and there is no documented "use publisher styles" toggle (unlike Kobo). `strip_fonts` therefore stays off - keeping them is lossless either way.
- **SVG** support level. `rasterize_svg` is on because a raster always renders.
- **Table** rendering fidelity.
- **Max image dimensions or file size**; whether images are downscaled.
- Which TOC source is read (EPUB3 `nav` vs NCX). We emit both.
- Behavior on very large single XHTML chapters.

## What users actually do

There is **no established "pre-process your EPUB for PocketBook" recipe** in the sources - which is precisely the gap this tool fills. What is well documented is that power users install **KOReader** on PocketBook to escape the stock reader. The catch, and the reason the stock reader still matters: **KOReader cannot open Adobe-DRM books**, so library and store purchases still go through PB Reader.

## Sources

pocketbook.ch and products.pocketbook.ch (PRIMARY: catalog, screens, panels) · support.pocketbook-int.com firmware manuals, esp. `User_Manual_Era_Color_EN.pdf` FW 6.8.3283 (PRIMARY: the Adobe Reader Mobile attribution, format list, the two PB Readers, TOC, footnotes, display settings) · MobileRead PocketBook forum (the RMSDK/WebKit split; EPUB2-vs-EPUB3 image scaling) · "Your EPUB Is Fine. Kobo Disagrees. Blame Adobe." (RMSDK's CSS fault-intolerance - community) · Good e-Reader, The eBook Reader, Notebookcheck (prices, release dates, Era Lite).
