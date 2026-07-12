# ePUB Tailor

Books, made to measure. `epub-tailor` cleans, fixes and transforms EPUB files, driven by composable JSON profiles: a device profile carries your e-reader's actual measurements (screen, image budgets, CSS limits, which HTML it can render) and the book gets cut to fit them exactly. No profile at all and it simply repairs the book - regenerated packaging, junk files gone, epubcheck-clean output - without touching a hair on its typography.

EPUBs accumulate grime. Vendors leave marker files and watermark blocks in every chapter while conversion tools scatter `META-INF` droppings and duplicate ids, and e-ink firmware quietly bins your fonts, mashes your tables into rubble and draws your crisp SVG diagram as the literal word `[Image]`. This tool deals with all of it, and never touches your original file.\*

\* Unless you pass `--lets-get-dangerous`, which replaces the original in place. The write is staged in a temp file and renamed at the end, so even a failed run cannot eat your book - but you did ask for it by name.

## What it does

**Always (the repair core):**

- Rebuilds the packaging from scratch: a clean OPF, navigation document and NCX, all epubcheck-clean.
- Drops junk `META-INF` files (`cdp.info`, Apple display options, calibre leftovers).
- Removes duplicate element ids, a genuine EPUB spec violation.
- Normalizes text to NFC and strips XML-invalid characters.
- Refuses DRM-protected books with a clear error instead of pretending.

**With content filter rules (JSON, yours):**

- Replaces occurrences of a string with another, book-wide.
- Removes strings, then prunes the elements the removal left empty - a chapter-trailing `<div><p><a><i>watermark</i></a></p></div>` disappears whole, not as a husk of empty tags.
- Removes links by target and stray files by name, so a vendor's marker file does not ride along into the clean copy.

**With a device profile (x4, x3 or your own):**

- Transcodes every image to baseline grayscale JPEG or PNG, pre-fit to the screen and inside the device's byte budgets.
- Rasterizes SVG with a real renderer (resvg, 2x supersampled), because the device has no SVG decoder at all.
- Bakes ordered-list numbers into the text, so `1. 2. 3.` does not silently read "• • •".
- Linearizes tables into labeled paragraphs, or rasterizes complex ones into crisp line-art images with `--tables image`.
- Rebuilds `<pre>` and code blocks with explicit breaks and spacing so indentation survives.
- Strips embedded fonts the device will never load.
- Rescues and scopes chapter `<style>` blocks the firmware would otherwise ignore or misapply.
- Splits oversized chapters at heading boundaries before they stall the indexer.

## Install

macOS, with Homebrew:

```sh
brew install jordiboehme/tap/epub-tailor
```

Windows and Linux: grab a binary from [Releases](https://github.com/jordiboehme/epub-tailor/releases). Prebuilt for macOS (arm64 and Intel), Linux (arm64 and amd64, static) and Windows (amd64 and arm64).

## Quickstart

Repair a book (no device tailoring, just hygiene):

```sh
epub-tailor fit book.epub
```

Writes `book.tailored.epub` beside the original and prints exactly what changed.

Fit a book to an Xteink X4 running [CrossPoint](https://github.com/crosspoint-reader/crosspoint-reader):

```sh
epub-tailor fit book.epub --profile x4
```

Writes `book.x4.epub`, with every image, list, table and oversized chapter rewritten for the device.

Stack profiles - device tailoring plus your own content filters:

```sh
epub-tailor fit book.epub --profile x4 --profile ./strip-watermarks.json
```

Convert a Markdown file (with its local images) into a fresh EPUB:

```sh
epub-tailor md book.md --profile x4
```

Diagnose a book without converting it (structural checks by default, device checks with a profile):

```sh
epub-tailor check book.epub --profile x4
```

## Profiles

A profile is a JSON file bundling device capabilities, feature switches, tunables, an output filename appendix and content filter rules. Twenty-seven ship built in, one per device we have actually researched. Output lands as `book.<profile>.epub`.

| Name   | Screen  | Panel | What it is |
|--------|---------|-------|------------|
| `epub` | -       | -      | The default: repair and cleanup only, everything the EPUB standard allows stays. |
| `x4`   | 480x800 | gray4  | Xteink X4 running CrossPoint firmware, the full conversion. |
| `x3`   | 528x792 | gray4  | Xteink X3, same treatment with its own geometry. |
| `nomad` | 1404x1872 | gray16 | Supernote A6X2 Nomad running Chauvet. |
| `kindle` | 1072x1448 | gray16 | Kindle 11th gen (2024). |
| `kindle-paperwhite` | 1264x1680 | gray16 | Kindle Paperwhite 12th gen / Signature. |
| `kindle-colorsoft` | 1264x1680 | color | Kindle Colorsoft / Signature, Kaleido 3. |
| `kindle-scribe` | 1980x2640 | gray16 | Kindle Scribe 3rd gen (2025), 11in. |
| `kindle-scribe-colorsoft` | 1980x2640 | color | Kindle Scribe Colorsoft (2025), Kaleido 3. |
| `kobo-clara-bw` | 1072x1448 | gray16 | Kobo Clara BW, Carta 1300. |
| `kobo-clara-colour` | 1072x1448 | color | Kobo Clara Colour, Kaleido 3. |
| `kobo-libra-colour` | 1264x1680 | color | Kobo Libra Colour, Kaleido 3. |
| `kobo-elipsa-2e` | 1404x1872 | gray16 | Kobo Elipsa 2E, 10.3in at 227ppi. |
| `pocketbook-verse` | 758x1024 | gray16 | PocketBook Verse, 6in at 212ppi. |
| `pocketbook-verse-pro` | 1072x1448 | gray16 | PocketBook Verse Pro. |
| `pocketbook-era` | 1264x1680 | gray16 | PocketBook Era. |
| `pocketbook-era-color` | 1264x1680 | color | PocketBook Era Color, Kaleido 3. |
| `pocketbook-inkpad-4` | 1404x1872 | gray16 | PocketBook InkPad 4. |
| `pocketbook-inkpad-color-3` | 1404x1872 | color | PocketBook InkPad Color 3, Kaleido 3. |
| `boox-page` | 1264x1680 | gray16 | Onyx Boox Page. |
| `boox-go-7` | 1264x1680 | gray16 | Onyx Boox Go 7. |
| `boox-go-color-7` | 1264x1680 | color | Onyx Boox Go Color 7 Gen II, Kaleido 3. |
| `boox-palma-2-pro` | 824x1648 | color | Onyx Boox Palma 2 Pro, the phone-shaped one. |
| `tolino-shine` | 1072x1448 | gray16 | tolino shine 5th gen. |
| `tolino-shine-color` | 1072x1448 | color | tolino shine color, Kaleido 3. |
| `tolino-vision-color` | 1264x1680 | color | tolino vision color, Kaleido 3. |
| `tolino-epos-3` | 1440x1920 | gray16 | tolino epos 3 (the older Android platform). |

Kobo brands its colour models "Colour" and everyone else writes "Color", so both spellings resolve: `--profile kobo-clara-color` is the same thing as `--profile kobo-clara-colour`.

Four things worth knowing about the device profiles:

- **A Kindle cannot open an EPUB at all.** It ingests one through Send to Kindle, which converts it server side. So the `kindle-*` profiles tailor a book to survive Amazon's converter, not to drive a renderer.
- **A plain EPUB on a Kobo renders through Adobe RMSDK**, whose CSS parser is frozen around 2013 and throws away the *entire* stylesheet if it meets a single `calc()`, `var()` or `clamp()` - sometimes refusing to open the book at all. The same engine sits under PocketBook's EPUB2 path and tolino's RMSDK mode. Those profiles switch on `sanitize_css`, which removes exactly those constructs and leaves the rest of the stylesheet alone.
- **Only the Xteink readers get the full conversion.** Every other device here has a real HTML renderer, so the aggressive transforms (CSS subsetting, table linearization, code-block rebuilding) stay off - they would damage the book. What those profiles do is repair it, fit its images to the panel and rasterize its SVG.
- **Boox is the weakest target and we say so.** It is plain Android with the Play Store, its stock NeoReader is widely considered mediocre, and most experienced owners install KOReader instead. The profile still helps: fitting images to the panel is what stops NeoReader's V2 engine getting stuck on an oversized image, and it helps whatever app you read in.

The reasoning, device by device with sources and an explicit list of what nobody publishes, is in [`research/`](research/).

`--profile` repeats and composes left to right: scalar settings later-wins, feature switches merge per key and filter rules concatenate. `epub-tailor profiles` lists the built-ins; `epub-tailor profiles x4 ./mine.json` prints the fully resolved composition as JSON, which is the fastest way to see what a stack actually does.

### Writing a filter profile

```json
{
  "name": "strip-watermarks",
  "filters": [
    { "action": "remove", "match": "FreeBookStamp.example", "in": ["text"] },
    { "action": "remove", "match": "freebookstamp.example", "in": ["href", "file"] },
    { "action": "replace", "match": "colour", "with": "color" }
  ]
}
```

Matching is plain case-sensitive substring search. `in` says where to look: `text` (chapter text, title, authors, TOC labels), `href` (link targets; a `remove` match detaches the whole link) and `file` (archive paths; a `remove` match drops the whole file). When a removal empties an element, the empty husk is pruned upward too - images, table cells and other structure are never pruned. A filter profile carries no device settings, so it composes with any device profile or stands alone on top of the repair core.

### Writing a device profile

Start from a built-in (`epub-tailor profiles x4` prints one) and adjust. Every field is optional; anything you omit keeps the value from earlier layers:

```json
{
  "name": "my-reader",
  "device": {
    "screen": { "width": 600, "height": 800, "ppi": 213 },
    "panel": "gray16",
    "images": { "inline_max": [600, 730], "cover_max": [600, 800], "inline_budget_kb": 150, "cover_budget_kb": 200 },
    "css": { "max_file_kb": 256, "max_rules": 4000 }
  },
  "features": { "strip_fonts": true, "transcode_images": true, "linearize_tables": false },
  "output": { "appendix": "my-reader" }
}
```

`panel` is the one to get right: `"gray4"`, `"gray16"` or `"color"`. It says what the screen can actually paint, and it is what stops a color e-reader having its images quietly grayscaled. Leave it out and color is kept.

The full schema, every feature switch and the composition rules are documented in [`docs/profiles.md`](docs/profiles.md).

## Flags

Available on `fit` and `md` unless noted. Flags override profile values; flags you do not pass leave the profile alone.

| Flag | Default | What it does |
|---|---|---|
| `--profile <NAME\|PATH>` | `epub` | A built-in profile or a path to your own JSON. Repeatable, composes left to right. |
| `--quality low\|std\|high\|1-100` | from profile | JPEG quality. `low` is 70, `std` is 82, `high` is 90. |
| `--tables text\|image\|image-all` | from profile | `image` rasterizes a table only when flattening would hurt it; `image-all` rasterizes every table it safely can. |
| `--split-tall-images` | from profile | Slice an image taller than the screen into page-sized tiles. |
| `--split-level 1\|2` | `1` | `md` only: heading level that starts a new chapter. |
| `--max-chapter-kb <N>` | from profile | Split a chapter larger than this at a heading boundary. |
| `--dry-run` | off | Report what would change and write nothing. |
| `--report human\|json` | `human` | Use `json` for machine-readable output. |
| `-o, --output <PATH>` | next to the input | Where to write the result. |
| `--lets-get-dangerous` | off | `fit` only: replace the original file in place instead of writing a copy. Conflicts with `-o`. Lets. Get. Dangerous. |

## FAQ

**What does the default profile actually change?**
As little as possible. The packaging is regenerated, junk `META-INF` files are dropped, duplicate ids are fixed, text is NFC-normalized and every document is re-serialized as strict XHTML. Fonts, CSS, images and tables pass through byte for byte.

**Why is my SVG now a JPEG or a PNG?**
Only under a device profile, and because the X4 has no SVG decoder at all - an SVG left in place renders as nothing or an `[Image]` label. Pure vector art comes out a crisp PNG; an SVG carrying an embedded photo gets classified from its pixels, which usually means JPEG.

**Why did my numbered list turn into paragraphs?**
The CrossPoint firmware draws every list item with a "•" and no number, so a genuine `<ol>` reads "• • •". Under a device profile each item becomes a paragraph with its number ("1.", "2." and "2.1." for nested lists) baked into the text.

**Why did my table become a picture?**
Only with `--tables image`, and only for tables where flattening would actually hurt: three or more columns, a nested table or cells that are mostly numbers. A table carrying a link or an anchor target is never rasterized, because a picture cannot be clicked or jumped to.

**Where did my beautiful embedded fonts go?**
Under a device profile, into the bin, and your file is lighter for it. The device renders only its own built-in faces and never loads an embedded one. Under the default profile they stay exactly where they were.

**Why will it not convert my DRM book?**
Encrypted content cannot be read, let alone repaired or tailored. Strip the DRM first (Calibre and the usual suspects) then run it again.

**Does the filter syntax support regex?**
Not yet, on purpose. Plain substrings cover the real cases we have met and cannot catastrophically backtrack. If a pattern spans styled text (`Some<b>Watermark</b>.example`), match the link target with `"in": ["href"]` instead - that is the robust move anyway.

**What about my book's description and publisher metadata?**
Known v0.1 limitation: the regenerated OPF currently keeps title, authors, language and identifier. `dc:description`, `dc:publisher`, subjects and dates are dropped in the rewrite. Richer metadata passthrough is planned for 0.2.

**What is Markdown frontmatter?**
An optional YAML block at the very top of your `.md` that sets the book metadata:

```yaml
---
title: My Book
author: Jane Author
language: en
cover: images/cover.png
---
```

`author` takes one name or a list. Omit the block entirely and the first `# H1` becomes the title.

## The deep lore

None of the device quirks are guesses. We read the CrossPoint firmware source so you do not have to, and wrote down every decode cap, supported CSS property and rendering trap in [`docs/device-constraints.md`](docs/device-constraints.md), with full citations in [`research/`](research/). It is a genuinely fun read if you have ever wondered what a 480x800, 4-gray, 400KB-of-RAM e-reader thinks of your stylesheet.

## License and compatibility

MIT licensed, see [LICENSE](LICENSE). The built-in device profiles target the Xteink X4 and X3 running the [CrossPoint](https://github.com/crosspoint-reader/crosspoint-reader) reader firmware. This is an independent project and is not affiliated with or endorsed by Xteink or CrossPoint.
