//! Content filter rules: profile-defined string replacements and removals
//! applied to chapter text (and link targets) before any device transform,
//! plus the cascade pruning that removes elements a deletion emptied.

mod prune;

use kuchikiki::NodeRef;
use serde::{Deserialize, Serialize};

use crate::epub::{Book, Metadata, TocEntry};
use crate::html::dom::{collect_by_name, get_attr, local_name, set_attr};
use crate::report::{Transformation, Warning};

/// What a [`FilterRule`] does with its matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterAction {
    /// Delete every occurrence, then prune elements the deletion emptied.
    Remove,
    /// Replace every occurrence with [`FilterRule::with`].
    Replace,
}

/// Where a [`FilterRule`] looks for its pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterTarget {
    /// Text nodes (and book metadata strings).
    Text,
    /// `<a href>` values; a `remove` match detaches the whole anchor.
    Href,
    /// Resource paths inside the archive; a `remove` match drops the whole
    /// file (vendor marker files, watermark images). Spine documents and the
    /// package/navigation documents are protected.
    File,
}

/// One profile-defined content filter rule.
///
/// Matching is plain case-sensitive substring search; `in` defaults to
/// `["text"]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilterRule {
    pub action: FilterAction,
    /// The substring to search for.
    #[serde(rename = "match")]
    pub pattern: String,
    /// The replacement text (`replace` only; ignored by `remove`).
    #[serde(default)]
    pub with: Option<String>,
    /// Where to search.
    #[serde(rename = "in", default = "default_targets")]
    pub targets: Vec<FilterTarget>,
}

fn default_targets() -> Vec<FilterTarget> {
    vec![FilterTarget::Text]
}

impl FilterRule {
    /// Whether this rule applies to text nodes.
    pub fn targets_text(&self) -> bool {
        self.targets.contains(&FilterTarget::Text)
    }

    /// Whether this rule applies to link hrefs.
    pub fn targets_href(&self) -> bool {
        self.targets.contains(&FilterTarget::Href)
    }

    /// Whether this rule applies to archive resource paths.
    pub fn targets_file(&self) -> bool {
        self.targets.contains(&FilterTarget::File)
    }
}

/// Apply every filter rule to one chapter document, in place and in order.
/// Called before any device transform, so rules match the source structure a
/// profile author inspected.
pub(crate) fn apply_chapter_filters(
    doc: &NodeRef,
    rules: &[FilterRule],
    transformations: &mut Vec<Transformation>,
    chapter_path: &str,
) {
    for rule in rules {
        match rule.action {
            FilterAction::Replace => replace_in_chapter(doc, rule, transformations, chapter_path),
            FilterAction::Remove => remove_in_chapter(doc, rule, transformations, chapter_path),
        }
    }
}

/// Replace every occurrence of the rule's pattern in the targeted places.
fn replace_in_chapter(
    doc: &NodeRef,
    rule: &FilterRule,
    transformations: &mut Vec<Transformation>,
    chapter_path: &str,
) {
    let with = rule.with.as_deref().unwrap_or("");
    if rule.targets_text() {
        for node in prose_text_nodes(doc) {
            let text = node.as_text().expect("prose_text_nodes yields text");
            let current = text.borrow().clone();
            if !current.contains(&rule.pattern) {
                continue;
            }
            let occurrences = current.matches(&rule.pattern).count();
            *text.borrow_mut() = current.replace(&rule.pattern, with);
            transformations.push(Transformation {
                kind: "filter-replaced".to_string(),
                detail: format!(
                    "replaced {occurrences} occurrence(s) of \"{}\"",
                    rule.pattern
                ),
                file: Some(chapter_path.to_string()),
            });
        }
    }
    if rule.targets_href() {
        for anchor in collect_by_name(doc, "a") {
            let Some(href) = get_attr(&anchor, "href") else {
                continue;
            };
            if !href.contains(&rule.pattern) {
                continue;
            }
            set_attr(&anchor, "href", &href.replace(&rule.pattern, with));
            transformations.push(Transformation {
                kind: "filter-replaced".to_string(),
                detail: format!("rewrote a link containing \"{}\"", rule.pattern),
                file: Some(chapter_path.to_string()),
            });
        }
    }
}

/// Delete every occurrence of the rule's pattern in the targeted places, then
/// cascade-prune whatever the deletions emptied.
fn remove_in_chapter(
    doc: &NodeRef,
    rule: &FilterRule,
    transformations: &mut Vec<Transformation>,
    chapter_path: &str,
) {
    let mut prune_from: Vec<NodeRef> = Vec::new();

    if rule.targets_href() {
        for anchor in collect_by_name(doc, "a") {
            let Some(href) = get_attr(&anchor, "href") else {
                continue;
            };
            if !href.contains(&rule.pattern) {
                continue;
            }
            let parent = anchor.parent();
            anchor.detach();
            transformations.push(Transformation {
                kind: "filter-removed".to_string(),
                detail: format!("removed a link whose target contains \"{}\"", rule.pattern),
                file: Some(chapter_path.to_string()),
            });
            if let Some(parent) = parent {
                prune_from.push(parent);
            }
        }
    }

    if rule.targets_text() {
        for node in prose_text_nodes(doc) {
            let text = node.as_text().expect("prose_text_nodes yields text");
            let current = text.borrow().clone();
            if !current.contains(&rule.pattern) {
                continue;
            }
            let occurrences = current.matches(&rule.pattern).count();
            let cleaned = current.replace(&rule.pattern, "");
            transformations.push(Transformation {
                kind: "filter-removed".to_string(),
                detail: format!(
                    "removed {occurrences} occurrence(s) of \"{}\"",
                    rule.pattern
                ),
                file: Some(chapter_path.to_string()),
            });
            if cleaned.trim().is_empty() {
                let parent = node.parent();
                node.detach();
                if let Some(parent) = parent {
                    prune_from.push(parent);
                }
            } else {
                *text.borrow_mut() = cleaned;
            }
        }
    }

    for start in prune_from {
        prune::prune_upward(&start, transformations, chapter_path);
    }
}

/// Every prose text node in the document: skips `<style>` and `<script>`
/// contents, which are code, not book text.
fn prose_text_nodes(doc: &NodeRef) -> Vec<NodeRef> {
    doc.inclusive_descendants()
        .filter(|node| node.as_text().is_some())
        .filter(|node| {
            node.parent()
                .and_then(|parent| local_name(&parent))
                .is_none_or(|name| name != "style" && name != "script")
        })
        .collect()
}

/// Apply the text-targeting rules to the book's metadata and table of
/// contents. A title (or TOC label) a removal would empty is left untouched,
/// with a warning; an author entry that empties is dropped.
pub(crate) fn apply_metadata_filters(
    metadata: &mut Metadata,
    toc: &mut [TocEntry],
    rules: &[FilterRule],
    transformations: &mut Vec<Transformation>,
    warnings: &mut Vec<Warning>,
) {
    for rule in rules.iter().filter(|rule| rule.targets_text()) {
        let replacement = match rule.action {
            FilterAction::Replace => rule.with.clone().unwrap_or_default(),
            FilterAction::Remove => String::new(),
        };

        if metadata.title.contains(&rule.pattern) {
            let cleaned = metadata.title.replace(&rule.pattern, &replacement);
            if cleaned.trim().is_empty() {
                warnings.push(Warning {
                    message: format!(
                        "filtering \"{}\" would empty the title; left it unchanged",
                        rule.pattern
                    ),
                    file: None,
                });
            } else {
                metadata.title = cleaned;
                transformations.push(Transformation {
                    kind: filter_kind(rule.action),
                    detail: format!("filtered \"{}\" out of the title", rule.pattern),
                    file: None,
                });
            }
        }

        let authors_before = metadata.authors.len();
        for author in &mut metadata.authors {
            if author.contains(&rule.pattern) {
                *author = author.replace(&rule.pattern, &replacement);
                transformations.push(Transformation {
                    kind: filter_kind(rule.action),
                    detail: format!("filtered \"{}\" out of an author entry", rule.pattern),
                    file: None,
                });
            }
        }
        metadata.authors.retain(|author| !author.trim().is_empty());
        let dropped = authors_before - metadata.authors.len();
        if dropped > 0 {
            transformations.push(Transformation {
                kind: "filter-removed".to_string(),
                detail: format!("dropped {dropped} emptied author entr(y/ies)"),
                file: None,
            });
        }

        for entry in toc.iter_mut() {
            if !entry.title.contains(&rule.pattern) {
                continue;
            }
            let cleaned = entry.title.replace(&rule.pattern, &replacement);
            if cleaned.trim().is_empty() {
                warnings.push(Warning {
                    message: format!(
                        "filtering \"{}\" would empty the TOC entry for {}; left it unchanged",
                        rule.pattern, entry.href
                    ),
                    file: Some(entry.href.clone()),
                });
            } else {
                entry.title = cleaned;
                transformations.push(Transformation {
                    kind: filter_kind(rule.action),
                    detail: format!("filtered \"{}\" out of a TOC entry", rule.pattern),
                    file: None,
                });
            }
        }
    }
}

fn filter_kind(action: FilterAction) -> String {
    match action {
        FilterAction::Replace => "filter-replaced".to_string(),
        FilterAction::Remove => "filter-removed".to_string(),
    }
}

/// Drop every resource whose zip path contains a `file`-targeted remove
/// rule's pattern. Spine documents, the package document and the nav/NCX are
/// protected so a reckless pattern cannot break the book; a dropped cover
/// clears `book.cover`.
pub(crate) fn apply_resource_filters(
    book: &mut Book,
    rules: &[FilterRule],
    transformations: &mut Vec<Transformation>,
) {
    let rules: Vec<&FilterRule> = rules
        .iter()
        .filter(|rule| rule.action == FilterAction::Remove && rule.targets_file())
        .collect();
    if rules.is_empty() {
        return;
    }
    let protected: Vec<&str> = book
        .spine
        .iter()
        .map(String::as_str)
        .chain([book.opf_path.as_str()])
        .chain(book.nav_path.as_deref())
        .chain(book.ncx_path.as_deref())
        .collect();
    let doomed: Vec<String> = book
        .resources
        .keys()
        .filter(|path| !protected.contains(&path.as_str()))
        .filter(|path| rules.iter().any(|rule| path.contains(&rule.pattern)))
        .cloned()
        .collect();
    for path in doomed {
        book.resources.shift_remove(&path);
        if book.cover.as_deref() == Some(path.as_str()) {
            book.cover = None;
        }
        transformations.push(Transformation {
            kind: "filter-removed".to_string(),
            detail: format!("dropped the resource {path} matched by a file filter"),
            file: Some(path),
        });
    }
}
