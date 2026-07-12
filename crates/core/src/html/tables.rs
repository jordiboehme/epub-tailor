//! Decide each table's fate and linearize the ones that stay text.
//!
//! The firmware has no table layout at all and drops nested tables outright.
//! Under [`TableMode::Text`] every table is flattened to a sequence of
//! paragraphs; with a header row each body cell becomes a "Header: value"
//! paragraph, without one each row becomes a single em-dash-separated paragraph.
//! Innermost tables are processed first so a nested table's content survives as
//! paragraphs inside its parent's cell.
//!
//! Under [`TableMode::Image`]/[`TableMode::ImageAll`], [`table_decision`] picks
//! per top-level table between rasterization (the table is left intact and
//! tagged with the `data-et-table-render` sentinel for `convert` to render into
//! an image) and linearization. Tables carrying anchor targets, links or
//! images, or that are too tall to rasterize, are always linearized so nothing
//! is lost.

use kuchikiki::{NodeData, NodeRef};

use crate::html::dom::{
    child_elements, collect_by_name, element, get_attr, has_descendant_named, is_named,
    move_children, replace_with, set_attr, text, text_content,
};
use crate::html::table_render::{Cell, TableModel};
use crate::options::{ConvertOptions, TableMode};
use crate::report::{Transformation, Warning};

/// The characters stripped from a body cell before deciding whether what
/// remains is a bare run of digits (so "$1,200.50" and "13%" read as numeric).
const NUMERIC_STRIP: &str = "$€£%,()+-.";

/// The maximum number of effective rows a table may have and still be
/// rasterized: its own BODY rows (own rows minus the detected header row) plus
/// every `<tr>` a nested table contributes. Past this the rendered image would
/// be uncomfortably tall on the screen.
const MAX_RASTER_BODY_ROWS: usize = 24;

/// U+2014 EM DASH, used as the visual cell/row separator in linearized output.
const EM_DASH: char = '\u{2014}';

/// Block-level children of a cell that cannot live inside the generated
/// paragraph and are spliced out as their own blocks instead.
const CELL_BLOCK: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "pre",
    "ul",
    "ol",
    "dl",
    "table",
    "figure",
    "section",
];

/// What to do with one top-level table under the active [`TableMode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Render the table (and everything nested inside it) to a rasterized image.
    Rasterize,
    /// Flatten the table to paragraphs, for the given reason.
    Linearize(&'static str),
}

/// Process every top-level table in `doc` per the active [`TableMode`].
///
/// Under [`TableMode::Text`] every table is flattened. Under the image modes,
/// each top-level table is either tagged with the `data-et-table-render`
/// sentinel (so `convert` renders it to an image, leaving its subtree - nested
/// tables included - intact) or flattened per [`table_decision`].
pub(crate) fn linearize_tables(
    doc: &NodeRef,
    opts: &ConvertOptions,
    report: &mut Vec<Transformation>,
    chapter_path: &str,
) {
    let mode = opts.tables;
    for table in top_level_tables(doc) {
        match mode {
            TableMode::Text => linearize_table_node(&table, report, chapter_path, None),
            TableMode::Image | TableMode::ImageAll => match table_decision(&table, mode) {
                Decision::Rasterize => set_attr(&table, "data-et-table-render", "1"),
                Decision::Linearize(reason) => {
                    linearize_table_node(&table, report, chapter_path, Some(reason));
                }
            },
        }
    }
}

/// The tables in `doc` that are not nested inside another table, in document
/// order. Only these get a per-table [`Decision`]; nested tables share their
/// top-level ancestor's fate (rendered inside its image, or flattened with it).
fn top_level_tables(doc: &NodeRef) -> Vec<NodeRef> {
    collect_by_name(doc, "table")
        .into_iter()
        .filter(|t| !has_table_ancestor(t))
        .collect()
}

/// Whether `table` is nested inside another `<table>`.
fn has_table_ancestor(table: &NodeRef) -> bool {
    let mut current = table.parent();
    while let Some(node) = current {
        if is_named(&node, "table") {
            return true;
        }
        current = node.parent();
    }
    false
}

/// Decide whether a top-level `table` should be rasterized or linearized under
/// `mode` (one of [`TableMode::Image`]/[`TableMode::ImageAll`]).
///
/// Safety fallbacks come first and force linearization even under `ImageAll`, so
/// no anchor target, link, image or over-tall table is ever baked into a
/// non-interactive picture. Otherwise `ImageAll` rasterizes unconditionally,
/// while `Image` rasterizes only tables that look genuinely tabular (three or
/// more columns, a nested table, or a majority of numeric body cells).
pub(crate) fn table_decision(table: &NodeRef, mode: TableMode) -> Decision {
    // --- Safety fallbacks (apply even under ImageAll). ---
    if table.inclusive_descendants().any(|n| has_id(&n)) {
        return Decision::Linearize("table has anchor targets");
    }
    if table
        .inclusive_descendants()
        .any(|n| is_named(&n, "a") && get_attr(&n, "href").is_some())
    {
        return Decision::Linearize("table contains links");
    }
    if table
        .inclusive_descendants()
        .any(|n| is_named(&n, "img") || is_named(&n, "svg") || is_named(&n, "image"))
    {
        return Decision::Linearize("table contains images");
    }

    let all_rows = collect_rows(table);
    let head_rows = header_rows(table);
    let (header_cells, body_rows) = split_header(&all_rows, &head_rows);
    // Effective rows are how tall the rendered image would be: this table's BODY
    // rows (its own rows minus the detected header row) plus every `<tr>` a
    // nested table contributes, since nested tables render inside the same image
    // (all `<tr>` descendants minus this table's own rows).
    let own_rows = all_rows.len();
    let nested_rows = table
        .descendants()
        .filter(|n| is_named(n, "tr"))
        .count()
        .saturating_sub(own_rows);
    if body_rows.len() + nested_rows > MAX_RASTER_BODY_ROWS {
        return Decision::Linearize("table too tall to rasterize");
    }

    match mode {
        // Never reached (Text is handled in `linearize_tables`), but keeps the
        // match total.
        TableMode::Text => Decision::Linearize("text mode"),
        TableMode::ImageAll => Decision::Rasterize,
        TableMode::Image => {
            let mut cols = header_cells.as_ref().map_or(0, Vec::len);
            for row in &body_rows {
                cols = cols.max(cells_of(row).len());
            }
            if cols >= 3
                || has_descendant_named(table, &["table"])
                || numeric_density(&body_rows) > 0.5
            {
                Decision::Rasterize
            } else {
                Decision::Linearize("simple table")
            }
        }
    }
}

/// Whether an element node carries a non-empty `id` attribute (an anchor target).
fn has_id(node: &NodeRef) -> bool {
    get_attr(node, "id").is_some_and(|id| !id.trim().is_empty())
}

/// Fraction of non-empty body cells whose text is purely numeric. A cell counts
/// as numeric when, after removing [`NUMERIC_STRIP`] and all whitespace, what
/// remains is non-empty and only ASCII digits. Empty cells are ignored; a table
/// with no non-empty body cell has density 0.
fn numeric_density(body_rows: &[NodeRef]) -> f64 {
    let mut non_empty = 0usize;
    let mut numeric = 0usize;
    for row in body_rows {
        for cell in cells_of(row) {
            let text = text_content(&cell);
            if text.trim().is_empty() {
                continue;
            }
            non_empty += 1;
            let stripped: String = text
                .chars()
                .filter(|c| !c.is_whitespace() && !NUMERIC_STRIP.contains(*c))
                .collect();
            if !stripped.is_empty() && stripped.bytes().all(|b| b.is_ascii_digit()) {
                numeric += 1;
            }
        }
    }
    if non_empty == 0 {
        0.0
    } else {
        numeric as f64 / non_empty as f64
    }
}

/// Collapse a table marked for rasterization into a [`TableModel`]: its caption
/// and header cells stay plain text, while each body cell keeps its own text
/// plus - at the outer level only - one nested table rendered as an inner grid.
///
/// The parent grid plus one nested level render as grids (two grid levels). A
/// table nested a third level deep is collapsed to text with a warning: at 480px
/// a third grid level is unreadable.
pub(crate) fn build_table_model(
    table: &NodeRef,
    chapter_path: &str,
    warnings: &mut Vec<Warning>,
) -> TableModel {
    build_model(table, 0, chapter_path, warnings)
}

/// Collapse `table` at nesting `depth` (0 = the outer table). Body cells render
/// a nested table as an inner grid only when `depth == 0`.
fn build_model(
    table: &NodeRef,
    depth: usize,
    chapter_path: &str,
    warnings: &mut Vec<Warning>,
) -> TableModel {
    let caption = child_elements(table)
        .into_iter()
        .find(|c| is_named(c, "caption"))
        .map(|c| collapse_whitespace(&text_content(&c)))
        .filter(|s| !s.is_empty());
    let all_rows = collect_rows(table);
    let head_rows = header_rows(table);
    let (header_cells, body_rows) = split_header(&all_rows, &head_rows);
    let headers = header_cells.map(|cells| {
        cells
            .iter()
            .map(|c| collapse_whitespace(&text_content(c)))
            .collect()
    });
    let rows = body_rows
        .iter()
        .map(|row| {
            cells_of(row)
                .iter()
                .map(|c| build_cell(c, depth, chapter_path, warnings))
                .collect()
        })
        .collect();
    TableModel {
        caption,
        headers,
        rows,
    }
}

/// Build one body [`Cell`]. When the cell holds a nested table and this is the
/// outer level, the FIRST descendant `<table>` in document order (guaranteed to
/// be the outermost) becomes the cell's inner grid, and the cell's text is
/// everything else in the cell. Any ADDITIONAL nested tables in the same cell
/// stay flattened into the text (a documented limitation). One level deeper a
/// nested table would be a third grid level, so it is collapsed to text with a
/// warning instead.
fn build_cell(
    cell: &NodeRef,
    depth: usize,
    chapter_path: &str,
    warnings: &mut Vec<Warning>,
) -> Cell {
    let inner = cell.descendants().find(|n| is_named(n, "table"));
    match inner {
        Some(inner) if depth == 0 => {
            let sub = build_model(&inner, depth + 1, chapter_path, warnings);
            // An empty nested table (no rows, no headers) contributes no grid;
            // keep the cell as plain text so it never renders a "(nested)"
            // marker for a table with nothing in it.
            if sub.rows.is_empty() && sub.headers.is_none() {
                Cell {
                    text: collapse_whitespace(&text_content(cell)),
                    sub: None,
                }
            } else {
                Cell {
                    text: cell_text_excluding(cell, Some(&inner)),
                    sub: Some(sub),
                }
            }
        }
        Some(_) => {
            warnings.push(Warning {
                message: format!(
                    "a table nested two levels deep in {chapter_path} was collapsed to text"
                ),
                file: Some(chapter_path.to_string()),
            });
            Cell {
                text: collapse_whitespace(&text_content(cell)),
                sub: None,
            }
        }
        None => Cell {
            text: collapse_whitespace(&text_content(cell)),
            sub: None,
        },
    }
}

/// Collapsed text of `cell`'s subtree, skipping exactly the `skip` node's subtree
/// (the nested table chosen to render as an inner grid) when given.
fn cell_text_excluding(cell: &NodeRef, skip: Option<&NodeRef>) -> String {
    let mut buf = String::new();
    collect_text_excluding(cell, skip, &mut buf);
    collapse_whitespace(&buf)
}

fn collect_text_excluding(node: &NodeRef, skip: Option<&NodeRef>, buf: &mut String) {
    if skip == Some(node) {
        return;
    }
    if let Some(text) = node.as_text() {
        buf.push_str(&text.borrow());
    } else {
        for child in node.children() {
            collect_text_excluding(&child, skip, buf);
        }
    }
}

/// Flatten `table` and every table nested inside it to paragraphs, innermost
/// first, so a nested table's content survives as paragraphs in its parent's
/// cell. One `table-linearized` transformation is recorded per table; `reason`,
/// when present (the image modes), is appended to each detail line. This is the
/// single reusable flattening path `convert` also calls as its
/// rasterization-failure fallback.
pub(crate) fn linearize_table_node(
    table: &NodeRef,
    report: &mut Vec<Transformation>,
    chapter_path: &str,
    reason: Option<&str>,
) {
    loop {
        let innermost: Vec<NodeRef> = table
            .descendants()
            .filter(|n| is_named(n, "table") && !has_descendant_named(n, &["table"]))
            .collect();
        if innermost.is_empty() {
            break;
        }
        for nested in innermost {
            flatten_one(&nested, report, chapter_path, reason);
        }
    }
    flatten_one(table, report, chapter_path, reason);
}

fn flatten_one(
    table: &NodeRef,
    report: &mut Vec<Transformation>,
    chapter_path: &str,
    reason: Option<&str>,
) {
    if table.parent().is_none() {
        return;
    }
    let caption = child_elements(table)
        .into_iter()
        .find(|c| is_named(c, "caption"));
    let all_rows = collect_rows(table);
    let head_rows = header_rows(table);
    let (header_cells, body_rows) = split_header(&all_rows, &head_rows);

    let mut cols = header_cells.as_ref().map_or(0, Vec::len);
    for row in &body_rows {
        cols = cols.max(cells_of(row).len());
    }

    let mut out = Vec::new();
    if let Some(caption) = caption {
        let strong = element("strong", &[]);
        move_children(&caption, &strong);
        let paragraph = element("p", &[("class", "et-table-caption")]);
        paragraph.append(strong);
        out.push(paragraph);
    }

    if cols <= 1 {
        for row in &all_rows {
            for cell in cells_of(row) {
                let paragraph = element("p", &[]);
                let extras = distribute_cell(&cell, &paragraph);
                out.push(paragraph);
                out.extend(extras);
            }
        }
    } else if let Some(headers) = &header_cells {
        let labels: Vec<String> = headers
            .iter()
            .map(|h| collapse_whitespace(&text_content(h)))
            .collect();
        let separated = body_rows.len() >= 2 && cols >= 2;
        for (ri, row) in body_rows.iter().enumerate() {
            for (ci, cell) in cells_of(row).iter().enumerate() {
                let paragraph = element("p", &[("class", "et-table-cell")]);
                if let Some(label) = labels.get(ci).filter(|l| !l.is_empty()) {
                    let strong = element("strong", &[]);
                    strong.append(text(&format!("{label}:")));
                    paragraph.append(strong);
                    paragraph.append(text(" "));
                }
                let extras = distribute_cell(cell, &paragraph);
                out.push(paragraph);
                out.extend(extras);
            }
            if separated && ri + 1 < body_rows.len() {
                let sep = element("p", &[("class", "et-table-row-sep")]);
                sep.append(text(&EM_DASH.to_string()));
                out.push(sep);
            }
        }
    } else {
        for row in &body_rows {
            let cells: Vec<NodeRef> = cells_of(row).into_iter().filter(cell_has_content).collect();
            if cells.is_empty() {
                continue;
            }
            let paragraph = element("p", &[("class", "et-table-row")]);
            for (i, cell) in cells.iter().enumerate() {
                if i > 0 {
                    paragraph.append(text(&format!(" {EM_DASH} ")));
                }
                append_flattened_inline(cell, &paragraph);
            }
            out.push(paragraph);
        }
    }

    let rows = all_rows.len();
    replace_with(table, out);
    let detail = match reason {
        Some(reason) => format!("linearized a table ({rows} rows x {cols} cols) - {reason}"),
        None => format!("linearized a table ({rows} rows x {cols} cols)"),
    };
    report.push(Transformation {
        kind: "table-linearized".to_string(),
        detail,
        file: Some(chapter_path.to_string()),
    });
}

/// All `<tr>` in the table, flattening `<thead>`/`<tbody>`/`<tfoot>` groups but
/// not descending into nested tables: only the outer table's own rows. (In the
/// text path a nested table is already flattened by this point; in the image
/// modes it is still nested and renders inside the same image, so its rows are
/// counted separately by `table_decision`.)
fn collect_rows(table: &NodeRef) -> Vec<NodeRef> {
    let mut rows = Vec::new();
    for child in child_elements(table) {
        if is_named(&child, "tr") {
            rows.push(child);
        } else if is_named(&child, "thead")
            || is_named(&child, "tbody")
            || is_named(&child, "tfoot")
        {
            for row in child_elements(&child) {
                if is_named(&row, "tr") {
                    rows.push(row);
                }
            }
        }
    }
    rows
}

fn header_rows(table: &NodeRef) -> Vec<NodeRef> {
    child_elements(table)
        .into_iter()
        .find(|c| is_named(c, "thead"))
        .map(|thead| {
            child_elements(&thead)
                .into_iter()
                .filter(|r| is_named(r, "tr"))
                .collect()
        })
        .unwrap_or_default()
}

/// Split rows into (optional header cells, body rows): the `<thead>`'s first
/// row if present, else the first row when every one of its cells is a `<th>`.
fn split_header(
    all_rows: &[NodeRef],
    head_rows: &[NodeRef],
) -> (Option<Vec<NodeRef>>, Vec<NodeRef>) {
    let body_excluding_head: Vec<NodeRef> = all_rows
        .iter()
        .filter(|r| !head_rows.iter().any(|h| h == *r))
        .cloned()
        .collect();
    if let Some(first_head) = head_rows.first() {
        (Some(cells_of(first_head)), body_excluding_head)
    } else if body_excluding_head.first().is_some_and(all_cells_th) {
        let mut body = body_excluding_head;
        let header = body.remove(0);
        (Some(cells_of(&header)), body)
    } else {
        (None, body_excluding_head)
    }
}

fn cells_of(row: &NodeRef) -> Vec<NodeRef> {
    child_elements(row)
        .into_iter()
        .filter(|c| is_named(c, "td") || is_named(c, "th"))
        .collect()
}

fn all_cells_th(row: &NodeRef) -> bool {
    let cells = cells_of(row);
    !cells.is_empty() && cells.iter().all(|c| is_named(c, "th"))
}

/// Whether a cell has anything worth keeping: non-whitespace text anywhere in
/// its subtree, or any element child at all. Text-only emptiness checks alone
/// would drop a cell holding nothing but an `<img>` - real content with no
/// text.
fn cell_has_content(cell: &NodeRef) -> bool {
    !text_content(cell).trim().is_empty()
        || cell
            .children()
            .any(|c| matches!(c.data(), NodeData::Element(_)))
}

/// Move a cell's inline content into `target`, returning its block children
/// (kept as-is) to splice as sibling blocks after the labeled paragraph.
fn distribute_cell(cell: &NodeRef, target: &NodeRef) -> Vec<NodeRef> {
    let mut extras = Vec::new();
    for child in cell.children() {
        if is_cell_block(&child) {
            extras.push(child);
        } else {
            target.append(child);
        }
    }
    extras
}

/// Move a cell's inline content into `target`, flattening one level of block
/// children (used for the header-less row form, which must stay one paragraph).
fn append_flattened_inline(cell: &NodeRef, target: &NodeRef) {
    for child in cell.children() {
        if is_cell_block(&child) {
            move_children(&child, target);
        } else {
            target.append(child);
        }
    }
}

fn is_cell_block(node: &NodeRef) -> bool {
    matches!(node.data(), NodeData::Element(e) if CELL_BLOCK.contains(&e.name.local.as_ref()))
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::testutil::{doc_from_body, serialize};

    fn run(body: &str) -> (String, Vec<Transformation>) {
        let doc = doc_from_body(body);
        let mut report = Vec::new();
        linearize_tables(&doc, &ConvertOptions::default(), &mut report, "ch.xhtml");
        (serialize(&doc), report)
    }

    #[test]
    fn thead_table_snapshot() {
        let (out, report) = run(
            "<table><caption>Prices</caption><thead><tr><th>Item</th><th>Cost</th></tr></thead>\
             <tbody><tr><td>Pen</td><td>1</td></tr><tr><td>Ink</td><td>2</td></tr></tbody></table>",
        );
        insta::assert_snapshot!(out);
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].kind, "table-linearized");
        assert!(!out.contains("<table"), "table must be gone");
    }

    #[test]
    fn implicit_header_first_row_all_th_snapshot() {
        let (out, _) =
            run("<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn headerless_row_snapshot() {
        let (out, _) = run(
            "<table><tr><td>a</td><td>b</td><td>c</td></tr><tr><td>d</td><td></td><td>f</td></tr></table>",
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn single_column_unwraps_snapshot() {
        let (out, _) = run("<table><tr><td>only</td></tr><tr><td>rows</td></tr></table>");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn nested_table_flattened_innermost_first_snapshot() {
        let (out, report) = run("<table><tr><th>Outer</th></tr>\
             <tr><td><table><tr><td>inner</td></tr></table></td></tr></table>");
        insta::assert_snapshot!(out);
        assert_eq!(report.len(), 2, "one transformation per table");
        assert!(!out.contains("<table"), "no tables should remain");
    }

    #[test]
    fn inline_markup_in_cells_survives() {
        let (out, _) = run(
            "<table><thead><tr><th>H</th></tr></thead><tbody><tr><td>x <b>bold</b></td></tr></tbody></table>",
        );
        // Single column path -> plain <p>; markup preserved.
        assert!(out.contains("<b>bold</b>"), "got: {out}");
    }

    #[test]
    fn headerless_row_image_only_cell_survives() {
        // No <thead> and the first row is not all-<th>, so this takes the
        // headerless-row path (cols >= 2). A cell holding only an <img> has
        // no text content but must not be dropped as empty.
        let (out, _) = run(
            r#"<table><tr><td>a</td><td>b</td></tr><tr><td><img src="x.png" alt="x"/></td><td>d</td></tr></table>"#,
        );
        assert!(
            out.contains("<img"),
            "an image-only cell must survive: {out}"
        );
    }

    // --- Model builder (`build_table_model`) -----------------------------

    /// Build the model for the first (top-level) `<table>` in `body`, along with
    /// any warnings it raised.
    fn model_and_warnings(body: &str) -> (TableModel, Vec<Warning>) {
        let doc = doc_from_body(body);
        let table = collect_by_name(&doc, "table")
            .into_iter()
            .next()
            .expect("a table");
        let mut warnings = Vec::new();
        let model = build_table_model(&table, "ch.xhtml", &mut warnings);
        (model, warnings)
    }

    #[test]
    fn nested_table_becomes_a_sub_grid_not_merged_text() {
        let (model, warnings) = model_and_warnings(
            "<table><tr><td>intro<table><thead><tr><th>K</th></tr></thead>\
             <tbody><tr><td>v</td></tr></tbody></table></td></tr></table>",
        );
        let cell = &model.rows[0][0];
        assert_eq!(
            cell.text, "intro",
            "the parent cell keeps only its own text"
        );
        let sub = cell
            .sub
            .as_ref()
            .expect("the nested table becomes a sub-grid");
        assert_eq!(
            sub.headers.as_deref(),
            Some(["K".to_string()].as_slice()),
            "the nested header survives"
        );
        assert_eq!(sub.rows.len(), 1, "the nested body has one row");
        assert_eq!(
            sub.rows[0][0].text, "v",
            "the nested body cell text survives"
        );
        assert!(
            warnings.is_empty(),
            "one nested level warns nothing: {warnings:?}"
        );
    }

    #[test]
    fn third_level_table_collapses_to_text_with_a_warning() {
        // Outer -> second level (grid) -> third level (collapses to text).
        let (model, warnings) = model_and_warnings(
            "<table><tr><td>a<table><tr><td>b<table><tr><td>c</td></tr></table>\
             </td></tr></table></td></tr></table>",
        );
        let sub = model.rows[0][0]
            .sub
            .as_ref()
            .expect("the second level renders as a grid");
        assert_eq!(
            sub.rows[0][0].text, "bc",
            "the third-level table flattens into its cell's text"
        );
        assert!(sub.rows[0][0].sub.is_none(), "no third grid level");
        assert_eq!(
            warnings.len(),
            1,
            "exactly one collapse warning: {warnings:?}"
        );
        assert_eq!(
            warnings[0].message,
            "a table nested two levels deep in ch.xhtml was collapsed to text"
        );
    }

    #[test]
    fn empty_nested_table_leaves_the_cell_plain_text_not_a_sub_grid() {
        // A cell holding an empty <table></table> plus text: the empty table
        // has no rows and no headers, so it must not become a sub-grid - which
        // would wrongly tag the rendered detail "(nested)" for a table with
        // nothing in it.
        let (model, warnings) =
            model_and_warnings("<table><tr><td>text<table></table></td></tr></table>");
        let cell = &model.rows[0][0];
        assert!(
            cell.sub.is_none(),
            "an empty nested table must not become a sub-grid"
        );
        assert!(
            cell.text.contains("text"),
            "the cell's own text survives: {:?}",
            cell.text
        );
        assert!(
            warnings.is_empty(),
            "an empty nested table warns nothing: {warnings:?}"
        );
    }

    #[test]
    fn two_nested_tables_in_one_cell_keep_first_as_grid_rest_as_text() {
        let (model, _) = model_and_warnings(
            "<table><tr><td>lead<table><tr><td>first</td></tr></table>\
             mid<table><tr><td>second</td></tr></table></td></tr></table>",
        );
        let cell = &model.rows[0][0];
        let sub = cell
            .sub
            .as_ref()
            .expect("the first nested table becomes the grid");
        assert_eq!(
            sub.rows[0][0].text, "first",
            "the first nested table is the inner grid"
        );
        assert!(
            cell.text.contains("second"),
            "the second nested table survives as text: {:?}",
            cell.text
        );
        assert!(
            cell.text.contains("lead") && cell.text.contains("mid"),
            "the cell's own text survives: {:?}",
            cell.text
        );
    }

    // --- Heuristic (`table_decision`) matrix -----------------------------

    /// Decide the fate of the first (top-level) `<table>` in `body`.
    fn decide(body: &str, mode: TableMode) -> Decision {
        let doc = doc_from_body(body);
        let table = collect_by_name(&doc, "table")
            .into_iter()
            .next()
            .expect("a table");
        table_decision(&table, mode)
    }

    #[test]
    fn three_column_table_rasterizes_under_image() {
        let d = decide(
            "<table><tr><th>A</th><th>B</th><th>C</th></tr>\
             <tr><td>x</td><td>y</td><td>z</td></tr></table>",
            TableMode::Image,
        );
        assert_eq!(d, Decision::Rasterize);
    }

    #[test]
    fn two_column_simple_table_linearizes_under_image() {
        let d = decide(
            "<table><tr><td>foo</td><td>bar</td></tr>\
             <tr><td>baz</td><td>qux</td></tr></table>",
            TableMode::Image,
        );
        assert_eq!(d, Decision::Linearize("simple table"));
    }

    #[test]
    fn nested_table_rasterizes_under_image() {
        let d = decide(
            "<table><tr><td>a</td><td><table><tr><td>x</td></tr></table></td></tr></table>",
            TableMode::Image,
        );
        assert_eq!(d, Decision::Rasterize);
    }

    #[test]
    fn numeric_dense_two_column_table_rasterizes_under_image() {
        // 4 non-empty body cells, 3 of them numeric -> density 0.75 > 0.5.
        let d = decide(
            "<table><tr><td>1</td><td>2</td></tr><tr><td>3</td><td>x</td></tr></table>",
            TableMode::Image,
        );
        assert_eq!(d, Decision::Rasterize);
    }

    #[test]
    fn currency_and_percent_cells_count_as_numeric() {
        // Stripping `$ , . %` leaves ASCII digits -> numeric; density 1.0.
        let d = decide(
            "<table><tr><td>$1,200.50</td><td>13%</td></tr></table>",
            TableMode::Image,
        );
        assert_eq!(d, Decision::Rasterize);
    }

    #[test]
    fn id_in_a_cell_forces_linearize_under_image_and_image_all() {
        let body = r#"<table><tr><td id="anchor">a</td><td>b</td></tr></table>"#;
        for mode in [TableMode::Image, TableMode::ImageAll] {
            assert_eq!(
                decide(body, mode),
                Decision::Linearize("table has anchor targets"),
                "id must force linearize under {mode:?}"
            );
        }
    }

    #[test]
    fn link_in_a_cell_forces_linearize() {
        let body = r#"<table><tr><td><a href="x.html">link</a></td><td>b</td></tr></table>"#;
        for mode in [TableMode::Image, TableMode::ImageAll] {
            assert_eq!(
                decide(body, mode),
                Decision::Linearize("table contains links"),
                "a link must force linearize under {mode:?}"
            );
        }
    }

    #[test]
    fn image_in_a_cell_forces_linearize() {
        let body = r#"<table><tr><td><img src="x.png"/></td><td>b</td></tr></table>"#;
        for mode in [TableMode::Image, TableMode::ImageAll] {
            assert_eq!(
                decide(body, mode),
                Decision::Linearize("table contains images"),
                "an image must force linearize under {mode:?}"
            );
        }
    }

    #[test]
    fn tall_table_over_twenty_four_body_rows_forces_linearize() {
        let mut body = String::from("<table>");
        for _ in 0..25 {
            body.push_str("<tr><td>x</td></tr>");
        }
        body.push_str("</table>");
        assert_eq!(
            decide(&body, TableMode::ImageAll),
            Decision::Linearize("table too tall to rasterize"),
            "25 body rows exceeds the 24-row rasterization cap"
        );
    }

    #[test]
    fn header_plus_exactly_twenty_four_body_rows_rasterizes() {
        // A header row plus exactly 24 body rows is 24 BODY rows: right at the
        // cap, so it must still rasterize. RED before the fix: the cap counted
        // the header too (25 > 24) and linearized this.
        let mut body = String::from("<table><thead><tr><th>H</th></tr></thead><tbody>");
        for _ in 0..24 {
            body.push_str("<tr><td>x</td></tr>");
        }
        body.push_str("</tbody></table>");
        assert_eq!(
            decide(&body, TableMode::ImageAll),
            Decision::Rasterize,
            "24 body rows plus a header is exactly at the cap and must rasterize"
        );
    }

    #[test]
    fn header_plus_twenty_five_body_rows_linearizes() {
        // One body row past the cap (25 body rows), header excluded from the
        // count, so it linearizes.
        let mut body = String::from("<table><thead><tr><th>H</th></tr></thead><tbody>");
        for _ in 0..25 {
            body.push_str("<tr><td>x</td></tr>");
        }
        body.push_str("</tbody></table>");
        assert_eq!(
            decide(&body, TableMode::ImageAll),
            Decision::Linearize("table too tall to rasterize"),
            "25 body rows exceeds the cap even with the header excluded"
        );
    }

    #[test]
    fn nested_rows_count_toward_the_rasterization_cap() {
        // 5 parent body rows, each holding a 5-row nested table = 30 effective
        // rows, over the 24-row cap even under ImageAll (which otherwise always
        // rasterizes).
        let mut body = String::from("<table>");
        for _ in 0..5 {
            body.push_str("<tr><td><table>");
            for _ in 0..5 {
                body.push_str("<tr><td>x</td></tr>");
            }
            body.push_str("</table></td></tr>");
        }
        body.push_str("</table>");
        assert_eq!(
            decide(&body, TableMode::ImageAll),
            Decision::Linearize("table too tall to rasterize"),
            "30 effective rows exceeds the 24-row cap"
        );
    }

    #[test]
    fn numeric_density_exactly_one_half_is_not_dense_enough() {
        // 2 non-empty body cells, 1 numeric -> density 0.5, which is NOT > 0.5.
        let d = decide(
            "<table><tr><td>42</td><td>hello</td></tr></table>",
            TableMode::Image,
        );
        assert_eq!(d, Decision::Linearize("simple table"));
    }

    #[test]
    fn image_all_rasterizes_a_plain_simple_table() {
        let d = decide(
            "<table><tr><td>foo</td><td>bar</td></tr></table>",
            TableMode::ImageAll,
        );
        assert_eq!(d, Decision::Rasterize);
    }
}
