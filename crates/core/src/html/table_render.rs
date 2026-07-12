//! Pure, deterministic SVG table-layout generator - the foundation for
//! `--tables image`. Turns a collapsed [`TableModel`] into a standalone `<svg>`
//! string that resvg plus the bundled DejaVu Sans (see `image/svg.rs`) can
//! rasterize crisply.
//!
//! Layout is at 1x: the rasterizer already renders at 2x and downscales, so this
//! module never supersamples. Given the same model and `max_w` it emits a
//! byte-identical string - no randomness, no map iteration on any emission path.

use crate::html::escape::escape_text;

/// Font size, in px, of every `<text>` line.
const FONT_PX: i64 = 15;
/// Conservative per-character advance for DejaVu Sans (~0.6em) used to size
/// wrapping so text never overflows its cell.
const AVG_CHAR_W: f64 = 9.0;
/// Vertical advance between wrapped lines.
const LINE_H: i64 = 20;
/// Horizontal cell padding.
const PAD_X: i64 = 6;
/// Vertical cell padding.
const PAD_Y: i64 = 4;
/// Grid line / border thickness.
const BORDER: i64 = 1;
/// Baseline offset from a line box's top (ascent approximation for `FONT_PX`).
const BASELINE_DY: i64 = 15;
/// Minimum column width, in px, before proportional space is shared out.
const MIN_COL_W: i64 = 40;
/// Character-count clamp used to weight a column's proportional width.
const MIN_CHARS: usize = 3;
const MAX_CHARS: usize = 24;

/// A table collapsed for layout. Header cells and captions are plain
/// whitespace-collapsed text; each body cell carries its own text plus at most
/// one nested table rendered as an inner grid. Ragged body rows are padded at
/// layout time.
pub(crate) struct TableModel {
    pub caption: Option<String>,
    /// Header cells stay text-only.
    pub headers: Option<Vec<String>>,
    /// Body rows; each cell is text plus an optional nested grid.
    pub rows: Vec<Vec<Cell>>,
}

/// One body cell: its own collapsed text (with the chosen nested table's subtree
/// excluded) plus at most one nested table rendered as an inner grid. A cell can
/// legally hold both text and a table, so this is a struct, not an enum.
pub(crate) struct Cell {
    pub text: String,
    pub sub: Option<TableModel>,
}

/// Build a text-only [`Cell`], for the model-construction tests.
#[cfg(test)]
pub(crate) fn cell(s: &str) -> Cell {
    Cell {
        text: s.to_string(),
        sub: None,
    }
}

/// Render `model` as a standalone SVG string exactly `max_w` px wide.
///
/// An empty model (no header and no non-empty row) yields a minimal but valid
/// bordered white box rather than panicking; T5's heuristic should never send
/// one, but the function must be total.
pub(crate) fn render_table_svg(model: &TableModel, max_w: u32) -> String {
    let max_w = i64::from(max_w);
    let grid = layout_grid(model, max_w);
    if grid.widths.is_empty() {
        return empty_svg(max_w);
    }
    let total_h = grid.height;

    let mut s = String::new();
    s.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{max_w}" height="{total_h}" viewBox="0 0 {max_w} {total_h}">"#
    ));
    // White background.
    s.push_str(&format!(
        r#"<rect x="0" y="0" width="{max_w}" height="{total_h}" fill="white"/>"#
    ));
    emit_grid(&grid, 0, 0, &mut s);
    s.push_str("</svg>");
    s
}

/// A fully measured grid ready to emit at any offset: its column widths, its
/// rows (already wrapped and sized, with nested grids laid out), its total pixel
/// height (caption included) and its caption line(s).
struct GridLayout {
    widths: Vec<i64>,
    rows: Vec<RowLayout>,
    height: i64,
    caption_lines: Vec<String>,
}

/// A header/body row after layout: whether it is the header, its per-column
/// cells and its pixel height (its tallest cell plus padding).
struct RowLayout {
    is_header: bool,
    cells: Vec<CellLayout>,
    height: i64,
}

/// One laid-out cell: its wrapped text lines plus an optional nested grid drawn
/// below the text.
struct CellLayout {
    lines: Vec<String>,
    sub: Option<Box<GridLayout>>,
}

/// Total pixel width of a grid (column widths plus the framing borders).
fn grid_width(grid: &GridLayout) -> i64 {
    grid.widths.iter().sum::<i64>() + (grid.widths.len() as i64 + 1) * BORDER
}

/// Measure `model` into a [`GridLayout`] exactly `width` px wide. The top-level
/// grid shares column space proportionally; nested grids (reached by recursion)
/// use equal columns via [`layout_grid_impl`] with `equal = true`, since 480px
/// is too cramped for a second round of proportional negotiation.
fn layout_grid(model: &TableModel, width: i64) -> GridLayout {
    layout_grid_impl(model, width, false)
}

fn layout_grid_impl(model: &TableModel, width: i64, equal: bool) -> GridLayout {
    // Column count = the widest of header and body rows; ragged rows pad.
    let mut cols = model.headers.as_ref().map_or(0, Vec::len);
    for row in &model.rows {
        cols = cols.max(row.len());
    }

    // Caption: bold full-width line(s) above the grid; blank captions skipped.
    let caption_lines = model
        .caption
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .map(|c| wrap(c, line_capacity(width)))
        .unwrap_or_default();
    let caption_height = if caption_lines.is_empty() {
        0
    } else {
        caption_lines.len() as i64 * LINE_H + 2 * PAD_Y
    };

    if cols == 0 {
        // An empty model has no grid; the caller renders a minimal box.
        return GridLayout {
            widths: Vec::new(),
            rows: Vec::new(),
            height: caption_height,
            caption_lines,
        };
    }

    // Per-column character weight: the widest header/body cell, but a cell
    // holding a nested table weights its column at `max(text_chars, MAX_CHARS)`
    // so the inner grid gets room.
    let mut char_len = vec![0usize; cols];
    if let Some(h) = &model.headers {
        for (c, slot) in char_len.iter_mut().enumerate() {
            let len = h.get(c).map_or(0, |s| s.chars().count());
            *slot = (*slot).max(len);
        }
    }
    for row in &model.rows {
        for (c, slot) in char_len.iter_mut().enumerate() {
            let weight = row.get(c).map_or(0, |cell| {
                let chars = cell.text.chars().count();
                if cell.sub.is_some() {
                    chars.max(MAX_CHARS)
                } else {
                    chars
                }
            });
            *slot = (*slot).max(weight);
        }
    }
    let avail = (width - (cols as i64 + 1) * BORDER).max(0);
    let widths = if equal {
        equal_widths(cols, avail)
    } else {
        column_widths(&char_len, avail)
    };

    let mut rows: Vec<RowLayout> = Vec::with_capacity(model.rows.len() + 1);
    if let Some(h) = &model.headers {
        rows.push(layout_header_row(h, &widths));
    }
    for row in &model.rows {
        rows.push(layout_body_row(row, &widths));
    }

    // Vertical geometry: grid sits below the caption; borders frame each row.
    let mut y = caption_height + BORDER;
    for row in &rows {
        y += row.height + BORDER;
    }
    GridLayout {
        widths,
        rows,
        height: y,
        caption_lines,
    }
}

/// Lay out a text-only header row: wrap each cell, size the row to its tallest.
fn layout_header_row(header: &[String], widths: &[i64]) -> RowLayout {
    let mut cells = Vec::with_capacity(widths.len());
    let mut content_h = LINE_H;
    for (c, &w) in widths.iter().enumerate() {
        let text = header.get(c).map_or("", String::as_str);
        let lines = wrap(text, line_capacity(w));
        content_h = content_h.max(lines.len() as i64 * LINE_H);
        cells.push(CellLayout { lines, sub: None });
    }
    RowLayout {
        is_header: true,
        cells,
        height: content_h + 2 * PAD_Y,
    }
}

/// Lay out a body row: each cell wraps its own text and, if it holds a nested
/// table, lays that out as an inner grid stacked below the text.
fn layout_body_row(row: &[Cell], widths: &[i64]) -> RowLayout {
    let mut cells = Vec::with_capacity(widths.len());
    let mut content_h = LINE_H;
    for (c, &w) in widths.iter().enumerate() {
        let model_cell = row.get(c);
        let text = model_cell.map_or("", |cell| cell.text.as_str());
        let has_text = !text.trim().is_empty();
        let lines = wrap(text, line_capacity(w));
        // Nested grid width = the cell's content width (inside its padding);
        // empty inner grids are dropped so no 1px box is drawn.
        let sub = model_cell
            .and_then(|cell| cell.sub.as_ref())
            .map(|m| Box::new(layout_grid_impl(m, (w - 2 * PAD_X).max(0), true)))
            .filter(|g| !g.widths.is_empty());
        let cell_h = if let Some(sub) = &sub {
            let text_h = if has_text {
                lines.len() as i64 * LINE_H
            } else {
                0
            };
            let separator = if has_text { PAD_Y } else { 0 };
            text_h + separator + sub.height
        } else {
            lines.len() as i64 * LINE_H
        };
        content_h = content_h.max(cell_h);
        cells.push(CellLayout { lines, sub });
    }
    RowLayout {
        is_header: false,
        cells,
        height: content_h + 2 * PAD_Y,
    }
}

/// Emit `grid` into `out` with its top-left corner at (`ox`, `oy`). Recurses
/// once per nested grid. Byte-for-byte deterministic: no map iteration here.
fn emit_grid(grid: &GridLayout, ox: i64, oy: i64, out: &mut String) {
    if grid.widths.is_empty() {
        return;
    }
    let cols = grid.widths.len();
    let total_w = grid_width(grid);
    let caption_height = if grid.caption_lines.is_empty() {
        0
    } else {
        grid.caption_lines.len() as i64 * LINE_H + 2 * PAD_Y
    };

    // Left content edge of each column (x just past its left border).
    let mut col_left = Vec::with_capacity(cols);
    let mut x = BORDER;
    for &w in &grid.widths {
        col_left.push(x);
        x += w + BORDER;
    }

    // Top of each row and of the grid box (below the caption).
    let grid_top = caption_height;
    let mut row_top = Vec::with_capacity(grid.rows.len());
    let mut y = grid_top + BORDER;
    for row in &grid.rows {
        row_top.push(y);
        y += row.height + BORDER;
    }
    let grid_bottom = y;
    let grid_height = grid_bottom - grid_top;

    // Caption line(s), bold, left-aligned at full width.
    for (i, line) in grid.caption_lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let ty = oy + PAD_Y + i as i64 * LINE_H + BASELINE_DY;
        out.push_str(&text_element(ox + PAD_X, ty, true, line));
    }
    // Outer border, inset half a pixel so its 1px stroke stays on-canvas.
    out.push_str(&format!(
        r#"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="black" stroke-width="1"/>"#,
        ox as f64 + 0.5,
        (oy + grid_top) as f64 + 0.5,
        total_w - 1,
        grid_height - 1,
    ));
    // Interior vertical rules (skip the rightmost column edge).
    for (&left, &w) in col_left.iter().zip(&grid.widths).take(cols - 1) {
        let lx = (ox + left + w) as f64 + 0.5;
        out.push_str(&format!(
            r#"<line x1="{lx}" y1="{}" x2="{lx}" y2="{}" stroke="black" stroke-width="1"/>"#,
            (oy + grid_top) as f64 + 0.5,
            (oy + grid_bottom) as f64 - 0.5,
        ));
    }
    // Interior horizontal rules; a header gets a heavier separator below it.
    let row_count = grid.rows.len();
    for (&rtop, row) in row_top.iter().zip(&grid.rows).take(row_count - 1) {
        let ly = (oy + rtop + row.height) as f64 + 0.5;
        let width = if row.is_header { 2 } else { 1 };
        out.push_str(&format!(
            r#"<line x1="{}" y1="{ly}" x2="{}" y2="{ly}" stroke="black" stroke-width="{width}"/>"#,
            ox as f64 + 0.5,
            (ox + total_w) as f64 - 0.5,
        ));
    }
    // Cell content: text lines first, then any nested grid below them.
    for (ri, row) in grid.rows.iter().enumerate() {
        let rtop = row_top[ri];
        for (c, cell) in row.cells.iter().enumerate() {
            let cx = ox + col_left[c] + PAD_X;
            let has_text = cell.lines.iter().any(|l| !l.is_empty());
            for (li, line) in cell.lines.iter().enumerate() {
                if line.is_empty() {
                    continue;
                }
                let ty = oy + rtop + PAD_Y + li as i64 * LINE_H + BASELINE_DY;
                out.push_str(&text_element(cx, ty, row.is_header, line));
            }
            if let Some(sub) = &cell.sub {
                let text_h = if has_text {
                    cell.lines.len() as i64 * LINE_H
                } else {
                    0
                };
                let separator = if has_text { PAD_Y } else { 0 };
                let sub_y = oy + rtop + PAD_Y + text_h + separator;
                emit_grid(sub, cx, sub_y, out);
            }
        }
    }
}

/// One `<text>` line at baseline (`x`, `y`), bold for headers/captions, with the
/// content XML-escaped.
fn text_element(x: i64, y: i64, bold: bool, line: &str) -> String {
    let weight = if bold { r#" font-weight="bold""# } else { "" };
    format!(
        r#"<text x="{x}" y="{y}" font-family="DejaVu Sans" font-size="{FONT_PX}"{weight} fill="black">{}</text>"#,
        escape_text(line)
    )
}

/// A minimal valid SVG for an empty model: a bordered white box.
fn empty_svg(max_w: i64) -> String {
    let h = LINE_H + 2 * PAD_Y;
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{max_w}" height="{h}" viewBox="0 0 {max_w} {h}"><rect x="0" y="0" width="{max_w}" height="{h}" fill="white"/><rect x="0.5" y="0.5" width="{}" height="{}" fill="none" stroke="black" stroke-width="1"/></svg>"#,
        max_w - 1,
        h - 1,
    )
}

/// Characters that fit on one line inside a `content_w`-px-wide box.
fn line_capacity(content_w: i64) -> usize {
    let usable = content_w - 2 * PAD_X;
    if usable <= 0 {
        return 1;
    }
    ((usable as f64 / AVG_CHAR_W).floor() as usize).max(1)
}

/// Integer column widths summing exactly to `avail`: an equal minimum floor,
/// then the remainder shared out proportionally to the clamped character
/// weights via the largest-remainder method (ties broken by column index, so
/// the result is deterministic).
fn column_widths(char_len: &[usize], avail: i64) -> Vec<i64> {
    let n = char_len.len();
    if n == 0 {
        return Vec::new();
    }
    let min_w = MIN_COL_W.min(avail / n as i64);
    let remaining = (avail - min_w * n as i64).max(0);

    let weights: Vec<f64> = char_len
        .iter()
        .map(|&l| l.clamp(MIN_CHARS, MAX_CHARS) as f64)
        .collect();
    let wsum: f64 = weights.iter().sum();
    let ideal: Vec<f64> = if wsum > 0.0 {
        weights
            .iter()
            .map(|w| remaining as f64 * w / wsum)
            .collect()
    } else {
        vec![0.0; n]
    };

    let mut extra: Vec<i64> = ideal.iter().map(|x| x.floor() as i64).collect();
    let mut leftover = remaining - extra.iter().sum::<i64>();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        let fa = ideal[a] - ideal[a].floor();
        let fb = ideal[b] - ideal[b].floor();
        fb.total_cmp(&fa).then(a.cmp(&b))
    });
    for &i in &order {
        if leftover <= 0 {
            break;
        }
        extra[i] += 1;
        leftover -= 1;
    }
    (0..n).map(|i| min_w + extra[i]).collect()
}

/// Split `avail` px across `n` equal columns via the largest-remainder method:
/// every column gets `avail / n`, and the `avail % n` leftover pixels go to the
/// lowest-indexed columns so the result is deterministic. Used for nested grids,
/// where 480px leaves no room for proportional width negotiation.
fn equal_widths(n: usize, avail: i64) -> Vec<i64> {
    if n == 0 {
        return Vec::new();
    }
    let n = n as i64;
    let base = avail / n;
    let leftover = avail - base * n;
    (0..n).map(|i| base + i64::from(i < leftover)).collect()
}

/// Greedy word-wrap of `text` to at most `max_chars` characters per line. Words
/// longer than a line are hard-broken. Always returns at least one line (an
/// empty string for an empty cell), so a row is always at least one line tall.
fn wrap(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for raw in text.split_whitespace() {
        let mut word = raw;
        // Hard-break a word too long to ever fit on one line.
        while word.chars().count() > max_chars {
            if current_len > 0 {
                lines.push(std::mem::take(&mut current));
                current_len = 0;
            }
            let split = word
                .char_indices()
                .nth(max_chars)
                .map_or(word.len(), |(i, _)| i);
            lines.push(word[..split].to_string());
            word = &word[split..];
        }
        let word_len = word.chars().count();
        if word_len == 0 {
            continue;
        }
        if current_len == 0 {
            current.push_str(word);
            current_len = word_len;
        } else if current_len + 1 + word_len <= max_chars {
            current.push(' ');
            current.push_str(word);
            current_len += 1 + word_len;
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
            current_len = word_len;
        }
    }
    if current_len > 0 || lines.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use resvg::usvg;

    fn sample_3x3() -> TableModel {
        TableModel {
            caption: None,
            headers: Some(vec!["Name".into(), "Qty".into(), "Price".into()]),
            rows: vec![
                vec![cell("Apple"), cell("3"), cell("1.20")],
                vec![cell("Pear"), cell("5"), cell("0.90")],
            ],
        }
    }

    #[test]
    fn header_table_lays_out_and_parses() {
        let svg = render_table_svg(&sample_3x3(), 480);

        assert!(
            svg.contains(r#"width="480""#),
            "canvas is exactly max_w wide: {svg}"
        );
        // Bold appears exactly once per (single-line) header cell.
        assert_eq!(
            svg.matches(r#"font-weight="bold""#).count(),
            3,
            "bold only on the three header cells: {svg}"
        );
        for cell in [
            "Name", "Qty", "Price", "Apple", "3", "1.20", "Pear", "5", "0.90",
        ] {
            assert!(svg.contains(cell), "cell text {cell} present");
        }

        assert!(
            roxmltree::Document::parse(&svg).is_ok(),
            "well-formed XML: {svg}"
        );
        let tree = usvg::Tree::from_str(&svg, &crate::image::svg::options())
            .expect("usvg parses with the bundled font");
        assert!(tree.size().width() > 0.0 && tree.size().height() > 0.0);
    }

    #[test]
    fn long_cell_text_wraps_into_multiple_lines() {
        let model = TableModel {
            caption: None,
            headers: None,
            rows: vec![vec![cell(&"a".repeat(200))]],
        };
        let svg = render_table_svg(&model, 480);
        assert!(
            svg.matches("<text").count() > 1,
            "a 200-char cell wraps to more than one <text>: {svg}"
        );
        assert!(roxmltree::Document::parse(&svg).is_ok());
    }

    #[test]
    fn ragged_rows_are_padded_to_the_column_count() {
        let model = TableModel {
            caption: None,
            headers: Some(vec!["A".into(), "B".into(), "C".into()]),
            rows: vec![vec![cell("1")], vec![cell("x"), cell("y"), cell("z")]],
        };
        let svg = render_table_svg(&model, 480);
        assert!(roxmltree::Document::parse(&svg).is_ok());
        // 3 columns (=> 2 interior verticals) and 3 rows (=> 2 interior
        // horizontals) means the short row was padded, not truncated.
        assert_eq!(svg.matches("<line").count(), 4, "grid lines: {svg}");
        for cell in ["A", "B", "C", "1", "x", "y", "z"] {
            assert!(svg.contains(cell), "cell {cell} present");
        }
    }

    #[test]
    fn empty_model_is_minimal_and_valid() {
        let model = TableModel {
            caption: None,
            headers: None,
            rows: vec![],
        };
        let svg = render_table_svg(&model, 480);
        assert!(svg.contains(r#"width="480""#));
        assert!(roxmltree::Document::parse(&svg).is_ok(), "{svg}");
        let tree = usvg::Tree::from_str(&svg, &crate::image::svg::options())
            .expect("empty table still renders");
        assert!(tree.size().width() > 0.0 && tree.size().height() > 0.0);
    }

    #[test]
    fn output_is_deterministic() {
        let a = render_table_svg(&sample_3x3(), 480);
        let b = render_table_svg(&sample_3x3(), 480);
        assert_eq!(a, b, "same model + max_w -> byte-identical");
    }

    #[test]
    fn special_characters_are_escaped() {
        let model = TableModel {
            caption: None,
            headers: None,
            rows: vec![vec![cell(r#"<b>&"'"#)]],
        };
        let svg = render_table_svg(&model, 480);
        assert!(svg.contains("&lt;b&gt;&amp;"), "entities escaped: {svg}");
        assert!(!svg.contains("<b>"), "no raw markup leaked: {svg}");
        assert!(
            roxmltree::Document::parse(&svg).is_ok(),
            "still parses: {svg}"
        );
    }

    #[test]
    fn caption_is_rendered_bold_above_the_grid() {
        let model = TableModel {
            caption: Some("Table 1 - fruit prices".into()),
            headers: Some(vec!["Name".into(), "Qty".into(), "Price".into()]),
            rows: vec![vec![cell("Apple"), cell("3"), cell("1.20")]],
        };
        let svg = render_table_svg(&model, 480);
        assert!(
            svg.contains("Table 1 - fruit prices"),
            "caption text: {svg}"
        );
        assert!(roxmltree::Document::parse(&svg).is_ok());
        let tree = usvg::Tree::from_str(&svg, &crate::image::svg::options())
            .expect("captioned table renders");
        assert!(tree.size().height() > 0.0);
    }

    #[test]
    fn sample_rasterizes_non_blank_over_white() {
        let svg = render_table_svg(&sample_3x3(), 480);
        let mut warnings = Vec::new();
        let rasterized = crate::image::svg::rasterize_sized(
            &svg,
            crate::profile::DeviceCaps::x4().inline_max,
            &mut warnings,
            "table.svg",
        )
        .expect("the table SVG rasterizes");
        assert!(
            warnings.is_empty(),
            "no warning for a good SVG: {warnings:?}"
        );
        let gray = rasterized.image.to_luma8();
        assert!(
            gray.pixels().any(|p| p.0[0] < 128),
            "text and rules must render as dark pixels"
        );
        assert!(
            gray.pixels().any(|p| p.0[0] >= 250),
            "cell backgrounds must stay white"
        );
    }

    // --- Nested grids ----------------------------------------------------

    /// A 3-col parent whose middle body cell holds "note" plus a nested 2x2.
    fn nested_sample() -> TableModel {
        let inner = TableModel {
            caption: None,
            headers: None,
            rows: vec![
                vec![cell("inA"), cell("inB")],
                vec![cell("inC"), cell("inD")],
            ],
        };
        TableModel {
            caption: None,
            headers: Some(vec!["H1".into(), "H2".into(), "H3".into()]),
            rows: vec![
                vec![cell("a"), cell("b"), cell("c")],
                vec![
                    cell("d"),
                    Cell {
                        text: "note".into(),
                        sub: Some(inner),
                    },
                    cell("f"),
                ],
            ],
        }
    }

    /// The baseline `y` of the `<text>` element whose content is exactly `body`.
    fn text_y(svg: &str, body: &str) -> i64 {
        let needle = format!(">{body}</text>");
        let end = svg.find(&needle).expect("content present");
        let tag = svg[..end].rfind("<text").expect("opening <text");
        let seg = &svg[tag..end];
        let yi = seg.find(r#" y=""#).expect("y attr") + 4;
        let yj = seg[yi..].find('"').expect("y end") + yi;
        seg[yi..yj].parse().expect("y is an integer")
    }

    #[test]
    fn nested_grid_renders_inside_a_parent_cell() {
        let svg = render_table_svg(&nested_sample(), 480);

        assert!(
            roxmltree::Document::parse(&svg).is_ok(),
            "well-formed XML: {svg}"
        );
        let tree = usvg::Tree::from_str(&svg, &crate::image::svg::options())
            .expect("usvg parses the nested grid with the bundled font");
        assert!(tree.size().width() > 0.0 && tree.size().height() > 0.0);

        for text in [
            "H1", "H2", "H3", "a", "b", "c", "d", "f", "note", "inA", "inB", "inC", "inD",
        ] {
            assert!(svg.contains(text), "cell text {text} present: {svg}");
        }
        // A cell with both text and a nested grid renders its text ABOVE the
        // inner grid (a smaller y is higher on the canvas).
        assert!(
            text_y(&svg, "note") < text_y(&svg, "inA"),
            "the cell's own text draws above its inner grid: {svg}"
        );
    }

    #[test]
    fn nested_render_is_deterministic() {
        let a = render_table_svg(&nested_sample(), 480);
        let b = render_table_svg(&nested_sample(), 480);
        assert_eq!(a, b, "same nested model + max_w -> byte-identical");
    }

    #[test]
    fn equal_widths_distributes_remainder_to_low_indices() {
        assert_eq!(equal_widths(3, 100), vec![34, 33, 33]);
        assert_eq!(equal_widths(4, 100), vec![25, 25, 25, 25]);
        assert_eq!(equal_widths(1, 50), vec![50]);
        assert_eq!(equal_widths(0, 100), Vec::<i64>::new());
    }
}
