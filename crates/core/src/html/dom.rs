//! Shared kuchikiki DOM helpers used by the HTML transforms.
//!
//! kuchikiki does not re-export the `html5ever`/`markup5ever` name types
//! ([`QualName`], [`LocalName`], [`Namespace`]) that its own
//! [`NodeRef::new_element`] constructor requires, so element construction goes
//! through [`element`] here, which pins every generated element to the XHTML
//! namespace (matching what the parser produces, so later selector-free tree
//! walks by local name behave uniformly).

use html5ever::{LocalName, Namespace, QualName};
use kuchikiki::{Attribute, ExpandedName, NodeData, NodeRef};

/// The XHTML namespace every generated element is placed in.
const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";

/// Build an XHTML-namespaced qualified name for the element `local`.
fn qual(local: &str) -> QualName {
    QualName::new(None, Namespace::from(XHTML_NS), LocalName::from(local))
}

/// Create a new `<local ...attrs>` element node in the XHTML namespace.
///
/// Attributes are stored in the null namespace with no prefix, exactly as the
/// HTML parser stores plain attributes, so the serializer round-trips them.
pub(crate) fn element(local: &str, attrs: &[(&str, &str)]) -> NodeRef {
    let attributes = attrs.iter().map(|(name, value)| {
        (
            ExpandedName::new(Namespace::from(""), *name),
            Attribute {
                prefix: None,
                value: (*value).to_string(),
            },
        )
    });
    NodeRef::new_element(qual(local), attributes)
}

/// Create a new text node.
pub(crate) fn text(value: &str) -> NodeRef {
    NodeRef::new_text(value)
}

/// The element's local (tag) name, or `None` for non-element nodes.
pub(crate) fn local_name(node: &NodeRef) -> Option<String> {
    match node.data() {
        NodeData::Element(e) => Some(e.name.local.as_ref().to_string()),
        _ => None,
    }
}

/// Whether `node` is an element with the given local name.
pub(crate) fn is_named(node: &NodeRef, name: &str) -> bool {
    matches!(node.data(), NodeData::Element(e) if e.name.local.as_ref() == name)
}

/// Get an attribute value (null namespace) on an element node.
pub(crate) fn get_attr(node: &NodeRef, name: &str) -> Option<String> {
    match node.data() {
        NodeData::Element(e) => e.attributes.borrow().get(name).map(str::to_string),
        _ => None,
    }
}

/// Set (or replace) an attribute value on an element node.
pub(crate) fn set_attr(node: &NodeRef, name: &str, value: &str) {
    if let NodeData::Element(e) = node.data() {
        e.attributes.borrow_mut().insert(name, value.to_string());
    }
}

/// Remove an attribute from an element node, returning whether it was present.
pub(crate) fn remove_attr(node: &NodeRef, name: &str) -> bool {
    match node.data() {
        NodeData::Element(e) => e.attributes.borrow_mut().remove(name).is_some(),
        _ => false,
    }
}

/// The direct element children of `node`, collected (so the caller can mutate
/// the tree while iterating the snapshot).
pub(crate) fn child_elements(node: &NodeRef) -> Vec<NodeRef> {
    node.children()
        .filter(|c| matches!(c.data(), NodeData::Element(_)))
        .collect()
}

/// Move every child of `from` to the end of `to`, preserving order.
pub(crate) fn move_children(from: &NodeRef, to: &NodeRef) {
    let mut child = from.first_child();
    while let Some(c) = child {
        let next = c.next_sibling();
        to.append(c);
        child = next;
    }
}

/// Replace `node` in the tree with `replacements` (inserted in order in its
/// place), then detach `node`. A no-op position-wise if `replacements` is
/// empty, which simply removes `node`.
pub(crate) fn replace_with(node: &NodeRef, replacements: Vec<NodeRef>) {
    for replacement in replacements {
        node.insert_before(replacement);
    }
    node.detach();
}

/// Splice `node`'s children into its position and drop the wrapper element.
pub(crate) fn unwrap_element(node: &NodeRef) {
    let mut child = node.first_child();
    while let Some(c) = child {
        let next = c.next_sibling();
        node.insert_before(c);
        child = next;
    }
    node.detach();
}

/// The concatenated text of `node`'s subtree.
pub(crate) fn text_content(node: &NodeRef) -> String {
    node.text_contents()
}

/// Whether `node`'s subtree contains an element with any of the given local
/// names (not counting `node` itself).
pub(crate) fn has_descendant_named(node: &NodeRef, names: &[&str]) -> bool {
    node.descendants()
        .any(|d| local_name(&d).is_some_and(|n| names.contains(&n.as_str())))
}

/// Find the `<head>` element of a parsed document, if any.
pub(crate) fn find_head(doc: &NodeRef) -> Option<NodeRef> {
    doc.inclusive_descendants().find(|n| is_named(n, "head"))
}

/// Find the `<body>` element of a parsed document, if any.
pub(crate) fn find_body(doc: &NodeRef) -> Option<NodeRef> {
    doc.inclusive_descendants().find(|n| is_named(n, "body"))
}

/// Collect every element in `doc`'s subtree with the given local name, in
/// document order.
pub(crate) fn collect_by_name(doc: &NodeRef, name: &str) -> Vec<NodeRef> {
    doc.inclusive_descendants()
        .filter(|n| is_named(n, name))
        .collect()
}
