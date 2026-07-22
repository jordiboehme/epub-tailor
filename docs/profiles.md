# Profile reference

A profile is a JSON object. Every key at every level is optional; a profile
says only what it wants to change. Unknown keys are rejected so a typo never
silently does nothing.

## Composition

`--profile` repeats and composes left to right on top of the repair-only
baseline (the built-in `epub` profile):

- **Scalar settings** (caps, tunables, `name`, `description`, `output.appendix`): the last layer that sets a value wins.
- **`features`**: merged per key. A layer that sets `"strip_fonts": false` changes only that switch.
- **`filters`**: concatenated in composition order and applied in that order.

Built-in names are case-insensitive: `epub` (alias `default`), `x4`, `x3`,
`nomad`, `kindle`, `kindle-paperwhite`, `kindle-colorsoft`, `kindle-scribe`,
`kindle-scribe-colorsoft`, `tolino-shine`, `tolino-shine-color`,
`tolino-vision-color` and `tolino-epos-3`. Anything containing a path separator
or ending in `.json` is loaded from disk. `epub-tailor profiles` lists them;
`epub-tailor profiles <spec>...` prints the resolved composition.

CLI flags (`--quality`, `--tables`, `--split-tall-images`,
`--max-chapter-kb`) override the composed profile last; flags you do not pass
leave the profile values untouched.

## Schema

```json
{
  "name": "my-profile",
  "description": "One line about what this profile is for",

  "device": {
    "screen": { "width": 480, "height": 800, "ppi": 220 },
    "panel": "gray4",
    "images": {
      "max_source_px": [2048, 1536],
      "inline_max": [480, 730],
      "cover_max": [480, 800],
      "inline_budget_kb": 100,
      "cover_budget_kb": 127
    },
    "css": { "max_file_kb": 128, "max_rules": 1500 }
  },

  "features": {
    "strip_fonts": true,
    "filter_css": true,
    "relocate_styles": true,
    "transcode_images": true,
    "rasterize_svg": true,
    "linearize_tables": true,
    "degrade_boxes": true,
    "bake_ordered_lists": true,
    "preserve_code_blocks": true,
    "normalize_footnotes": true,
    "relocate_anchors": true,
    "dedupe_ids": true,
    "unicode_hygiene": true,
    "chapter_split": true
  },

  "options": {
    "jpeg_quality": 82,
    "tables": "text",
    "split_tall_images": false,
    "max_chapter_kb": 200
  },

  "output": { "appendix": "x4" },

  "filters": [
    { "action": "remove", "match": "FreeBookStamp.example", "in": ["text", "href", "file"] },
    { "action": "replace", "match": "colour", "with": "color" }
  ]
}
```

## `device` - capability numbers

Consulted only by the transforms that need them; a profile with the matching
feature switched off never reads the cap.

| Field | Meaning |
|---|---|
| `screen.width` / `screen.height` | Screen geometry in pixels. |
| `screen.ppi` | Pixel density, informational. |
| `panel` | What the panel paints: `"gray4"`, `"gray16"` or `"color"`. See below. |
| `images.max_source_px` | `[w, h]` decode hard cap; larger source images abort decoding on device. |
| `images.inline_max` | `[w, h]` box an inline image is fitted into (no upscaling). |
| `images.cover_max` | `[w, h]` box the cover is fitted into. |
| `images.inline_budget_kb` | Byte budget for an inline image; quality drops until it fits. |
| `images.cover_budget_kb` | Byte budget for the cover. |
| `css.max_file_kb` | Cap on bytes the device reads from a single CSS file; larger sheets split. |
| `css.max_rules` | Book-wide CSS rule cap; exceeding it warns. |

### `panel`

One value carries both facts the image pipeline needs, because they are not
independent - a Kaleido panel renders 4096 colors *and* 16 grays, so a single
attribute keeps a profile from claiming a combination no panel has.

| Value | Panel | Images become |
|---|---|---|
| `"gray4"` | 2-bit grayscale (Xteink/CrossPoint, Bayer-dithered) | 8-bit luma |
| `"gray16"` | 4-bit grayscale (E Ink Carta and friends) | 8-bit luma |
| `"color"` | Color (E Ink Kaleido 3) | RGB, kept through fitting, budgeting and encoding |

This is the one cap that destroys content when it is wrong: mark a color device
grayscale and `transcode_images` will grayscale its images with no warning.
Alpha is composited onto white for every panel - no target device renders
transparency, and Amazon says so outright.

The baseline is `"color"`, so a profile that never mentions `panel` never loses
color. A grayscale device opts in.

## `features` - the transform switches

Every switch maps to exactly one pipeline step. The `epub` profile has only
`dedupe_ids` and `unicode_hygiene` on (they fix genuine spec violations);
`x4`/`x3` have everything on. Archive repair - META-INF cleanup, OPF/nav/NCX
regeneration, strict XHTML re-serialization - is unconditional and has no
switch.

**Most of these switches exist to work around CrossPoint firmware defects, and
turning them on for a capable reader damages the book.** `filter_css` reduces a
stylesheet to CrossPoint's ~12-property grammar; a Kindle honors `@font-face`,
floats, borders and positioning, and a Kobo-based tolino honors the publisher
stylesheet too. `linearize_tables` flattens a table into labelled paragraphs;
Kindle and Kobo render real tables. `preserve_code_blocks` rebuilds code with
`<br/>` and `&nbsp;`; Kindle has a real monospaced font for `<pre>`/`<code>`.
So every non-Xteink profile keeps only what genuinely helps on a capable device:
repair, image fitting to the panel, and SVG rasterization. They share those
switches through an internal base layer, so the set cannot drift apart between
devices. See `docs/device-constraints.md` and `research/` for the per-device
evidence behind each switch.

### `filter_css` vs `sanitize_css`

Two CSS passes for two very different renderers. They are not variations of each
other, and only one of them is ever right for a given device.

| | `filter_css` | `sanitize_css` |
|---|---|---|
| For | Xteink / CrossPoint | Kobo, PocketBook, tolino |
| Because | its CSS grammar is ~12 properties | Adobe RMSDK's parser is frozen around 2013 |
| Does | demolishes the sheet down to that grammar | keeps the sheet whole |
| Removes | everything not on the allow-list | only `calc()`, `var()`, `clamp()`, `min()`, `max()`, `env()`, `@supports` and range-syntax media queries |

`sanitize_css` exists because RMSDK has no fault tolerance: one construct it
cannot read and it discards the **entire stylesheet**, and on some firmware
refuses to open the book at all, which the reader sees as a corrupt file with no
explanation. Dropping those few declarations costs nothing - none of them do
anything on an e-ink reader anyway - and it saves the sheet.

Where a value can be folded to a plain one (`min(1em, 2em)` is just `1em`), the
declaration survives with its value intact rather than being dropped.

| Switch | What it enables |
|---|---|
| `strip_fonts` | Remove embedded font files and the links pointing at them. |
| `filter_css` | Filter stylesheets to the device-supported grammar and enforce the CSS caps. |
| `sanitize_css` | Remove only the CSS constructs Adobe RMSDK cannot parse, keeping the rest. |
| `relocate_styles` | Lift `<head>`/inline `<style>` CSS into an external stylesheet, scoped per chapter. |
| `transcode_images` | Re-encode raster images to baseline grayscale JPEG/8-bit PNG, fitted and budgeted. |
| `rasterize_svg` | Rasterize SVG resources and inline `<svg>` elements. |
| `linearize_tables` | Flatten tables to labeled paragraphs (or rasterize per `options.tables`). |
| `degrade_boxes` | Degrade `<aside>`, `<figure>`/`<figcaption>` and `<dl>` to plain flow content. |
| `bake_ordered_lists` | Bake `<ol>` numbering into the item text. |
| `preserve_code_blocks` | Rebuild `<pre>`/`<code>` with explicit breaks and non-breaking spaces. |
| `normalize_footnotes` | Normalize footnote links, drop `javascript:` hrefs. |
| `relocate_anchors` | Move anchor ids onto block elements and cap them per chapter. |
| `dedupe_ids` | Remove duplicate element ids. |
| `unicode_hygiene` | NFC-normalize text, strip XML-invalid characters. |
| `chapter_split` | Split chapters over `options.max_chapter_kb` at heading boundaries. |
| `remap_colors` | Remap text (CSS) and diagram (SVG) colors to perceptually spaced gray tones: each color keeps its apparent brightness while staying distinguishable on the panel's gray levels. Document colors get one solve per book, each SVG its own. Never applies on a color panel. |

## `options` - tunables

| Field | Values | Meaning |
|---|---|---|
| `jpeg_quality` | 1-100 | JPEG encode quality. |
| `tables` | `"text"`, `"image"`, `"image-all"` | How linearized tables are represented. |
| `split_tall_images` | bool | Slice images taller than the screen into page tiles. |
| `max_chapter_kb` | number | Chapter split threshold in KiB. |

## `output`

| Field | Meaning |
|---|---|
| `appendix` | Output filename appendix: `book.epub` becomes `book.<appendix>.epub`. When no composed layer sets one, `tailored` is used. |

## `filters` - content filter rules

Applied per chapter (and to the title, authors and TOC labels) before any
device transform, in rule order. Matching is plain case-sensitive substring
search; no regex.

| Field | Values | Meaning |
|---|---|---|
| `action` | `"remove"`, `"replace"` | Delete matches or substitute them. |
| `match` | string | The substring to search for. |
| `with` | string | Replacement text (`replace` only). |
| `in` | array of `"text"`, `"href"`, `"file"` | Where to look. Defaults to `["text"]`. |

Targets:

- `text`: text nodes plus book metadata strings. A `remove` that empties a
  text node prunes the emptied ancestors too, stopping at elements that still
  hold content, at media elements (`img`, `br`, `hr`, `svg`, ...) and at
  document/table structure, which is never pruned. A title or TOC label a
  removal would empty is left unchanged, with a warning.
- `href`: `<a>` targets. A `remove` match detaches the whole anchor and
  prunes upward; `replace` rewrites inside the URL.
- `file`: archive resource paths. A `remove` match drops the file (vendor
  marker files, watermark images). Spine documents and the package and
  navigation documents are protected.

Known v1 limitation: a text match cannot span inline element boundaries
(`Some<b>Stamp</b>.example` will not match as text). Matching the link target
with `href` covers the common watermark case robustly.
