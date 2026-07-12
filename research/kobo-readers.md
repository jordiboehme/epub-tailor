# Rakuten Kobo - devices and EPUB engine

Researched 2026-07-12. Kobo is the best-documented target we have: it publishes a real publisher spec at **`github.com/kobolabs/epub-spec`**, which is the PRIMARY source for everything in the engine section below.

## Devices

Screen specs are PRIMARY from the Kobo eReader Store product pages (`us.kobobooks.com/products/*`), cross-checked against Wikipedia's spec table. Note kobo.com 403s automated fetches, so some numbers were read via Wikipedia citing those same pages.

| Profile | Device | Screen | Panel |
|---|---|---|---|
| `kobo-clara-bw` | Clara BW (Apr 2024), 6" | 1072x1448, 300 PPI¹ | Carta 1300, mono |
| `kobo-clara-colour` | Clara Colour (Apr 2024), 6" | 1072x1448, **300 PPI mono / 150 PPI colour** | Kaleido 3 |
| `kobo-libra-colour` | Libra Colour (Apr 2024), 7" | 1264x1680, **300 / 150 PPI** | Kaleido 3 |
| `kobo-elipsa-2e` | Elipsa 2E (Apr 2023), 10.3" | 1404x1872, **227 PPI** | Carta 1200, mono |

Kobo's own wording for the colour panels is "300 PPI - black-and-white content" and "150 PPI - colour content".

1. **Clara BW's PPI is not published by Kobo** - the page gives only the panel name and `1448 x 1072`. 300 PPI is secondary (Wikipedia, The eBook Reader) and is arithmetically consistent with 1448x1072 on a 6" panel.

**Discontinued, so no profile:** the **Sage** (8", listed but sold out for over a year - treat as EOL, no successor), the **Libra 2** (killed when the Libra Colour launched - there is currently no 7" mono Kobo at all), the **Nia**, and the **Clara 2E**. Kobo shipped **no new e-reader hardware in 2025 or so far in 2026**; the only changes were a silent Clara BW battery revision (P365, Apr 2025) and a white Clara Colour colourway. 2026 models are rumor only.

Also **not published by Kobo**: the 4096-colour figure for Kaleido 3 (that is E Ink's platform spec), gray levels, and refresh rates.

## The engine: two renderers, chosen by file extension

This is the thing to understand about Kobo. From the spec, verbatim:

> "To trigger the Kobo WebKit, change the file extension to `.kepub.epub`."

| Extension | Renderer |
|---|---|
| `.epub` | **Adobe RMSDK** - the default for sideloaded content |
| `.kepub.epub` | Kobo WebKit |

So a plain sideloaded `.epub` - which is exactly what this tool produces - renders through **Adobe RMSDK**. That is why every `kobo-*` profile turns on `sanitize_css`: RMSDK's CSS parser is frozen around 2013 and has no fault tolerance, and one `calc()`, `var()` or `clamp()` reportedly makes it discard the **entire stylesheet** or refuse the book outright.

**Sourcing caveat, stated plainly:** the RMSDK fault-intolerance claim is *not* in Kobo's spec, and neither Adobe nor Kobo document it. It rests on community sources (the `kobofix` project; the write-up *"Your EPUB Is Fine. Kobo Disagrees. Blame Adobe."*) plus PocketBook's manual confirming the same Adobe engine sits under their EPUB2 path. We act on it because the fix is cheap and non-destructive and the failure it prevents is a book that will not open.

**We do not emit `.kepub.epub`.** Kepubifying is not a rename - it means injecting `koboSpan` elements throughout the content, which is what Calibre's KoboTouch driver does. That is a real feature and a separate piece of work.

## What the spec says, and what we do about it

### CSS

- **`page-break-*` does nothing on e-ink.** Kobo's own support matrix marks every `page-break-before/after: always|avoid|left|right` as **N** for EPD (e-ink), and says: *"Creating a new file is the best way to establish page breaks across all Kobo apps."* So `chapter_split` is the only lever that forces a break on a Kobo.
- **Kobo injects its own `div` and `span` tags during processing**, so a bare `div {}` or `span {}` type selector picks up unintended inheritance. Avoid emitting one.
- Avoid `background-color` in reflowable books (it breaks sepia/night mode), `%` font-sizes (known to break the user's font-size control on e-ink and desktop), and `em` margins (they scale with the user's font choice and run away).
- Kobo advises against inline `style=""` entirely; use a linked stylesheet.

### Images

- **JPG, PNG, SVG and WebP are all supported.**
- **3,800,000 px per viewport** (roughly 1950x1950) - hence `max_source_px: [1950, 1950]`, which `check` uses to warn.
- **10 MB of embedded content per XHTML file**; **1 GB per EPUB**.
- Kobo's own pipeline **may re-optimize images**, preserving resolution and aspect ratio.
- Size images in **percentages** (`width: 80%; height: auto`), not fixed pixels.
- The **cover must live in its own XHTML file, referenced with an `<img>`** - never a CSS `background-image`, which cover extraction ignores.

### SVG

Supported on all Kobo platforms, but with one concrete, spec-documented breakage: the Illustrator/InDesign export idiom, where a 1x1 `<image>` is blown up by a scaling matrix, **fails to display on e-ink**:

```xml
<!-- broken on e-ink -->
<g transform="matrix(850.4 0 0 680.3 0 0)">
  <image width="1" height="1" transform="matrix(1 0 0 -1 0 1)" xlink:href="images/img_001.jpg"/>
</g>
```

`rasterize_svg` sidesteps this entirely, so it stays on.

### Fonts

TTF, OTF and WOFF 1.0 are supported, and so is font obfuscation. So `strip_fonts` stays **off** - a Kobo can use them. One catch worth knowing: **faux bold and italic do not work on embedded fonts**; a book must embed separate faces and reference them as distinct families.

### TOC and footnotes

- **An NCX is ignored entirely in EPUB3.** Without a `toc nav`, e-ink falls back to listing raw spine filenames. We regenerate **both** a nav document and an NCX unconditionally, which satisfies Kobo and CrossPoint at once - pinned by a test.
- **Nested TOCs are flattened on e-ink** (three levels work on iOS/Android).
- **Footnotes pop up on e-ink.** Kobo auto-detects a popup from a plain link when all four hold: the link targets an internal node id, the target is **at least 9 characters**, **at most 5000 characters**, and appears **after** the link. A note longer than 5000 characters silently stops popping up.

### Validation

Kobo validates against **EPUBCheck 4.2.6** and recommends distributing only files that pass without flags. Our output is epubcheck-clean, which is a direct win here.

## Unknown / not documented

- **Tables** - the spec does not mention them at all. No documented limits.
- **`<pre>` / monospace** - not mentioned anywhere.
- Whether the store-side image re-optimization also applies to **sideloaded** files.
- Whether colour images from an ordinary EPUB render in colour on the Kaleido models - not addressed by the spec (though it is true of every other Kaleido device we researched, so `panel: "color"` is the safe setting).

## Sources

github.com/kobolabs/epub-spec (PRIMARY: renderers, CSS matrix, images, SVG, fonts, TOC, footnotes, EPUBCheck) · us.kobobooks.com product + compare pages (PRIMARY: screens) · en.wikipedia.org/wiki/Kobo_eReader (spec table citing Kobo's pages) · Good e-Reader and Notebookcheck (Sage discontinued; no 2025/2026 hardware) · The eBook Reader (Clara BW P365 revision; white Clara Colour) · github.com/dmang-dev/kobofix and "Your EPUB Is Fine. Kobo Disagrees. Blame Adobe." (the RMSDK CSS fragility - community, not vendor).
