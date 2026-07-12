# tolino (Thalia alliance) - devices and EPUB engine

Researched 2026-07-12. Sources are largely German and are labelled PRIMARY (mytolino.de / thalia.de / tolino-media.de) or COMMUNITY (e-reader-forum.de, lesen.net, literatur-digital.de, MobileRead).

## The headline finding: there are two tolino platforms

**The current tolinos are not Android any more. They run Kobo's firmware.**

| Generation | Models | OS / engine | Firmware |
|---|---|---|---|
| **New** (May 2024 ->) | shine 5th gen, shine color, vision color | **Kobo Linux, Qt6 - not Android** | 5.15.x / 5.16.x (2026) |
| **Old** (frozen) | **epos 3**, vision 6, shine 4, page 2 | Android, custom tolino reader | 16.2.x (2024, no updates since) |

Proof is on tolino's own update page: firmware for the new models is served from **`ereaderfiles.kobo.com`**, with filenames like `tolino-qt6-update-5.15.253009`, and users are told to copy it into the **`.kobo`** folder. The old models get theirs from `download.pageplace.de`.

This is why `tolino-epos-3` is a separate profile rather than a geometry tweak: it is a different reading engine, a different format list (EPUB/PDF/TXT only, no CBZ/MOBI) and a different DRM flow.

## The devices

All four are current as of 2026-07 per tolino's own comparison page. All are 300 PPI.

| Profile | Screen | Panel |
|---|---|---|
| `tolino-vision-color` | 7", **1264x1680** | Kaleido 3, 4096 colors, 300 PPI mono / **150 PPI color** |
| `tolino-shine-color` | 6", **1072x1448** | Kaleido 3, 4096 colors, 300 / 150 PPI |
| `tolino-shine` (5th gen) | 6", **1072x1448** | Carta 1300, mono |
| `tolino-epos-3` | 8", **1440x1920** | Carta 1200, mono, **16 gray levels** (the only model tolino publishes this for) |

Naming: the current mono 6" model is officially "tolino shine (5. Generation)", marketed as plain *tolino shine*. There is no epos 4, no vision 7, no shine 6. Color images from an ordinary sideloaded EPUB **do** display in color on the two Kaleido models, at half resolution.

## What the engine does

Kobo ships two renderers and picks by file extension: `.epub` goes to the Adobe RMSDK engine, `.kepub.epub` to the Kobo WebKit. **tolino does not follow that split** - it sends everything through its own WebKit-derived default renderer (MobileRead: *"the default renderer on Tolino's v5 software is very similar to the WebKit based renderer on a Kobo"*).

Consequences, all COMMUNITY-confirmed on firmware 5.x:

- The publisher stylesheet **is** applied, so `filter_css` stays off.
- **`@font-face` is ignored** by the default renderer. Embedded fonts simply do not load, even with "Verlagsschrift" selected. (RMSDK mode does support them; it can be switched on via `EpubSideloadedRenderer=RMSDK` in `.kobo/Kobo/Kobo eReader.conf`.) We still leave `strip_fonts` off - stripping is destructive and buys nothing, since ignored fonts cost only bytes.
- **`page-break-*`, orphans and widows are ignored** on the e-ink engine. Kobo's own spec says the same, and adds: *"Creating a new file is the best way to establish page breaks."*
- **`text-align` and book margins are overridden** by the reader.
- Real HTML tables render; Kobo warns that tables wider than four columns may be unreadable on e-ink. `linearize_tables` stays off - flattening would be worse than what the device already does.
- **SVG scaling is unreliable** in tolino mode. `rasterize_svg` is on.
- In-book links were **broken entirely** until firmware 5.13 (2026-01), which finally underlined and enabled them.
- **Do not ship `.kepub.epub` to a tolino**: it is recognized but renders with the same engine, performs worse, and often breaks the TOC. Calibre already skips kepubification when a tolino is attached.

### The Absatzbug - a real bug we do not yet fix

The default renderer mishandles **comma-grouped CSS selectors**. This is ignored for `p`:

```css
body, div, p, h1, li { margin: 0; padding: 0; }
```

while the identical rule as its own selector works:

```css
p { margin: 0; padding: 0; }
```

The standard German-community fix is to inject exactly that single-selector rule via Calibre's KoboTouch driver ("Modify CSS"). `epub-tailor` has no transform that ungroups selectors, so the profiles do not fix this. It is the highest-value tolino-specific transform we could add.

## Image and size numbers

The only published tolino figures come from **tolino media**, their self-publishing ingestion platform - these are production requirements, **not** documented device limits, but they are the only numbers that exist and they are sane targets:

- **JPG or PNG only, RGB only** (no GIF, no WebP, no CMYK)
- **Max 4 million pixels per image** (hence `max_source_px: [2000, 2000]`, used only by `check` to warn)
- **Max 500 KB of text per section** -> `max_chapter_kb: 500`
- Max 30 images and 4 MB of images per section; max 100 MB per EPUB

Kobo's engine-side spec adds 10 MB of embedded content per HTML file and notes that Kobo's pipeline may itself re-optimize images.

**Not documented anywhere**: any device-enforced maximum image dimension or file size, `<pre>`/monospace handling, nested-table behavior, or popup-footnote support on current firmware. Nothing is guessed here.

## Known failure modes (community, new generation)

Slow EPUB opening (seconds to 30s+, versus near-instant in RMSDK mode); occasional crashes; books that load and render blank; the last few lines of a chapter swallowed when the chapter ends at the bottom of the screen. Auto-hyphenation arrived in 5.13 but only fires in short chapters, and the same release **broke `&shy;` soft hyphens** - so injecting soft hyphens is now a firmware-version bet, not a fix.

## Sources

mytolino.de product pages + January 2026 datasheets + software-updates page (the `ereaderfiles.kobo.com` URLs) · mytolino.com/ereader-comparison (current lineup) · tolino-media.de EPUB upload requirements · github.com/kobolabs/epub-spec (engine vendor's own spec) · e-reader-forum.de threads 160872 (new-tolino FAQ), 161344 (Absatzbug), 161575 (kepub), 162220 (FW 5.15) · literatur-digital.de (Absatzbug write-up, Calibre workarounds) · rustysoft.de firmware notes · MobileRead post 4474112 (renderer identification) · OverDrive device list (DRM flow split) · geizhals.de (gray levels, prices - secondary).
