# Onyx Boox - devices and the NeoReader question

Researched 2026-07-12. PRIMARY sources are Boox's own product pages (`shop.boox.com`, `boox.com`); everything about the reader's behavior is COMMUNITY, because Boox publishes nothing about it.

## Read this before trusting a Boox profile

Boox is **plain Android with the Google Play Store preinstalled**, and that changes what a profile is worth:

- The stock reader is **NeoReader**, and MobileRead's consensus is blunt: *"most experienced users abandon NeoReader entirely for dedicated EPUB applications"* - overwhelmingly **KOReader**, then Moon+ Reader Pro, Librera, AlReader X.
- NeoReader **opens no DRM at all**. Adobe, Kobo and B&N books simply do not open; users strip DRM in Calibre first.
- Boox has never published its EPUB engine, its CSS support, or any image limit.

So a `boox-*` profile helps two groups: the people who do stay on NeoReader, and - because image fitting is engine-agnostic - anyone using *any* reader app on the device. It is the weakest-evidence family we ship, and that is said plainly in the README.

## Devices

| Profile | Device | Screen | Panel |
|---|---|---|---|
| `boox-page` | Page, 7" | 1264x1680, 300 PPI | Carta 1200, mono |
| `boox-go-7` | Go 7, 7" | 1264x1680, 300 PPI | Carta 1300, mono |
| `boox-go-color-7` | Go Color 7 (Gen II), 7" | 1264x1680 B&W, **300 / 150 PPI** | Kaleido 3, 4096 colours |
| `boox-palma-2-pro` | Palma 2 Pro, 6.13" | **824x1648** (the phone-shaped one) | Kaleido 3, 4096 colours |

Also current: Go 6, Go 10.3 (Gen II), Note Air4 C / Note Air5 C (10.3", 1860x2480), Note Max, Tab X C (13.3", 2400x3200). The **Palma 2 (mono) is discontinued**, replaced by the Palma 2 Pro. The **Leaf line is China-only** and not in the global lineup.

Colour images from an ordinary EPUB **do render in colour** on the Kaleido models, at the colour layer's 150 PPI - reviewers consistently describe them as muted and newspaper-like.

## What NeoReader actually does (all COMMUNITY)

There is a **V1 / V2 engine toggle** (Settings > Other Settings > "Use the V2 engine to open the document"). The community reading: **V2 preserves the publisher's formatting, V1 discards it.** Boox has an official help article titled "Switch to V2 Engine" but its body is not fetchable.

Reported quality, from MobileRead's "Does NeoReader support epub well?" thread:

- Fancy CSS: *"supports a little, but not too much"*
- **SVG: exists but "not solid"** -> `rasterize_svg` stays on
- **EPUB3: "very discontinuous"** and file-dependent
- Properly formatted **tables render acceptably** -> `linearize_tables` stays off
- **Image rendering is "really bad"** in NeoReader 2; images render noticeably darker than in Moon+ Reader on the same file
- No text-alignment control for EPUB; no separate heading/body font control

### The one bug our image work directly mitigates

With the V2 engine on, some EPUBs **get stuck and will not page forward** - it happens at text/image boundaries where the image is too large to fit on the next page. Fitting every image to the panel, which `transcode_images` already does, is exactly the mitigation. This is the concrete reason a Boox profile earns its place.

Also reported: partial TOC-link failure on large multi-level TOCs; the font picker sometimes only changing chapter titles and not body text (consistent with the book's own CSS winning).

## Unknown / not documented

Boox publishes none of this, and none of it is guessed here:

- The **EPUB rendering engine** behind NeoReader. No source names it.
- Whether NeoReader honors **`@font-face`**.
- The **CSS subset**. No spec, no compatibility table, anywhere.
- **Max image dimensions or file size.** Only PDF has a published limit (2 GB). The stuck-page bug implies an effective per-page image ceiling, but no number exists - hence `max_source_px` is a generous `[4096, 4096]`, used only by `check` to warn.
- **Footnote** handling (popup vs jump).
- Large-chapter / single-XHTML performance limits.

Because the engine is undocumented and DRM is unsupported, the Boox profiles do **not** enable `sanitize_css`: there is no evidence Boox uses Adobe RMSDK, and we do not switch on a transform on a hunch.

## Sources

shop.boox.com product pages (PRIMARY: screens, panels, RAM, Android version, Play Store) · boox.com (PRIMARY: lineup) · MobileRead threads 350513 ("Does NeoReader support epub well?"), 334817 (preferred third-party reader), 343059 (no DRM) · help.boox.com community posts 4406220255764 (V2 stuck-page bug), 4406232611988 (TOC links), 6794635355028 (blurry/dark images) · Good e-Reader and The eBook Reader (Palma 2 discontinued; Leaf is China-only; colour rendering).
